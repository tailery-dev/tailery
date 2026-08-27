use color_eyre::Result;
use ratatui::prelude::Widget;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, StatefulWidget},
};
use std::collections::HashMap;
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    action::Action,
    components::Component,
    state::{ContainerConfig, PortMapping, ServerConfig, ToolFilter},
    state::AppState,
    tui::Event,
};

#[derive(Debug, Clone)]

#[derive(Default)]
pub struct PromptStep {
    pub question: String,
    pub hint: Option<String>,
    pub default_value: Option<String>,
    pub options: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AnsweredStep {
    pub question: String,
    pub answer: String,
}

// -----------------------------------------------------------------------------
// New Profile Wizard
// -----------------------------------------------------------------------------


#[derive(Debug, Clone)]
pub struct NewProfileResult {
    pub name: String,
    pub enabled_clients: Vec<String>,
    pub copy_servers: bool,
    pub import_clients: bool,
    pub project_search_paths: Vec<String>,
    pub activate_now: bool,
}

pub fn parse_client_selection(input: &str) -> Vec<String> {
    let trimmed = input.trim().to_lowercase();
    if trimmed.is_empty() || trimmed == "all" || trimmed == "*" {
        return vec![
            "zed".to_string(),
            "antigravity".to_string(),
        ];
    }
    if trimmed == "none" || trimmed == "0" {
        return Vec::new();
    }

    let mut selected = Vec::new();
    for part in trimmed.split([',', ' ']) {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        match p {
            "1" | "zed" => {
                if !selected.contains(&"zed".to_string()) {
                    selected.push("zed".to_string());
                }
            }
            "2" | "antigravity" | "agy" | "google_antigravity" | "google-antigravity" => {
                if !selected.contains(&"antigravity".to_string()) {
                    selected.push("antigravity".to_string());
                }
            }
            custom => {
                let s = custom.to_string();
                if !selected.contains(&s) {
                    selected.push(s);
                }
            }
        }
    }
    selected
}

#[derive(Debug, Clone)]
pub struct NewProfileWizard {
    pub current_step: usize,
    pub history: Vec<AnsweredStep>,
    pub input_buffer: String,
    pub active_profile_name: String,
    pub client_options: Vec<String>,
    pub client_selections: Vec<bool>,
    pub focused_client_idx: usize,
}

impl Default for NewProfileWizard {
    fn default() -> Self {
        Self::new("default")
    }
}

impl NewProfileWizard {
    pub fn new(active_profile: &str) -> Self {
        Self {
            current_step: 0,
            history: Vec::new(),
            input_buffer: String::new(),
            active_profile_name: active_profile.to_string(),
            client_options: vec!["Zed".to_string(), "Google Antigravity".to_string()],
            client_selections: vec![true, true],
            focused_client_idx: 0,
        }
    }

    pub fn current_prompt(&self) -> PromptStep {
        match self.current_step {
            0 => PromptStep {
                question: "Profile Name".to_string(),
                hint: Some("e.g. work, audit, client-x, sandbox".to_string()),
                default_value: None,
                options: vec![],
            },
            1 => PromptStep {
                question: "Select enabled AI / IDE clients".to_string(),
                hint: Some("e.g. 1,2 or zed,agy or all (default: all)".to_string()),
                default_value: Some("all".to_string()),
                options: vec![
                    "1) Zed".to_string(),
                    "2) Google Antigravity".to_string(),
                ],
            },
            2 => PromptStep {
                question: format!("Copy active servers from '{}'?", self.active_profile_name),
                hint: Some("y = clone enabled servers, n = start empty".to_string()),
                default_value: Some("y".to_string()),
                options: vec!["y (Yes)".to_string(), "n (No)".to_string()],
            },
            3 => PromptStep {
                question: "Import global configurations from enabled clients?".to_string(),
                hint: Some("y = scan and import existing servers, n = skip".to_string()),
                default_value: Some("y".to_string()),
                options: vec!["y (Yes)".to_string(), "n (No)".to_string()],
            },
            4 => PromptStep {
                question: "Auto-discover per-project MCPs in these paths?".to_string(),
                hint: Some("e.g. ~/code, ~/projects (comma separated)".to_string()),
                default_value: Some("".to_string()),
                options: vec![],
            },
            5 => PromptStep {
                question: "Switch to this new profile immediately?".to_string(),
                hint: Some("y = activate right away, n = stay on current profile".to_string()),
                default_value: Some("y".to_string()),
                options: vec!["y (Yes)".to_string(), "n (No)".to_string()],
            },
            _ => PromptStep {
                question: "Confirm creation".to_string(),
                hint: Some("Press [Enter] to create profile".to_string()),
                default_value: Some("y".to_string()),
                options: vec![],
            },
        }
    }

