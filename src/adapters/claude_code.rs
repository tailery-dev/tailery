use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{AdapterError, ClientAdapter};
use crate::state::{ServerConfig, ToolFilter};

#[derive(Debug, Default, Clone)]
pub struct ClaudeCodeAdapter;

#[derive(Debug, Serialize, Deserialize)]
struct ClaudeMcpFile {
    #[serde(rename = "mcpServers", default)]
    mcp_servers: HashMap<String, Value>,
}

impl ClientAdapter for ClaudeCodeAdapter {
    fn name(&self) -> &'static str {
        "claude_code"
    }

    fn display_name(&self) -> &'static str {
        "Claude Code"
    }

    fn config_path(&self, workspace: Option<&Path>) -> Result<PathBuf, AdapterError> {
        if let Some(ws) = workspace {
            Ok(ws.join(".mcp.json"))
        } else {
            let home = dirs::home_dir().ok_or_else(|| AdapterError::PathResolution {
                adapter: self.name(),
                message: "Could not resolve user home directory".to_string(),
            })?;
            Ok(home.join(".claude.json"))
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
                        obj.insert("type".to_string(), Value::String("sse".to_string()));
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
                        obj.insert("type".to_string(), Value::String("sse".to_string()));
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
        let parsed: ClaudeMcpFile =
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
                let is_sse = val.get("type").and_then(|v| v.as_str()) == Some("sse");
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
            home.join(".claude").exists()
                || home.join(".claude.json").exists()
                || std::env::var_os("CLAUDE_CONFIG_DIR").is_some()
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claude_generate_config() {
        let adapter = ClaudeCodeAdapter::default();
        let mut servers = HashMap::new();
        servers.insert(
            "github-server".to_string(),
            ServerConfig::Local {
                command: Some("npx".to_string()),
                args: vec![
                    "-y".to_string(),
                    "@modelcontextprotocol/server-github".to_string(),
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
        assert!(mcp_servers.contains_key("github-server"));
        assert_eq!(mcp_servers["github-server"]["command"], "docker");
    }

    #[test]
    fn test_claude_generate_config_custom_container() {
        let adapter = ClaudeCodeAdapter::default();
        let mut servers = HashMap::new();
        servers.insert(
            "sandboxed-fs".to_string(),
            ServerConfig::Local {
                command: Some("".to_string()),
                args: vec![],
                container: crate::state::ContainerConfig {
                    image: "mcp/filesystem".to_string(),
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
        let entry = mcp_servers["sandboxed-fs"].as_object().unwrap();
        let args: Vec<String> = entry["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();

        assert!(args.contains(&"-l".to_string()));
        assert!(args.contains(&"dev.tailery.managed=true".to_string()));
        assert!(args.contains(&"dev.tailery.server=sandboxed-fs".to_string()));
        assert!(args.contains(&"--entrypoint".to_string()));
        assert!(args.contains(&"/.tailery/shim".to_string()));
    }
}

#[cfg(test)]
mod roundtrip_tests {
    use super::*;

    #[test]
    fn test_claude_read_write_roundtrip() {
        let adapter = ClaudeCodeAdapter::default();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("tailery_test_claude_{}", nanos));
        let _ = std::fs::remove_dir_all(&temp_dir);
        let _ = std::fs::create_dir_all(&temp_dir);
        let path = temp_dir.join(".mcp.json");

        let mut servers = HashMap::new();
        servers.insert(
            "claude-tool".to_string(),
            ServerConfig::Remote {
                url: "http://localhost:3000".to_string(),
                headers: HashMap::from([("X-Key".to_string(), "abc".to_string())]),
                env: HashMap::new(),
                transport: crate::state::RemoteTransport::StreamableHttp,
            },
        );

        adapter.write_servers("default", &path, &servers).unwrap();
        assert!(path.exists());

        let read_back = adapter.read_servers(&path).unwrap();
        assert!(read_back.contains_key("claude-tool"));
        if let ServerConfig::Remote { url, headers, .. } = &read_back["claude-tool"] {
            assert_eq!(url, "http://localhost:3000");
            assert_eq!(headers.get("X-Key").map(|s| s.as_str()), Some("abc"));
        } else {
            panic!("Expected Remote config");
        }

        let _ = std::fs::remove_dir_all(temp_dir);
    }
}
