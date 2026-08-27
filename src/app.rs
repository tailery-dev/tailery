use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind};
use ratatui::prelude::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::mpsc;

use crate::{
    action::Action,
    adapters::all_adapters,
    components::{
        Component,
        clients::{CachedClientDiff, Clients, compute_client_diff_full},
        containers::Containers,
        diff_viewer::DiffViewer,
        greeting::Greeting,
        help::Help,
        inspector::Inspector,
        mcp_browser::McpBrowser,
        mcps::{ContainerFilterMode, Mcps},
        profile_editor::ProfileEditor,
        profile_switcher::ProfileSwitcher,
        server_browser::ServerBrowser,
        sidebar::Sidebar,
        skills::Skills,
        sync_confirm::SyncConfirm,
        wizard::Wizard,
    },
    config::Config,
    docker::{ContainerStatusInfo, DockerManager},
    shim::TelemetryMessage,
    state::{AppState, ProfileConfig, ServerConfig, resolve_config_path, save_config_to_path},
    tui::{Event, Tui},
};

#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    Clients,
    Mcps,
    Skills,
}

#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ActiveModal {
    #[default]
    None,
    Help,
    McpBrowser,
    ProfileSwitcher,
    ProfileEditor,
    Wizard,
    SyncConfirm,
    ServerBrowser,
    DiffViewer,
    Inspector,
}

