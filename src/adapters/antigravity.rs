use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{AdapterError, ClientAdapter};
use crate::state::{ServerConfig, ToolFilter};

#[derive(Debug, Default, Clone)]
pub struct AntigravityAdapter;

#[derive(Debug, Serialize, Deserialize)]
#[allow(dead_code)]
struct AntigravityMcpFile {
    #[serde(rename = "mcpServers", default)]
    mcp_servers: HashMap<String, Value>,
}

impl ClientAdapter for AntigravityAdapter {
    fn name(&self) -> &'static str {
        "antigravity"
    }

    fn display_name(&self) -> &'static str {
        "Google Antigravity"
    }

    fn config_path(&self, workspace: Option<&Path>) -> Result<PathBuf, AdapterError> {
        if let Some(ws) = workspace {
            Ok(ws.join(".agents").join("mcp_config.json"))
        } else {
            let home = dirs::home_dir().ok_or_else(|| AdapterError::PathResolution {
                adapter: self.name(),
                message: "Could not resolve user home directory".to_string(),
            })?;
            Ok(home.join(".gemini").join("config").join("mcp_config.json"))
        }
    }

    fn generate_config(
        &self,
        servers: &HashMap<String, ServerConfig>,
    ) -> Result<Value, AdapterError> {
        let mut mcp_servers = serde_json::Map::new();

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
                        let mut obj = serde_json::Map::new();
                        obj.insert("command".to_string(), Value::String(cmd));
                        if !docker_args.is_empty() {
                            obj.insert("args".to_string(), json!(docker_args));
                        }
                        Value::Object(obj)
                    }
                    crate::state::LocalTransport::StreamableHttp { port, path }
                    | crate::state::LocalTransport::Http { port, path }
                    | crate::state::LocalTransport::Sse { port, path } => {
                        let mut obj = serde_json::Map::new();
                        obj.insert(
                            "serverUrl".to_string(),
                            Value::String(format!("http://localhost:{}{}", port, path)),
                        );
                        Value::Object(obj)
                    }
                },
                ServerConfig::Remote {
                    url,
                    headers,
                    shim_port,
                    ..
                } => {
                    let mut obj = serde_json::Map::new();
                    let target_url = if let Some(port) = shim_port {
                        format!("http://localhost:{}", port)
                    } else {
                        url.clone()
                    };
                    obj.insert("serverUrl".to_string(), Value::String(target_url));
                    if !headers.is_empty() {
                        obj.insert("headers".to_string(), json!(headers));
                    }
                    Value::Object(obj)
                }
            };
            mcp_servers.insert(name.to_string(), server_val);
        }

        let mut root = serde_json::Map::new();
        root.insert("mcpServers".to_string(), Value::Object(mcp_servers));
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
        let root: Value = super::parse_json_relaxed(&content);

        let mut result = HashMap::new();
        if let Some(mcp_servers) = root.get("mcpServers").and_then(|v| v.as_object()) {
            for (name, val) in mcp_servers {
                if let Some(cmd) = val.get("command").and_then(|v| v.as_str()) {
                    let args = val
                        .get("args")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|s| s.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    let env = val
                        .get("env")
                        .and_then(|v| v.as_object())
                        .map(|obj| {
                            obj.iter()
                                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                                .collect()
                        })
                        .unwrap_or_default();
                    result.insert(
                        name.clone(),
                        ServerConfig::Local {
                            command: Some(cmd.to_string()),
                            args,
                            env,
                            tool_filter: ToolFilter::default(),
                            container: crate::state::ContainerConfig::default(),
                            transport: crate::state::LocalTransport::Stdio,
                        },
                    );
                } else if let Some(url) = val
                    .get("serverUrl")
                    .or_else(|| val.get("url"))
                    .and_then(|v| v.as_str())
                {
                    let headers = val
                        .get("headers")
                        .and_then(|v| v.as_object())
                        .map(|obj| {
                            obj.iter()
                                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                                .collect()
                        })
                        .unwrap_or_default();
                    result.insert(
                        name.clone(),
                        ServerConfig::Remote {
                            url: url.to_string(),
                            headers,
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

    fn extract_managed_config(
        &self,
        _path: &Path,
        content_json: &Value,
    ) -> Value {
        let mut managed = serde_json::Map::new();
        let servers = content_json
            .get("mcpServers")
            .cloned()
            .unwrap_or_else(|| json!({}));
        managed.insert("mcpServers".to_string(), servers);
        Value::Object(managed)
    }

    fn merge_managed_config(
        &self,
        _path: &Path,
        existing_json: Option<&Value>,
        servers: &HashMap<String, ServerConfig>,
    ) -> Result<Value, AdapterError> {
        let generated = self.generate_config(servers)?;
        let mut root = match existing_json {
            Some(Value::Object(map)) => Value::Object(map.clone()),
            _ => json!({}),
        };

        if let Some(root_map) = root.as_object_mut() {
            let mut target_servers = if let Some(existing_servers) = root_map.get("mcpServers").and_then(|v| v.as_object()) {
                existing_servers.clone()
            } else {
                serde_json::Map::new()
            };

            let mut keys_to_remove = Vec::new();
            for (k, v) in &target_servers {
                let is_tailery_managed = if servers.contains_key(k) {
                    true
                } else if let Some(cmd) = v.get("command").and_then(|c| c.as_str()) {
                    if cmd == "docker" {
                        if let Some(args) = v.get("args").and_then(|a| a.as_array()) {
                            args.iter().any(|arg| arg.as_str().map_or(false, |s| s.contains("dev.tailery.managed=true") || s.contains("dev.tailery.server=")))
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    false
                };

                if is_tailery_managed && !servers.contains_key(k) {
                    keys_to_remove.push(k.clone());
                }
            }

            for k in keys_to_remove {
                target_servers.remove(&k);
            }

            if let Some(new_servers) = generated.get("mcpServers").and_then(|v| v.as_object()) {
                for (k, v) in new_servers {
                    target_servers.insert(k.clone(), v.clone());
                }
            }

            root_map.insert("mcpServers".to_string(), Value::Object(target_servers));
        }

        Ok(root)
    }

    fn discover_mcps(
        &self,
        path: Option<&Path>,
        state: &crate::state::AppState,
    ) -> Result<Vec<super::DiscoveredMcp>, AdapterError> {
        let Some(p) = path else {
            return Ok(Vec::new());
        };
        if !p.exists() {
            return Ok(Vec::new());
        }
        let servers = self.read_servers(p)?;
        let mut discovered = Vec::new();
        for (name, cfg) in servers {
            let status = super::classify_mcp_status(&name, &cfg, state);
            let transport_label = match &cfg {
                ServerConfig::Local { transport, command, .. } => {
                    if command.as_deref() == Some("docker") {
                        "docker".to_string()
                    } else {
                        match transport {
                            crate::state::LocalTransport::Stdio => "stdio".to_string(),
                            crate::state::LocalTransport::StreamableHttp { .. } => "streamable-http".to_string(),
                            crate::state::LocalTransport::Http { .. } => "http".to_string(),
                            crate::state::LocalTransport::Sse { .. } => "sse".to_string(),
                        }
                    }
                }
                ServerConfig::Remote { transport, .. } => match transport {
                    crate::state::RemoteTransport::StreamableHttp => "streamable-http".to_string(),
                    crate::state::RemoteTransport::Http => "http".to_string(),
                    crate::state::RemoteTransport::Sse => "sse".to_string(),
                },
            };
            discovered.push(super::DiscoveredMcp {
                name,
                scope: super::McpSourceScope::User,
                config: cfg,
                status,
                transport_label,
                raw_json: serde_json::json!({}),
            });
        }
        Ok(discovered)
    }

    fn sync_servers(
        &self,
        profile: &str,
        managed_servers: &HashMap<String, crate::state::ManagedServer>,
    ) -> Result<usize, AdapterError> {
        let mut global_servers = HashMap::new();
        let mut project_servers_by_path: HashMap<PathBuf, HashMap<String, ServerConfig>> = HashMap::new();
        let mut synced_count = 0;

        for (name, srv) in managed_servers {
            if srv.is_global {
                global_servers.insert(name.clone(), srv.config.clone());
            } else {
                for path in &srv.in_repo_paths {
                    project_servers_by_path
                        .entry(path.clone())
                        .or_default()
                        .insert(name.clone(), srv.config.clone());
                }
            }
        }

        // Write global servers
        let global_path = self.config_path(None)?;
        self.write_servers(profile, &global_path, &global_servers)?;
        synced_count += 1;

        // Write project servers
        for (ws_path, servers) in project_servers_by_path {
            let proj_path = self.config_path(Some(&ws_path))?;
            self.write_servers(profile, &proj_path, &servers)?;
            synced_count += 1;
        }

        Ok(synced_count)
    }

    fn detect_installed(&self) -> bool {
        if let Some(home) = dirs::home_dir() {
            if home.join(".gemini").exists()
                || home.join(".gemini/antigravity").exists()
                || home.join(".gemini/config").exists()
                || home.join(".gemini/antigravity-cli").exists()
                || home.join(".gemini/antigravity-ide").exists()
                || home.join(".local/bin/agy").exists()
            {
                return true;
            }

            #[cfg(target_os = "macos")]
            {
                if Path::new("/Applications/Antigravity.app").exists()
                    || Path::new("/Applications/Antigravity IDE.app").exists()
                    || Path::new("/Applications/Google Antigravity.app").exists()
                    || home.join("Applications/Antigravity.app").exists()
                    || home.join("Applications/Antigravity IDE.app").exists()
                    || home.join("Applications/Google Antigravity.app").exists()
                {
                    return true;
                }
            }
        }

        // Check if `agy` binary is found in PATH
        if let Ok(path_var) = std::env::var("PATH") {
            for dir in std::env::split_paths(&path_var) {
                if dir.join("agy").exists() {
                    return true;
                }
            }
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_antigravity_generate_config() {
        let adapter = AntigravityAdapter::default();
        let mut servers = HashMap::new();
        servers.insert(
            "ansible".to_string(),
            ServerConfig::Local {
                command: Some("npx".to_string()),
                args: vec!["-y".to_string(), "@ansible/ansible-mcp-server".to_string()],
                env: HashMap::from([("WORKSPACE_ROOT".to_string(), "/workspace".to_string())]),
                tool_filter: ToolFilter::default(),
                container: crate::state::ContainerConfig::default(),
                transport: crate::state::LocalTransport::Stdio,
            },
        );

        let config = adapter.generate_config(&servers).unwrap();
        assert!(config.get("mcpServers").is_some());
        let mcp_servers = config["mcpServers"].as_object().unwrap();
        assert!(mcp_servers.contains_key("ansible"));
        assert_eq!(mcp_servers["ansible"]["command"], "docker");
    }

    #[test]
    fn test_antigravity_generate_config_custom_container() {
        let adapter = AntigravityAdapter::default();
        let mut servers = HashMap::new();
        servers.insert(
            "sandboxed-postgres".to_string(),
            ServerConfig::Local {
                command: Some("".to_string()),
                args: vec![],
                container: crate::state::ContainerConfig {
                    image: "mcp/postgres".to_string(),
                    read_only_rootfs: true,
                    mounts: vec![],
                    ports: vec![],
                    network: "none".to_string(),
                    resources: None,
                    auto_start: false,
                },
                env: HashMap::new(),
                tool_filter: ToolFilter::default(),
                transport: crate::state::LocalTransport::Stdio,
            },
        );

        let config = adapter.generate_config(&servers).unwrap();
        let mcp_servers = config["mcpServers"].as_object().unwrap();
        let entry = mcp_servers["sandboxed-postgres"].as_object().unwrap();
        let args: Vec<String> = entry["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();

        assert!(args.contains(&"-l".to_string()));
        assert!(args.contains(&"dev.tailery.managed=true".to_string()));
        assert!(args.contains(&"dev.tailery.server=sandboxed-postgres".to_string()));
        assert!(args.contains(&"--entrypoint".to_string()));
        assert!(args.contains(&"/.tailery/shim".to_string()));
    }

    #[test]
    fn test_antigravity_remote_server_urls() {
        let adapter = AntigravityAdapter::default();
        let mut servers = HashMap::new();
        servers.insert(
            "remote-docs".to_string(),
            ServerConfig::Remote {
                url: "https://docs.example.com/mcp".to_string(),
                headers: HashMap::from([(
                    "Authorization".to_string(),
                    "Bearer token123".to_string(),
                )]),
                env: HashMap::new(),
                transport: crate::state::RemoteTransport::StreamableHttp,
                shim_port: None,
                tool_filter: ToolFilter::default(),
            },
        );

        let config = adapter.generate_config(&servers).unwrap();
        let mcp_servers = config["mcpServers"].as_object().unwrap();
        assert_eq!(
            mcp_servers["remote-docs"]["serverUrl"],
            "https://docs.example.com/mcp"
        );
        assert_eq!(
            mcp_servers["remote-docs"]["headers"]["Authorization"],
            "Bearer token123"
        );
    }
}

#[cfg(test)]
mod roundtrip_tests {
    use super::*;

    #[test]
    fn test_antigravity_read_write_roundtrip() {
        let adapter = AntigravityAdapter::default();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("tailery_test_antigravity_{}", nanos));
        let _ = std::fs::remove_dir_all(&temp_dir);
        let _ = std::fs::create_dir_all(&temp_dir);
        let path = temp_dir.join("mcp_config.json");

        let mut servers = HashMap::new();
        servers.insert(
            "custom-tool".to_string(),
            ServerConfig::Local {
                command: Some("python3".to_string()),
                args: vec!["server.py".to_string()],
                env: HashMap::from([("API_KEY".to_string(), "secret".to_string())]),
                tool_filter: ToolFilter::default(),
                container: crate::state::ContainerConfig::default(),
                transport: crate::state::LocalTransport::Stdio,
            },
        );
        servers.insert(
            "remote-tool".to_string(),
            ServerConfig::Remote {
                url: "https://remote.test/sse".to_string(),
                headers: HashMap::from([("X-Custom".to_string(), "val".to_string())]),
                env: HashMap::new(),
                transport: crate::state::RemoteTransport::StreamableHttp,
                shim_port: None,
                tool_filter: ToolFilter::default(),
            },
        );

        adapter.write_servers("default", &path, &servers).unwrap();
        assert!(path.exists());

        let read_back = adapter.read_servers(&path).unwrap();
        assert!(read_back.contains_key("custom-tool"));
        assert!(read_back.contains_key("remote-tool"));

        if let ServerConfig::Local { command, args, .. } = &read_back["custom-tool"] {
            assert_eq!(command.as_deref(), Some("docker"));
            assert!(args.contains(&"python:3.11-alpine".to_string()));
        } else {
            panic!("Expected Local server config");
        }

        if let ServerConfig::Remote { url, headers, .. } = &read_back["remote-tool"] {
            assert_eq!(url, "https://remote.test/sse");
            assert_eq!(headers.get("X-Custom").map(|s| s.as_str()), Some("val"));
        } else {
            panic!("Expected Remote server config");
        }

        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn test_antigravity_preserves_unmanaged_keys_and_diff_extract() {
        let adapter = AntigravityAdapter::default();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("tailery_test_antigravity_preserve_{}", nanos));
        let _ = std::fs::create_dir_all(&temp_dir);
        let path = temp_dir.join("mcp_config.json");

        // Seed with existing user settings
        let seed = json!({
            "model_preferences": { "default": "gemini-1.5-pro" },
            "telemetry_enabled": false,
            "mcpServers": {
                "old_server": {
                    "serverUrl": "http://old.test"
                }
            }
        });
        std::fs::write(&path, serde_json::to_string_pretty(&seed).unwrap()).unwrap();

        // Check extract_managed_config only extracts mcpServers
        let managed = adapter.extract_managed_config(&path, &seed);
        assert!(managed.get("mcpServers").is_some());
        assert!(managed.get("model_preferences").is_none());
        assert!(managed.get("telemetry_enabled").is_none());

        let mut servers = HashMap::new();
        servers.insert(
            "new_server".to_string(),
            ServerConfig::Remote {
                url: "http://new.test".to_string(),
                headers: HashMap::new(),
                env: HashMap::new(),
                transport: crate::state::RemoteTransport::StreamableHttp,
                shim_port: None,
                tool_filter: ToolFilter::default(),
            },
        );

        adapter.write_servers("default", &path, &servers).unwrap();

        let updated_raw = std::fs::read_to_string(&path).unwrap();
        let updated: Value = serde_json::from_str(&updated_raw).unwrap();

        // Verify unmanaged keys are preserved intact!
        assert_eq!(updated["model_preferences"]["default"], "gemini-1.5-pro");
        assert_eq!(updated["telemetry_enabled"], false);
        assert!(updated["mcpServers"]["new_server"].is_object());
        assert!(updated["mcpServers"]["old_server"].is_object());

        let _ = std::fs::remove_dir_all(temp_dir);
    }
}
