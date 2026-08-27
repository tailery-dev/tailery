use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::state::{ContainerConfig, ServerConfig, ToolFilter};

pub mod antigravity;
pub mod claude_code;
pub mod cursor;
pub mod zed;

pub use antigravity::AntigravityAdapter;
pub use claude_code::ClaudeCodeAdapter;
pub use cursor::CursorAdapter;
pub use zed::ZedAdapter;

/// Inferred default container image for command if none is explicitly configured.
pub fn infer_container_image(command: &str) -> String {
    if command.contains('/') || command.contains(':') || command.starts_with("mcp/") {
        command.to_string()
    } else if command == "npx" || command == "node" || command == "npm" {
        "node:22-alpine".to_string()
    } else if command == "python" || command == "python3" || command == "uv" || command == "uvx" {
        "python:3.11-alpine".to_string()
    } else {
        "alpine:latest".to_string()
    }
}

/// Constructs the container execution arguments for a stdio MCP server.
/// Tailery transparently runs all stdio MCPs inside an isolated Docker container with the shim.
pub fn build_stdio_docker_args(
    name: &str,
    command_opt: Option<&str>,
    args: &[String],
    env: &HashMap<String, String>,
    container: &ContainerConfig,
    tool_filter: &ToolFilter,
    _transport: &crate::state::LocalTransport,
) -> (String, Vec<String>) {
    let command = command_opt.unwrap_or("");
    if command == "docker" {
        return ("docker".to_string(), args.to_vec());
    }

    let default_net = "none".to_string();
    let is_custom_image = !container.image.is_empty();
    let image = if is_custom_image {
        container.image.clone()
    } else {
        infer_container_image(if command.is_empty() {
            "alpine:latest"
        } else {
            command
        })
    };

    let read_only_rootfs = container.read_only_rootfs;
    let network = if container.network.is_empty() {
        &default_net
    } else {
        &container.network
    };

    let mut docker_args = vec![
        "run".to_string(),
        "-i".to_string(),
        "--rm".to_string(),
        "-l".to_string(),
        "dev.tailery.managed=true".to_string(),
        "-l".to_string(),
        format!("dev.tailery.server={}", name),
    ];

    if read_only_rootfs {
        docker_args.push("--read-only".to_string());
    }

    docker_args.push(format!("--network={}", network));

    // Bind mount Tailery manager binary as headless shim and Unix socket
    let host_tailery_bin = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "/usr/local/bin/tailery".to_string());
    docker_args.push("-v".to_string());
    docker_args.push(format!("{}:/.tailery/shim:ro", host_tailery_bin));
    docker_args.push("-v".to_string());
    docker_args.push("/tmp/tailery.sock:/tmp/tailery.sock:rw".to_string());

    for mount in &container.mounts {
        let ro_flag = if mount.read_only { ":ro" } else { "" };
        docker_args.push("-v".to_string());
        docker_args.push(format!("{}:{}{}", mount.host, mount.guest, ro_flag));
    }

    if let Some(res) = &container.resources {
        if let Some(mem) = res.memory_mb {
            docker_args.push(format!("--memory={}m", mem));
        }
        if let Some(cpus) = res.cpus {
            docker_args.push(format!("--cpus={}", cpus));
        }
    }

    for (k, v) in env {
        docker_args.push("-e".to_string());
        docker_args.push(format!("{}={}", k, v));
    }

    if is_custom_image {
        // Model 2: Target container image with overridden entrypoint
        docker_args.push("--entrypoint".to_string());
        docker_args.push("/.tailery/shim".to_string());
        docker_args.push(image);
        docker_args.push("shim".to_string());
        docker_args.push("--server".to_string());
        docker_args.push(name.to_string());

        if !tool_filter.allow.is_empty() {
            docker_args.push("--allow".to_string());
            docker_args.push(tool_filter.allow.join(","));
        }
        if !tool_filter.deny.is_empty() {
            docker_args.push("--deny".to_string());
            docker_args.push(tool_filter.deny.join(","));
        }

        docker_args.push("--".to_string());
        if !command.is_empty() && command != container.image {
            docker_args.push(command.to_string());
        }
        for arg in args {
            docker_args.push(arg.clone());
        }
    } else {
        // Model 1: Local command in slim/alpine container executing shim
        docker_args.push(image);
        docker_args.push("/.tailery/shim".to_string());
        docker_args.push("shim".to_string());
        docker_args.push("--server".to_string());
        docker_args.push(name.to_string());

        if !tool_filter.allow.is_empty() {
            docker_args.push("--allow".to_string());
            docker_args.push(tool_filter.allow.join(","));
        }
        if !tool_filter.deny.is_empty() {
            docker_args.push("--deny".to_string());
            docker_args.push(tool_filter.deny.join(","));
        }

        docker_args.push("--".to_string());
        if !command.is_empty() {
            docker_args.push(command.to_string());
        }
        for arg in args {
            docker_args.push(arg.clone());
        }
    }

    ("docker".to_string(), docker_args)
}