#[derive(Clone, Debug, Default)]
pub struct DockerPollResult {
    pub docker_status: String,
    pub containers: Vec<ContainerStatusInfo>,
    pub logs: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct DockerPollRequest {
    pub servers: HashMap<String, ServerConfig>,
    pub configured_containers: HashMap<String, crate::state::ContainerConfig>,
    pub filter_mode: ContainerFilterMode,
    pub selected_item_name: Option<String>,
}

pub fn start_docker_poller(
    docker: Option<DockerManager>,
    mut req_rx: mpsc::UnboundedReceiver<DockerPollRequest>,
    res_tx: mpsc::UnboundedSender<DockerPollResult>,
) {
    tokio::spawn(async move {
        let mut latest_req: Option<DockerPollRequest> = None;
        let mut interval = tokio::time::interval(Duration::from_millis(1500));
        loop {
            tokio::select! {
                req = req_rx.recv() => {
                    match req {
                        Some(r) => latest_req = Some(r),
                        None => break,
                    }
                }
                _ = interval.tick() => {}
            }

            let Some(req) = latest_req.clone() else {
                continue;
            };

            let daemon_online = if let Some(ref d) = docker {
                d.ping().await
            } else {
                false
            };

            let docker_status = if let Some(ref d) = docker {
                if let Some(info) = d.daemon_info() {
                    if daemon_online {
                        format!("Online ({})", info.name)
                    } else {
                        format!("Offline ({})", info.name)
                    }
                } else if daemon_online {
                    "Online".to_string()
                } else {
                    "Offline (No socket)".to_string()
                }
            } else {
                "Offline (No socket)".to_string()
            };

            let mut all_list: Vec<ContainerStatusInfo> = req
                .servers
                .iter()
                .filter_map(|(name, srv)| {
                    if let ServerConfig::Local { container, .. } = srv {
                        if !container.image.is_empty() {
                            Some(ContainerStatusInfo::from_managed_config(
                                name,
                                container,
                                daemon_online,
                            ))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .collect();

            for (name, container) in &req.configured_containers {
                if !all_list.iter().any(|c| c.name == *name) {
                    all_list.push(ContainerStatusInfo::from_managed_config(
                        name,
                        container,
                        daemon_online,
                    ));
                }
            }

            if daemon_online {
                if let Some(ref d) = docker {
                    if let Ok(live_containers) = d.list_containers_summary().await {
                        for live in live_containers {
                            let clean_live_name = live.name.trim_start_matches('/');
                            if let Some(pos) = all_list
                                .iter()
                                .position(|c| c.name == live.name || c.name == clean_live_name)
                            {
                                all_list[pos].id = live.id.clone();
                                all_list[pos].running = live.running;
                                all_list[pos].state = live.state.clone();
                                all_list[pos].status = live.status.clone();
                                all_list[pos].is_managed = true;
                                if !live.ports.is_empty() {
                                    all_list[pos].ports = live.ports;
                                }
                                if !live.labels.is_empty() {
                                    all_list[pos].labels = live.labels;
                                }
                                all_list[pos].daemon_online = true;
                                all_list[pos].error = None;
                            } else {
                                let mut ext = live;
                                ext.configured = false;
                                ext.daemon_online = true;
                                let is_managed = ext.is_managed
                                    || crate::docker::is_tailery_container(
                                        &ext.name,
                                        &ext.labels,
                                        None,
                                        &req.servers,
                                    );
                                ext.is_managed = is_managed;
                                all_list.push(ext);
                            }
                        }
                    }
                }
            }

            let filtered_list: Vec<ContainerStatusInfo> = match req.filter_mode {
                ContainerFilterMode::ManagedOnly => all_list
                    .into_iter()
                    .filter(|c| c.configured || c.is_managed)
                    .collect(),
                ContainerFilterMode::All => all_list,
            };

            let mut logs = Vec::new();
            if daemon_online {
                if let Some(ref d) = docker {
                    if let Some(ref selected_name) = req.selected_item_name {
                        let matched_container = filtered_list.iter().find(|c| {
                            c.name == *selected_name
                                || c.name.trim_start_matches('/') == selected_name.trim_start_matches('/')
                        });
                        if let Some(c) = matched_container {
                            if c.running && c.id != "-" {
                                if let Ok(fetched_logs) = d.fetch_container_logs(&c.id, 50).await {
                                    logs = fetched_logs;
                                }
                            }
                        }
                    }
                }
            }

            let _ = res_tx.send(DockerPollResult {
                docker_status,
                containers: filtered_list,
                logs,
            });
        }
    });
}

pub struct App {
    #[allow(dead_code)]
    config: Config,
    app_state: AppState,
    config_path: PathBuf,
    tick_rate: f64,
    frame_rate: f64,
    should_quit: bool,
    should_suspend: bool,
    mode: Mode,
    active_modal: ActiveModal,
    last_tick_key_events: Vec<KeyEvent>,
    action_tx: mpsc::UnboundedSender<Action>,
    action_rx: mpsc::UnboundedReceiver<Action>,

    docker: Option<DockerManager>,
    docker_poller_tx: mpsc::UnboundedSender<DockerPollRequest>,
    docker_poller_rx: mpsc::UnboundedReceiver<DockerPollResult>,

    diff_tx: mpsc::UnboundedSender<(usize, CachedClientDiff)>,
    diff_rx: mpsc::UnboundedReceiver<(usize, CachedClientDiff)>,

    telemetry_rx: Option<mpsc::UnboundedReceiver<TelemetryMessage>>,
    status_message: Option<(String, Instant)>,

    sidebar: Sidebar,
    clients: Clients,
    mcps: Mcps,
    containers: Containers,
    skills: Skills,
    mcp_browser: McpBrowser,
    profile_switcher: ProfileSwitcher,
    profile_editor: ProfileEditor,
    wizard: Wizard,
    diff_viewer: DiffViewer,
    inspector: Inspector,
    server_browser: ServerBrowser,
    greeting: Greeting,
    help: Help,
    sync_confirm: SyncConfirm,
}

pub fn start_telemetry_listener(socket_path: String, tx: mpsc::UnboundedSender<TelemetryMessage>) {
    tokio::spawn(async move {
        let _ = std::fs::remove_file(&socket_path);
        if let Ok(listener) = UnixListener::bind(&socket_path) {
            loop {
                if let Ok((stream, _)) = listener.accept().await {
                    let tx_clone = tx.clone();
                    tokio::spawn(async move {
                        let mut lines = BufReader::new(stream).lines();
                        while let Ok(Some(line)) = lines.next_line().await {
                            if let Ok(msg) = serde_json::from_str::<TelemetryMessage>(&line) {
                                let _ = tx_clone.send(msg);
                            }
                        }
                    });
                }
            }
        }
    });
}

impl App {
    pub fn new(tick_rate: f64, frame_rate: f64) -> color_eyre::Result<Self> {
        let (action_tx, action_rx) = mpsc::unbounded_channel();
        let (docker_poller_tx, _docker_req_rx) = mpsc::unbounded_channel::<DockerPollRequest>();
        let (_docker_res_tx, docker_poller_rx) = mpsc::unbounded_channel::<DockerPollResult>();
        let (diff_tx, diff_rx) = mpsc::unbounded_channel();
        let config_path = resolve_config_path(None);

        let app_state = if let Ok(content) = std::fs::read_to_string(&config_path) {
            serde_json::from_str(&content).unwrap_or_else(|_| AppState {
                version: "1.0.0".into(),
                settings: crate::state::GlobalSettings {
                    active_profile: "default".into(),
                    docker_socket: None,
                    sync_clients: vec![],
                    filter_managed_containers_only: None,
                },
                servers: Default::default(),
                configured_containers: Default::default(),
                profiles: Default::default(),
                workspaces: Default::default(),
                docker_status: String::new(),
                containers: Vec::new(),
                container_logs: Vec::new(),
                inspector_events: Vec::new(),
                managed_servers: Default::default(),
            })
        } else {
            AppState {
                version: "1.0.0".into(),
                settings: crate::state::GlobalSettings {
                    active_profile: "default".into(),
                    docker_socket: None,
                    sync_clients: vec![],
                    filter_managed_containers_only: None,
                },
                servers: Default::default(),
                configured_containers: Default::default(),
                profiles: Default::default(),
                workspaces: Default::default(),
                docker_status: String::new(),
                containers: Vec::new(),
                container_logs: Vec::new(),
                inspector_events: Vec::new(),
                managed_servers: Default::default(),
            }
        };

        let filter_mode = if app_state.settings.filter_managed_containers_only == Some(false) {
            ContainerFilterMode::All
        } else {
            ContainerFilterMode::ManagedOnly
        };

        Ok(Self {
            tick_rate,
            frame_rate,
            should_quit: false,
            should_suspend: false,
            config: Config::new()?,
            app_state,
            config_path,
            mode: Mode::Clients,
            active_modal: ActiveModal::None,
            last_tick_key_events: Vec::new(),
            action_tx,
            action_rx,

            docker: None,
            docker_poller_tx,
            docker_poller_rx,

            diff_tx,
            diff_rx,

            telemetry_rx: None,
            status_message: None,

            sidebar: Sidebar::default(),
            clients: Clients::default(),
            mcps: Mcps {
                selected_index: 0,
                bottom_mode: crate::components::mcps::McpBottomPaneMode::Logs,
                logs_scroll_offset: 0,
                inspector_selected_index: 0,
                inspector_scroll_offset: 0,
                filter_mode,
                command_tx: None,
            },
            containers: Containers::default(),
            skills: Skills::default(),
            mcp_browser: McpBrowser::default(),
            profile_switcher: ProfileSwitcher::default(),
            profile_editor: ProfileEditor::default(),
            wizard: Wizard::default(),
            diff_viewer: DiffViewer::default(),
            inspector: Inspector::default(),
            server_browser: ServerBrowser::default(),
            greeting: Greeting::default(),
            help: Help::default(),
            sync_confirm: SyncConfirm::default(),
        })
    }

    pub fn trigger_diff_recompute(&mut self, adapter_index: usize) {
        if let Some(entry) = self.clients.diff_cache.get_mut(&adapter_index) {
            entry.is_loading = true;
        } else {
            self.clients.diff_cache.insert(
                adapter_index,
                CachedClientDiff {
                    is_loading: true,
                    ..Default::default()
                },
            );
        }
        let state_clone = self.app_state.clone();
        let tx = self.diff_tx.clone();
        tokio::task::spawn_blocking(move || {
            let diff = compute_client_diff_full(&state_clone, adapter_index);
            let _ = tx.send((adapter_index, diff));
        });
    }

    pub fn refresh_all_client_diffs(&mut self) {
        let count = all_adapters().len();
        for i in 0..count {
            self.trigger_diff_recompute(i);
        }
    }

    pub fn trigger_docker_poll(&self) {
        let items = Mcps::build_items(&self.app_state);
        let selected_item_name = items.get(self.mcps.selected_index).map(|i| i.name.clone());
        let _ = self.docker_poller_tx.send(DockerPollRequest {
            servers: self.app_state.servers.clone(),
            configured_containers: self.app_state.configured_containers.clone(),
            filter_mode: self.mcps.filter_mode,
            selected_item_name,
        });
    }

    pub fn select_client_tab(&mut self, new_idx: usize) {
        let count = all_adapters().len();
        if count == 0 {
            return;
        }
        let target = new_idx % count;
        if self.clients.selected_adapter_index != target {
            self.clients.selected_adapter_index = target;
            self.clients.diff_scroll_offset = 0;
            self.clients.search_match_index = 0;
            // Switch first! Then compute/refresh in background
            self.trigger_diff_recompute(target);
        }
    }

    pub fn import_discovered_mcp(&mut self, mcp: crate::adapters::DiscoveredMcp) {
        let active_prof = self.app_state.settings.active_profile.clone();
        self.app_state.servers.insert(mcp.name.clone(), mcp.config.clone());
        let profile = self
            .app_state
            .profiles
            .entry(active_prof.clone())
            .or_insert_with(crate::state::ProfileConfig::default);
        if !profile.enabled_servers.contains(&mcp.name) {
            profile.enabled_servers.push(mcp.name.clone());
        }
        self.save_config();
        self.set_status(format!(
            "✔ Imported '{}' into Tailery and enabled in profile '{}'!",
            mcp.name, active_prof
        ));
    }

    pub fn save_config(&mut self) {
        if let Err(e) = save_config_to_path(&self.app_state, &self.config_path) {
            self.set_status(format!("✖ Failed to save config to disk: {}", e));
        } else {
            self.refresh_all_client_diffs();
            self.trigger_docker_poll();
        }
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status_message = Some((msg.into(), Instant::now()));
    }

    pub fn next_view(&mut self) {
        self.mode = match self.mode {
            Mode::Clients => Mode::Mcps,
            Mode::Mcps => Mode::Skills,
            Mode::Skills => Mode::Clients,
        };
        self.sidebar.active_view = match self.mode {
            Mode::Clients => crate::components::sidebar::ActiveView::Clients,
            Mode::Mcps => crate::components::sidebar::ActiveView::Mcps,
            Mode::Skills => crate::components::sidebar::ActiveView::Skills,
        };
        if self.mode == Mode::Clients {
            self.refresh_all_client_diffs();
        } else if self.mode == Mode::Mcps {
            self.trigger_docker_poll();
        }
    }

    pub fn previous_view(&mut self) {
        self.mode = match self.mode {
            Mode::Clients => Mode::Skills,
            Mode::Mcps => Mode::Clients,
            Mode::Skills => Mode::Mcps,
        };
        self.sidebar.active_view = match self.mode {
            Mode::Clients => crate::components::sidebar::ActiveView::Clients,
            Mode::Mcps => crate::components::sidebar::ActiveView::Mcps,
            Mode::Skills => crate::components::sidebar::ActiveView::Skills,
        };
        if self.mode == Mode::Clients {
            self.refresh_all_client_diffs();
        } else if self.mode == Mode::Mcps {
            self.trigger_docker_poll();
        }
    }

    pub fn open_mcp_browser_modal(&mut self) {
        self.mcp_browser.selected_index = 0;
        self.mcp_browser.search_query.clear();
        self.mcp_browser.search_focused = false;
        self.active_modal = ActiveModal::McpBrowser;
    }

    pub fn open_profile_switcher_modal(&mut self) {
        let mut names: Vec<String> = self.app_state.profiles.keys().cloned().collect();
        names.sort();
        let curr = &self.app_state.settings.active_profile;
        let selected_idx = names.iter().position(|p| p == curr).unwrap_or(0);
        self.profile_switcher.selected_index = selected_idx;
        self.active_modal = ActiveModal::ProfileSwitcher;
    }

    pub fn open_profile_editor_modal(&mut self, profile_name: &str) {
        self.profile_editor = ProfileEditor::new(profile_name);
        self.active_modal = ActiveModal::ProfileEditor;
    }

    pub fn open_new_profile_modal(&mut self) {
        let active = self.app_state.settings.active_profile.clone();
        self.wizard = Wizard::new(
            crate::components::wizard::WizardType::Profile(crate::components::wizard::NewProfileWizard::new(&active)),
            "Create New MCP Profile",
        );
        self.active_modal = ActiveModal::Wizard;
    }

    pub fn open_new_server_modal(&mut self) {
        let active = self.app_state.settings.active_profile.clone();
        self.wizard = Wizard::new(
            crate::components::wizard::WizardType::Server(crate::components::wizard::NewServerWizard::new(&active)),
            "Add MCP Server / Container",
        );
        self.active_modal = ActiveModal::Wizard;
    }

    pub fn open_sync_confirmation_modal(&mut self) {
        self.active_modal = ActiveModal::SyncConfirm;
    }

    pub fn open_help_modal(&mut self) {
        self.active_modal = ActiveModal::Help;
    }

    pub fn switch_profile(&mut self, profile_name: &str) {
        self.app_state.settings.active_profile = profile_name.to_string();
        self.save_config();
        self.set_status(format!("✔ Switched active profile to '{}'", profile_name));
    }

    pub fn delete_profile(&mut self, profile_name: &str) {
        if self.app_state.profiles.len() <= 1 {
            self.set_status("✖ Cannot delete the only profile.");
            return;
        }

        self.app_state.profiles.remove(profile_name);
        if self.app_state.settings.active_profile == profile_name {
            if let Some(first) = self.app_state.profiles.keys().next() {
                self.app_state.settings.active_profile = first.clone();
            } else {
                self.app_state.settings.active_profile = "default".to_string();
            }
        }
        self.save_config();
        self.set_status(format!("✔ Deleted profile '{}'", profile_name));
    }

    pub fn install_registry_entry(&mut self, entry: &crate::components::mcp_browser::RegistryMcpEntry) {
        let active_prof = self.app_state.settings.active_profile.clone();
        self.app_state
            .servers
            .insert(entry.name.to_string(), entry.default_config.clone());

        let profile = self
            .app_state
            .profiles
            .entry(active_prof.clone())
            .or_insert_with(ProfileConfig::default);
        if !profile.enabled_servers.contains(&entry.name.to_string()) {
            profile.enabled_servers.push(entry.name.to_string());
        }

        self.save_config();
        self.set_status(format!(
            "✔ Installed & enabled '{}' in profile '{}'!",
            entry.name, active_prof
        ));
    }

    pub fn sync_all_clients(&mut self) {
        let mut synced = 0;
        let active_prof = self.app_state.settings.active_profile.clone();
        let servers = self.app_state.get_active_profile_servers();

        for adapter in all_adapters() {
            if !self
                .app_state
                .is_client_enabled_in_active_profile(adapter.name())
            {
                continue;
            }
            if let Ok(path) = adapter.config_path(None) {
                if adapter.write_servers(&active_prof, &path, &servers).is_ok() {
                    synced += 1;
                }
            }
        }
        self.refresh_all_client_diffs();
        self.set_status(format!(
            "✔ Synced MCP configuration to {} enabled client(s) in profile '{}'",
            synced, active_prof
        ));
    }

    pub fn toggle_current_client_for_profile(&mut self) {
        let adapters = all_adapters();
        let active_prof = self.app_state.settings.active_profile.clone();
        if let Some(adapter) = adapters.get(self.clients.selected_adapter_index % adapters.len()) {
            let client_name = adapter.name();
            let profile = self
                .app_state
                .profiles
                .entry(active_prof.clone())
                .or_insert_with(|| ProfileConfig {
                    enabled_servers: Vec::new(),
                    enabled_clients: crate::state::default_enabled_clients(), include_project_mcps: false });
            let is_enabled = profile.toggle_client(client_name);
            self.save_config();
            if is_enabled {
                self.set_status(format!(
                    "✔ ENABLED {} for profile '{}'",
                    adapter.display_name(),
                    active_prof
                ));
            } else {
                self.set_status(format!(
                    "○ DISABLED {} for profile '{}'",
                    adapter.display_name(),
                    active_prof
                ));
            }
        }
    }

    pub fn disable_current_client_for_profile(&mut self) {
        let adapters = all_adapters();
        let active_prof = self.app_state.settings.active_profile.clone();
        if let Some(adapter) = adapters.get(self.clients.selected_adapter_index % adapters.len()) {
            let client_name = adapter.name();
            let profile = self
                .app_state
                .profiles
                .entry(active_prof.clone())
                .or_insert_with(|| ProfileConfig {
                    enabled_servers: Vec::new(),
                    enabled_clients: crate::state::default_enabled_clients(), include_project_mcps: false });
            profile.disable_client(client_name);
            self.save_config();
            self.set_status(format!(
                "○ DISABLED {} for profile '{}'",
                adapter.display_name(),
                active_prof
            ));
        }
    }

    pub fn toggle_current_server_for_profile(&mut self) {
        let items = Mcps::build_items(&self.app_state);
        if let Some(item) = items.get(self.mcps.selected_index) {
            let name = item.name.clone();
            let active_prof = self.app_state.settings.active_profile.clone();
            let profile = self
                .app_state
                .profiles
                .entry(active_prof.clone())
                .or_insert_with(|| ProfileConfig {
                    enabled_servers: Vec::new(),
                    enabled_clients: crate::state::default_enabled_clients(), include_project_mcps: false });
            let is_enabled = profile.toggle_server(&name);
            self.save_config();
            if is_enabled {
                self.set_status(format!(
                    "✔ ENABLED MCP server '{}' for profile '{}'",
                    name, active_prof
                ));
            } else {
                self.set_status(format!(
                    "○ DISABLED MCP server '{}' for profile '{}'",
                    name, active_prof
                ));
            }
        }
    }

    pub fn backup_current_client(&mut self) {
        let adapters = all_adapters();
        let active_prof = self.app_state.settings.active_profile.clone();
        if let Some(adapter) = adapters.get(self.clients.selected_adapter_index % adapters.len()) {
            if let Ok(path) = adapter.config_path(None) {
                if path.exists() {
                    match crate::backup::create_backup(&active_prof, adapter.name(), &path) {
                        Ok(Some(entry)) => {
                            self.trigger_diff_recompute(self.clients.selected_adapter_index);
                            self.set_status(format!(
                                "✔ Backed up {} for profile '{}' (ID: {}, 10 max retained)",
                                adapter.display_name(),
                                active_prof,
                                entry.id
                            ));
                        }
                        Ok(None) => self.set_status(format!(
                            "○ {} config file does not exist",
                            adapter.display_name()
                        )),
                        Err(e) => self.set_status(format!("✖ Backup failed: {}", e)),
                    }
                } else {
                    self.set_status(format!(
                        "○ {} config file does not exist on disk",
                        adapter.display_name()
                    ));
                }
            }
        }
    }

    pub fn restore_current_client_latest(&mut self) {
        let adapters = all_adapters();
        let active_prof = self.app_state.settings.active_profile.clone();
        if let Some(adapter) = adapters.get(self.clients.selected_adapter_index % adapters.len()) {
            match crate::backup::restore_latest(&active_prof, adapter.name(), None) {
                Ok(entry) => {
                    self.trigger_diff_recompute(self.clients.selected_adapter_index);
                    self.set_status(format!(
                        "✔ Restored latest backup ({}) for {} in profile '{}'",
                        entry.id,
                        adapter.display_name(),
                        active_prof
                    ));
                }
                Err(e) => self.set_status(format!("✖ Restore failed: {}", e)),
            }
        }
    }

    pub fn get_diff_search_matches(&self) -> Vec<usize> {
        if self.clients.search_query.is_empty() {
            return Vec::new();
        }
        let q_lower = self.clients.search_query.to_lowercase();
        if let Some(cached) = self.clients.diff_cache.get(&self.clients.selected_adapter_index) {
            cached
                .changes
                .iter()
                .enumerate()
                .filter(|(_, (_, text))| text.to_lowercase().contains(&q_lower))
                .map(|(idx, _)| idx)
                .collect()
        } else {
            Vec::new()
        }
    }

    pub fn next_diff_search_match(&mut self) {
        let matches = self.get_diff_search_matches();
        if matches.is_empty() {
            if !self.clients.search_query.is_empty() {
                self.set_status(format!("Pattern not found: \"{}\"", self.clients.search_query));
            }
            return;
        }
        self.clients.search_match_index = (self.clients.search_match_index + 1) % matches.len();
        let target_line = matches[self.clients.search_match_index];
        self.clients.diff_scroll_offset = target_line.saturating_sub(3);
        self.set_status(format!(
            "/{} [{}/{}]",
            self.clients.search_query,
            self.clients.search_match_index + 1,
            matches.len()
        ));
    }

    pub fn prev_diff_search_match(&mut self) {
        let matches = self.get_diff_search_matches();
        if matches.is_empty() {
            if !self.clients.search_query.is_empty() {
                self.set_status(format!("Pattern not found: \"{}\"", self.clients.search_query));
            }
            return;
        }
        self.clients.search_match_index = if self.clients.search_match_index == 0 {
            matches.len() - 1
        } else {
            self.clients.search_match_index - 1
        };
        let target_line = matches[self.clients.search_match_index];
        self.clients.diff_scroll_offset = target_line.saturating_sub(3);
        self.set_status(format!(
            "/{} [{}/{}]",
            self.clients.search_query,
            self.clients.search_match_index + 1,
            matches.len()
        ));
    }

    pub fn toggle_container_filter(&mut self) {
        self.mcps.filter_mode = match self.mcps.filter_mode {
            ContainerFilterMode::ManagedOnly => ContainerFilterMode::All,
            ContainerFilterMode::All => ContainerFilterMode::ManagedOnly,
        };
        let mode_str = match self.mcps.filter_mode {
            ContainerFilterMode::ManagedOnly => "Managed only",
            ContainerFilterMode::All => "All system containers",
        };
        self.set_status(format!("Container view: {}", mode_str));
        self.trigger_docker_poll();
    }

    pub fn register_selected_container_as_mcp(&mut self) {
        let items = Mcps::build_items(&self.app_state);
        if let Some(item) = items.get(self.mcps.selected_index) {
            let matched_container = self
                .app_state
                .containers
                .iter()
                .find(|c| {
                    c.name == item.name
                        || c.name.trim_start_matches('/') == item.name.trim_start_matches('/')
                })
                .cloned();

            if let Some(c) = matched_container {
                let active_prof = self.app_state.settings.active_profile.clone();
                let host_port = c
                    .ports
                    .first()
                    .and_then(|p| p.split(':').next())
                    .unwrap_or("3000");
                let url = format!("http://localhost:{}", host_port);

                let server_name = c.name.clone();
                self.app_state.servers.insert(
                    server_name.clone(),
                    ServerConfig::Remote {
                        url,
                        transport: crate::state::RemoteTransport::StreamableHttp,
                        headers: HashMap::new(),
                        env: HashMap::new(),
                        tool_filter: crate::state::ToolFilter::default(),
                        shim_port: None,
                    },
                );

                let profile = self
                    .app_state
                    .profiles
                    .entry(active_prof.clone())
                    .or_insert_with(ProfileConfig::default);
                if !profile.enabled_servers.contains(&server_name) {
                    profile.enabled_servers.push(server_name.clone());
                }

                self.save_config();
                self.set_status(format!(
                    "✔ Auto-registered container '{}' as MCP server in profile '{}'!",
                    server_name, active_prof
                ));
            } else {
                self.set_status(format!(
                    "Item '{}' is not a container with published ports",
                    item.name
                ));
            }
        }
    }

    pub async fn run(&mut self) -> color_eyre::Result<()> {
        let (docker, daemon_info) =
            DockerManager::auto_detect(self.app_state.settings.docker_socket.as_deref()).await;

        if let (Some(d), Some(info)) = (&docker, &daemon_info) {
            if d.ping().await {
                self.app_state.docker_status = format!("Online ({})", info.name);
            } else {
                self.app_state.docker_status = format!("Offline ({})", info.name);
            }
        } else if let Some(info) = daemon_info.as_ref() {
            self.app_state.docker_status = format!("Offline ({})", info.name);
        } else {
            self.app_state.docker_status = "Offline (No socket)".to_string();
        }
        self.docker = docker.clone();

        let (docker_req_tx, docker_req_rx) = mpsc::unbounded_channel::<DockerPollRequest>();
        let (docker_res_tx, docker_res_rx) = mpsc::unbounded_channel::<DockerPollResult>();
        self.docker_poller_tx = docker_req_tx;
        self.docker_poller_rx = docker_res_rx;
        start_docker_poller(docker, docker_req_rx, docker_res_tx);

        let (telemetry_tx, telemetry_rx) = mpsc::unbounded_channel::<TelemetryMessage>();
        start_telemetry_listener(crate::shim::DEFAULT_SOCKET_PATH.to_string(), telemetry_tx);
        self.telemetry_rx = Some(telemetry_rx);

        // Pre-cache client detection & pre-compute all diffs in parallel
        for (i, a) in all_adapters().iter().enumerate() {
            self.clients.installed_cache.insert(i, a.detect_installed());
        }
        self.refresh_all_client_diffs();
        self.trigger_docker_poll();

        let mut tui = Tui::new()?
            .tick_rate(self.tick_rate)
            .frame_rate(self.frame_rate)
            .mouse(true);
        tui.enter()?;

        // Register action handler tx
        self.sidebar.register_action_handler(self.action_tx.clone())?;
        self.clients.register_action_handler(self.action_tx.clone())?;
        self.mcps.register_action_handler(self.action_tx.clone())?;
        self.containers.register_action_handler(self.action_tx.clone())?;
        self.skills.register_action_handler(self.action_tx.clone())?;
        self.mcp_browser.register_action_handler(self.action_tx.clone())?;
        self.profile_switcher.register_action_handler(self.action_tx.clone())?;
        self.profile_editor.register_action_handler(self.action_tx.clone())?;
        self.wizard.register_action_handler(self.action_tx.clone())?;
        self.diff_viewer.register_action_handler(self.action_tx.clone())?;
        self.inspector.register_action_handler(self.action_tx.clone())?;
        self.server_browser.register_action_handler(self.action_tx.clone())?;
        self.greeting.register_action_handler(self.action_tx.clone())?;
        self.help.register_action_handler(self.action_tx.clone())?;
        self.sync_confirm.register_action_handler(self.action_tx.clone())?;

        let action_tx = self.action_tx.clone();

        loop {
            // 1. Drain background docker poll results non-blockingly
            while let Ok(res) = self.docker_poller_rx.try_recv() {
                self.app_state.docker_status = res.docker_status;
                self.app_state.containers = res.containers;
                self.app_state.container_logs = res.logs;
            }

            let mut diff_updated = false;
            // 2. Drain background diff results non-blockingly
            while let Ok((idx, diff)) = self.diff_rx.try_recv() {
                self.clients.diff_cache.insert(idx, diff);
                diff_updated = true;
            }

            // 3. Drain background telemetry non-blockingly
            if let Some(ref mut rx) = self.telemetry_rx {
                while let Ok(msg) = rx.try_recv() {
                    self.app_state.inspector_events.insert(0, msg);
                    if self.app_state.inspector_events.len() > 1000 {
                        self.app_state.inspector_events.pop();
                    }
                }
            }

            // 4. Expire status message after 4s
            if let Some((_, created_at)) = self.status_message {
                if created_at.elapsed() > Duration::from_secs(4) {
                    self.status_message = None;
                }
            }

            // 5. High-throughput event handling with burst draining:
            let Some(first_event) = tui.next_event().await else {
                break;
            };

            let mut pending_events = vec![first_event];
            while let Some(extra_event) = tui.try_next_event() {
                pending_events.push(extra_event);
            }

            let mut needs_render = diff_updated;
            for event in pending_events {
                match event {
                    Event::Quit => {
                        self.should_quit = true;
                        break;
                    }
                    Event::Tick => {
                        self.last_tick_key_events.drain(..);
                    }
                    Event::Render => {
                        needs_render = true;
                    }
                    Event::Resize(w, h) => {
                        self.handle_resize(&mut tui, w, h)?;
                        needs_render = false;
                    }
                    Event::Key(key) => {
                        self.handle_key_event(key).await?;
                        needs_render = true;
                    }
                    Event::Mouse(mouse) => {
                        let size = tui.size()?;
                        self.handle_mouse_event(mouse, Rect::new(0, 0, size.width, size.height)).await?;
                        needs_render = true;
                    }
                    _ => {}
                }
            }

            if self.should_quit {
                tui.stop()?;
                break;
            }

            if needs_render {
                self.render(&mut tui)?;
            }

            self.handle_actions(&mut tui)?;
            if self.should_suspend {
                tui.suspend()?;
                action_tx.send(Action::Resume)?;
                action_tx.send(Action::ClearScreen)?;
                tui.enter()?;
            } else if self.should_quit {
                tui.stop()?;
                break;
            }
        }
        tui.exit()?;
        Ok(())
    }

    async fn handle_key_event(&mut self, key: KeyEvent) -> color_eyre::Result<()> {
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }
        let docker = self.docker.clone();

        match self.active_modal {
            ActiveModal::Help => match key.code {
                KeyCode::Esc
                | KeyCode::Enter
                | KeyCode::Char('?')
                | KeyCode::Char('q')
                | KeyCode::Char(' ') => {
                    self.active_modal = ActiveModal::None;
                }
                _ => {}
            },
            ActiveModal::McpBrowser => {
                let registry_entries = crate::components::mcp_browser::get_mcp_registry_entries();
                if self.mcp_browser.search_focused {
                    match key.code {
                        KeyCode::Esc | KeyCode::Enter | KeyCode::Tab => {
                            self.mcp_browser.search_focused = false;
                        }
                        KeyCode::Backspace => {
                            self.mcp_browser.search_query.pop();
                            self.mcp_browser.selected_index = 0;
                        }
                        KeyCode::Char(c) => {
                            self.mcp_browser.search_query.push(c);
                            self.mcp_browser.selected_index = 0;
                        }
                        _ => {}
                    }
                } else {
                    match key.code {
                        KeyCode::Esc => {
                            if !self.mcp_browser.search_query.is_empty() {
                                self.mcp_browser.search_query.clear();
                                self.mcp_browser.selected_index = 0;
                            } else {
                                self.active_modal = ActiveModal::None;
                            }
                        }
                        KeyCode::Char('/') | KeyCode::Tab => {
                            self.mcp_browser.search_focused = true;
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            if self.mcp_browser.selected_index > 0 {
                                self.mcp_browser.selected_index -= 1;
                            }
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            let filtered_count = if self.mcp_browser.search_query.is_empty() {
                                registry_entries.len()
                            } else {
                                registry_entries
                                    .iter()
                                    .filter(|e| e.matches_query(&self.mcp_browser.search_query))
                                    .count()
                            };
                            if self.mcp_browser.selected_index + 1 < filtered_count {
                                self.mcp_browser.selected_index += 1;
                            }
                        }
                        KeyCode::Backspace => {
                            if !self.mcp_browser.search_query.is_empty() {
                                self.mcp_browser.search_query.pop();
                                self.mcp_browser.selected_index = 0;
                            }
                        }
                        KeyCode::Enter | KeyCode::Char('a') => {
                            let filtered: Vec<crate::components::mcp_browser::RegistryMcpEntry> =
                                if self.mcp_browser.search_query.is_empty() {
                                    registry_entries.clone()
                                } else {
                                    registry_entries
                                        .iter()
                                        .filter(|e| e.matches_query(&self.mcp_browser.search_query))
                                        .cloned()
                                        .collect()
                                };
                            if let Some(entry) = filtered.get(self.mcp_browser.selected_index) {
                                self.install_registry_entry(entry);
                                self.active_modal = ActiveModal::None;
                            }
                        }
                        _ => {}
                    }
                }
            }
            ActiveModal::ProfileSwitcher => {
                let mut profiles: Vec<String> = self.app_state.profiles.keys().cloned().collect();
                profiles.sort();
                match key.code {
                    KeyCode::Esc => {
                        self.active_modal = ActiveModal::None;
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        if self.profile_switcher.selected_index > 0 {
                            self.profile_switcher.selected_index -= 1;
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if self.profile_switcher.selected_index + 1 < profiles.len() {
                            self.profile_switcher.selected_index += 1;
                        }
                    }
                    KeyCode::Enter => {
                        if let Some(p) = profiles.get(self.profile_switcher.selected_index) {
                            let prof = p.clone();
                            self.switch_profile(&prof);
                        }
                        self.active_modal = ActiveModal::None;
                    }
                    KeyCode::Char('e') => {
                        if let Some(p) = profiles.get(self.profile_switcher.selected_index) {
                            let prof = p.clone();
                            self.open_profile_editor_modal(&prof);
                        }
                    }
                    KeyCode::Char('n') => {
                        self.open_new_profile_modal();
                    }
                    KeyCode::Char('d') => {
                        if let Some(p) = profiles.get(self.profile_switcher.selected_index) {
                            let prof = p.clone();
                            self.delete_profile(&prof);
                        }
                        self.active_modal = ActiveModal::None;
                    }
                    _ => {}
                }
            }
            ActiveModal::ProfileEditor => {
                let mut sorted_servers: Vec<String> = self.app_state.servers.keys().cloned().collect();
                for k in self.app_state.configured_containers.keys() {
                    if !sorted_servers.contains(k) {
                        sorted_servers.push(k.clone());
                    }
                }
                sorted_servers.sort();
                let total_servers = sorted_servers.len();
                let total_clients = all_adapters().len();

                match key.code {
                    KeyCode::Esc => {
                        let prof_name = self.profile_editor.profile_name.clone();
                        self.save_config();
                        self.active_modal = ActiveModal::None;
                        self.set_status(format!("✔ Saved profile '{}'", prof_name));
                    }
                    KeyCode::Tab
                    | KeyCode::Right
                    | KeyCode::Left
                    | KeyCode::Char('h')
                    | KeyCode::Char('l') => {
                        self.profile_editor.switch_pane();
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.profile_editor.move_up();
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.profile_editor.move_down(total_servers, total_clients);
                    }
                    KeyCode::Char(' ') | KeyCode::Enter => {
                        let target = self.profile_editor.current_target(&sorted_servers);
                        let prof_name = self.profile_editor.profile_name.clone();
                        if let Some(t) = target {
                            let profile = self
                                .app_state
                                .profiles
                                .entry(prof_name.clone())
                                .or_insert_with(ProfileConfig::default);
                            let status_msg = match t {
                                crate::components::profile_editor::ToggleTarget::Server(srv) => {
                                    let enabled = profile.toggle_server(&srv);
                                    let status = if enabled { "ENABLED" } else { "DISABLED" };
                                    format!(
                                        "✔ {} MCP server '{}' in profile '{}'",
                                        status, srv, prof_name
                                    )
                                }
                                crate::components::profile_editor::ToggleTarget::Client(cli) => {
                                    let enabled = profile.toggle_client(&cli);
                                    let status = if enabled { "ENABLED" } else { "DISABLED" };
                                    let display = all_adapters()
                                        .iter()
                                        .find(|a| a.name() == cli)
                                        .map(|a| a.display_name())
                                        .unwrap_or(&cli);
                                    format!(
                                        "✔ {} client '{}' in profile '{}'",
                                        status, display, prof_name
                                    )
                                }
                            };
                            self.save_config();
                            self.set_status(status_msg);
                        }
                    }
                    KeyCode::Char('a') => {
                        let pane = self.profile_editor.focused_pane;
                        let prof_name = self.profile_editor.profile_name.clone();
                        let profile = self
                            .app_state
                            .profiles
                            .entry(prof_name.clone())
                            .or_insert_with(ProfileConfig::default);
                        let status_msg = if pane == 0 {
                            let all_enabled =
                                sorted_servers.iter().all(|s| profile.is_server_enabled(s));
                            if all_enabled {
                                profile.enabled_servers.clear();
                                format!(
                                    "○ Disabled all MCP servers for profile '{}'",
                                    prof_name
                                )
                            } else {
                                for s in &sorted_servers {
                                    profile.enable_server(s);
                                }
                                format!("✔ Enabled all MCP servers for profile '{}'", prof_name)
                            }
                        } else {
                            let adapters = all_adapters();
                            let all_enabled =
                                adapters.iter().all(|a| profile.is_client_enabled(a.name()));
                            if all_enabled {
                                profile.enabled_clients.clear();
                                format!("○ Disabled all clients for profile '{}'", prof_name)
                            } else {
                                for a in &adapters {
                                    profile.enable_client(a.name());
                                }
                                format!("✔ Enabled all clients for profile '{}'", prof_name)
                            }
                        };
                        self.save_config();
                        self.set_status(status_msg);
                    }
                    _ => {}
                }
            }
            ActiveModal::Wizard => {
                match &mut self.wizard.wizard_type {
                    crate::components::wizard::WizardType::Profile(w) => match key.code {
                        KeyCode::Esc => {
                            self.active_modal = ActiveModal::None;
                            self.set_status("Profile creation cancelled.");
                        }
                        KeyCode::Backspace => {
                            w.handle_backspace();
                        }
                        KeyCode::Up => {
                            w.handle_up();
                        }
                        KeyCode::Down => {
                            w.handle_down();
                        }
                        KeyCode::Char(' ') => {
                            w.handle_space();
                        }
                        KeyCode::Char(c) => {
                            w.handle_char(c);
                        }
                        KeyCode::Enter => {
                            if let Some((name, enabled_clients, copy_servers, activate_now)) = w.submit() {
                                let enabled_servers = if copy_servers {
                                    let curr = &self.app_state.settings.active_profile;
                                    self.app_state
                                        .profiles
                                        .get(curr)
                                        .map(|p| p.enabled_servers.clone())
                                        .unwrap_or_default()
                                } else {
                                    Vec::new()
                                };

                                self.app_state.profiles.insert(
                                    name.clone(),
                                    ProfileConfig::new(enabled_servers, enabled_clients),
                                );

                                if activate_now {
                                    self.app_state.settings.active_profile = name.clone();
                                }

                                self.save_config();
                                self.active_modal = ActiveModal::None;
                                self.set_status(format!("✔ Saved & created profile '{}'!", name));
                            }
                        }
                        _ => {}
                    },
                    crate::components::wizard::WizardType::Server(w) => match key.code {
                        KeyCode::Esc => {
                            self.active_modal = ActiveModal::None;
                            self.set_status("Add MCP server cancelled.");
                        }
                        KeyCode::Backspace => {
                            w.handle_backspace();
                        }
                        KeyCode::Char(c) => {
                            w.handle_char(c);
                        }
                        KeyCode::Enter => {
                            if let Some(res) = w.submit() {
                                let active_prof = self.app_state.settings.active_profile.clone();
                                let srv_name = res.name.clone();
                                self.app_state
                                    .servers
                                    .insert(srv_name.clone(), res.server_config);

                                if res.add_to_profile {
                                    let profile = self
                                        .app_state
                                        .profiles
                                        .entry(active_prof.clone())
                                        .or_insert_with(ProfileConfig::default);
                                    if !profile.enabled_servers.contains(&srv_name) {
                                        profile.enabled_servers.push(srv_name.clone());
                                    }
                                }

                                self.save_config();
                                self.active_modal = ActiveModal::None;
                                self.set_status(format!(
                                    "✔ Saved & added MCP server '{}' to profile '{}'!",
                                    srv_name, active_prof
                                ));

                                let is_auto_start_local = match self.app_state.servers.get(&srv_name) {
                                    Some(ServerConfig::Local { container, .. }) => {
                                        container.auto_start && !container.image.is_empty()
                                    }
                                    _ => false,
                                };

                                if is_auto_start_local {
                                    if let Some(ref d) = docker {
                                        if d.ping().await {
                                            if let Some(ServerConfig::Local { container, env, .. }) =
                                                self.app_state.servers.get(&srv_name).cloned()
                                            {
                                                self.set_status(format!("⏳ Launching container '{}'...", srv_name));
                                                match d.create_sandbox_container(&srv_name, &container, &env, None).await {
                                                    Ok(cid) => {
                                                        let _ = d.start_container(&cid).await;
                                                        self.set_status(format!("✔ Container '{}' saved & started in Docker!", srv_name));
                                                    }
                                                    Err(e) => {
                                                        self.set_status(format!("✔ Saved to config, but Docker launch failed: {}", e));
                                                    }
                                                }
                                                self.trigger_docker_poll();
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    },
                }
            }
            ActiveModal::SyncConfirm => match key.code {
                KeyCode::Enter
                | KeyCode::Char('y')
                | KeyCode::Char('Y')
                | KeyCode::Char('s') => {
                    self.active_modal = ActiveModal::None;
                    self.sync_all_clients();
                }
                KeyCode::Esc
                | KeyCode::Char('n')
                | KeyCode::Char('N')
                | KeyCode::Char('q') => {
                    self.active_modal = ActiveModal::None;
                    self.set_status("○ Client synchronization cancelled.");
                }
                _ => {}
            },
            ActiveModal::None => {
                if self.mode == Mode::Clients && self.clients.search_active {
                    match key.code {
                        KeyCode::Esc => {
                            self.clients.search_active = false;
                        }
                        KeyCode::Backspace => {
                            self.clients.search_query.pop();
                            self.clients.search_match_index = 0;
                        }
                        KeyCode::Enter => {
                            self.clients.search_active = false;
                            let matches = self.get_diff_search_matches();
                            if !matches.is_empty() {
                                let target_idx = matches
                                    .iter()
                                    .position(|&line| line >= self.clients.diff_scroll_offset)
                                    .unwrap_or(0);
                                self.clients.search_match_index = target_idx;
                                self.clients.diff_scroll_offset = matches[target_idx].saturating_sub(3);
                                self.set_status(format!(
                                    "/{} [{}/{}]",
                                    self.clients.search_query,
                                    target_idx + 1,
                                    matches.len()
                                ));
                            } else if !self.clients.search_query.is_empty() {
                                self.set_status(format!(
                                    "Pattern not found: \"{}\"",
                                    self.clients.search_query
                                ));
                            }
                        }
                        KeyCode::Char(c) => {
                            self.clients.search_query.push(c);
                            self.clients.search_match_index = 0;
                        }
                        _ => {}
                    }
                } else {
                    match key.code {
                        KeyCode::Char('q') => {
                            self.should_quit = true;
                            return Ok(());
                        }
                        KeyCode::Esc => {
                            if self.mode == Mode::Clients && !self.clients.search_query.is_empty() {
                                self.clients.search_query.clear();
                                self.clients.search_match_index = 0;
                                self.set_status("Search cleared.");
                            }
                        }
                        KeyCode::Tab => {
                            self.clients.search_active = false;
                            if self.mode == Mode::Clients {
                                self.clients.switch_pane();
                            } else {
                                self.next_view();
                            }
                        }
                        KeyCode::BackTab => {
                            self.clients.search_active = false;
                            self.previous_view();
                        }
                        KeyCode::Enter => {
                            if self.mode == Mode::Clients && self.clients.focused_pane == 0 {
                                if let Some(cached) = self.clients.diff_cache.get(&self.clients.selected_adapter_index) {
                                    if let Some(mcp) = cached.discovered_mcps.get(self.clients.mcp_selected_index).cloned() {
                                        self.import_discovered_mcp(mcp);
                                    }
                                }
                            }
                        }
                        KeyCode::Char('1') => {
                            self.mode = Mode::Clients;
                            self.sidebar.active_view = crate::components::sidebar::ActiveView::Clients;
                            self.clients.search_active = false;
                            self.refresh_all_client_diffs();
                        }
                        KeyCode::Char('r') => {
                            if self.mode == Mode::Clients {
                                self.refresh_all_client_diffs();
                                self.set_status("⏳ Refreshing client diffs from disk...");
                            }
                        }
                        KeyCode::Char('2') => {
                            self.mode = Mode::Mcps;
                            self.sidebar.active_view = crate::components::sidebar::ActiveView::Mcps;
                            self.clients.search_active = false;
                            self.trigger_docker_poll();
                        }
                        KeyCode::Char('3') => {
                            self.mode = Mode::Skills;
                            self.sidebar.active_view = crate::components::sidebar::ActiveView::Skills;
                            self.clients.search_active = false;
                        }
                        KeyCode::Char('/') => {
                            if self.mode == Mode::Clients {
                                self.clients.search_active = true;
                            }
                        }
                        KeyCode::Char('n') => {
                            if self.mode == Mode::Clients {
                                self.next_diff_search_match();
                            } else {
                                self.open_new_server_modal();
                            }
                        }
                        KeyCode::Char('N') => {
                            if self.mode == Mode::Clients {
                                self.prev_diff_search_match();
                            }
                        }
                        KeyCode::Char('?') => self.open_help_modal(),
                        KeyCode::Char('b') => self.open_mcp_browser_modal(),
                        KeyCode::Char('p') => self.open_profile_switcher_modal(),
                        KeyCode::Char('a') => self.open_new_server_modal(),
                        KeyCode::Char('i') => {
                            if self.mode == Mode::Clients {
                                if let Some(cached) = self.clients.diff_cache.get(&self.clients.selected_adapter_index) {
                                    if let Some(mcp) = cached.discovered_mcps.get(self.clients.mcp_selected_index).cloned() {
                                        self.import_discovered_mcp(mcp);
                                    }
                                }
                            } else if self.mode == Mode::Mcps {
                                self.mcps.bottom_mode = match self.mcps.bottom_mode {
                                    crate::components::mcps::McpBottomPaneMode::Logs => {
                                        crate::components::mcps::McpBottomPaneMode::Inspector
                                    }
                                    crate::components::mcps::McpBottomPaneMode::Inspector => {
                                        crate::components::mcps::McpBottomPaneMode::Logs
                                    }
                                };
                            }
                        }
                        KeyCode::Char('s') => {
                            if self.mode == Mode::Mcps {
                                let items = Mcps::build_items(&self.app_state);
                                if let Some(item) = items.get(self.mcps.selected_index) {
                                    let item_name = item.name.clone();
                                    let is_container = item.is_container;
                                    let server_cfg = self.app_state.servers.get(&item_name).cloned();

                                    if let Some(ref d) = docker {
                                        if d.ping().await {
                                            let matched_container = self
                                                .app_state
                                                .containers
                                                .iter()
                                                .find(|c| {
                                                    c.name == item_name
                                                        || c.name.trim_start_matches('/')
                                                            == item_name.trim_start_matches('/')
                                                })
                                                .cloned();

                                            if let Some(c) = matched_container {
                                                if c.running && c.id != "-" && !c.id.is_empty() {
                                                    self.set_status(format!(
                                                        "⏳ Stopping container '{}'...",
                                                        item_name
                                                    ));
                                                    match d.stop_container(&c.id, Some(3)).await {
                                                        Ok(_) => self.set_status(format!(
                                                            "✔ Stopped container '{}'",
                                                            item_name
                                                        )),
                                                        Err(e) => self.set_status(format!(
                                                            "✖ Failed to stop container '{}': {}",
                                                            item_name, e
                                                        )),
                                                    }
                                                } else if c.id != "-" && !c.id.is_empty() {
                                                    self.set_status(format!(
                                                        "⏳ Starting container '{}'...",
                                                        item_name
                                                    ));
                                                    match d.start_container(&c.id).await {
                                                        Ok(_) => self.set_status(format!(
                                                            "✔ Started container '{}'",
                                                            item_name
                                                        )),
                                                        Err(e) => self.set_status(format!(
                                                            "✖ Failed to start container '{}': {}",
                                                            item_name, e
                                                        )),
                                                    }
                                                } else if let Some(ServerConfig::Local {
                                                    container,
                                                    env,
                                                    ..
                                                }) = server_cfg
                                                {
                                                    if !container.image.is_empty() {
                                                        self.set_status(format!(
                                                            "⏳ Launching sandbox container '{}'...",
                                                            item_name
                                                        ));
                                                        match d
                                                            .create_sandbox_container(
                                                                &item_name, &container, &env, None,
                                                            )
                                                            .await
                                                        {
                                                            Ok(cid) => {
                                                                let _ = d.start_container(&cid).await;
                                                                self.set_status(format!(
                                                                    "✔ Started sandbox container '{}'",
                                                                    item_name
                                                                ));
                                                            }
                                                            Err(e) => self.set_status(format!(
                                                                "✖ Failed to create sandbox container: {}",
                                                                e
                                                            )),
                                                        }
                                                    } else {
                                                        self.set_status(format!(
                                                            "Server '{}' is a local command (not a container)",
                                                            item_name
                                                        ));
                                                    }
                                                } else {
                                                    self.set_status(format!(
                                                        "Container '{}' has no active instance or saved configuration",
                                                        item_name
                                                    ));
                                                }
                                            } else if let Some(ServerConfig::Local {
                                                container,
                                                env,
                                                ..
                                            }) = server_cfg
                                            {
                                                if !container.image.is_empty() {
                                                    self.set_status(format!(
                                                        "⏳ Launching sandbox container '{}'...",
                                                        item_name
                                                    ));
                                                    match d
                                                        .create_sandbox_container(
                                                            &item_name, &container, &env, None,
                                                        )
                                                        .await
                                                    {
                                                        Ok(cid) => {
                                                            let _ = d.start_container(&cid).await;
                                                            self.set_status(format!(
                                                                "✔ Started sandbox container '{}'",
                                                                item_name
                                                            ));
                                                        }
                                                        Err(e) => self.set_status(format!(
                                                            "✖ Failed to create sandbox container: {}",
                                                            e
                                                        )),
                                                    }
                                                } else {
                                                    self.set_status(format!(
                                                        "Server '{}' is a local command (not a container)",
                                                        item_name
                                                    ));
                                                }
                                            } else if is_container {
                                                self.set_status(format!(
                                                    "Container '{}' was not found on daemon",
                                                    item_name
                                                ));
                                            } else {
                                                self.set_status(format!(
                                                    "'{}' is a {} service (start/stop is for Docker containers)",
                                                    item_name, item.transport
                                                ));
                                            }
                                            self.trigger_docker_poll();
                                        } else {
                                            self.set_status(
                                                "Cannot start/stop: Docker daemon is offline or unreachable.",
                                            );
                                        }
                                    } else {
                                        self.set_status("Cannot start/stop: Docker daemon not available.");
                                    }
                                }
                            } else if self.mode == Mode::Clients {
                                let adapters = all_adapters();
                                if let Some(adapter) = adapters.get(self.clients.selected_adapter_index % adapters.len()) {
                                    if !self.app_state.is_client_enabled_in_active_profile(adapter.name()) {
                                        let active_prof = self.app_state.settings.active_profile.clone();
                                        self.set_status(format!(
                                            "○ {} is DISABLED for profile '{}'. Press [e] to enable sync.",
                                            adapter.display_name(), active_prof
                                        ));
                                    } else {
                                        self.open_sync_confirmation_modal();
                                    }
                                } else {
                                    self.open_sync_confirmation_modal();
                                }
                            } else {
                                self.open_sync_confirmation_modal();
                            }
                        }
                        KeyCode::Char('B') => {
                            if self.mode == Mode::Clients {
                                self.backup_current_client();
                            }
                        }
                        KeyCode::Char('R') => {
                            if self.mode == Mode::Clients {
                                self.restore_current_client_latest();
                            }
                        }
                        KeyCode::Char('f') => {
                            if self.mode == Mode::Mcps {
                                self.toggle_container_filter();
                            }
                        }
                        KeyCode::Char('+') => {
                            if self.mode == Mode::Mcps {
                                self.register_selected_container_as_mcp();
                            }
                        }
                        KeyCode::Char('e') | KeyCode::Char(' ') => {
                            if self.mode == Mode::Clients {
                                if self.clients.focused_pane == 0 {
                                    if let Some(cached) = self.clients.diff_cache.get(&self.clients.selected_adapter_index) {
                                        if let Some(mcp) = cached.discovered_mcps.get(self.clients.mcp_selected_index).cloned() {
                                            match mcp.status {
                                                crate::adapters::DiscoveredMcpStatus::Unmanaged | crate::adapters::DiscoveredMcpStatus::ManagedDiff => {
                                                    self.import_discovered_mcp(mcp);
                                                }
                                                crate::adapters::DiscoveredMcpStatus::ManagedEnabled => {
                                                    let active_prof = self.app_state.settings.active_profile.clone();
                                                    if let Some(prof) = self.app_state.profiles.get_mut(&active_prof) {
                                                        prof.disable_server(&mcp.name);
                                                    }
                                                    self.save_config();
                                                    self.set_status(format!("○ Disabled '{}' in profile '{}'", mcp.name, active_prof));
                                                }
                                                crate::adapters::DiscoveredMcpStatus::ManagedDisabled => {
                                                    let active_prof = self.app_state.settings.active_profile.clone();
                                                    if let Some(prof) = self.app_state.profiles.get_mut(&active_prof) {
                                                        prof.enable_server(&mcp.name);
                                                    }
                                                    self.save_config();
                                                    self.set_status(format!("● Enabled '{}' in profile '{}'", mcp.name, active_prof));
                                                }
                                            }
                                        }
                                    }
                                } else {
                                    self.toggle_current_client_for_profile();
                                }
                            } else if self.mode == Mode::Mcps {
                                self.toggle_current_server_for_profile();
                            }
                        }
                        KeyCode::Char('d') | KeyCode::Char('x') => {
                            if self.mode == Mode::Clients {
                                self.disable_current_client_for_profile();
                            } else if self.mode == Mode::Mcps {
                                let items = Mcps::build_items(&self.app_state);
                                if let Some(item) = items.get(self.mcps.selected_index) {
                                    let name = item.name.clone();
                                    self.app_state.servers.remove(&name);
                                    self.save_config();

                                    if let Some(ref d) = docker {
                                        if d.ping().await {
                                            let _ = d.remove_container(&name, true).await;
                                        }
                                    }
                                    self.set_status(format!("✔ Removed '{}' from configuration", name));
                                    self.trigger_docker_poll();
                                }
                            }
                        }
                        KeyCode::Char('c') => {
                            self.app_state.inspector_events.clear();
                            self.mcps.inspector_selected_index = 0;
                            self.set_status("Inspector logs cleared.");
                        }
                        KeyCode::Up | KeyCode::Char('k') => match self.mode {
                            Mode::Clients => {
                                self.clients.move_up();
                            }
                            Mode::Mcps => {
                                if self.mcps.selected_index > 0 {
                                    self.mcps.selected_index -= 1;
                                    self.trigger_docker_poll();
                                }
                            }
                            Mode::Skills => {
                                if self.skills.selected_index > 0 {
                                    self.skills.selected_index -= 1;
                                }
                            }
                        },
                        KeyCode::Down | KeyCode::Char('j') => match self.mode {
                            Mode::Clients => {
                                let total_discovered = self.clients.diff_cache.get(&self.clients.selected_adapter_index).map(|c| c.discovered_mcps.len()).unwrap_or(0);
                                let total_diff_lines = self.clients.diff_cache.get(&self.clients.selected_adapter_index).map(|c| c.changes.len()).unwrap_or(0);
                                self.clients.move_down(total_discovered, total_diff_lines);
                            }
                            Mode::Mcps => {
                                let items = Mcps::build_items(&self.app_state);
                                if !items.is_empty() && self.mcps.selected_index + 1 < items.len() {
                                    self.mcps.selected_index += 1;
                                    self.trigger_docker_poll();
                                }
                            }
                            Mode::Skills => {
                                if self.skills.selected_index + 1 < crate::components::skills::PREVIEW_SKILLS.len() {
                                    self.skills.selected_index += 1;
                                }
                            }
                        },
                        KeyCode::Left | KeyCode::Char('h') => {
                            if self.mode == Mode::Clients && self.clients.selected_adapter_index > 0 {
                                self.select_client_tab(self.clients.selected_adapter_index - 1);
                            }
                        }
                        KeyCode::Right | KeyCode::Char('l') => {
                            if self.mode == Mode::Clients
                                && self.clients.selected_adapter_index + 1 < all_adapters().len()
                            {
                                self.select_client_tab(self.clients.selected_adapter_index + 1);
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {
                self.active_modal = ActiveModal::None;
            }
        }

        Ok(())
    }

    async fn handle_mouse_event(&mut self, mouse: MouseEvent, tui_size: Rect) -> color_eyre::Result<()> {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let col = mouse.column;
                let row = mouse.row;

                match self.active_modal {
                    ActiveModal::Help => {
                        self.active_modal = ActiveModal::None;
                    }
                    ActiveModal::McpBrowser => {
                        let registry_entries = crate::components::mcp_browser::get_mcp_registry_entries();
                        let filtered_count = if self.mcp_browser.search_query.is_empty() {
                            registry_entries.len()
                        } else {
                            registry_entries
                                .iter()
                                .filter(|e| e.matches_query(&self.mcp_browser.search_query))
                                .count()
                        };

                        if (3..=5).contains(&row) {
                            self.mcp_browser.search_focused = true;
                        } else if (6..=25).contains(&row) && (10..=50).contains(&col) {
                            self.mcp_browser.search_focused = false;
                            let clicked_idx = (row - 6) as usize;
                            if clicked_idx < filtered_count {
                                self.mcp_browser.selected_index = clicked_idx;
                            }
                        }
                    }
                    ActiveModal::ProfileSwitcher => {
                        let mut profiles: Vec<String> = self.app_state.profiles.keys().cloned().collect();
                        profiles.sort();
                        if row >= 8 && row <= 8 + profiles.len() as u16 {
                            let clicked_idx = (row - 8) as usize;
                            if clicked_idx < profiles.len() {
                                let profile_to_switch = profiles[clicked_idx].clone();
                                self.switch_profile(&profile_to_switch);
                                self.active_modal = ActiveModal::None;
                            }
                        }
                    }
                    ActiveModal::None => {
                        if col < 25 {
                            match row {
                                4..=5 => {
                                    self.mode = Mode::Clients;
                                    self.sidebar.active_view = crate::components::sidebar::ActiveView::Clients;
                                    self.refresh_all_client_diffs();
                                }
                                6 => {
                                    self.mode = Mode::Mcps;
                                    self.sidebar.active_view = crate::components::sidebar::ActiveView::Mcps;
                                    self.trigger_docker_poll();
                                }
                                7..=8 => {
                                    self.mode = Mode::Skills;
                                    self.sidebar.active_view = crate::components::sidebar::ActiveView::Skills;
                                }
                                10 => self.open_mcp_browser_modal(),
                                11 => self.open_profile_switcher_modal(),
                                12..=13 => self.open_help_modal(),
                                15 => self.open_sync_confirmation_modal(),
                                16..=17 => {
                                    self.should_quit = true;
                                    return Ok(());
                                }
                                r if r >= 20 => self.open_profile_switcher_modal(),
                                _ => {}
                            }
                        } else {
                            match self.mode {
                                Mode::Clients => {
                                    if row <= 3 {
                                        let adapter_count = all_adapters().len();
                                        let tab_width = (tui_size.width.saturating_sub(25))
                                            / (adapter_count as u16).max(1);
                                        let clicked_tab =
                                            ((col.saturating_sub(25)) / tab_width.max(1)) as usize;
                                        if clicked_tab < adapter_count {
                                            self.select_client_tab(clicked_tab);
                                        }
                                    } else if row > 6 {
                                        let mid_x = 25 + (tui_size.width.saturating_sub(25)) * 45 / 100;
                                        if col < mid_x {
                                            self.clients.focused_pane = 0;
                                            let clicked_mcp = (row.saturating_sub(8)) as usize;
                                            let total_discovered = self.clients.diff_cache.get(&self.clients.selected_adapter_index).map(|c| c.discovered_mcps.len()).unwrap_or(0);
                                            if clicked_mcp < total_discovered {
                                                self.clients.mcp_selected_index = clicked_mcp;
                                            }
                                        } else {
                                            self.clients.focused_pane = 1;
                                        }
                                    }
                                }
                                Mode::Mcps => {
                                    let term_height = tui_size.height;
                                    let mid_y = term_height / 2;

                                    if row < mid_y {
                                        if row >= 2 {
                                            let clicked_row = (row - 2) as usize;
                                            let total_items =
                                                self.app_state.servers.len() + self.app_state.containers.len();
                                            if clicked_row < total_items {
                                                self.mcps.selected_index = clicked_row;
                                                self.trigger_docker_poll();
                                            }
                                        }
                                    } else if row == mid_y || row == mid_y + 1 {
                                        if col < 55 {
                                            self.mcps.bottom_mode =
                                                crate::components::mcps::McpBottomPaneMode::Logs;
                                        } else {
                                            self.mcps.bottom_mode =
                                                crate::components::mcps::McpBottomPaneMode::Inspector;
                                        }
                                    }
                                }
                                Mode::Skills => {
                                    if row >= 8 {
                                        let clicked_idx = (row - 8) as usize;
                                        if clicked_idx < crate::components::skills::PREVIEW_SKILLS.len() {
                                            self.skills.selected_index = clicked_idx;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            MouseEventKind::ScrollUp => match self.active_modal {
                ActiveModal::McpBrowser => {
                    if self.mcp_browser.selected_index > 0 {
                        self.mcp_browser.selected_index -= 1;
                    }
                }
                ActiveModal::ProfileSwitcher => {
                    if self.profile_switcher.selected_index > 0 {
                        self.profile_switcher.selected_index -= 1;
                    }
                }
                ActiveModal::ProfileEditor => {
                    self.profile_editor.move_up();
                }
                ActiveModal::None => match self.mode {
                    Mode::Clients => {
                        self.clients.diff_scroll_offset =
                            self.clients.diff_scroll_offset.saturating_sub(2);
                    }
                    Mode::Mcps => {
                        let term_height = tui_size.height;
                        if mouse.row >= term_height / 2 {
                            match self.mcps.bottom_mode {
                                crate::components::mcps::McpBottomPaneMode::Logs => {
                                    self.mcps.logs_scroll_offset =
                                        self.mcps.logs_scroll_offset.saturating_sub(2);
                                }
                                crate::components::mcps::McpBottomPaneMode::Inspector => {
                                    if self.mcps.inspector_selected_index > 0 {
                                        self.mcps.inspector_selected_index -= 1;
                                    }
                                }
                            }
                        } else if self.mcps.selected_index > 0 {
                            self.mcps.selected_index -= 1;
                            self.trigger_docker_poll();
                        }
                    }
                    Mode::Skills => {
                        if self.skills.selected_index > 0 {
                            self.skills.selected_index -= 1;
                        }
                    }
                },
                _ => {}
            },
            MouseEventKind::ScrollDown => match self.active_modal {
                ActiveModal::McpBrowser => {
                    let registry_entries = crate::components::mcp_browser::get_mcp_registry_entries();
                    let max_len = if self.mcp_browser.search_query.is_empty() {
                        registry_entries.len()
                    } else {
                        registry_entries
                            .iter()
                            .filter(|e| e.matches_query(&self.mcp_browser.search_query))
                            .count()
                    };
                    if self.mcp_browser.selected_index + 1 < max_len {
                        self.mcp_browser.selected_index += 1;
                    }
                }
                ActiveModal::ProfileSwitcher => {
                    let total = self.app_state.profiles.len();
                    if self.profile_switcher.selected_index + 1 < total {
                        self.profile_switcher.selected_index += 1;
                    }
                }
                ActiveModal::ProfileEditor => {
                    let total_servers = self.app_state.servers.len();
                    let total_clients = all_adapters().len();
                    self.profile_editor.move_down(total_servers, total_clients);
                }
                ActiveModal::None => match self.mode {
                    Mode::Clients => {
                        self.clients.diff_scroll_offset += 2;
                    }
                    Mode::Mcps => {
                        let term_height = tui_size.height;
                        if mouse.row >= term_height / 2 {
                            match self.mcps.bottom_mode {
                                crate::components::mcps::McpBottomPaneMode::Logs => {
                                    self.mcps.logs_scroll_offset += 2;
                                }
                                crate::components::mcps::McpBottomPaneMode::Inspector => {
                                    if self.mcps.inspector_selected_index + 1
                                        < self.app_state.inspector_events.len()
                                    {
                                        self.mcps.inspector_selected_index += 1;
                                    }
                                }
                            }
                        } else {
                            let total_items =
                                self.app_state.servers.len() + self.app_state.containers.len();
                            if self.mcps.selected_index + 1 < total_items {
                                self.mcps.selected_index += 1;
                                self.trigger_docker_poll();
                            }
                        }
                    }
                    Mode::Skills => {
                        if self.skills.selected_index + 1 < crate::components::skills::PREVIEW_SKILLS.len() {
                            self.skills.selected_index += 1;
                        }
                    }
                },
                _ => {}
            },
            _ => {}
        }
        Ok(())
    }

    fn handle_actions(&mut self, tui: &mut Tui) -> color_eyre::Result<()> {
        while let Ok(action) = self.action_rx.try_recv() {
            match action {
                Action::Tick => {
                    self.last_tick_key_events.drain(..);
                }
                Action::Quit => self.should_quit = true,
                Action::Suspend => self.should_suspend = true,
                Action::Resume => self.should_suspend = false,
                Action::ClearScreen => tui.terminal.clear()?,
                Action::Resize(w, h) => self.handle_resize(tui, w, h)?,
                Action::Render => self.render(tui)?,
                Action::Error(ref err) => tracing::error!(?err),
                _ => {}
            }

            // Pass to components
            self.sidebar.update(action.clone())?;
            self.clients.update(action.clone())?;
            self.mcps.update(action.clone())?;
            self.skills.update(action.clone())?;
        }
        Ok(())
    }

    fn handle_resize(&mut self, tui: &mut Tui, w: u16, h: u16) -> color_eyre::Result<()> {
        tui.resize(Rect::new(0, 0, w, h))?;
        self.render(tui)?;
        Ok(())
    }

    fn render(&mut self, tui: &mut Tui) -> color_eyre::Result<()> {
        let matches = self.get_diff_search_matches();
        let current_match_line = if !matches.is_empty() && self.clients.search_match_index < matches.len() {
            Some(matches[self.clients.search_match_index])
        } else {
            None
        };
        self.clients.total_matches = matches.len();
        self.clients.current_match_line = current_match_line;

        tui.draw(|frame| {
            let size = frame.area();

            // Vertical Split: Main Workspace (Sidebar + View) vs Bottom Status Bar
            let root_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(10),   // Workspace Area
                    Constraint::Length(1), // Bottom Status Bar
                ])
                .split(size);

            // Horizontal Split: Left Sidebar vs Active View Content
            let main_split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(25), // Left Sidebar Navigation
                    Constraint::Min(20),    // Main View Content
                ])
                .split(root_chunks[0]);

            // 1. Render Sidebar
            frame.render_stateful_widget(&self.sidebar, main_split[0], &mut self.app_state);

            // 2. Render Active View
            match self.mode {
                Mode::Clients => frame.render_stateful_widget(&self.clients, main_split[1], &mut self.app_state),
                Mode::Mcps => frame.render_stateful_widget(&self.mcps, main_split[1], &mut self.app_state),
                Mode::Skills => frame.render_widget(&self.skills, main_split[1]),
            }

            // 3. Render Bottom Status Bar
            let status_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(root_chunks[1]);

            if let Some((ref msg, _)) = self.status_message {
                let notify_line = Line::from(Span::styled(
                    format!("  {}  ", msg),
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));
                frame.render_widget(Paragraph::new(notify_line), status_chunks[0]);
            } else {
                let active_profile = &self.app_state.settings.active_profile;
                let docker_badge = if self.app_state.docker_status.contains("Online") {
                    Span::styled(
                        "● Online",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    Span::styled("○ Offline", Style::default().fg(Color::Red))
                };

                let status_line = Line::from(vec![
                    Span::styled(" Profile: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!("[{}]", active_profile),
                        Style::default()
                            .fg(Color::Magenta)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("  │  Docker: ", Style::default().fg(Color::DarkGray)),
                    docker_badge,
                ]);
                frame.render_widget(Paragraph::new(status_line), status_chunks[0]);
            }

            let shortcuts_line = if self.mode == Mode::Clients && self.clients.search_active {
                Line::from(vec![
                    Span::styled(
                        "[Enter] ",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("Confirm Search  ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        "[Esc] ",
                        Style::default()
                            .fg(Color::Magenta)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("Cancel", Style::default().fg(Color::DarkGray)),
                ])
            } else {
                let is_curr_client_disabled = if self.mode == Mode::Clients {
                    let adapters = all_adapters();
                    adapters.get(self.clients.selected_adapter_index % adapters.len())
                        .map(|a| !self.app_state.is_client_enabled_in_active_profile(a.name()))
                        .unwrap_or(false)
                } else {
                    false
                };

                let sync_span = if is_curr_client_disabled {
                    vec![
                        Span::styled("[s] ", Style::default().fg(Color::DarkGray)),
                        Span::styled("Sync (Disabled)  ", Style::default().fg(Color::DarkGray)),
                    ]
                } else {
                    vec![
                        Span::styled(
                            "[s] ",
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled("Sync  ", Style::default().fg(Color::DarkGray)),
                    ]
                };

                let mut spans = vec![
                    Span::styled(
                        "[?] ",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("Help  ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        "[Tab] ",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("View  ", Style::default().fg(Color::DarkGray)),
                ];
                spans.extend(sync_span);
                spans.extend(vec![
                    Span::styled(
                        "[q] ",
                        Style::default()
                            .fg(Color::Magenta)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("Quit", Style::default().fg(Color::DarkGray)),
                ]);
                Line::from(spans)
            };
            let shortcuts_widget = Paragraph::new(shortcuts_line).alignment(Alignment::Right);
            frame.render_widget(shortcuts_widget, status_chunks[1]);

            // 4. Modal Overlays
            match self.active_modal {
                ActiveModal::None => {}
                ActiveModal::Help => {
                    let modal_area = crate::components::wizard::centered_rect(80, 78, size);
                    frame.render_widget(Clear, modal_area);
                    frame.render_widget(&self.help, modal_area);
                }
                ActiveModal::McpBrowser => {
                    let modal_area = crate::components::wizard::centered_rect(84, 82, size);
                    frame.render_widget(Clear, modal_area);
                    frame.render_stateful_widget(&self.mcp_browser, modal_area, &mut self.app_state);
                }
                ActiveModal::ProfileSwitcher => {
                    let modal_area = crate::components::wizard::centered_rect(54, 48, size);
                    frame.render_widget(Clear, modal_area);
                    frame.render_stateful_widget(&self.profile_switcher, modal_area, &mut self.app_state);
                }
                ActiveModal::ProfileEditor => {
                    let modal_area = crate::components::wizard::centered_rect(76, 70, size);
                    frame.render_widget(Clear, modal_area);
                    frame.render_stateful_widget(&self.profile_editor, modal_area, &mut self.app_state);
                }
                ActiveModal::Wizard => {
                    let modal_area = crate::components::wizard::centered_rect(68, 68, size);
                    frame.render_widget(Clear, modal_area);
                    frame.render_stateful_widget(&self.wizard, modal_area, &mut self.app_state);
                }
                ActiveModal::SyncConfirm => {
                    let modal_area = crate::components::wizard::centered_rect(64, 52, size);
                    frame.render_widget(Clear, modal_area);
                    frame.render_stateful_widget(&self.sync_confirm, modal_area, &mut self.app_state);
                }
                ActiveModal::ServerBrowser => {
                    let modal_area = crate::components::wizard::centered_rect(80, 78, size);
                    frame.render_widget(Clear, modal_area);
                    frame.render_stateful_widget(&self.server_browser, modal_area, &mut self.app_state);
                }
                ActiveModal::DiffViewer => {
                    let modal_area = crate::components::wizard::centered_rect(80, 78, size);
                    frame.render_widget(Clear, modal_area);
                    frame.render_stateful_widget(&self.diff_viewer, modal_area, &mut self.app_state);
                }
                ActiveModal::Inspector => {
                    let modal_area = crate::components::wizard::centered_rect(80, 78, size);
                    frame.render_widget(Clear, modal_area);
                    frame.render_stateful_widget(&self.inspector, modal_area, &mut self.app_state);
                }
            }
        })?;
        Ok(())
    }
}
