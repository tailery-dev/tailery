use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppState {
    pub version: String,
    pub settings: GlobalSettings,
    pub servers: HashMap<String, ServerConfig>,
    #[serde(default)]
    pub profiles: HashMap<String, ProfileConfig>,
    #[serde(default)]
    pub workspaces: HashMap<PathBuf, WorkspaceConfig>,

    // Ephemeral UI State
    #[serde(skip)]
    pub docker_status: String,
    #[serde(skip)]
    pub containers: Vec<crate::docker::ContainerStatusInfo>,
    #[serde(skip)]
    pub container_logs: Vec<String>,
    #[serde(skip)]
    pub inspector_events: Vec<crate::shim::TelemetryMessage>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlobalSettings {
    #[serde(default = "default_profile")]
    pub active_profile: String,
    pub docker_socket: Option<String>,
    #[serde(default)]
    pub sync_clients: Vec<String>,
    #[serde(default)]
    pub filter_managed_containers_only: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "transport", rename_all = "kebab-case")]
pub enum LocalTransport {
    Stdio,
    StreamableHttp {
        port: u16,
        path: String,
    },
    Http {
        port: u16,
        path: String,
    },
    #[serde(rename = "sse")]
    Sse {
        port: u16,
        path: String,
    },
}

impl Default for LocalTransport {
    fn default() -> Self {
        Self::Stdio
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RemoteTransport {
    StreamableHttp,
    Http,
    #[serde(rename = "sse")]
    Sse,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ServerConfig {
    Local {
        #[serde(default)]
        command: Option<String>,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
        #[serde(default)]
        tool_filter: ToolFilter,
        #[serde(default)]
        container: ContainerConfig,
        #[serde(default)]
        transport: LocalTransport,
    },
    Remote {
        url: String,
        transport: RemoteTransport,
        #[serde(default)]
        headers: HashMap<String, String>,
        #[serde(default)]
        env: HashMap<String, String>,
        #[serde(default)]
        tool_filter: ToolFilter,
        #[serde(default)]
        shim_port: Option<u16>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContainerConfig {
    #[serde(default)]
    pub auto_start: bool,
    #[serde(default)]
    pub image: String,
    #[serde(default = "default_true")]
    pub read_only_rootfs: bool,
    #[serde(default)]
    pub mounts: Vec<MountConfig>,
    #[serde(default)]
    pub ports: Vec<PortMapping>,
    #[serde(default = "default_network")]
    pub network: String,
    pub resources: Option<ResourceLimits>,
}

impl Default for ContainerConfig {
    fn default() -> Self {
        Self {
            auto_start: false,
            image: String::new(),
            read_only_rootfs: true,
            mounts: Vec::new(),
            ports: Vec::new(),
            network: default_network(),
            resources: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortMapping {
    pub host_port: u16,
    pub container_port: u16,
    #[serde(default = "default_tcp")]
    pub protocol: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MountConfig {
    pub host: String,
    pub guest: String,
    #[serde(default = "default_true")]
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceLimits {
    pub memory_mb: Option<u64>,
    pub cpus: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ToolFilter {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    #[serde(default)]
    pub auto_approve: Vec<String>,
}

pub fn default_enabled_clients() -> Vec<String> {
    vec![
        "cursor".to_string(),
        "claude_code".to_string(),
        "zed".to_string(),
        "antigravity".to_string(),
    ]
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileConfig {
    #[serde(default)]
    pub enabled_servers: Vec<String>,
    #[serde(default = "default_enabled_clients")]
    pub enabled_clients: Vec<String>,
}

impl Default for ProfileConfig {
    fn default() -> Self {
        Self {
            enabled_servers: Vec::new(),
            enabled_clients: default_enabled_clients(),
        }
    }
}

impl ProfileConfig {
    pub fn new(enabled_servers: Vec<String>, enabled_clients: Vec<String>) -> Self {
        Self {
            enabled_servers,
            enabled_clients,
        }
    }

    pub fn with_servers(enabled_servers: Vec<String>) -> Self {
        Self {
            enabled_servers,
            enabled_clients: default_enabled_clients(),
        }
    }

    pub fn is_client_enabled(&self, client: &str) -> bool {
        let client_lower = client.to_lowercase();
        self.enabled_clients.iter().any(|c| {
            let cl = c.to_lowercase();
            cl == client_lower
                || match client_lower.as_str() {
                    "claude" | "claude-code" | "claude_code" => {
                        cl == "claude" || cl == "claude-code" || cl == "claude_code"
                    }
                    "antigravity" | "agy" | "google_antigravity" | "google-antigravity" => {
                        cl == "antigravity"
                            || cl == "agy"
                            || cl == "google_antigravity"
                            || cl == "google-antigravity"
                    }
                    _ => false,
                }
        })
    }

    pub fn enable_client(&mut self, client: &str) {
        let client_lower = client.to_lowercase();
        if !self.is_client_enabled(&client_lower) {
            self.enabled_clients.push(client_lower);
        }
    }

    pub fn disable_client(&mut self, client: &str) {
        let client_lower = client.to_lowercase();
        self.enabled_clients.retain(|c| {
            let cl = c.to_lowercase();
            if cl == client_lower {
                return false;
            }
            match client_lower.as_str() {
                "claude" | "claude-code" | "claude_code" => {
                    !(cl == "claude" || cl == "claude-code" || cl == "claude_code")
                }
                "antigravity" | "agy" | "google_antigravity" | "google-antigravity" => {
                    !(cl == "antigravity"
                        || cl == "agy"
                        || cl == "google_antigravity"
                        || cl == "google-antigravity")
                }
                _ => true,
            }
        });
    }

    pub fn toggle_client(&mut self, client: &str) -> bool {
        if self.is_client_enabled(client) {
            self.disable_client(client);
            false
        } else {
            self.enable_client(client);
            true
        }
    }

    pub fn is_server_enabled(&self, server: &str) -> bool {
        self.enabled_servers.iter().any(|s| s == server)
    }

    pub fn enable_server(&mut self, server: &str) {
        if !self.is_server_enabled(server) {
            self.enabled_servers.push(server.to_string());
        }
    }

    pub fn disable_server(&mut self, server: &str) {
        self.enabled_servers.retain(|s| s != server);
    }

    pub fn toggle_server(&mut self, server: &str) -> bool {
        if self.is_server_enabled(server) {
            self.disable_server(server);
            false
        } else {
            self.enable_server(server);
            true
        }
    }
}

impl AppState {
    pub fn is_client_enabled_in_active_profile(&self, client: &str) -> bool {
        if let Some(profile) = self.profiles.get(&self.settings.active_profile) {
            profile.is_client_enabled(client)
        } else {
            true
        }
    }

    pub fn get_active_profile_servers(&self) -> HashMap<String, ServerConfig> {
        if let Some(profile) = self.profiles.get(&self.settings.active_profile) {
            self.servers
                .iter()
                .filter(|(name, _)| profile.is_server_enabled(name))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        } else {
            self.servers.clone()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    pub profile: Option<String>,
    #[serde(default)]
    pub overrides: HashMap<String, serde_json::Value>,
}

fn default_profile() -> String {
    "default".to_string()
}
fn default_true() -> bool {
    true
}
fn default_network() -> String {
    "none".to_string()
}
fn default_tcp() -> String {
    "tcp".to_string()
}

use directories::ProjectDirs;

pub const QUALIFIER: &str = "dev";
pub const ORGANIZATION: &str = "tailery";
pub const APPLICATION: &str = "tailery";

// =============================================================================
// XDG Configuration Standard Utilities & directories-rs Integration
// =============================================================================

/// Returns `ProjectDirs` instance for Tailery based on standard platform conventions (`dev.tailery.tailery`).
pub fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
}

/// Resolves the configuration directory for Tailery following the XDG Base Directory specification.
///
/// Order of precedence:
/// 1. `TAILERY_CONFIG_DIR` or `TAILERY_CONFIG` (if pointing to directory)
/// 2. `XDG_CONFIG_HOME/tailery`
/// 3. Default XDG fallback: `~/.config/tailery` or `.`
pub fn get_config_dir() -> PathBuf {
    if let Ok(val) = std::env::var("TAILERY_CONFIG_DIR") {
        if !val.trim().is_empty() {
            return PathBuf::from(val.trim());
        }
    }
    if let Ok(val) = std::env::var("TAILERY_CONFIG") {
        if !val.trim().is_empty() {
            let p = PathBuf::from(val.trim());
            return if p.is_file() || p.extension().is_some() {
                p.parent().unwrap_or(&p).to_path_buf()
            } else {
                p
            };
        }
    }
    if let Ok(val) = std::env::var("XDG_CONFIG_HOME") {
        if !val.trim().is_empty() {
            return PathBuf::from(val.trim()).join("tailery");
        }
    }
    if let Some(home) = dirs::home_dir() {
        home.join(".config").join("tailery")
    } else {
        PathBuf::from(".")
    }
}

/// Resolves the canonical configuration file path (`<config_dir>/config.json`).
///
/// If `TAILERY_CONFIG` is set and points to an explicit file, that exact path is returned.
pub fn get_config_path() -> PathBuf {
    if let Ok(val) = std::env::var("TAILERY_CONFIG") {
        let p = PathBuf::from(val.trim());
        if !val.trim().is_empty() && (p.is_file() || p.extension().is_some()) {
            return p;
        }
    }
    get_config_dir().join("config.json")
}

/// Resolves the data directory for Tailery following the XDG Base Directory specification.
///
/// Order of precedence:
/// 1. `TAILERY_DATA_DIR` or `TAILERY_DATA`
/// 2. `XDG_DATA_HOME/tailery`
/// 3. Default XDG fallback: `~/.local/share/tailery` or `.tailery_data`
pub fn get_data_dir() -> PathBuf {
    if let Ok(val) = std::env::var("TAILERY_DATA_DIR") {
        if !val.trim().is_empty() {
            return PathBuf::from(val.trim());
        }
    }
    if let Ok(val) = std::env::var("TAILERY_DATA") {
        if !val.trim().is_empty() {
            return PathBuf::from(val.trim());
        }
    }
    if let Ok(val) = std::env::var("XDG_DATA_HOME") {
        if !val.trim().is_empty() {
            return PathBuf::from(val.trim()).join("tailery");
        }
    }
    if let Some(home) = dirs::home_dir() {
        home.join(".local").join("share").join("tailery")
    } else {
        PathBuf::from(".tailery_data")
    }
}

/// Resolves the cache directory for Tailery following the XDG Base Directory specification.
///
/// Order of precedence:
/// 1. `TAILERY_CACHE_DIR` or `TAILERY_CACHE`
/// 2. `XDG_CACHE_HOME/tailery`
/// 3. Default XDG fallback: `~/.cache/tailery` or `.cache`
pub fn get_cache_dir() -> PathBuf {
    if let Ok(val) = std::env::var("TAILERY_CACHE_DIR") {
        if !val.trim().is_empty() {
            return PathBuf::from(val.trim());
        }
    }
    if let Ok(val) = std::env::var("TAILERY_CACHE") {
        if !val.trim().is_empty() {
            return PathBuf::from(val.trim());
        }
    }
    if let Ok(val) = std::env::var("XDG_CACHE_HOME") {
        if !val.trim().is_empty() {
            return PathBuf::from(val.trim()).join("tailery");
        }
    }
    if let Some(home) = dirs::home_dir() {
        home.join(".cache").join("tailery")
    } else {
        PathBuf::from(".cache")
    }
}

/// Resolves the XDG-compliant config path for Tailery (alias for `get_config_path()`).
pub fn xdg_config_path() -> PathBuf {
    get_config_path()
}

/// Resolves the XDG-compliant data directory for Tailery (alias for `get_data_dir()`).
pub fn xdg_data_dir() -> PathBuf {
    get_data_dir()
}

/// Resolves the active configuration path, honoring command-line overrides, XDG/directories standard, or local fallback.
pub fn resolve_config_path(path_override: Option<&Path>) -> PathBuf {
    if let Some(p) = path_override {
        p.to_path_buf()
    } else {
        let canonical_path = get_config_path();
        if canonical_path.exists() {
            canonical_path
        } else if Path::new("tailery.json").exists() {
            PathBuf::from("tailery.json")
        } else {
            canonical_path
        }
    }
}

/// Persists an `AppState` instance to disk formatted as clean JSON, ensuring parent directories exist.
pub fn save_config_to_path(config: &AppState, path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(path, json)?;
    Ok(())
}




impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig::Local {
            command: Some(String::new()),
            args: vec![],
            env: std::collections::HashMap::new(),
            tool_filter: Default::default(),
            container: Default::default(),
            transport: Default::default(),
        }
    }
}
