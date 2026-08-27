use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{AdapterError, ClientAdapter};
use crate::state::{ServerConfig, ToolFilter};

#[derive(Debug, Default, Clone)]
pub struct ClaudeCodeAdapter;

#[derive(Debug, Serialize, Deserialize)]
#[allow(dead_code)]
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
        let root: Value = super::parse_json_relaxed(&content);

        let mut all_servers_map = serde_json::Map::new();

        // 1. Top-level user-scope / mcp.json servers
        if let Some(top_servers) = root.get("mcpServers").and_then(|v| v.as_object()) {
            for (k, v) in top_servers {
                all_servers_map.insert(k.clone(), v.clone());
            }
        }

        // 2. Project-scoped servers (under projects["<cwd>"].mcpServers)
        if let Some(projects) = root.get("projects").and_then(|v| v.as_object()) {
            let current_cwd_str = std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();

            for (proj_path, proj_val) in projects {
                if proj_path == &current_cwd_str || projects.len() == 1 {
                    if let Some(proj_servers) = proj_val.get("mcpServers").and_then(|v| v.as_object()) {
                        for (k, v) in proj_servers {
                            all_servers_map.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
        }

        let mut result = HashMap::new();
        for (name, val) in all_servers_map {
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

    fn extract_managed_config(
        &self,
        path: &Path,
        content_json: &Value,
    ) -> Value {
        let is_mcp_json = path.file_name().and_then(|n| n.to_str()) == Some(".mcp.json");
        let mut managed = serde_json::Map::new();

        // 1. Top-level mcpServers (User scope or .mcp.json)
        let servers = content_json
            .get("mcpServers")
            .cloned()
            .unwrap_or_else(|| json!({}));
        managed.insert("mcpServers".to_string(), servers);

        // 2. If ~/.claude.json, also extract predefined filter keys and projects["<cwd>"]
        if !is_mcp_json {
            let filter_keys = [
                "allowedMcpServers",
                "deniedMcpServers",
                "allowManagedMcpServersOnly",
                "allowAllClaudeAiMcps",
                "enabledMcpjsonServers",
                "disabledMcpjsonServers",
                "enableAllProjectMcpServers",
                "disableClaudeAiConnectors",
            ];
            for key in filter_keys {
                if let Some(val) = content_json.get(key) {
                    managed.insert(key.to_string(), val.clone());
                }
            }

            if let Some(projects) = content_json.get("projects").and_then(|v| v.as_object()) {
                let mut managed_projects = serde_json::Map::new();
                for (proj_key, proj_val) in projects {
                    if let Some(p_obj) = proj_val.as_object() {
                        let mut managed_proj = serde_json::Map::new();
                        if let Some(s) = p_obj.get("mcpServers") {
                            managed_proj.insert("mcpServers".to_string(), s.clone());
                        }
                        if let Some(d) = p_obj.get("disabledMcpServers") {
                            managed_proj.insert("disabledMcpServers".to_string(), d.clone());
                        }
                        if let Some(e) = p_obj.get("enabledMcpServers") {
                            managed_proj.insert("enabledMcpServers".to_string(), e.clone());
                        }
                        if let Some(t) = p_obj.get("hasTrustDialogAccepted") {
                            managed_proj.insert("hasTrustDialogAccepted".to_string(), t.clone());
                        }
                        if !managed_proj.is_empty() {
                            managed_projects.insert(proj_key.clone(), Value::Object(managed_proj));
                        }
                    }
                }
                if !managed_projects.is_empty() {
                    managed.insert("projects".to_string(), Value::Object(managed_projects));
                }
            }
        }

        Value::Object(managed)
    }

    fn merge_managed_config(
        &self,
        path: &Path,
        existing_json: Option<&Value>,
        servers: &HashMap<String, ServerConfig>,
    ) -> Result<Value, AdapterError> {
        let generated = self.generate_config(servers)?;
        let mut root = match existing_json {
            Some(Value::Object(map)) => Value::Object(map.clone()),
            _ => json!({}),
        };

        let is_mcp_json = path.file_name().and_then(|n| n.to_str()) == Some(".mcp.json");

        if let Some(root_map) = root.as_object_mut() {
            let mut target_mcp_servers = if let Some(existing_servers) = root_map.get("mcpServers").and_then(|v| v.as_object()) {
                existing_servers.clone()
            } else {
                serde_json::Map::new()
            };

            // Remove only Tailery-managed servers that are no longer active in `servers`
            let mut keys_to_remove = Vec::new();
            for (k, v) in &target_mcp_servers {
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
                target_mcp_servers.remove(&k);
            }

            // Insert/update active servers
            if let Some(new_mcp_servers) = generated.get("mcpServers").and_then(|v| v.as_object()) {
                for (k, v) in new_mcp_servers {
                    target_mcp_servers.insert(k.clone(), v.clone());
                }
            }

            root_map.insert("mcpServers".to_string(), Value::Object(target_mcp_servers));

            // If managing ~/.claude.json and projects exists:
            if !is_mcp_json {
                if let Some(projects_val) = root_map.get_mut("projects").and_then(|p| p.as_object_mut()) {
                    let current_cwd_str = std::env::current_dir()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_default();

                    if let Some(curr_proj) = projects_val.get_mut(&current_cwd_str).and_then(|p| p.as_object_mut()) {
                        let has_local_stdio = servers.values().any(|s| {
                            matches!(s, ServerConfig::Local { transport: crate::state::LocalTransport::Stdio, .. })
                        });
                        if has_local_stdio && !curr_proj.contains_key("hasTrustDialogAccepted") {
                            curr_proj.insert("hasTrustDialogAccepted".to_string(), Value::Bool(true));
                        }
                    }
                }
            }
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
        let content = std::fs::read_to_string(p).map_err(|source| AdapterError::Io {
            adapter: self.name(),
            source,
        })?;
        let root: Value = super::parse_json_relaxed(&content);
        let mut discovered = Vec::new();

        // 1. User-level mcpServers
        if let Some(mcp_servers) = root.get("mcpServers").and_then(|v| v.as_object()) {
            for (name, val) in mcp_servers {
                let cfg = parse_claude_server_entry(val);
                let status = super::classify_mcp_status(name, &cfg, state);
                let transport_label = claude_entry_transport_label(val, &cfg);
                discovered.push(super::DiscoveredMcp {
                    name: name.clone(),
                    scope: super::McpSourceScope::User,
                    config: cfg,
                    status,
                    transport_label,
                    raw_json: val.clone(),
                });
            }
        }

        // 2. Project-level mcpServers
        if let Some(projects) = root.get("projects").and_then(|v| v.as_object()) {
            for (proj_path, proj_val) in projects {
                if let Some(proj_servers) = proj_val.get("mcpServers").and_then(|v| v.as_object()) {
                    for (name, val) in proj_servers {
                        let cfg = parse_claude_server_entry(val);
                        let status = super::classify_mcp_status(name, &cfg, state);
                        let transport_label = claude_entry_transport_label(val, &cfg);
                        discovered.push(super::DiscoveredMcp {
                            name: name.clone(),
                            scope: super::McpSourceScope::Project(proj_path.clone()),
                            config: cfg,
                            status,
                            transport_label,
                            raw_json: val.clone(),
                        });
                    }
                }
            }
        }

        Ok(discovered)
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

fn parse_claude_server_entry(val: &Value) -> ServerConfig {
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
        ServerConfig::Local {
            command: Some(cmd.to_string()),
            args,
            env,
            tool_filter: ToolFilter::default(),
            container: crate::state::ContainerConfig::default(),
            transport: crate::state::LocalTransport::Stdio,
        }
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
        ServerConfig::Remote {
            url: url.to_string(),
            headers,
            env: HashMap::new(),
            transport,
            tool_filter: ToolFilter::default(),
            shim_port: None,
        }
    } else {
        ServerConfig::Local {
            command: None,
            args: vec![],
            env: HashMap::new(),
            tool_filter: ToolFilter::default(),
            container: crate::state::ContainerConfig::default(),
            transport: crate::state::LocalTransport::Stdio,
        }
    }
}

fn claude_entry_transport_label(val: &Value, cfg: &ServerConfig) -> String {
    if let Some(cmd) = val.get("command").and_then(|c| c.as_str()) {
        if cmd == "docker" {
            return "docker".to_string();
        }
    }
    match cfg {
        ServerConfig::Local { transport, .. } => match transport {
            crate::state::LocalTransport::Stdio => "stdio".to_string(),
            crate::state::LocalTransport::StreamableHttp { .. } => "streamable-http".to_string(),
            crate::state::LocalTransport::Http { .. } => "http".to_string(),
            crate::state::LocalTransport::Sse { .. } => "sse".to_string(),
        },
        ServerConfig::Remote { transport, .. } => match transport {
            crate::state::RemoteTransport::StreamableHttp => "streamable-http".to_string(),
            crate::state::RemoteTransport::Http => "http".to_string(),
            crate::state::RemoteTransport::Sse => "sse".to_string(),
        },
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
                    auto_start: false,
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
    use crate::adapters::{DiscoveredMcpStatus, McpSourceScope};
    use crate::state::AppState;

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
                shim_port: None,
                tool_filter: ToolFilter::default(),
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

    #[test]
    fn test_claude_json_preserves_unmanaged_keys_and_scopes() {
        let adapter = ClaudeCodeAdapter::default();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("tailery_test_claude_scopes_{}", nanos));
        let _ = std::fs::create_dir_all(&temp_dir);
        let claude_json_path = temp_dir.join(".claude.json");

        // Seed with complex existing ~/.claude.json structure
        let seed = json!({
            "theme": "dark",
            "oauthAccount": { "email": "user@example.com" },
            "telemetry": { "enabled": false },
            "mcpServers": {
                "user_old_mcp": {
                    "url": "http://old-user.test"
                }
            },
            "projects": {
                "/Users/vlad.fratila/code/project-a": {
                    "mcpServers": {
                        "proj_a_mcp": { "url": "http://proj-a.test" }
                    },
                    "disabledMcpServers": ["some-disabled"],
                    "enabledMcpServers": ["computer-use"],
                    "hasTrustDialogAccepted": true,
                    "projectCustomSetting": "do_not_touch"
                }
            }
        });
        std::fs::write(&claude_json_path, serde_json::to_string_pretty(&seed).unwrap()).unwrap();

        // 1. Check extract_managed_config
        let managed = adapter.extract_managed_config(&claude_json_path, &seed);
        // Only mcpServers and projects managed keys should exist!
        assert!(managed.get("mcpServers").is_some());
        assert!(managed.get("theme").is_none());
        assert!(managed.get("oauthAccount").is_none());
        assert!(managed.get("telemetry").is_none());

        let projects_managed = managed["projects"]["/Users/vlad.fratila/code/project-a"].as_object().unwrap();
        assert!(projects_managed.contains_key("mcpServers"));
        assert!(projects_managed.contains_key("disabledMcpServers"));
        assert!(projects_managed.contains_key("enabledMcpServers"));
        assert!(projects_managed.contains_key("hasTrustDialogAccepted"));
        assert!(!projects_managed.contains_key("projectCustomSetting"));

        // 2. Write servers to ~/.claude.json
        let mut servers = HashMap::new();
        servers.insert(
            "new_user_tool".to_string(),
            ServerConfig::Remote {
                url: "http://new-user.test".to_string(),
                headers: HashMap::new(),
                env: HashMap::new(),
                transport: crate::state::RemoteTransport::StreamableHttp,
                shim_port: None,
                tool_filter: ToolFilter::default(),
            },
        );

        adapter.write_servers("default", &claude_json_path, &servers).unwrap();

        // 3. Verify updated ~/.claude.json
        let updated_raw = std::fs::read_to_string(&claude_json_path).unwrap();
        let updated: Value = serde_json::from_str(&updated_raw).unwrap();

        // Unmanaged top-level settings are preserved!
        assert_eq!(updated["theme"], "dark");
        assert_eq!(updated["oauthAccount"]["email"], "user@example.com");
        assert_eq!(updated["telemetry"]["enabled"], false);

        // Top-level user-scope mcpServers is updated with new tool and preserves unmanaged MCPs
        assert!(updated["mcpServers"]["new_user_tool"].is_object());
        assert!(updated["mcpServers"]["user_old_mcp"].is_object());

        // Project scope is preserved intact with all subkeys
        let proj_a = &updated["projects"]["/Users/vlad.fratila/code/project-a"];
        assert_eq!(proj_a["disabledMcpServers"][0], "some-disabled");
        assert_eq!(proj_a["enabledMcpServers"][0], "computer-use");
        assert_eq!(proj_a["hasTrustDialogAccepted"], true);
        assert_eq!(proj_a["projectCustomSetting"], "do_not_touch");

        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn test_claude_discover_mcps_and_classify_status() {
        let adapter = ClaudeCodeAdapter::default();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("tailery_test_claude_discover_{}", nanos));
        let _ = std::fs::create_dir_all(&temp_dir);
        let path = temp_dir.join(".claude.json");

        let seed = json!({
            "mcpServers": {
                "SuperhumanDocs": {
                    "type": "http",
                    "url": "https://docs.superhuman.com/apis/mcp"
                },
                "demo-server": {
                    "command": "docker",
                    "args": [
                        "run", "-i", "--rm",
                        "-l", "dev.tailery.managed=true",
                        "-l", "dev.tailery.server=demo-server",
                        "alpine:latest"
                    ]
                }
            },
            "projects": {
                "/Users/vlad.fratila/code/terraform": {
                    "mcpServers": {
                        "groundcover": {
                            "type": "http",
                            "url": "https://mcp.groundcover.com/api/mcp"
                        }
                    }
                }
            }
        });
        std::fs::write(&path, serde_json::to_string_pretty(&seed).unwrap()).unwrap();

        let mut state = AppState::default();
        state.profiles.insert("default".to_string(), crate::state::ProfileConfig::new(vec![], vec![]));
        // Configure demo-server as a disabled server in profile
        state.servers.insert(
            "demo-server".to_string(),
            ServerConfig::Local {
                command: Some("docker".to_string()),
                args: vec!["run".to_string(), "-l".to_string(), "dev.tailery.server=demo-server".to_string()],
                env: HashMap::new(),
                tool_filter: ToolFilter::default(),
                container: crate::state::ContainerConfig::default(),
                transport: crate::state::LocalTransport::Stdio,
            },
        );

        let discovered = adapter.discover_mcps(Some(&path), &state).unwrap();
        assert_eq!(discovered.len(), 3);

        let superhuman = discovered.iter().find(|m| m.name == "SuperhumanDocs").unwrap();
        assert_eq!(superhuman.scope, McpSourceScope::User);
        assert_eq!(superhuman.status, DiscoveredMcpStatus::Unmanaged);
        assert_eq!(superhuman.transport_label, "streamable-http");

        let demo = discovered.iter().find(|m| m.name == "demo-server").unwrap();
        assert_eq!(demo.scope, McpSourceScope::User);
        assert_eq!(demo.status, DiscoveredMcpStatus::ManagedDisabled);
        assert_eq!(demo.transport_label, "docker");

        let groundcover = discovered.iter().find(|m| m.name == "groundcover").unwrap();
        assert_eq!(groundcover.scope, McpSourceScope::Project("/Users/vlad.fratila/code/terraform".to_string()));
        assert_eq!(groundcover.status, DiscoveredMcpStatus::Unmanaged);

        let _ = std::fs::remove_dir_all(temp_dir);
    }
}