#[derive(Error, Debug)]
#[allow(dead_code)]
pub enum AdapterError {
    #[error("I/O error for {adapter}: {source}")]
    Io {
        adapter: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("Serialization/Deserialization error for {adapter}: {source}")]
    Serialization {
        adapter: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("Path resolution failed for {adapter}: {message}")]
    PathResolution {
        adapter: &'static str,
        message: String,
    },
    #[error("Unsupported server configuration for {adapter}: {reason}")]
    UnsupportedConfig {
        adapter: &'static str,
        reason: String,
    },
}

/// Parses raw JSON or JSON5/JSONC (with comments, trailing commas) into serde_json::Value.
pub fn parse_json_relaxed(raw: &str) -> serde_json::Value {
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(raw) {
        return val;
    }
    if let Ok(val) = json5::from_str::<serde_json::Value>(raw) {
        return val;
    }
    serde_json::json!({})
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpSourceScope {
    User,
    Project(String),
}

impl std::fmt::Display for McpSourceScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McpSourceScope::User => write!(f, "user"),
            McpSourceScope::Project(p) => {
                let name = Path::new(p)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(p);
                write!(f, "proj: {}", name)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoveredMcpStatus {
    /// In Tailery & active in current profile
    ManagedEnabled,
    /// In Tailery & disabled in current profile
    ManagedDisabled,
    /// In Tailery, but on-disk config differs
    ManagedDiff,
    /// Found on disk, not in Tailery
    Unmanaged,
}

#[derive(Debug, Clone)]
pub struct DiscoveredMcp {
    pub name: String,
    pub scope: McpSourceScope,
    pub config: ServerConfig,
    pub status: DiscoveredMcpStatus,
    pub transport_label: String,
    pub raw_json: serde_json::Value,
}

pub fn classify_mcp_status(
    name: &str,
    on_disk_cfg: &ServerConfig,
    state: &crate::state::AppState,
) -> DiscoveredMcpStatus {
    if let Some(tailery_srv) = state.servers.get(name) {
        if tailery_srv_matches(tailery_srv, on_disk_cfg) {
            if state.is_server_enabled_in_active_profile(name) {
                DiscoveredMcpStatus::ManagedEnabled
            } else {
                DiscoveredMcpStatus::ManagedDisabled
            }
        } else {
            DiscoveredMcpStatus::ManagedDiff
        }
    } else if let Some(_container_cfg) = state.configured_containers.get(name) {
        if state.is_server_enabled_in_active_profile(name) {
            DiscoveredMcpStatus::ManagedEnabled
        } else {
            DiscoveredMcpStatus::ManagedDisabled
        }
    } else {
        match on_disk_cfg {
            ServerConfig::Local { args, .. } => {
                if args.iter().any(|a| {
                    a.contains("dev.tailery.managed=true") || a.contains("dev.tailery.server=")
                }) {
                    if state.is_server_enabled_in_active_profile(name) {
                        DiscoveredMcpStatus::ManagedEnabled
                    } else if state.servers.contains_key(name)
                        || state.configured_containers.contains_key(name)
                    {
                        DiscoveredMcpStatus::ManagedDisabled
                    } else {
                        DiscoveredMcpStatus::ManagedDiff
                    }
                } else {
                    DiscoveredMcpStatus::Unmanaged
                }
            }
            _ => DiscoveredMcpStatus::Unmanaged,
        }
    }
}

pub fn tailery_srv_matches(a: &ServerConfig, b: &ServerConfig) -> bool {
    match (a, b) {
        (ServerConfig::Remote { url: u1, .. }, ServerConfig::Remote { url: u2, .. }) => u1 == u2,
        (
            ServerConfig::Local {
                command: c1,
                args: a1,
                ..
            },
            ServerConfig::Local {
                command: c2,
                args: a2,
                ..
            },
        ) => {
            if c1 == c2 && a1 == a2 {
                return true;
            }
            let has_same_server_label = a2.iter().any(|arg| arg.contains("dev.tailery.server="));
            c2.as_deref() == Some("docker") && has_same_server_label
        }
        _ => false,
    }
}

/// Trait defining operations for syncing MCP configurations with specific AI/IDE clients.
pub trait ClientAdapter: Send + Sync + std::fmt::Debug {
    /// Identifier name of the adapter (e.g. "cursor", "claude_code", "zed", "antigravity").
    fn name(&self) -> &'static str;

    /// Human-friendly display name.
    fn display_name(&self) -> &'static str;

    /// Resolve the target configuration file path (global or workspace-scoped).
    fn config_path(&self, workspace: Option<&Path>) -> Result<PathBuf, AdapterError>;

    /// Generate the native JSON structure for this client from active Tailery servers.
    fn generate_config(
        &self,
        servers: &HashMap<String, ServerConfig>,
    ) -> Result<serde_json::Value, AdapterError>;

    /// Read and parse existing server configs from the client's configuration file.
    #[allow(dead_code)]
    fn read_servers(&self, path: &Path) -> Result<HashMap<String, ServerConfig>, AdapterError>;

    /// Extracts ONLY the managed portion of the configuration from the on-disk JSON structure.
    /// Used by the diff viewer so unmanaged keys (themes, user settings, etc.) are omitted.
    fn extract_managed_config(
        &self,
        path: &Path,
        content_json: &serde_json::Value,
    ) -> serde_json::Value;

    /// Merges generated server configurations into an existing on-disk JSON structure,
    /// updating ONLY predefined managed keys/paths and leaving everything else intact.
    fn merge_managed_config(
        &self,
        path: &Path,
        existing_json: Option<&serde_json::Value>,
        servers: &HashMap<String, ServerConfig>,
    ) -> Result<serde_json::Value, AdapterError>;

    /// Discover all MCP servers in client configuration files, classifying them as managed, diff, or unmanaged.
    fn discover_mcps(
        &self,
        path: Option<&Path>,
        state: &crate::state::AppState,
    ) -> Result<Vec<DiscoveredMcp>, AdapterError> {
        let Some(p) = path else {
            return Ok(Vec::new());
        };
        if !p.exists() {
            return Ok(Vec::new());
        }
        let servers = self.read_servers(p)?;
        let mut discovered = Vec::new();
        for (name, cfg) in servers {
            let status = classify_mcp_status(&name, &cfg, state);
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
            discovered.push(DiscoveredMcp {
                name,
                scope: McpSourceScope::User,
                config: cfg,
                status,
                transport_label,
                raw_json: serde_json::json!({}),
            });
        }
        Ok(discovered)
    }

    /// Returns the pretty-printed JSON string of ONLY the managed portion of the on-disk file.
    /// If the file does not exist or has no managed configuration, returns an empty string.
    fn managed_diff_content(&self, path: Option<&Path>) -> Result<String, AdapterError> {
        let Some(p) = path else {
            return Ok(String::new());
        };
        if !p.exists() {
            return Ok(String::new());
        }
        let raw = std::fs::read_to_string(p).map_err(|source| AdapterError::Io {
            adapter: self.name(),
            source,
        })?;
        let parsed = parse_json_relaxed(&raw);
        let managed = self.extract_managed_config(p, &parsed);
        if managed.is_null() {
            return Ok(String::new());
        }
        serde_json::to_string_pretty(&managed).map_err(|source| AdapterError::Serialization {
            adapter: self.name(),
            source,
        })
    }

    /// Write active servers to the target config file, creating a safety backup of the full file for the (profile, client) pair and preserving client-specific metadata and unmanaged keys.
    fn write_servers(
        &self,
        profile: &str,
        path: &std::path::Path,
        servers: &std::collections::HashMap<String, crate::state::ServerConfig>,
    ) -> Result<(), AdapterError> {
        if path.exists() {
            let _ = crate::backup::create_backup(profile, self.name(), path);
        }

        let existing_json: Option<serde_json::Value> = if path.exists() {
            let content = std::fs::read_to_string(path).map_err(|source| AdapterError::Io {
                adapter: self.name(),
                source,
            })?;
            Some(parse_json_relaxed(&content))
        } else {
            None
        };

        let merged_val = self.merge_managed_config(path, existing_json.as_ref(), servers)?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| AdapterError::Io {
                adapter: self.name(),
                source,
            })?;
        }

        let formatted = serde_json::to_string_pretty(&merged_val).map_err(|source| {
            AdapterError::Serialization {
                adapter: self.name(),
                source,
            }
        })?;

        std::fs::write(path, formatted).map_err(|source| AdapterError::Io {
            adapter: self.name(),
            source,
        })?;

        Ok(())
    }

    /// Synchronizes all ManagedServers to the client's respective configuration tiers (Global, Global-Per-Project, In-Repo).
    fn sync_servers(
        &self,
        profile: &str,
        managed_servers: &std::collections::HashMap<String, crate::state::ManagedServer>,
    ) -> Result<usize, AdapterError> {
        // Default implementation falls back to the old global-only behavior.
        // Adapters should override this to handle all tiers.
        let mut global_servers = std::collections::HashMap::new();
        for (name, srv) in managed_servers {
            if srv.is_global {
                global_servers.insert(name.clone(), srv.config.clone());
            }
        }
        let global_path = self.config_path(None)?;
        self.write_servers(profile, &global_path, &global_servers)?;
        Ok(1)
    }

    /// Check if the client is installed or detectable on the host machine.
    fn detect_installed(&self) -> bool;
}

