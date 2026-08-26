use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{AdapterError, ClientAdapter};
use crate::state::{ServerConfig, ToolFilter};

#[derive(Debug, Default, Clone)]
pub struct AntigravityAdapter;

#[derive(Debug, Serialize, Deserialize)]
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
        let parsed: AntigravityMcpFile =
            serde_json::from_str(&content).map_err(|source| AdapterError::Serialization {
                adapter: self.name(),
                source,
            })?;

        let mut result = HashMap::new();
        for (name, val) in parsed.mcp_servers {
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
                    name,
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
                    name,
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
        Ok(result)
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
}
