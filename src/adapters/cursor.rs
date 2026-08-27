use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{AdapterError, ClientAdapter};
use crate::state::{ServerConfig, ToolFilter};

#[derive(Debug, Default, Clone)]
pub struct CursorAdapter;

#[derive(Debug, Serialize, Deserialize)]
#[allow(dead_code)]
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
                        name.clone(),
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

        let global_path = self.config_path(None)?;
        self.write_servers(profile, &global_path, &global_servers)?;
        synced_count += 1;

        for (ws_path, servers) in project_servers_by_path {
            let proj_path = self.config_path(Some(&ws_path))?;
            self.write_servers(profile, &proj_path, &servers)?;
            synced_count += 1;
        }

        Ok(synced_count)
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
                    auto_start: false,
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
            assert_eq!(command.as_deref(), Some("docker"));
            assert!(args.contains(&"node:22-alpine".to_string()));
        } else {
            panic!("Expected Local server config");
        }

        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn test_cursor_preserves_unmanaged_keys_and_diff_extract() {
        let adapter = CursorAdapter::default();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("tailery_test_cursor_preserve_{}", nanos));
        let _ = std::fs::create_dir_all(&temp_dir);
        let path = temp_dir.join("mcp.json");

        // Seed with existing user settings
        let seed = json!({
            "customSetting": "keep_me",
            "cursorFeatureEnabled": true,
            "mcpServers": {
                "old_server": {
                    "url": "http://old.test"
                }
            }
        });
        std::fs::write(&path, serde_json::to_string_pretty(&seed).unwrap()).unwrap();

        // Check extract_managed_config only extracts mcpServers
        let managed = adapter.extract_managed_config(&path, &seed);
        assert!(managed.get("mcpServers").is_some());
        assert!(managed.get("customSetting").is_none());
        assert!(managed.get("cursorFeatureEnabled").is_none());

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
        assert_eq!(updated["customSetting"], "keep_me");
        assert_eq!(updated["cursorFeatureEnabled"], true);
        assert!(updated["mcpServers"]["new_server"].is_object());
        assert!(updated["mcpServers"]["old_server"].is_object());

        let _ = std::fs::remove_dir_all(temp_dir);
    }
}
