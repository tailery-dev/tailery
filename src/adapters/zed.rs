use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{AdapterError, ClientAdapter};
use crate::state::{ServerConfig, ToolFilter};

#[derive(Debug, Default, Clone)]
pub struct ZedAdapter;

impl ClientAdapter for ZedAdapter {
    fn name(&self) -> &'static str {
        "zed"
    }

    fn display_name(&self) -> &'static str {
        "Zed"
    }

    fn config_path(&self, workspace: Option<&Path>) -> Result<PathBuf, AdapterError> {
        if let Some(ws) = workspace {
            Ok(ws.join(".zed").join("settings.json"))
        } else {
            let home = dirs::home_dir().ok_or_else(|| AdapterError::PathResolution {
                adapter: self.name(),
                message: "Could not resolve user home directory".to_string(),
            })?;

            #[cfg(target_os = "macos")]
            let path = home.join("Library/Application Support/Zed/settings.json");

            #[cfg(not(target_os = "macos"))]
            let path = dirs::config_dir()
                .map(|p| p.join("zed").join("settings.json"))
                .unwrap_or_else(|| home.join(".config/zed/settings.json"));

            Ok(path)
        }
    }

    fn generate_config(
        &self,
        servers: &HashMap<String, ServerConfig>,
    ) -> Result<Value, AdapterError> {
        let mut context_servers = serde_json::Map::new();

        for (name, server) in servers {
            let server_val = match server {
                ServerConfig::Local {
                    command,
                    args,
                    env,
                    tool_filter,
                    container,
                    transport,
                } => match transport {
                    crate::state::LocalTransport::Stdio => {
                        let (cmd, docker_args) = super::build_stdio_docker_args(
                            name,
                            command.as_deref(),
                            args,
                            env,
                            container,
                            tool_filter,
                            transport,
                        );
                        let mut cmd_obj = serde_json::Map::new();
                        cmd_obj.insert("path".to_string(), Value::String(cmd));
                        if !docker_args.is_empty() {
                            cmd_obj.insert("args".to_string(), json!(docker_args));
                        }
                        let mut wrapper = serde_json::Map::new();
                        wrapper.insert("command".to_string(), Value::Object(cmd_obj));
                        Value::Object(wrapper)
                    }
                    crate::state::LocalTransport::StreamableHttp { port, path }
                    | crate::state::LocalTransport::Http { port, path }
                    | crate::state::LocalTransport::Sse { port, path } => {
                        let mut wrapper = serde_json::Map::new();
                        wrapper.insert(
                            "endpoint".to_string(),
                            Value::String(format!("http://localhost:{}{}", port, path)),
                        );
                        Value::Object(wrapper)
                    }
                },
                ServerConfig::Remote { url, shim_port, .. } => {
                    let mut wrapper = serde_json::Map::new();
                    let target_url = if let Some(port) = shim_port {
                        format!("http://localhost:{}", port)
                    } else {
                        url.clone()
                    };
                    wrapper.insert("endpoint".to_string(), Value::String(target_url));
                    Value::Object(wrapper)
                }
            };
            context_servers.insert(name.to_string(), server_val);
        }

        let mut root = serde_json::Map::new();
        root.insert(
            "context_servers".to_string(),
            Value::Object(context_servers),
        );
        Ok(Value::Object(root))
    }

    fn read_servers(&self, path: &Path) -> Result<HashMap<String, ServerConfig>, AdapterError> {
        if !path.exists() {
            return Ok(HashMap::new());
        }
        let content = std::fs::read_to_string(path).map_err(|source| AdapterError::Io {
            adapter: self.name(),
            source,
        })?;
        let root: Value =
            serde_json::from_str(&content).map_err(|source| AdapterError::Serialization {
                adapter: self.name(),
                source,
            })?;

        let mut result = HashMap::new();
        if let Some(context_servers) = root.get("context_servers").and_then(|v| v.as_object()) {
            for (name, val) in context_servers {
                if let Some(cmd_val) = val.get("command") {
                    let path_str = cmd_val
                        .get("path")
                        .and_then(|p| p.as_str())
                        .unwrap_or_default();
                    let args = cmd_val
                        .get("args")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|s| s.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    let env = cmd_val
                        .get("env")
                        .and_then(|v| v.as_object())
                        .map(|obj| {
                            obj.iter()
                                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                                .collect()
                        })
                        .unwrap_or_default();
                    result.insert(
                        name.to_string(),
                        ServerConfig::Local {
                            command: Some(path_str.to_string()),
                            args,
                            env,
                            tool_filter: ToolFilter::default(),
                            container: crate::state::ContainerConfig::default(),
                            transport: crate::state::LocalTransport::Stdio,
                        },
                    );
                } else if let Some(endpoint) = val.get("endpoint").and_then(|v| v.as_str()) {
                    result.insert(
                        name.to_string(),
                        ServerConfig::Remote {
                            url: endpoint.to_string(),
                            headers: HashMap::new(),
                            env: HashMap::new(),
                            transport: crate::state::RemoteTransport::StreamableHttp,
                            tool_filter: ToolFilter::default(),
                            shim_port: None,
                        },
                    );
                }
            }
        }
        Ok(result)
    }

    fn write_servers(
        &self,
        profile: &str,
        path: &Path,
        servers: &HashMap<String, ServerConfig>,
    ) -> Result<(), AdapterError> {
        if path.exists() {
            let _ = crate::backup::create_backup(profile, self.name(), path);
        }

        let mut root: Value = if path.exists() {
            let content = std::fs::read_to_string(path).map_err(|source| AdapterError::Io {
                adapter: self.name(),
                source,
            })?;
            serde_json::from_str(&content).unwrap_or_else(|_| json!({}))
        } else {
            json!({})
        };

        let generated = self.generate_config(servers)?;
        if let Some(new_servers) = generated.get("context_servers") {
            if let Some(root_map) = root.as_object_mut() {
                root_map.insert("context_servers".to_string(), new_servers.clone());
            }
        }

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| AdapterError::Io {
                adapter: self.name(),
                source,
            })?;
        }

        let formatted =
            serde_json::to_string_pretty(&root).map_err(|source| AdapterError::Serialization {
                adapter: self.name(),
                source,
            })?;
        std::fs::write(path, formatted).map_err(|source| AdapterError::Io {
            adapter: self.name(),
            source,
        })?;
        Ok(())
    }

    fn detect_installed(&self) -> bool {
        if let Some(home) = dirs::home_dir() {
            #[cfg(target_os = "macos")]
            let zed_config = home.join("Library/Application Support/Zed/settings.json");
            #[cfg(not(target_os = "macos"))]
            let zed_config = home.join(".config/zed/settings.json");

            zed_config.exists()
                || home.join(".zed").exists()
                || Path::new("/Applications/Zed.app").exists()
                || home.join("Applications/Zed.app").exists()
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zed_generate_config() {
        let adapter = ZedAdapter::default();
        let mut servers = HashMap::new();
        servers.insert(
            "zed-mcp".to_string(),
            ServerConfig::Local {
                command: Some("npx".to_string()),
                args: vec!["run".to_string()],
                env: HashMap::new(),
                tool_filter: ToolFilter::default(),
                container: crate::state::ContainerConfig::default(),
                transport: crate::state::LocalTransport::Stdio,
            },
        );

        let config = adapter.generate_config(&servers).unwrap();
        assert!(config.get("context_servers").is_some());
        let ctx = config["context_servers"].as_object().unwrap();
        assert!(ctx.contains_key("zed-mcp"));
        assert_eq!(ctx["zed-mcp"]["command"]["path"], "docker");
    }

    #[test]
    fn test_zed_generate_config_custom_container() {
        let adapter = ZedAdapter::default();
        let mut servers = HashMap::new();
        servers.insert(
            "sandboxed-git".to_string(),
            ServerConfig::Local {
                command: Some("".to_string()),
                args: vec![],
                env: HashMap::new(),
                tool_filter: ToolFilter::default(),
                container: crate::state::ContainerConfig {
                    image: "mcp/git".to_string(),
                    read_only_rootfs: true,
                    mounts: vec![],
                    ports: vec![],
                    network: "none".to_string(),
                    resources: None,
                },
                transport: crate::state::LocalTransport::Stdio,
            },
        );

        let config = adapter.generate_config(&servers).unwrap();
        let ctx = config["context_servers"].as_object().unwrap();
        let entry = ctx["sandboxed-git"].as_object().unwrap();
        let cmd = entry["command"].as_object().unwrap();
        let args: Vec<String> = cmd["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();

        assert!(args.contains(&"-l".to_string()));
        assert!(args.contains(&"dev.tailery.managed=true".to_string()));
        assert!(args.contains(&"dev.tailery.server=sandboxed-git".to_string()));
        assert!(args.contains(&"--entrypoint".to_string()));
        assert!(args.contains(&"/.tailery/shim".to_string()));
    }
}

#[cfg(test)]
mod roundtrip_tests {
    use super::*;

    #[test]
    fn test_zed_read_write_roundtrip() {
        let adapter = ZedAdapter::default();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("tailery_test_zed_{}", nanos));
        let _ = std::fs::create_dir_all(&temp_dir);
        let path = temp_dir.join("settings.json");

        // Seed settings.json with existing user settings
        std::fs::write(&path, r#"{"theme": "One Dark", "context_servers": {}}"#).unwrap();

        let mut servers = HashMap::new();
        servers.insert(
            "custom-zed-server".to_string(),
            ServerConfig::Local {
                command: Some("python3".to_string()),
                args: vec!["main.py".to_string()],
                env: HashMap::new(),
                tool_filter: ToolFilter::default(),
                container: crate::state::ContainerConfig::default(),
                transport: crate::state::LocalTransport::Stdio,
            },
        );

        adapter.write_servers("default", &path, &servers).unwrap();

        let updated_content = std::fs::read_to_string(&path).unwrap();
        let val: Value = serde_json::from_str(&updated_content).unwrap();
        // Check that theme was preserved
        assert_eq!(val["theme"], "One Dark");
        assert!(val["context_servers"]["custom-zed-server"].is_object());

        let read_back = adapter.read_servers(&path).unwrap();
        assert!(read_back.contains_key("custom-zed-server"));

        let _ = std::fs::remove_dir_all(temp_dir);
    }
}