    pub fn handle_char(&mut self, c: char) {
        self.input_buffer.push(c);
    }

    pub fn handle_backspace(&mut self) {
        self.input_buffer.pop();
    }

    pub fn handle_up(&mut self) {
        if self.current_step == 1 && self.focused_client_idx > 0 {
            self.focused_client_idx -= 1;
        }
    }

    pub fn handle_down(&mut self) {
        if self.current_step == 1 && self.focused_client_idx + 1 < self.client_options.len() {
            self.focused_client_idx += 1;
        }
    }

    pub fn handle_space(&mut self) {
        if self.current_step == 1 {
            let idx = self.focused_client_idx;
            if idx < self.client_selections.len() {
                self.client_selections[idx] = !self.client_selections[idx];
            }
        } else {
            self.input_buffer.push(' ');
        }
    }

    pub fn submit(&mut self) -> Option<NewProfileResult> {
        let prompt = self.current_prompt();
        let value = if self.input_buffer.trim().is_empty() {
            prompt.default_value.clone().unwrap_or_default()
        } else {
            self.input_buffer.trim().to_string()
        };

        if self.current_step == 0 && value.is_empty() {
            return None; // Name cannot be empty
        }

        self.history.push(AnsweredStep {
            question: prompt.question,
            answer: value,
        });
        self.input_buffer.clear();
        self.current_step += 1;

        if self.current_step >= 6 {
            let name = self.history[0].answer.clone();
            let enabled_clients = parse_client_selection(&self.history[1].answer);
            let copy_servers = self.history[2].answer.to_lowercase().starts_with('y');
            let import_clients = self.history[3].answer.to_lowercase().starts_with('y');
            let search_paths = self.history[4].answer.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            let activate_now = self.history[5].answer.to_lowercase().starts_with('y');
            Some(NewProfileResult {
                name,
                enabled_clients,
                copy_servers,
                import_clients,
                project_search_paths: search_paths,
                activate_now,
            })
        } else {
            None
        }
    }
}

// -----------------------------------------------------------------------------
// New Server Wizard
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WizardTransport {
    Stdio,
    StreamableHttp,
    Docker,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewServerResult {
    pub name: String,
    pub server_config: ServerConfig,
    pub add_to_profile: bool,
}

pub fn parse_port_mappings(input: &str) -> Vec<PortMapping> {
    let mut ports = Vec::new();
    for part in input.split([',', ' ']) {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        if let Some((h, c)) = p.split_once(':') {
            if let (Ok(h_port), Ok(c_port)) = (h.trim().parse::<u16>(), c.trim().parse::<u16>()) {
                ports.push(PortMapping {
                    host_port: h_port,
                    container_port: c_port,
                    protocol: "tcp".to_string(),
                });
            }
        } else if let Ok(port) = p.parse::<u16>() {
            ports.push(PortMapping {
                host_port: port,
                container_port: port,
                protocol: "tcp".to_string(),
            });
        }
    }
    ports
}

#[derive(Debug, Clone)]

#[derive(Default)]
pub struct NewServerWizard {
    pub current_step: usize,
    pub history: Vec<AnsweredStep>,
    pub input_buffer: String,
    pub active_profile: String,
    pub transport: Option<WizardTransport>,
}

impl NewServerWizard {
    pub fn new(active_profile: &str) -> Self {
        Self {
            current_step: 0,
            history: Vec::new(),
            input_buffer: String::new(),
            active_profile: active_profile.to_string(),
            transport: None,
        }
    }

    pub fn current_prompt(&self) -> PromptStep {
        match self.current_step {
            0 => PromptStep {
                question: "MCP Server Identifier".to_string(),
                hint: Some("Unique name (e.g. filesystem-server, web-search)".to_string()),
                default_value: None,
                options: vec![],
            },
            1 => PromptStep {
                question: "Transport Protocol".to_string(),
                hint: Some("Enter 1, 2, or 3".to_string()),
                default_value: Some("1".to_string()),
                options: vec![
                    "1) stdio (Command or CLI tool)".to_string(),
                    "2) streamable-http (Remote HTTP endpoint)".to_string(),
                    "3) docker (Docker container service or image)".to_string(),
                ],
            },
            _ => match self.transport {
                Some(WizardTransport::Stdio) => match self.current_step {
                    2 => PromptStep {
                        question: "Command or Image to execute".to_string(),
                        hint: Some("e.g. npx, python3, mcp/filesystem:latest".to_string()),
                        default_value: None,
                        options: vec![],
                    },
                    3 => PromptStep {
                        question: "Arguments (space-separated, or empty)".to_string(),
                        hint: Some("e.g. -y @modelcontextprotocol/server-filesystem".to_string()),
                        default_value: Some("".to_string()),
                        options: vec![],
                    },
                    4 => PromptStep {
                        question: "Environment variables (KEY=VAL KEY2=VAL2 or empty)".to_string(),
                        hint: Some("e.g. GITHUB_TOKEN=${keychain:pat}".to_string()),
                        default_value: Some("".to_string()),
                        options: vec![],
                    },
                    5 => PromptStep {
                        question: "Allowed tools (comma-separated, * for all)".to_string(),
                        hint: Some("e.g. read_file, list_dir or *".to_string()),
                        default_value: Some("*".to_string()),
                        options: vec![],
                    },
                    6 => PromptStep {
                        question: "Denied tools (comma-separated or empty)".to_string(),
                        hint: Some("e.g. write_file, delete_file".to_string()),
                        default_value: Some("".to_string()),
                        options: vec![],
                    },
                    _ => PromptStep {
                        question: format!(
                            "Add server to active profile '{}'?",
                            self.active_profile
                        ),
                        hint: Some(
                            "y = add to active profile, n = register globally only".to_string(),
                        ),
                        default_value: Some("y".to_string()),
                        options: vec!["y (Yes)".to_string(), "n (No)".to_string()],
                    },
                },
                Some(WizardTransport::StreamableHttp) => match self.current_step {
                    2 => PromptStep {
                        question: "Endpoint URL".to_string(),
                        hint: Some(
                            "e.g. http://localhost:8080/mcp, https://api.mcp.io/stream".to_string(),
                        ),
                        default_value: None,
                        options: vec![],
                    },
                    3 => PromptStep {
                        question: "Authorization Header / Secret (or empty)".to_string(),
                        hint: Some("e.g. Bearer ${keychain:api-key}".to_string()),
                        default_value: Some("".to_string()),
                        options: vec![],
                    },
                    4 => PromptStep {
                        question: "Apply Tailery HTTP proxy shim? (y/n)".to_string(),
                        hint: Some("Run a background container to enforce tool filtering on this remote server".to_string()),
                        default_value: Some("n".to_string()),
                        options: vec!["y (Yes)".to_string(), "n (No)".to_string()],
                    },
                    5 => {
                        let shim_enabled = self.history.get(4).map(|h| h.answer.to_lowercase().starts_with('y')).unwrap_or(false);
                        if shim_enabled {
                            PromptStep {
                                question: "Local port for the HTTP proxy shim".to_string(),
                                hint: Some("e.g. 8080, 8081".to_string()),
                                default_value: Some("8080".to_string()),
                                options: vec![],
                            }
                        } else {
                            PromptStep {
                                question: format!("Add server to active profile '{}'?", self.active_profile),
                                hint: Some("y = add to active profile, n = register globally only".to_string()),
                                default_value: Some("y".to_string()),
                                options: vec!["y (Yes)".to_string(), "n (No)".to_string()],
                            }
                        }
                    },
                    _ => PromptStep {
                        question: format!(
                            "Add server to active profile '{}'?",
                            self.active_profile
                        ),
                        hint: Some(
                            "y = add to active profile, n = register globally only".to_string(),
                        ),
                        default_value: Some("y".to_string()),
                        options: vec!["y (Yes)".to_string(), "n (No)".to_string()],
                    },
                },
                Some(WizardTransport::Docker) => {
                    let is_docker_run = self
                        .history
                        .get(2)
                        .map(|h| {
                            h.answer.trim().starts_with("docker run")
                                || h.answer.trim().starts_with("docker ")
                        })
                        .unwrap_or(false);

                    if is_docker_run {
                        match self.current_step {
                            2 => PromptStep {
                                question: "Docker Image or Run Command".to_string(),
                                hint: Some("e.g. ghcr.io/aas-ee/open-web-search:latest or docker run -p 3000:3000 image".to_string()),
                                default_value: None,
                                options: vec![],
                            },
                            3 => PromptStep {
                                question: "MCP Endpoint Subpath (e.g. / or /mcp or /sse)".to_string(),
                                hint: Some("Subpath to append to container host port URL".to_string()),
                                default_value: Some("/".to_string()),
                                options: vec![],
                            },
                            _ => PromptStep {
                                question: format!("Add server to active profile '{}'?", self.active_profile),
                                hint: Some("y = add to active profile, n = register globally only".to_string()),
                                default_value: Some("y".to_string()),
                                options: vec!["y (Yes)".to_string(), "n (No)".to_string()],
                            },
                        }
                    } else {
                        match self.current_step {
                            2 => PromptStep {
                                question: "Docker Image or Run Command".to_string(),
                                hint: Some("e.g. ghcr.io/aas-ee/open-web-search:latest or mcp/filesystem:latest".to_string()),
                                default_value: None,
                                options: vec![],
                            },
                            3 => PromptStep {
                                question: "Port Mapping (host:container or empty for stdio sandbox)".to_string(),
                                hint: Some("e.g. 3000:3000, 8080:8080 (or leave empty for stdio sandbox)".to_string()),
                                default_value: Some("".to_string()),
                                options: vec![],
                            },
                            4 => PromptStep {
                                question: "Environment variables (KEY=VAL KEY2=VAL2 or empty)".to_string(),
                                hint: Some("e.g. API_KEY=secret ENABLE_CORS=true".to_string()),
                                default_value: Some("".to_string()),
                                options: vec![],
                            },
                            5 => {
                                let has_ports = self
                                    .history
                                    .get(3)
                                    .map(|h| !h.answer.trim().is_empty())
                                    .unwrap_or(false);

                                if has_ports {
                                    PromptStep {
                                        question: "MCP Endpoint Subpath (e.g. / or /mcp or /sse)".to_string(),
                                        hint: Some("Subpath to append to container host port URL".to_string()),
                                        default_value: Some("/".to_string()),
                                        options: vec![],
                                    }
                                } else {
                                    PromptStep {
                                        question: "Command arguments (optional, or empty)".to_string(),
                                        hint: Some("Arguments passed to container command".to_string()),
                                        default_value: Some("".to_string()),
                                        options: vec![],
                                    }
                                }
                            }
                            _ => PromptStep {
                                question: format!("Add server to active profile '{}'?", self.active_profile),
                                hint: Some("y = add to active profile, n = register globally only".to_string()),
                                default_value: Some("y".to_string()),
                                options: vec!["y (Yes)".to_string(), "n (No)".to_string()],
                            },
                        }
                    }
                }
                None => PromptStep {
                    question: "Unknown step".to_string(),
                    hint: None,
                    default_value: None,
                    options: vec![],
                },
            },
        }
    }

    pub fn handle_char(&mut self, c: char) {
        self.input_buffer.push(c);
    }

    pub fn handle_backspace(&mut self) {
        self.input_buffer.pop();
    }

    pub fn submit(&mut self) -> Option<NewServerResult> {
        let prompt = self.current_prompt();
        let value = if self.input_buffer.trim().is_empty() {
            prompt.default_value.clone().unwrap_or_default()
        } else {
            self.input_buffer.trim().to_string()
        };

        if (self.current_step == 0 || self.current_step == 2) && value.is_empty() {
            return None; // Critical fields cannot be empty
        }

        // Parse transport on step 1
        if self.current_step == 1 {
            let transport = match value.as_str() {
                "2" | "http" | "streamable-http" => WizardTransport::StreamableHttp,
                "3" | "docker" | "container" => WizardTransport::Docker,
                _ => WizardTransport::Stdio,
            };
            self.transport = Some(transport);
        }

        self.history.push(AnsweredStep {
            question: prompt.question,
            answer: value,
        });
        self.input_buffer.clear();
        self.current_step += 1;

        // Check if finished based on transport
        match self.transport {
            Some(WizardTransport::Stdio) if self.current_step >= 8 => {
                let name = self.history[0].answer.clone();
                let cmd = self.history[2].answer.clone();
                let args_raw = &self.history[3].answer;
                let args: Vec<String> = if args_raw.is_empty() {
                    vec![]
                } else {
                    args_raw.split_whitespace().map(String::from).collect()
                };

                let env_raw = &self.history[4].answer;
                let mut env = HashMap::new();
                for pair in env_raw.split_whitespace() {
                    if let Some((k, v)) = pair.split_once('=') {
                        env.insert(k.to_string(), v.to_string());
                    }
                }

                let allow_raw = &self.history[5].answer;
                let allow = if allow_raw == "*" || allow_raw.is_empty() {
                    vec![]
                } else {
                    allow_raw.split(',').map(|s| s.trim().to_string()).collect()
                };

                let deny_raw = &self.history[6].answer;
                let deny = if deny_raw.is_empty() {
                    vec![]
                } else {
                    deny_raw.split(',').map(|s| s.trim().to_string()).collect()
                };

                let add_to_profile = self.history[7].answer.to_lowercase().starts_with('y');

                let container = if cmd.contains('/') || cmd.contains(':') || cmd.starts_with("mcp/")
                {
                    ContainerConfig {
                        auto_start: false,
                        image: cmd.clone(),
                        read_only_rootfs: true,
                        mounts: vec![],
                        ports: vec![],
                        network: "none".to_string(),
                        resources: None,
                    }
                } else {
                    ContainerConfig::default()
                };

                let server_config = ServerConfig::Local {
                    command: Some(cmd),
                    args,
                    env,
                    tool_filter: ToolFilter {
                        allow,
                        deny,
                        auto_approve: vec![],
                    },
                    container,
                    transport: crate::state::LocalTransport::Stdio,
                };

                Some(NewServerResult {
                    name,
                    server_config,
                    add_to_profile,
                })
            }
            Some(WizardTransport::StreamableHttp) => {
                let shim_enabled = self
                    .history
                    .get(4)
                    .map(|h| h.answer.to_lowercase().starts_with('y'))
                    .unwrap_or(false);
                let required_steps = if shim_enabled { 7 } else { 6 };

                if self.current_step >= required_steps {
                    let name = self.history[0].answer.clone();
                    let url = self.history[2].answer.clone();
                    let auth_header = self.history[3].answer.clone();

                    let add_to_profile_idx = if shim_enabled { 6 } else { 5 };
                    let add_to_profile = self.history[add_to_profile_idx]
                        .answer
                        .to_lowercase()
                        .starts_with('y');

                    let shim_port = if shim_enabled {
                        self.history[5].answer.parse::<u16>().ok()
                    } else {
                        None
                    };

                    let mut headers = HashMap::new();
                    if !auth_header.is_empty() {
                        if let Some((k, v)) = auth_header.split_once(':') {
                            headers.insert(k.trim().to_string(), v.trim().to_string());
                        } else {
                            headers.insert("Authorization".to_string(), auth_header);
                        }
                    }

                    let server_config = if let Some(port) = shim_port {
                        let host_tailery_bin = std::env::current_exe()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_else(|_| "/usr/local/bin/tailery".to_string());

                        crate::state::ServerConfig::Local {
                            command: Some("/.tailery/shim".to_string()),
                            args: vec![
                                "shim".to_string(),
                                "--server".to_string(),
                                name.clone(),
                                "--remote-url".to_string(),
                                url.clone(),
                                "--port".to_string(),
                                port.to_string(),
                            ],
                            env: HashMap::new(),
                            tool_filter: ToolFilter::default(),
                            container: crate::state::ContainerConfig {
                                auto_start: true,
                                image: "debian:bullseye-slim".to_string(),
                                ports: vec![crate::state::PortMapping {
                                    host_port: port,
                                    container_port: port,
                                    protocol: "tcp".to_string(),
                                }],
                                mounts: vec![crate::state::MountConfig {
                                    host: host_tailery_bin,
                                    guest: "/.tailery/shim".to_string(),
                                    read_only: true,
                                }],
                                read_only_rootfs: false,
                                network: "host".to_string(),
                                resources: None,
                            },
                            transport: crate::state::LocalTransport::StreamableHttp {
                                port,
                                path: "/".to_string(),
                            },
                        }
                    } else {
                        crate::state::ServerConfig::Remote {
                            url: url.clone(),
                            headers,
                            env: HashMap::new(),
                            transport: crate::state::RemoteTransport::StreamableHttp,
                            tool_filter: ToolFilter::default(),
                            shim_port: None,
                        }
                    };

                    Some(NewServerResult {
                        name,
                        server_config,
                        add_to_profile,
                    })
                } else {
                    None
                }
            }
            Some(WizardTransport::Docker) => {
                let is_docker_run = self
                    .history
                    .get(2)
                    .map(|h| {
                        h.answer.trim().starts_with("docker run")
                            || h.answer.trim().starts_with("docker ")
                    })
                    .unwrap_or(false);

                if is_docker_run && self.current_step >= 5 {
                    let name = self.history[0].answer.clone();
                    let cmd_str = &self.history[2].answer;
                    let subpath = self.history[3].answer.clone();
                    let add_to_profile = self.history[4].answer.to_lowercase().starts_with('y');

                    if let Ok(parsed) = crate::docker::parse_docker_run_command(cmd_str) {
                        if !parsed.ports.is_empty() {
                            let host_port = parsed.ports[0].host_port;
                            let clean_subpath = if subpath.starts_with('/') {
                                subpath
                            } else {
                                format!("/{}", subpath)
                            };

                            let server_config = ServerConfig::Local {
                                command: parsed.command_args.first().cloned(),
                                args: if parsed.command_args.is_empty() {
                                    vec![]
                                } else {
                                    parsed.command_args[1..].to_vec()
                                },
                                env: parsed.env,
                                tool_filter: ToolFilter::default(),
                                container: crate::state::ContainerConfig {
                                    auto_start: true,
                                    image: parsed.image,
                                    ports: parsed.ports,
                                    mounts: parsed.mounts,
                                    read_only_rootfs: parsed.read_only,
                                    network: parsed.network.unwrap_or_else(|| "bridge".to_string()),
                                    resources: None,
                                },
                                transport: crate::state::LocalTransport::StreamableHttp {
                                    port: host_port,
                                    path: clean_subpath,
                                },
                            };

                            Some(NewServerResult {
                                name,
                                server_config,
                                add_to_profile,
                            })
                        } else {
                            let server_config = ServerConfig::Local {
                                command: Some(parsed.image.clone()),
                                args: parsed.command_args.clone(),
                                env: parsed.env.clone(),
                                tool_filter: ToolFilter::default(),
                                container: ContainerConfig {
                                    auto_start: false,
                                    image: parsed.image,
                                    read_only_rootfs: parsed.read_only,
                                    mounts: parsed.mounts,
                                    ports: vec![],
                                    network: parsed.network.unwrap_or_else(|| "none".to_string()),
                                    resources: None,
                                },
                                transport: crate::state::LocalTransport::Stdio,
                            };
                            Some(NewServerResult {
                                name,
                                server_config,
                                add_to_profile,
                            })
                        }
                    } else {
                        None
                    }
                } else if !is_docker_run && self.current_step >= 7 {
                    let name = self.history[0].answer.clone();
                    let image = self.history[2].answer.clone();
                    let ports_raw = &self.history[3].answer;
                    let ports = parse_port_mappings(ports_raw);

                    let env_raw = &self.history[4].answer;
                    let mut env = HashMap::new();
                    for pair in env_raw.split_whitespace() {
                        if let Some((k, v)) = pair.split_once('=') {
                            env.insert(k.to_string(), v.to_string());
                        }
                    }

                    let step5_answer = self.history[5].answer.clone();
                    let add_to_profile = self.history[6].answer.to_lowercase().starts_with('y');

                    if !ports.is_empty() {
                        let host_port = ports[0].host_port;
                        let clean_subpath = if step5_answer.starts_with('/') {
                            step5_answer
                        } else {
                            format!("/{}", step5_answer)
                        };

                        let server_config = ServerConfig::Local {
                            command: None,
                            args: vec![],
                            env: env.clone(),
                            tool_filter: ToolFilter::default(),
                            container: crate::state::ContainerConfig {
                                auto_start: true,
                                image,
                                ports,
                                mounts: vec![],
                                read_only_rootfs: false, // Defaulting to false since they just gave an image
                                network: "bridge".to_string(),
                                resources: None,
                            },
                            transport: crate::state::LocalTransport::StreamableHttp {
                                port: host_port,
                                path: clean_subpath,
                            },
                        };

                        Some(NewServerResult {
                            name,
                            server_config,
                            add_to_profile,
                        })
                    } else {
                        let args: Vec<String> = if step5_answer.is_empty() {
                            vec![]
                        } else {
                            step5_answer.split_whitespace().map(String::from).collect()
                        };

                        let server_config = ServerConfig::Local {
                            command: Some(image.clone()),
                            args,
                            env,
                            tool_filter: ToolFilter::default(),
                            container: ContainerConfig {
                                auto_start: false,
                                image,
                                read_only_rootfs: true,
                                mounts: vec![],
                                ports: vec![],
                                network: "none".to_string(),
                                resources: None,
                            },
                            transport: crate::state::LocalTransport::Stdio,
                        };

                        Some(NewServerResult {
                            name,
                            server_config,
                            add_to_profile,
                        })
                    }
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

// -----------------------------------------------------------------------------
// CLI Wizard Component
// -----------------------------------------------------------------------------

pub enum WizardType {
    Profile(NewProfileWizard),
    Server(NewServerWizard),
}


#[derive(Default)]
pub struct Wizard {
    pub wizard_type: WizardType,
    pub title: String,
    pub command_tx: Option<UnboundedSender<Action>>,
}

impl Wizard {
    pub fn new(wizard_type: WizardType, title: &str) -> Self {
        Self {
            wizard_type,
            title: title.to_string(),
            command_tx: None,
        }
    }
}

impl Component for Wizard {
    fn register_action_handler(&mut self, tx: UnboundedSender<Action>) -> Result<()> {
        self.command_tx = Some(tx);
        Ok(())
    }

    fn handle_events(&mut self, _event: Option<Event>) -> Result<Option<Action>> {
        Ok(None)
    }

    fn update(&mut self, _action: Action) -> Result<Option<Action>> {
        Ok(None)
    }
}

impl StatefulWidget for &Wizard {
    type State = AppState;

    fn render(self, area: Rect, buf: &mut Buffer, _state: &mut Self::State) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(
                format!(" {} ", self.title),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ))
            .style(Style::default().bg(Color::Rgb(15, 20, 30)));

        let inner = block.inner(area);
        block.render(area, buf);

        let (history, current_prompt, input_buffer) = match &self.wizard_type {
            WizardType::Profile(w) => (&w.history, w.current_prompt(), &w.input_buffer),
            WizardType::Server(w) => (&w.history, w.current_prompt(), &w.input_buffer),
        };

        let mut lines = Vec::new();

        for answered in history {
            lines.push(Line::from(vec![
                Span::styled(
                    " ● ",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{}: ", answered.question),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    &answered.answer,
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        }

        if !history.is_empty() {
            lines.push(Line::raw(""));
        }

        let def_hint = current_prompt
            .default_value
            .as_ref()
            .map(|d| format!(" [default: {}]", d))
            .unwrap_or_default();

        lines.push(Line::from(vec![
            Span::styled(
                " > ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                &current_prompt.question,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(def_hint, Style::default().fg(Color::DarkGray)),
        ]));

        for opt in &current_prompt.options {
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(opt, Style::default().fg(Color::Magenta)),
            ]));
        }

        let display_val = if input_buffer.is_empty() {
            if let Some(ref def) = current_prompt.default_value {
                Span::styled(
                    format!("{} (press enter)", def),
                    Style::default().fg(Color::DarkGray),
                )
            } else {
                Span::raw("")
            }
        } else {
            Span::styled(
                input_buffer,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )
        };

        lines.push(Line::from(vec![
            Span::styled(
                "   > ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            display_val,
            Span::styled("▌", Style::default().fg(Color::Magenta)),
        ]));

        if let Some(ref hint) = current_prompt.hint {
            lines.push(Line::from(vec![
                Span::raw("     "),
                Span::styled(hint, Style::default().fg(Color::DarkGray)),
            ]));
        }

        lines.push(Line::raw(""));
        lines.push(Line::from(vec![
            Span::styled(
                " [Enter] ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("Submit Step   "),
            Span::styled(
                " [Esc] ",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("Cancel"),
        ]));

        Paragraph::new(lines).render(inner, buf);
    }
}

pub fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_wizard_workflow() {
        let mut wizard = NewProfileWizard::new("default");

        // Step 0: Name
        wizard.handle_char('w');
        wizard.handle_char('o');
        wizard.handle_char('r');
        wizard.handle_char('k');
        assert!(wizard.submit().is_none());

        // Step 1: Enabled clients (type "1,2" -> zed, antigravity)
        wizard.handle_char('1');
        wizard.handle_char(',');
        wizard.handle_char('2');
        assert!(wizard.submit().is_none());

        // Step 2: Copy servers (default 'y')
        assert!(wizard.submit().is_none());

        // Step 3: Import configs (default 'y')
        assert!(wizard.submit().is_none());
        
        // Step 4: Auto discover paths (type "/code")
        wizard.handle_char('/');
        wizard.handle_char('c');
        wizard.handle_char('o');
        wizard.handle_char('d');
        wizard.handle_char('e');
        assert!(wizard.submit().is_none());

        // Step 5: Activate now (default 'y')
        let result = wizard.submit();
        assert!(result.is_some());
        let res = result.unwrap();
        assert_eq!(res.name, "work");
        assert_eq!(res.enabled_clients, vec!["zed", "antigravity"]);
        assert!(res.copy_servers);
        assert!(res.import_clients);
        assert_eq!(res.project_search_paths, vec!["/code".to_string()]);
        assert!(res.activate_now);
    }

    #[test]
    fn test_parse_client_selection_options() {
        assert_eq!(
            parse_client_selection("all"),
            vec!["zed", "antigravity"]
        );
        assert_eq!(
            parse_client_selection(""),
            vec!["zed", "antigravity"]
        );
        assert_eq!(
            parse_client_selection("1, 2"),
            vec!["zed", "antigravity"]
        );
        assert_eq!(
            parse_client_selection("agy,zed"),
            vec!["antigravity", "zed"]
        );
        assert!(parse_client_selection("none").is_empty());
    }
}
impl Default for WizardType {
    fn default() -> Self {
        WizardType::Profile(crate::components::wizard::NewProfileWizard::default())
    }
}