/// Returns a list of all built-in client adapters.
pub fn all_adapters() -> Vec<Box<dyn ClientAdapter>> {
    vec![
        Box::new(CursorAdapter),
        Box::new(ClaudeCodeAdapter),
        Box::new(ZedAdapter),
        Box::new(AntigravityAdapter),
    ]
}

/// Look up a client adapter by its identifier name.
#[allow(dead_code)]
pub fn get_adapter(name: &str) -> Option<Box<dyn ClientAdapter>> {
    match name.to_lowercase().as_str() {
        "cursor" => Some(Box::new(CursorAdapter)),
        "claude" | "claude_code" | "claude-code" => Some(Box::new(ClaudeCodeAdapter)),
        "zed" => Some(Box::new(ZedAdapter)),
        "antigravity" | "google_antigravity" | "google-antigravity" | "agy" => {
            Some(Box::new(AntigravityAdapter))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_adapters_includes_antigravity() {
        let adapters = all_adapters();
        assert!(adapters.iter().any(|a| a.name() == "antigravity"));
        assert!(
            adapters
                .iter()
                .any(|a| a.display_name() == "Google Antigravity")
        );
    }

    #[test]
    fn test_get_adapter_aliases() {
        assert!(get_adapter("antigravity").is_some());
        assert!(get_adapter("google_antigravity").is_some());
        assert!(get_adapter("google-antigravity").is_some());
        assert!(get_adapter("agy").is_some());
        assert_eq!(get_adapter("AGY").unwrap().name(), "antigravity");
        assert!(get_adapter("cursor").is_some());
        assert!(get_adapter("claude_code").is_some());
        assert!(get_adapter("zed").is_some());
        assert!(get_adapter("unknown").is_none());
    }
}
