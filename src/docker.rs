use bollard::Docker;
use bollard::container::{
    AttachContainerOptions, AttachContainerResults, Config, CreateContainerOptions,
    InspectContainerOptions, ListContainersOptions, LogsOptions, RemoveContainerOptions,
    StartContainerOptions, StopContainerOptions,
};
use bollard::image::CreateImageOptions;
use bollard::models::{
    HostConfig, Mount, MountTypeEnum, PortBinding, PortMap, RestartPolicy, RestartPolicyNameEnum,
};
use futures_util::StreamExt;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;

use crate::state::{ContainerConfig, MountConfig, PortMapping};

#[derive(Error, Debug)]
pub enum DockerError {
    #[error("Docker API error: {0}")]
    Bollard(#[from] bollard::errors::Error),
    #[error("Failed to discover or connect to container runtime: {0}")]
    Connection(String),
    #[error("Container '{0}' was not found")]
    NotFound(String),
    #[error("Invalid container configuration: {0}")]
    InvalidConfig(String),
    #[error("Host binary could not be resolved for bind-mounting: {0}")]
    BinaryResolution(String),
    #[error("Failed to parse docker run command: {0}")]
    CommandParse(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerEngineKind {
    DockerDesktop,
    Colima,
    OrbStack,
    RancherDesktop,
    Podman,
    GenericDocker,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedDaemon {
    pub kind: ContainerEngineKind,
    pub name: String,
    pub socket_path: String,
    pub is_running: bool,
}

pub const LABEL_MANAGED: &str = "dev.tailery.managed";
pub const LABEL_NAME: &str = "dev.tailery.name";
pub const LABEL_SERVER: &str = "dev.tailery.server";
pub const LABEL_TYPE: &str = "dev.tailery.type";
pub const LABEL_VERSION: &str = "dev.tailery.version";

#[derive(Debug, Clone, PartialEq)]
pub struct ContainerStatusInfo {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub status: String,
    pub running: bool,
    pub ports: Vec<String>,
    pub configured: bool,
    pub is_managed: bool,
    pub labels: HashMap<String, String>,
    pub daemon_online: bool,
    pub error: Option<String>,
    pub env_count: usize,
    pub mount_count: usize,
    pub network_name: String,
    pub auto_start: bool,
}

impl ContainerStatusInfo {
    pub fn from_managed_config(
        name: &str,
        cfg: &crate::state::ContainerConfig,
        daemon_online: bool,
    ) -> Self {
        let ports: Vec<String> = cfg
            .ports
            .iter()
            .map(|p| format!("{}:{}", p.host_port, p.container_port))
            .collect();

        let (state, status) = if daemon_online {
            ("stopped".to_string(), "configured (idle)".to_string())
        } else {
            ("offline".to_string(), "daemon unreachable".to_string())
        };

        let mut labels = HashMap::new();
        labels.insert(LABEL_MANAGED.to_string(), "true".to_string());
        labels.insert(LABEL_NAME.to_string(), name.to_string());
        labels.insert(LABEL_TYPE.to_string(), "service".to_string());

        Self {
            id: "-".to_string(),
            name: name.to_string(),
            image: cfg.image.clone(),
            state,
            status,
            running: false,
            ports,
            configured: true,
            is_managed: true,
            labels,
            daemon_online,
            error: if daemon_online {
                None
            } else {
                Some("Docker daemon is offline / socket unreachable".to_string())
            },
            env_count: 0,
            mount_count: cfg.mounts.len(),
            network_name: cfg.network.clone(),
            auto_start: cfg.auto_start,
        }
    }
}

/// Evaluates whether a container is started or managed by Tailery.
///
/// Multi-tier detection criteria:
/// 1. Label check: Has `dev.tailery.managed == "true"` or `tailery.managed == "true"` or any `dev.tailery.*` key.
/// 2. Configuration match: Container name or ID matches configured containers or servers in `AppConfig`.
/// 3. Name prefix heuristic: Container name starts with `tailery-`, `tailery_`, or `tailery.`.
/// 4. Environment check: Environment contains `TAILERY_MANAGED=true` or `TAILERY_SERVER_NAME=...`.
pub fn is_tailery_container(
    name: &str,
    labels: &HashMap<String, String>,
    env: Option<&[String]>,
    config_servers: &HashMap<String, crate::state::ServerConfig>,
) -> bool {
    let clean_name = name.trim_start_matches('/');

    // 1. Explicit Docker label checks
    if let Some(val) = labels
        .get(LABEL_MANAGED)
        .or_else(|| labels.get("tailery.managed"))
    {
        if val == "true" || val == "1" {
            return true;
        }
    }
    if labels
        .keys()
        .any(|k| k.starts_with("dev.tailery.") || k.starts_with("tailery."))
    {
        return true;
    }

    // 2. Configured in Tailery configuration
    if config_servers.contains_key(clean_name) {
        return true;
    }

    // 3. Name prefix heuristic
    if clean_name.starts_with("tailery-")
        || clean_name.starts_with("tailery_")
        || clean_name.starts_with("tailery.")
    {
        return true;
    }

    // 4. Environment variable check
    if let Some(envs) = env {
        for e in envs {
            if e.starts_with("TAILERY_MANAGED=") || e.starts_with("TAILERY_SERVER_NAME=") {
                return true;
            }
        }
    }

    false
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedDockerRun {
    pub name: Option<String>,
    pub image: String,
    pub detached: bool,
    pub ports: Vec<PortMapping>,
    pub env: HashMap<String, String>,
    pub mounts: Vec<MountConfig>,
    pub network: Option<String>,
    pub read_only: bool,
    pub command_args: Vec<String>,
}

/// Identifies the container runtime flavor and name from a given socket path, following symlinks if necessary.
pub fn identify_engine_kind(path: &str) -> (ContainerEngineKind, String) {
    let resolved_path = if let Ok(target) = std::fs::read_link(path) {
        target.to_string_lossy().to_string()
    } else {
        path.to_string()
    };

    let check_str = format!("{} {}", path.to_lowercase(), resolved_path.to_lowercase());

    if check_str.contains("colima") {
        let profile = if let Some(idx) = check_str.find(".colima/") {
            let sub = &check_str[idx + 8..];
            sub.split('/').next().unwrap_or("default")
        } else {
            "default"
        };
        (ContainerEngineKind::Colima, format!("Colima ({})", profile))
    } else if check_str.contains("orbstack") {
        (ContainerEngineKind::OrbStack, "OrbStack".to_string())
    } else if check_str.contains("podman") {
        (ContainerEngineKind::Podman, "Podman".to_string())
    } else if check_str.contains(".rd") || check_str.contains("rancher") {
        (
            ContainerEngineKind::RancherDesktop,
            "Rancher Desktop".to_string(),
        )
    } else if check_str.contains("com.docker.docker")
        || check_str.contains(".docker/run")
        || check_str.contains(".docker/desktop")
    {
        (
            ContainerEngineKind::DockerDesktop,
            "Docker Desktop".to_string(),
        )
    } else if check_str.contains("docker.sock") {
        (
            ContainerEngineKind::GenericDocker,
            "Docker Daemon".to_string(),
        )
    } else {
        (ContainerEngineKind::Custom, "Container Engine".to_string())
    }
}

/// Reads the current active Docker context from `~/.docker/config.json` and resolves its endpoint socket.
pub fn get_current_docker_context_socket() -> Option<(String, PathBuf)> {
    let home = dirs::home_dir()?;
    let config_path = home.join(".docker").join("config.json");
    if !config_path.exists() {
        return None;
    }

    let config_content = std::fs::read_to_string(&config_path).ok()?;
    let config_json: serde_json::Value = serde_json::from_str(&config_content).ok()?;
    let current_context = config_json.get("currentContext")?.as_str()?;

    if current_context == "default" {
        return None;
    }

    // Search ~/.docker/contexts/meta/*/meta.json
    let meta_dir = home.join(".docker").join("contexts").join("meta");
    if meta_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&meta_dir) {
            for entry in entries.flatten() {
                let meta_file = entry.path().join("meta.json");
                if meta_file.exists() {
                    if let Ok(content) = std::fs::read_to_string(&meta_file) {
                        if let Ok(meta_json) = serde_json::from_str::<serde_json::Value>(&content) {
                            if meta_json.get("Name").and_then(|n| n.as_str())
                                == Some(current_context)
                            {
                                if let Some(host_str) = meta_json
                                    .get("Endpoints")
                                    .and_then(|e| e.get("docker"))
                                    .and_then(|d| d.get("Host"))
                                    .and_then(|h| h.as_str())
                                {
                                    let clean_path = host_str.trim_start_matches("unix://");
                                    return Some((
                                        format!("Docker Context ({})", current_context),
                                        PathBuf::from(clean_path),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

/// Collects candidate socket paths across macOS and Linux for Colima, Docker Desktop, Podman, OrbStack, and Rancher Desktop.
pub fn candidate_socket_paths(
    socket_override: Option<&str>,
) -> Vec<(ContainerEngineKind, String, PathBuf)> {
    let mut candidates = Vec::new();

    // 1. Explicit override if provided
    if let Some(sock) = socket_override {
        let path = PathBuf::from(sock.trim_start_matches("unix://"));
        let (kind, name) = identify_engine_kind(&path.to_string_lossy());
        candidates.push((kind, format!("{} [Override]", name), path));
    }

    // 2. DOCKER_HOST or CONTAINER_HOST environment variables
    if let Ok(host) = std::env::var("DOCKER_HOST") {
        if host.starts_with("unix://") {
            let path = PathBuf::from(host.trim_start_matches("unix://"));
            let (kind, name) = identify_engine_kind(&path.to_string_lossy());
            candidates.push((kind, format!("{} [$DOCKER_HOST]", name), path));
        }
    }

    if let Ok(host) = std::env::var("CONTAINER_HOST") {
        if host.starts_with("unix://") {
            let path = PathBuf::from(host.trim_start_matches("unix://"));
            let (kind, name) = identify_engine_kind(&path.to_string_lossy());
            candidates.push((kind, format!("{} [$CONTAINER_HOST]", name), path));
        }
    }

    // 3. Active Docker Context (e.g. colima, orbstack, rootless)
    if let Some((ctx_name, ctx_path)) = get_current_docker_context_socket() {
        let (kind, _) = identify_engine_kind(&ctx_path.to_string_lossy());
        candidates.push((kind, ctx_name, ctx_path));
    }

    // 4. User Home Directory Candidate Paths
    if let Some(home) = dirs::home_dir() {
        // Colima (v0.4.0+ default, legacy root, and custom profiles)
        candidates.push((
            ContainerEngineKind::Colima,
            "Colima (default)".to_string(),
            home.join(".colima").join("default").join("docker.sock"),
        ));
        candidates.push((
            ContainerEngineKind::Colima,
            "Colima (legacy)".to_string(),
            home.join(".colima").join("docker.sock"),
        ));

        // Scan other colima profiles under ~/.colima/*/docker.sock
        let colima_dir = home.join(".colima");
        if colima_dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&colima_dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_dir() {
                        let sock = p.join("docker.sock");
                        let prof_name = p
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string();
                        if prof_name != "default" && prof_name != "_wrapper" {
                            candidates.push((
                                ContainerEngineKind::Colima,
                                format!("Colima ({})", prof_name),
                                sock,
                            ));
                        }
                    }
                }
            }
        }

        // OrbStack (macOS)
        candidates.push((
            ContainerEngineKind::OrbStack,
            "OrbStack".to_string(),
            home.join(".orbstack").join("run").join("docker.sock"),
        ));

        // Docker Desktop (macOS & Linux user socket)
        candidates.push((
            ContainerEngineKind::DockerDesktop,
            "Docker Desktop (User Socket)".to_string(),
            home.join(".docker").join("run").join("docker.sock"),
        ));
        candidates.push((
            ContainerEngineKind::DockerDesktop,
            "Docker Desktop (macOS Data)".to_string(),
            home.join("Library")
                .join("Containers")
                .join("com.docker.docker")
                .join("Data")
                .join("docker.sock"),
        ));
        candidates.push((
            ContainerEngineKind::DockerDesktop,
            "Docker Desktop (Desktop Socket)".to_string(),
            home.join(".docker").join("desktop").join("docker.sock"),
        ));

        // Rancher Desktop
        candidates.push((
            ContainerEngineKind::RancherDesktop,
            "Rancher Desktop".to_string(),
            home.join(".rd").join("docker.sock"),
        ));

        // Podman Machine (macOS default & named machines)
        let podman_machine_base = home
            .join(".local")
            .join("share")
            .join("containers")
            .join("podman")
            .join("machine");
        candidates.push((
            ContainerEngineKind::Podman,
            "Podman Machine (default)".to_string(),
            podman_machine_base
                .join("podman-machine-default")
                .join("podman.sock"),
        ));
        candidates.push((
            ContainerEngineKind::Podman,
            "Podman Machine".to_string(),
            podman_machine_base.join("podman.sock"),
        ));

        if podman_machine_base.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&podman_machine_base) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_dir() {
                        let sock = p.join("podman.sock");
                        let m_name = p
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string();
                        if m_name != "podman-machine-default" {
                            candidates.push((
                                ContainerEngineKind::Podman,
                                format!("Podman Machine ({})", m_name),
                                sock,
                            ));
                        }
                    }
                }
            }
        }
    }

    // 5. Linux Rootless Paths ($XDG_RUNTIME_DIR or /run/user/<UID>)
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        let r_path = PathBuf::from(runtime_dir);
        candidates.push((
            ContainerEngineKind::Podman,
            "Podman (Linux Rootless XDG)".to_string(),
            r_path.join("podman").join("podman.sock"),
        ));
        candidates.push((
            ContainerEngineKind::GenericDocker,
            "Docker (Linux Rootless XDG)".to_string(),
            r_path.join("docker.sock"),
        ));
    }

    let run_user_dir = Path::new("/run/user");
    if run_user_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(run_user_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    candidates.push((
                        ContainerEngineKind::Podman,
                        "Podman (Linux Rootless)".to_string(),
                        p.join("podman").join("podman.sock"),
                    ));
                    candidates.push((
                        ContainerEngineKind::GenericDocker,
                        "Docker (Linux Rootless)".to_string(),
                        p.join("docker.sock"),
                    ));
                }
            }
        }
    }

    // 6. Standard Global System Sockets
    candidates.push((
        ContainerEngineKind::GenericDocker,
        "System Docker (/var/run/docker.sock)".to_string(),
        PathBuf::from("/var/run/docker.sock"),
    ));
    candidates.push((
        ContainerEngineKind::GenericDocker,
        "System Docker (/run/docker.sock)".to_string(),
        PathBuf::from("/run/docker.sock"),
    ));
    candidates.push((
        ContainerEngineKind::Podman,
        "System Podman (/run/podman/podman.sock)".to_string(),
        PathBuf::from("/run/podman/podman.sock"),
    ));
    candidates.push((
        ContainerEngineKind::Podman,
        "System Podman (/var/run/podman/podman.sock)".to_string(),
        PathBuf::from("/var/run/podman/podman.sock"),
    ));

    candidates
}

/// Discovers and tests all available container runtime daemons across macOS and Linux.
pub async fn detect_all_daemons(socket_override: Option<&str>) -> Vec<DetectedDaemon> {
    let candidates = candidate_socket_paths(socket_override);
    let mut detected = Vec::new();
    let mut seen_paths = std::collections::HashSet::new();

    for (kind, name, path) in candidates {
        let path_str = path.to_string_lossy().to_string();
        if !seen_paths.insert(path_str.clone()) {
            continue;
        }

        let socket_exists = path.exists();
        if !socket_exists {
            continue;
        }

        // Test if daemon actively responds to ping
        let is_running = if let Ok(client) =
            Docker::connect_with_unix(&path_str, 2, bollard::API_DEFAULT_VERSION)
        {
            tokio::time::timeout(Duration::from_millis(600), client.ping())
                .await
                .map(|r| r.is_ok())
                .unwrap_or(false)
        } else {
            false
        };

        detected.push(DetectedDaemon {
            kind,
            name,
            socket_path: path_str,
            is_running,
        });
    }

    detected
}

/// Automatically selects and connects to the primary active container daemon.
pub async fn auto_detect_daemon(
    socket_override: Option<&str>,
) -> (Option<DockerManager>, Option<DetectedDaemon>) {
    let daemons = detect_all_daemons(socket_override).await;

    // 1. First priority: look for an actively responding daemon
    for d in &daemons {
        if d.is_running {
            if let Ok(client) =
                Docker::connect_with_unix(&d.socket_path, 120, bollard::API_DEFAULT_VERSION)
            {
                let mut info = d.clone();
                // Query version to enhance display name
                if let Ok(ver) = client.version().await {
                    if let Some(v_str) = ver.version {
                        info.name = format!("{} (v{})", info.name, v_str);
                    }
                }
                let manager = DockerManager {
                    client,
                    socket_path: Some(d.socket_path.clone()),
                    daemon_info: Some(info.clone()),
                };
                return (Some(manager), Some(info));
            }
        }
    }

    // 2. Second priority: return the first detected existing socket for reporting
    if let Some(first) = daemons.first() {
        if let Ok(client) =
            Docker::connect_with_unix(&first.socket_path, 120, bollard::API_DEFAULT_VERSION)
        {
            let manager = DockerManager {
                client,
                socket_path: Some(first.socket_path.clone()),
                daemon_info: Some(first.clone()),
            };
            return (Some(manager), Some(first.clone()));
        }
    }

    // 3. Fallback: standard local defaults
    if let Ok(client) = Docker::connect_with_local_defaults() {
        let (kind, name) = identify_engine_kind("/var/run/docker.sock");
        let default_daemon = DetectedDaemon {
            kind,
            name,
            socket_path: "/var/run/docker.sock".to_string(),
            is_running: false,
        };
        let manager = DockerManager {
            client,
            socket_path: Some("/var/run/docker.sock".to_string()),
            daemon_info: Some(default_daemon.clone()),
        };
        (Some(manager), Some(default_daemon))
    } else {
        (None, None)
    }
}

/// Parses a `docker run` command string into typed parameters.
pub fn parse_docker_run_command(cmd_line: &str) -> Result<ParsedDockerRun, DockerError> {
    let parts: Vec<&str> = cmd_line.split_whitespace().collect();
    if parts.is_empty() {
        return Err(DockerError::CommandParse("Empty command line".to_string()));
    }

    let mut idx = 0;
    // Skip optional leading 'docker' and 'run' tokens
    if parts[idx] == "docker" {
        idx += 1;
    }
    if idx < parts.len() && parts[idx] == "run" {
        idx += 1;
    }

    let mut name = None;
    let mut detached = false;
    let mut ports = Vec::new();
    let mut env = HashMap::new();
    let mut mounts = Vec::new();
    let mut network = None;
    let mut read_only = false;
    let mut image = None;
    let mut command_args = Vec::new();

    while idx < parts.len() {
        let arg = parts[idx];
        if arg == "-d" || arg == "--detach" {
            detached = true;
            idx += 1;
        } else if arg == "--read-only" {
            read_only = true;
            idx += 1;
        } else if arg == "--rm" || arg == "-it" || arg == "-i" || arg == "-t" {
            // Flags to acknowledge and skip
            idx += 1;
        } else if arg == "--name" {
            if idx + 1 < parts.len() {
                name = Some(parts[idx + 1].to_string());
                idx += 2;
            } else {
                return Err(DockerError::CommandParse(
                    "Missing value for --name".to_string(),
                ));
            }
        } else if arg.starts_with("--name=") {
            name = Some(arg.trim_start_matches("--name=").to_string());
            idx += 1;
        } else if arg == "-p" || arg == "--publish" {
            if idx + 1 < parts.len() {
                let p_str = parts[idx + 1];
                if let Some((h, c)) = p_str.split_once(':') {
                    let host_port: u16 = h.parse().map_err(|_| {
                        DockerError::CommandParse(format!("Invalid host port: {}", h))
                    })?;
                    let container_port: u16 = c.parse().map_err(|_| {
                        DockerError::CommandParse(format!("Invalid container port: {}", c))
                    })?;
                    ports.push(PortMapping {
                        host_port,
                        container_port,
                        protocol: "tcp".to_string(),
                    });
                }
                idx += 2;
            } else {
                return Err(DockerError::CommandParse(
                    "Missing value for -p/--publish".to_string(),
                ));
            }
        } else if arg.starts_with("-p=") || arg.starts_with("--publish=") {
            let p_str = if arg.starts_with("-p=") {
                arg.trim_start_matches("-p=")
            } else {
                arg.trim_start_matches("--publish=")
            };
            if let Some((h, c)) = p_str.split_once(':') {
                let host_port: u16 = h
                    .parse()
                    .map_err(|_| DockerError::CommandParse(format!("Invalid host port: {}", h)))?;
                let container_port: u16 = c.parse().map_err(|_| {
                    DockerError::CommandParse(format!("Invalid container port: {}", c))
                })?;
                ports.push(PortMapping {
                    host_port,
                    container_port,
                    protocol: "tcp".to_string(),
                });
            }
            idx += 1;
        } else if arg == "-e" || arg == "--env" {
            if idx + 1 < parts.len() {
                let e_str = parts[idx + 1];
                if let Some((k, v)) = e_str.split_once('=') {
                    env.insert(k.to_string(), v.to_string());
                }
                idx += 2;
            } else {
                return Err(DockerError::CommandParse(
                    "Missing value for -e/--env".to_string(),
                ));
            }
        } else if arg.starts_with("-e=") || arg.starts_with("--env=") {
            let e_str = if arg.starts_with("-e=") {
                arg.trim_start_matches("-e=")
            } else {
                arg.trim_start_matches("--env=")
            };
            if let Some((k, v)) = e_str.split_once('=') {
                env.insert(k.to_string(), v.to_string());
            }
            idx += 1;
        } else if arg == "-v" || arg == "--volume" {
            if idx + 1 < parts.len() {
                let v_str = parts[idx + 1];
                let v_parts: Vec<&str> = v_str.split(':').collect();
                if v_parts.len() >= 2 {
                    let ro = v_parts.get(2).map(|s| *s == "ro").unwrap_or(false);
                    mounts.push(MountConfig {
                        host: v_parts[0].to_string(),
                        guest: v_parts[1].to_string(),
                        read_only: ro,
                    });
                }
                idx += 2;
            } else {
                return Err(DockerError::CommandParse(
                    "Missing value for -v/--volume".to_string(),
                ));
            }
        } else if arg == "--network" || arg == "--net" {
            if idx + 1 < parts.len() {
                network = Some(parts[idx + 1].to_string());
                idx += 2;
            } else {
                return Err(DockerError::CommandParse(
                    "Missing value for --network".to_string(),
                ));
            }
        } else if !arg.starts_with('-') {
            // First positional argument is the image name
            image = Some(arg.to_string());
            idx += 1;
            // Any remaining args are the command to run inside container
            while idx < parts.len() {
                command_args.push(parts[idx].to_string());
                idx += 1;
            }
            break;
        } else {
            // Skip unrecognized flag
            idx += 1;
        }
    }

    let image = image.ok_or_else(|| {
        DockerError::CommandParse("No container image specified in docker run command".to_string())
    })?;

    Ok(ParsedDockerRun {
        name,
        image,
        detached,
        ports,
        env,
        mounts,
        network,
        read_only,
        command_args,
    })
}

#[derive(Clone)]
pub struct DockerManager {
    client: Docker,
    socket_path: Option<String>,
    daemon_info: Option<DetectedDaemon>,
}

impl DockerManager {
    /// Create a new DockerManager using basic connection or socket override.
    pub fn new(socket_override: Option<&str>) -> Result<Self, DockerError> {
        let (client, socket_path) = if let Some(sock) = socket_override {
            (
                Docker::connect_with_unix(sock, 120, bollard::API_DEFAULT_VERSION)?,
                Some(sock.to_string()),
            )
        } else if let Ok(host) = std::env::var("DOCKER_HOST") {
            if host.starts_with("unix://") {
                let path = host.trim_start_matches("unix://");
                (
                    Docker::connect_with_unix(path, 120, bollard::API_DEFAULT_VERSION)?,
                    Some(path.to_string()),
                )
            } else {
                (Docker::connect_with_local_defaults()?, Some(host))
            }
        } else {
            let candidates = candidate_socket_paths(None);
            let mut connected = None;

            for (_kind, _name, path) in candidates {
                if path.exists() {
                    let path_str = path.to_string_lossy().to_string();
                    if let Ok(client) =
                        Docker::connect_with_unix(&path_str, 120, bollard::API_DEFAULT_VERSION)
                    {
                        connected = Some((client, Some(path_str)));
                        break;
                    }
                }
            }

            match connected {
                Some(conn) => conn,
                None => (Docker::connect_with_local_defaults()?, None),
            }
        };

        let daemon_info = socket_path.as_ref().map(|s| {
            let (kind, name) = identify_engine_kind(s);
            DetectedDaemon {
                kind,
                name,
                socket_path: s.clone(),
                is_running: false,
            }
        });

        Ok(Self {
            client,
            socket_path,
            daemon_info,
        })
    }

    /// Auto-detects and connects to the active container daemon asynchronously.
    pub async fn auto_detect(
        socket_override: Option<&str>,
    ) -> (Option<Self>, Option<DetectedDaemon>) {
        auto_detect_daemon(socket_override).await
    }

    /// Check if the container daemon is alive and reachable.
    pub async fn ping(&self) -> bool {
        self.client.ping().await.is_ok()
    }

    /// Get socket path being used.
    pub fn socket_path(&self) -> Option<&str> {
        self.socket_path.as_deref()
    }

    /// Get detected daemon metadata.
    pub fn daemon_info(&self) -> Option<&DetectedDaemon> {
        self.daemon_info.as_ref()
    }

    /// Pull an image from registry if not already present locally.
    pub async fn pull_image(&self, image: &str) -> Result<(), DockerError> {
        let options = Some(CreateImageOptions {
            from_image: image,
            ..Default::default()
        });

        let mut stream = self.client.create_image(options, None, None);
        while let Some(result) = stream.next().await {
            let _ = result?;
        }
        Ok(())
    }

    /// Locate the host binary to bind-mount into containers (defaults to current executable).
    pub fn resolve_host_binary(custom_path: Option<&Path>) -> Result<PathBuf, DockerError> {
        if let Some(p) = custom_path {
            if p.exists() {
                return Ok(p.to_path_buf());
            }
        }
        std::env::current_exe().map_err(|e| {
            DockerError::BinaryResolution(format!(
                "Could not locate current running executable: {}",
                e
            ))
        })
    }

    /// Build HostConfig containing security sandbox policies, port bindings, mounts, resource limits, and binary bind mount.
    pub fn build_host_config(
        container_config: &ContainerConfig,
        host_binary: Option<&Path>,
    ) -> Result<HostConfig, DockerError> {
        let mut mounts: Vec<Mount> = Vec::new();

        // 1. Mount user requested volumes
        for m in &container_config.mounts {
            mounts.push(Mount {
                target: Some(m.guest.clone()),
                source: Some(m.host.clone()),
                typ: Some(MountTypeEnum::BIND),
                read_only: Some(m.read_only),
                ..Default::default()
            });
        }

        // 2. Bind mount the host binary (e.g. tailery / swbd shim) into /usr/local/bin/tailery-shim
        if let Some(binary) = host_binary {
            if binary.exists() {
                mounts.push(Mount {
                    target: Some("/usr/local/bin/tailery-shim".to_string()),
                    source: Some(binary.to_string_lossy().to_string()),
                    typ: Some(MountTypeEnum::BIND),
                    read_only: Some(true),
                    ..Default::default()
                });
            }
        }

        // 3. Port Bindings
        let mut port_bindings: PortMap = HashMap::new();
        for p in &container_config.ports {
            let container_key = format!("{}/{}", p.container_port, p.protocol);
            port_bindings.insert(
                container_key,
                Some(vec![PortBinding {
                    host_ip: Some("0.0.0.0".to_string()),
                    host_port: Some(p.host_port.to_string()),
                }]),
            );
        }

        let mut host_config = HostConfig {
            readonly_rootfs: Some(container_config.read_only_rootfs),
            network_mode: Some(container_config.network.clone()),
            mounts: Some(mounts),
            port_bindings: if port_bindings.is_empty() {
                None
            } else {
                Some(port_bindings)
            },
            restart_policy: Some(RestartPolicy {
                name: Some(RestartPolicyNameEnum::NO),
                maximum_retry_count: None,
            }),
            ..Default::default()
        };

        // 4. Apply Resource Limits (CPU & Memory)
        if let Some(res) = &container_config.resources {
            if let Some(mem_mb) = res.memory_mb {
                host_config.memory = Some((mem_mb * 1024 * 1024) as i64);
            }
            if let Some(cpus) = res.cpus {
                host_config.nano_cpus = Some((cpus * 1_000_000_000.0) as i64);
            }
        }

        Ok(host_config)
    }

    /// Create and configure an isolated MCP container sandbox for stdio processes.
    pub async fn create_sandbox_container(
        &self,
        server_name: &str,
        container_config: &ContainerConfig,
        env_vars: &HashMap<String, String>,
        host_binary: Option<&Path>,
    ) -> Result<String, DockerError> {
        let container_name = format!("tailery-mcp-{}", server_name.replace('/', "-"));

        // Clean up previous container instance if it exists
        let _ = self.remove_container(&container_name, true).await;

        let host_config = Self::build_host_config(container_config, host_binary)?;

        let mut env_vec: Vec<String> = env_vars
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();
        env_vec.push("TAILERY_MANAGED=true".to_string());
        env_vec.push(format!("TAILERY_SERVER_NAME={}", server_name));

        let mut labels = HashMap::new();
        labels.insert(LABEL_MANAGED.to_string(), "true".to_string());
        labels.insert(LABEL_SERVER.to_string(), server_name.to_string());
        labels.insert(LABEL_TYPE.to_string(), "sandbox".to_string());
        labels.insert(
            LABEL_VERSION.to_string(),
            env!("CARGO_PKG_VERSION").to_string(),
        );

        let mut exposed_ports = HashMap::new();
        for p in &container_config.ports {
            exposed_ports.insert(
                format!("{}/{}", p.container_port, p.protocol),
                HashMap::new(),
            );
        }

        let config = Config {
            image: Some(container_config.image.clone()),
            attach_stdin: Some(true),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            open_stdin: Some(true),
            stdin_once: Some(true),
            env: Some(env_vec),
            labels: Some(labels),
            exposed_ports: if exposed_ports.is_empty() {
                None
            } else {
                Some(exposed_ports)
            },
            host_config: Some(host_config),
            ..Default::default()
        };

        let options = CreateContainerOptions {
            name: container_name.as_str(),
            platform: None,
        };

        let response = self.client.create_container(Some(options), config).await?;
        Ok(response.id)
    }

    /// Launch a container directly from a parsed `docker run` command line.
    pub async fn run_parsed_container(
        &self,
        parsed: &ParsedDockerRun,
    ) -> Result<String, DockerError> {
        let container_name = parsed.name.clone().unwrap_or_else(|| {
            let sanitized_img = parsed.image.replace(['/', ':'], "-");
            format!("tailery-{}", sanitized_img)
        });

        // Clean up previous container instance if exists
        let _ = self.remove_container(&container_name, true).await;

        // Try pulling image in case it's not present locally
        let _ = self.pull_image(&parsed.image).await;

        let mut mounts: Vec<Mount> = Vec::new();
        for m in &parsed.mounts {
            mounts.push(Mount {
                target: Some(m.guest.clone()),
                source: Some(m.host.clone()),
                typ: Some(MountTypeEnum::BIND),
                read_only: Some(m.read_only),
                ..Default::default()
            });
        }

        let mut port_bindings: PortMap = HashMap::new();
        let mut exposed_ports = HashMap::new();
        for p in &parsed.ports {
            let container_key = format!("{}/{}", p.container_port, p.protocol);
            port_bindings.insert(
                container_key.clone(),
                Some(vec![PortBinding {
                    host_ip: Some("0.0.0.0".to_string()),
                    host_port: Some(p.host_port.to_string()),
                }]),
            );
            exposed_ports.insert(container_key, HashMap::new());
        }

        let host_config = HostConfig {
            readonly_rootfs: Some(parsed.read_only),
            network_mode: parsed.network.clone(),
            mounts: Some(mounts),
            port_bindings: if port_bindings.is_empty() {
                None
            } else {
                Some(port_bindings)
            },
            restart_policy: Some(RestartPolicy {
                name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
                maximum_retry_count: None,
            }),
            ..Default::default()
        };

        let mut env_vec: Vec<String> = parsed
            .env
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();
        env_vec.push("TAILERY_MANAGED=true".to_string());

        let mut labels = HashMap::new();
        labels.insert(LABEL_MANAGED.to_string(), "true".to_string());
        labels.insert(LABEL_NAME.to_string(), container_name.to_string());
        labels.insert(LABEL_TYPE.to_string(), "service".to_string());
        labels.insert(
            LABEL_VERSION.to_string(),
            env!("CARGO_PKG_VERSION").to_string(),
        );

        let cmd_vec = if parsed.command_args.is_empty() {
            None
        } else {
            Some(parsed.command_args.clone())
        };

        let config = Config {
            image: Some(parsed.image.clone()),
            env: Some(env_vec),
            cmd: cmd_vec,
            labels: Some(labels),
            exposed_ports: if exposed_ports.is_empty() {
                None
            } else {
                Some(exposed_ports)
            },
            host_config: Some(host_config),
            ..Default::default()
        };

        let options = CreateContainerOptions {
            name: container_name.as_str(),
            platform: None,
        };

        let response = self.client.create_container(Some(options), config).await?;
        self.start_container(&response.id).await?;
        Ok(response.id)
    }

    /// Start a created container.
    pub async fn start_container(&self, container_id: &str) -> Result<(), DockerError> {
        self.client
            .start_container(container_id, None::<StartContainerOptions<String>>)
            .await?;
        Ok(())
    }

    /// Stop a running container.
    pub async fn stop_container(
        &self,
        container_id: &str,
        timeout_secs: Option<i64>,
    ) -> Result<(), DockerError> {
        let options = StopContainerOptions {
            t: timeout_secs.unwrap_or(5),
        };
        self.client
            .stop_container(container_id, Some(options))
            .await?;
        Ok(())
    }

    /// Remove a container.
    pub async fn remove_container(
        &self,
        container_id: &str,
        force: bool,
    ) -> Result<(), DockerError> {
        let options = RemoveContainerOptions {
            force,
            v: true,
            ..Default::default()
        };
        let _ = self
            .client
            .remove_container(container_id, Some(options))
            .await;
        Ok(())
    }

    /// Attach bidirectional stdio streams to an MCP container.
    pub async fn attach_stdio(
        &self,
        container_id: &str,
    ) -> Result<AttachContainerResults, DockerError> {
        let options = AttachContainerOptions::<String> {
            stdin: Some(true),
            stdout: Some(true),
            stderr: Some(true),
            stream: Some(true),
            logs: Some(false),
            detach_keys: None,
        };
        let attached = self
            .client
            .attach_container(container_id, Some(options))
            .await?;
        Ok(attached)
    }

    /// Retrieve the most recent log lines from a container.
    pub async fn fetch_container_logs(
        &self,
        container_id: &str,
        tail: usize,
    ) -> Result<Vec<String>, DockerError> {
        let options = LogsOptions::<String> {
            stdout: true,
            stderr: true,
            tail: tail.to_string(),
            ..Default::default()
        };

        let mut stream = self.client.logs(container_id, Some(options));
        let mut log_lines = Vec::new();

        while let Some(log_res) = stream.next().await {
            if let Ok(output) = log_res {
                let text = output.to_string();
                for line in text.lines() {
                    log_lines.push(line.to_string());
                }
            }
        }

        Ok(log_lines)
    }

    /// Query and list all active containers from the container daemon.
    pub async fn list_containers_summary(&self) -> Result<Vec<ContainerStatusInfo>, DockerError> {
        let options = ListContainersOptions::<String> {
            all: true,
            ..Default::default()
        };

        let containers = self.client.list_containers(Some(options)).await?;
        let mut summaries = Vec::new();

        for c in containers {
            let id = c.id.unwrap_or_default();
            let name = c
                .names
                .and_then(|n| n.first().cloned())
                .unwrap_or_default()
                .trim_start_matches('/')
                .to_string();
            let image = c.image.unwrap_or_default();
            let state = c.state.unwrap_or_default();
            let status = c.status.unwrap_or_default();
            let running = state == "running";
            let labels = c.labels.unwrap_or_default();

            let is_managed = labels
                .get(LABEL_MANAGED)
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false)
                || labels
                    .keys()
                    .any(|k| k.starts_with("dev.tailery.") || k.starts_with("tailery."))
                || name.starts_with("tailery-")
                || name.starts_with("tailery_")
                || name.starts_with("tailery.");

            let mut ports = Vec::new();
            if let Some(port_list) = c.ports {
                for p in port_list {
                    if let Some(pub_p) = p.public_port {
                        ports.push(format!("{}:{}", pub_p, p.private_port));
                    }
                }
            }

            summaries.push(ContainerStatusInfo {
                id,
                name,
                image,
                state,
                status,
                running,
                ports,
                configured: false,
                is_managed,
                labels,
                daemon_online: true,
                error: None,
                env_count: 0,
                mount_count: 0,
                network_name: "bridge".to_string(),
                auto_start: false,
            });
        }

        Ok(summaries)
    }

    /// Inspect a container status.
    pub async fn inspect_container(
        &self,
        container_id: &str,
    ) -> Result<ContainerStatusInfo, DockerError> {
        let inspect = self
            .client
            .inspect_container(container_id, None::<InspectContainerOptions>)
            .await?;

        let id = inspect.id.unwrap_or_default();
        let name = inspect
            .name
            .unwrap_or_default()
            .trim_start_matches('/')
            .to_string();
        let image = inspect
            .config
            .as_ref()
            .and_then(|c| c.image.clone())
            .unwrap_or_default();
        let env_count = inspect
            .config
            .as_ref()
            .and_then(|c| c.env.as_ref())
            .map(|e| e.len())
            .unwrap_or(0);
        let state_obj = inspect.state.unwrap_or_default();
        let state = state_obj
            .status
            .map(|s| format!("{:?}", s))
            .unwrap_or_else(|| "unknown".to_string());
        let running = state_obj.running.unwrap_or(false);
        let status = if running {
            "running".to_string()
        } else {
            "stopped".to_string()
        };

        let labels = inspect
            .config
            .as_ref()
            .and_then(|c| c.labels.clone())
            .unwrap_or_default();
        let env_slice = inspect
            .config
            .as_ref()
            .and_then(|c| c.env.as_ref())
            .map(|v| v.as_slice());
        let is_managed = labels
            .get(LABEL_MANAGED)
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false)
            || labels
                .keys()
                .any(|k| k.starts_with("dev.tailery.") || k.starts_with("tailery."))
            || name.starts_with("tailery-")
            || name.starts_with("tailery_")
            || name.starts_with("tailery.")
            || env_slice
                .map(|envs| {
                    envs.iter().any(|e| {
                        e.starts_with("TAILERY_MANAGED=") || e.starts_with("TAILERY_SERVER_NAME=")
                    })
                })
                .unwrap_or(false);

        let mut ports = Vec::new();
        if let Some(network_settings) = inspect.network_settings {
            if let Some(port_map) = network_settings.ports {
                for (container_p, host_bindings) in port_map {
                    if let Some(bindings) = host_bindings {
                        for b in bindings {
                            if let Some(hp) = b.host_port {
                                ports.push(format!("{}:{}", hp, container_p));
                            }
                        }
                    }
                }
            }
        }

        let mount_count = inspect.mounts.map(|m| m.len()).unwrap_or(0);

        Ok(ContainerStatusInfo {
            id,
            name,
            image,
            state,
            status,
            running,
            ports,
            configured: false,
            is_managed,
            labels,
            daemon_online: true,
            error: None,
            env_count,
            mount_count,
            network_name: "bridge".to_string(),
            auto_start: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ResourceLimits;

    #[test]
    fn test_identify_engine_kind() {
        assert_eq!(
            identify_engine_kind("/Users/user/.colima/default/docker.sock").0,
            ContainerEngineKind::Colima
        );
        assert_eq!(
            identify_engine_kind("/Users/user/.colima/arm64/docker.sock").0,
            ContainerEngineKind::Colima
        );
        assert_eq!(
            identify_engine_kind("/Users/user/.orbstack/run/docker.sock").0,
            ContainerEngineKind::OrbStack
        );
        assert_eq!(
            identify_engine_kind("/Users/user/.local/share/containers/podman/machine/podman-machine-default/podman.sock").0,
            ContainerEngineKind::Podman
        );
        assert_eq!(
            identify_engine_kind("/run/user/1000/podman/podman.sock").0,
            ContainerEngineKind::Podman
        );
        assert_eq!(
            identify_engine_kind("/Users/user/.rd/docker.sock").0,
            ContainerEngineKind::RancherDesktop
        );
        assert_eq!(
            identify_engine_kind("/Users/user/.docker/run/docker.sock").0,
            ContainerEngineKind::DockerDesktop
        );
        assert_eq!(
            identify_engine_kind("/run/system/docker.sock").0,
            ContainerEngineKind::GenericDocker
        );
    }

    #[test]
    fn test_candidate_socket_paths_inclusion() {
        let candidates = candidate_socket_paths(None);
        let path_strings: Vec<String> = candidates
            .iter()
            .map(|(_, _, p)| p.to_string_lossy().to_string())
            .collect();

        // Ensure Colima, System Docker, and Podman patterns exist in candidate search paths
        assert!(path_strings.iter().any(|s| s.contains(".colima")));
        assert!(path_strings.iter().any(|s| s.contains("docker.sock")));
        assert!(path_strings.iter().any(|s| s.contains("podman")));
    }

    #[test]
    fn test_parse_docker_run_command() {
        let cmd = "docker run -d --name web-search -p 3000:3000 -e ENABLE_CORS=true -e CORS_ORIGIN=* ghcr.io/aas-ee/open-web-search:latest";
        let parsed = parse_docker_run_command(cmd).unwrap();

        assert_eq!(parsed.name, Some("web-search".to_string()));
        assert!(parsed.detached);
        assert_eq!(parsed.image, "ghcr.io/aas-ee/open-web-search:latest");
        assert_eq!(
            parsed.ports,
            vec![PortMapping {
                host_port: 3000,
                container_port: 3000,
                protocol: "tcp".to_string()
            }]
        );
        assert_eq!(
            parsed.env.get("ENABLE_CORS").map(|s| s.as_str()),
            Some("true")
        );
        assert_eq!(parsed.env.get("CORS_ORIGIN").map(|s| s.as_str()), Some("*"));
    }

    #[test]
    fn test_build_host_config_with_ports() {
        let config = ContainerConfig {
            image: "ghcr.io/aas-ee/open-web-search:latest".to_string(),
            read_only_rootfs: true,
            network: "bridge".to_string(),
            mounts: vec![MountConfig {
                host: "/tmp".to_string(),
                guest: "/sandbox".to_string(),
                read_only: true,
            }],
            ports: vec![PortMapping {
                host_port: 3000,
                container_port: 3000,
                protocol: "tcp".to_string(),
            }],
            resources: Some(ResourceLimits {
                memory_mb: Some(512),
                cpus: Some(1.5),
            }),
        };

        let host_config = DockerManager::build_host_config(&config, None).unwrap();
        assert_eq!(host_config.readonly_rootfs, Some(true));
        assert_eq!(host_config.network_mode, Some("bridge".to_string()));
        assert_eq!(host_config.memory, Some(512 * 1024 * 1024));

        let port_bindings = host_config.port_bindings.unwrap();
        assert!(port_bindings.contains_key("3000/tcp"));
    }

    #[tokio::test]
    async fn test_auto_detect_daemon_execution() {
        let (docker_mgr, daemon_info) = auto_detect_daemon(None).await;
        if let Some(info) = &daemon_info {
            println!(
                "==> Detected Container Daemon: {} at {} (running: {})",
                info.name, info.socket_path, info.is_running
            );
            assert!(!info.socket_path.is_empty());
        }
        if let Some(mgr) = &docker_mgr {
            println!(
                "==> Connected to Docker Manager at: {:?}",
                mgr.socket_path()
            );
            assert!(mgr.socket_path().is_some());
        }
    }

    #[test]
    fn test_is_tailery_container_detection_tiers() {
        let mut config_servers = HashMap::new();
        config_servers.insert(
            "fs-server".to_string(),
            crate::state::ServerConfig::StreamableHttp {
                url: "http://localhost:8080".to_string(),
                headers: HashMap::new(),
                env: HashMap::new(),
            },
        );

        // Tier 1: Labels
        let mut labels_managed = HashMap::new();
        labels_managed.insert(LABEL_MANAGED.to_string(), "true".to_string());
        assert!(is_tailery_container(
            "random-name",
            &labels_managed,
            None,
            &config_servers
        ));

        let mut labels_legacy = HashMap::new();
        labels_legacy.insert("tailery.managed".to_string(), "true".to_string());
        assert!(is_tailery_container(
            "random-name",
            &labels_legacy,
            None,
            &config_servers
        ));

        let mut labels_server = HashMap::new();
        labels_server.insert("dev.tailery.server".to_string(), "myserver".to_string());
        assert!(is_tailery_container(
            "random-name",
            &labels_server,
            None,
            &config_servers
        ));

        // Tier 2: Config match
        let empty_labels = HashMap::new();
        assert!(is_tailery_container(
            "custom-search",
            &empty_labels,
            None,
            &config_servers
        ));
        assert!(is_tailery_container(
            "/custom-search",
            &empty_labels,
            None,
            &config_servers
        ));
        assert!(is_tailery_container(
            "fs-server",
            &empty_labels,
            None,
            &config_servers
        ));

        // Tier 3: Name prefix
        assert!(is_tailery_container(
            "tailery-mcp-github",
            &empty_labels,
            None,
            &config_servers
        ));
        assert!(is_tailery_container(
            "/tailery-postgres",
            &empty_labels,
            None,
            &config_servers
        ));
        assert!(is_tailery_container(
            "tailery_service",
            &empty_labels,
            None,
            &config_servers
        ));

        // Tier 4: Environment variables
        let envs = vec!["FOO=bar".to_string(), "TAILERY_MANAGED=true".to_string()];
        assert!(is_tailery_container(
            "unlabeled-container",
            &empty_labels,
            Some(&envs),
            &config_servers
        ));

        // Negative check: Completely unrelated external containers
        assert!(!is_tailery_container(
            "postgres-db",
            &empty_labels,
            None,
            &config_servers
        ));
        assert!(!is_tailery_container(
            "/redis_cache_prod",
            &empty_labels,
            None,
            &config_servers
        ));
        assert!(!is_tailery_container(
            "k8s_node_control_plane",
            &empty_labels,
            None,
            &config_servers
        ));
    }
}
