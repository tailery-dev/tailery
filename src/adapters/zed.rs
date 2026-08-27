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

            #[cfg(target_os = "windows")]
            {
                if let Some(app_data) = std::env::var_os("APPDATA") {
                    return Ok(PathBuf::from(app_data).join("Zed").join("settings.json"));
                }
            }

            // Per Zed MCP spec: macOS uses XDG ~/.config/zed/settings.json with fallback to Library/Application Support
            let xdg_path = home.join(".config").join("zed").join("settings.json");
            #[cfg(target_os = "macos")]
            {
                let legacy_path = home.join("Library/Application Support/Zed/settings.json");
                if !xdg_path.exists() && legacy_path.exists() {
                    return Ok(legacy_path);
                }
            }

            Ok(xdg_path)
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
        let root: Value = super::parse_json_relaxed(&content);

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
                } else if let Some(url_str) = val
                    .get("url")
                    .or_else(|| val.get("endpoint"))
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
                        name.to_string(),
                        ServerConfig::Remote {
                            url: url_str.to_string(),
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

    fn extract_managed_config(&self, _path: &Path, content_json: &Value) -> Value {
        let mut managed = serde_json::Map::new();
        let servers = content_json
            .get("context_servers")
            .cloned()
            .unwrap_or_else(|| json!({}));
        managed.insert("context_servers".to_string(), servers);
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
            let mut target_servers = if let Some(existing_servers) =
                root_map.get("context_servers").and_then(|v| v.as_object())
            {
                existing_servers.clone()
            } else {
                serde_json::Map::new()
            };

            // Remove only Tailery-managed servers that are no longer active
            let mut keys_to_remove = Vec::new();
            for (k, v) in &target_servers {
                let is_tailery_managed = if servers.contains_key(k) {
                    true
                } else if let Some(cmd_obj) = v.get("command").and_then(|c| c.as_object()) {
                    if let Some(args) = cmd_obj.get("args").and_then(|a| a.as_array()) {
                        args.iter().any(|arg| {
                            arg.as_str().is_some_and(|s| {
                                s.contains("dev.tailery.managed=true")
                                    || s.contains("dev.tailery.server=")
                            })
                        })
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

            if let Some(new_servers) = generated.get("context_servers").and_then(|v| v.as_object())
            {
                for (k, v) in new_servers {
                    target_servers.insert(k.clone(), v.clone());
                }
            }

            root_map.insert("context_servers".to_string(), Value::Object(target_servers));
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
                ServerConfig::Local {
                    transport, command, ..
                } => {
                    if command.as_deref() == Some("docker") {
                        "docker".to_string()
                    } else {
                        match transport {
                            crate::state::LocalTransport::Stdio => "stdio".to_string(),
                            crate::state::LocalTransport::StreamableHttp { .. } => {
                                "streamable-http".to_string()
                            }
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
        let mut project_servers_by_path: HashMap<PathBuf, HashMap<String, ServerConfig>> =
            HashMap::new();
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
        let adapter = ZedAdapter;
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
        let adapter = ZedAdapter;
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
                    auto_start: false,
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
        let adapter = ZedAdapter;
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

    #[test]
    fn test_zed_preserves_unmanaged_keys_and_diff_extract() {
        let adapter = ZedAdapter;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("tailery_test_zed_preserve_{}", nanos));
        let _ = std::fs::create_dir_all(&temp_dir);
        let path = temp_dir.join("settings.json");

        // Seed with multiple user settings
        let seed = json!({
            "theme": "Nord",
            "vim_mode": true,
            "buffer_font_size": 16,
            "context_servers": {
                "old_server": {
                    "endpoint": "http://old.test"
                }
            }
        });
        std::fs::write(&path, serde_json::to_string_pretty(&seed).unwrap()).unwrap();

        // Check extract_managed_config only extracts context_servers
        let managed = adapter.extract_managed_config(&path, &seed);
        assert!(managed.get("context_servers").is_some());
        assert!(managed.get("theme").is_none());
        assert!(managed.get("vim_mode").is_none());
        assert!(managed.get("buffer_font_size").is_none());

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
        assert_eq!(updated["theme"], "Nord");
        assert_eq!(updated["vim_mode"], true);
        assert_eq!(updated["buffer_font_size"], 16);
        assert!(updated["context_servers"]["new_server"].is_object());
        assert!(updated["context_servers"]["old_server"].is_object());

        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn test_zed_jsonc_with_comments_and_trailing_commas() {
        let adapter = ZedAdapter;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("tailery_test_zed_jsonc_{}", nanos));
        let _ = std::fs::create_dir_all(&temp_dir);
        let path = temp_dir.join("settings.json");

        // Seed with realistic Zed settings containing comments & trailing commas
        let seed = r#"// Zed settings
{
  "theme": "Ayu Dark",
  //"provider": "zed"
  "language_models": {
    "lmstudio": {
      "api_url": "http://localhost:1234/api/v0",
    },
  },
}
"#;
        std::fs::write(&path, seed).unwrap();

        // 1. managed_diff_content should extract { "context_servers": {} } without failing
        let diff_content = adapter.managed_diff_content(Some(&path)).unwrap();
        assert!(diff_content.contains("context_servers"));

        // 2. Write a new server
        let mut servers = HashMap::new();
        servers.insert(
            "my-mcp".to_string(),
            ServerConfig::Remote {
                url: "http://localhost:8080".to_string(),
                headers: HashMap::new(),
                env: HashMap::new(),
                transport: crate::state::RemoteTransport::StreamableHttp,
                shim_port: None,
                tool_filter: ToolFilter::default(),
            },
        );

        adapter.write_servers("default", &path, &servers).unwrap();

        let updated_raw = std::fs::read_to_string(&path).unwrap();
        assert!(updated_raw.contains("my-mcp"));
        assert!(updated_raw.contains("Ayu Dark"));

        let _ = std::fs::remove_dir_all(temp_dir);
    }
}
