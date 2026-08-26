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

/// Trait defining operations for syncing MCP configurations with specific AI/IDE clients.
pub trait ClientAdapter: Send + Sync {
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
    fn read_servers(&self, path: &Path) -> Result<HashMap<String, ServerConfig>, AdapterError>;

    /// Write active servers to the target config file, creating a safety backup for the (profile, client) pair and preserving client-specific metadata where applicable.
    fn write_servers(
        &self,
        profile: &str,
        path: &Path,
        servers: &HashMap<String, ServerConfig>,
    ) -> Result<(), AdapterError> {
        if path.exists() {
            let _ = crate::backup::create_backup(profile, self.name(), path);
        }
        let json_val = self.generate_config(servers)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| AdapterError::Io {
                adapter: self.name(),
                source,
            })?;
        }
        let formatted = serde_json::to_string_pretty(&json_val).map_err(|source| {
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
