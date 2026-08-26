use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{AdapterError, ClientAdapter};
use crate::state::{ServerConfig, ToolFilter};

#[derive(Debug, Default, Clone)]
pub struct CursorAdapter;

#[derive(Debug, Serialize, Deserialize)]
struct CursorMcpFile {
    #[serde(rename = "mcpServers", default)]
    mcp_servers: HashMap<String, Value>,
}

impl ClientAdapter for CursorAdapter {
    fn name(&self) -> &'static str {
        "cursor"
    }

    fn display_name(&self) -> &'static str {
        "Cursor"
    }

    fn config_path(&self, workspace: Option<&Path>) -> Result<PathBuf, AdapterError> {
        if let Some(ws) = workspace {
            Ok(ws.join(".cursor").join("mcp.json"))
        } else {
            let home = dirs::home_dir().ok_or_else(|| AdapterError::PathResolution {
                adapter: self.name(),
                message: "Could not resolve user home directory".to_string(),
            })?;
            Ok(home.join(".cursor").join("mcp.json"))
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
                    | crate::state::LocalTransport::Http { port, path } => {
                        let mut obj = serde_json::Map::new();
                        obj.insert(
                            "url".to_string(),
                            Value::String(format!("http://localhost:{}{}", port, path)),
                        );
                        Value::Object(obj)
                    }
                    crate::state::LocalTransport::Sse { port, path } => {
                        let mut obj = serde_json::Map::new();
                        obj.insert(
                            "url".to_string(),
                            Value::String(format!("http://localhost:{}{}", port, path)),
                        );
                        obj.insert("transport".to_string(), Value::String("sse".to_string()));
                        Value::Object(obj)
                    }
                },
                ServerConfig::Remote {
                    url,
                    transport,
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
                    obj.insert("url".to_string(), Value::String(target_url));
                    if !headers.is_empty() {
                        obj.insert("headers".to_string(), json!(headers));
                    }
                    if *transport == crate::state::RemoteTransport::Sse {
                        obj.insert("transport".to_string(), Value::String("sse".to_string()));
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
        let parsed: CursorMcpFile =
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
            } else if let Some(url) = val.get("url").and_then(|v| v.as_str()) {
                let headers = val
                    .get("headers")
                    .and_then(|v| v.as_object())
                    .map(|obj| {
                        obj.iter()
                            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                            .collect()
                    })
                    .unwrap_or_default();
                let is_sse = val.get("transport").and_then(|v| v.as_str()) == Some("sse");
                let transport = if is_sse {
                    crate::state::RemoteTransport::Sse
                } else {
                    crate::state::RemoteTransport::StreamableHttp
                };
                result.insert(
                    name,
                    ServerConfig::Remote {
                        url: url.to_string(),
                        headers,
                        env: HashMap::new(),
                        transport,
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
            home.join(".cursor").exists()
                || home.join("Library/Application Support/Cursor").exists()
                || home.join(".config/Cursor").exists()
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cursor_generate_config() {
        let adapter = CursorAdapter::default();
        let mut servers = HashMap::new();
        servers.insert(
            "test-server".to_string(),
            ServerConfig::Local {
                command: Some("npx".to_string()),
                args: vec![
                    "-y".to_string(),
                    "@modelcontextprotocol/server-filesystem".to_string(),
                ],
                env: HashMap::new(),
                tool_filter: ToolFilter::default(),
                container: crate::state::ContainerConfig::default(),
                transport: crate::state::LocalTransport::Stdio,
            },
        );

        let config = adapter.generate_config(&servers).unwrap();
        assert!(config.get("mcpServers").is_some());
        let mcp_servers = config["mcpServers"].as_object().unwrap();
        assert!(mcp_servers.contains_key("test-server"));
        assert_eq!(mcp_servers["test-server"]["command"], "docker");
        let args = mcp_servers["test-server"]["args"].as_array().unwrap();
        let args_str: Vec<&str> = args.iter().filter_map(|v| v.as_str()).collect();
        assert!(args_str.contains(&"node:22-alpine"));
        assert!(args_str.contains(&"/.tailery/shim"));
    }

    #[test]
    fn test_cursor_generate_config_custom_container() {
        let adapter = CursorAdapter::default();
        let mut servers = HashMap::new();
        servers.insert(
            "sandboxed-db".to_string(),
            ServerConfig::Local {
                command: Some("".to_string()),
                args: vec![],
                container: crate::state::ContainerConfig {
                    image: "mcp/sqlite".to_string(),
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
        let entry = mcp_servers["sandboxed-db"].as_object().unwrap();
        let args: Vec<String> = entry["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();

        assert!(args.contains(&"-l".to_string()));
        assert!(args.contains(&"dev.tailery.managed=true".to_string()));
        assert!(args.contains(&"dev.tailery.server=sandboxed-db".to_string()));
        assert!(args.contains(&"--entrypoint".to_string()));
        assert!(args.contains(&"/.tailery/shim".to_string()));
    }
}

#[cfg(test)]
mod roundtrip_tests {
    use super::*;

    #[test]
    fn test_cursor_read_write_roundtrip() {
        let adapter = CursorAdapter::default();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("tailery_test_cursor_{}", nanos));
        let _ = std::fs::remove_dir_all(&temp_dir);
        let _ = std::fs::create_dir_all(&temp_dir);
        let path = temp_dir.join("mcp.json");

        let mut servers = HashMap::new();
        servers.insert(
            "my-service".to_string(),
            ServerConfig::Local {
                command: Some("node".to_string()),
                args: vec!["index.js".to_string()],
                env: HashMap::from([("ENV_VAR".to_string(), "val".to_string())]),
                tool_filter: ToolFilter::default(),
                container: crate::state::ContainerConfig::default(),
                transport: crate::state::LocalTransport::Stdio,
            },
        );

        adapter.write_servers("default", &path, &servers).unwrap();
        assert!(path.exists());

        let read_back = adapter.read_servers(&path).unwrap();
        assert!(read_back.contains_key("my-service"));
        if let ServerConfig::Local { command, args, .. } = &read_back["my-service"] {
            assert_eq!(command.as_deref(), Some("node"));
            assert!(args.contains(&"index.js".to_string()));
        } else {
            panic!("Expected Local server config");
        }

        let _ = std::fs::remove_dir_all(temp_dir);
    }
}
