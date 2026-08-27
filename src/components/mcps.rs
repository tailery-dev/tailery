use color_eyre::Result;
use ratatui::prelude::Widget;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Row, StatefulWidget, Table, Tabs},
};
use tokio::sync::mpsc::UnboundedSender;

use crate::action::Action;
use crate::components::Component;
use crate::components::inspector::InspectorEvent;
use crate::state::AppState;
use crate::tui::Event;

pub fn is_remote_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    let stripped = lower
        .strip_prefix("http://")
        .or_else(|| lower.strip_prefix("https://"))
        .unwrap_or(&lower);

    let host = if stripped.starts_with('[') {
        if let Some(end) = stripped.find(']') {
            &stripped[1..end]
        } else {
            stripped.split(&['/', ':', '?'][..]).next().unwrap_or("")
        }
    } else {
        stripped.split(&['/', ':', '?'][..]).next().unwrap_or("")
    };

    !(host.is_empty()
        || host == "localhost"
        || host == "127.0.0.1"
        || host == "0.0.0.0"
        || host == "::1"
        || host == "[::1]")
}

#[derive(Clone, Debug, Default)]
pub struct UnifiedMcpItem {
    pub name: String,
    pub scope: String,     // "local" or "remote" (inferred)
    pub transport: String, // "stdio", "streamable-http", "sse", "http"
    pub is_container: bool,
    pub is_enabled_in_profile: bool,
    pub running: bool,
    pub status_text: String,
    pub image_or_cmd: String,
    pub ports: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContainerFilterMode {
    #[default]
    ManagedOnly,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum McpBottomPaneMode {
    #[default]
    Logs,
    Inspector,
}

#[derive(Default)]
pub struct Mcps {
    pub selected_index: usize,
    pub bottom_mode: McpBottomPaneMode,
    pub logs_scroll_offset: usize,
    pub inspector_selected_index: usize,
    pub inspector_scroll_offset: usize,
    pub filter_mode: ContainerFilterMode,
    pub command_tx: Option<UnboundedSender<Action>>,
}

impl Mcps {
    pub fn build_items(state: &AppState) -> Vec<UnifiedMcpItem> {
        let active_profile = &state.settings.active_profile;
        let enabled_in_profile = state.profiles.get(active_profile).map(|p| &p.enabled_servers);

        let mut items = Vec::new();

        // 1. Configured MCP servers
        for (name, s) in &state.servers {
            let is_enabled = enabled_in_profile.map(|list| list.contains(name)).unwrap_or(true);

            let (scope, transport, is_container, img_or_cmd) = match s {
                crate::state::ServerConfig::Local { command, args, container, transport, .. } => {
                    let transport_str = match transport {
                        crate::state::LocalTransport::Stdio => "stdio",
                        crate::state::LocalTransport::StreamableHttp { .. } => "streamable-http",
                        crate::state::LocalTransport::Http { .. } => "http",
                        crate::state::LocalTransport::Sse { .. } => "sse",
                    };
                    if !container.image.is_empty() {
                        ("local", transport_str, true, container.image.clone())
                    } else if let Some(cmd) = command {
                        ("local", transport_str, true, format!("{} {}", cmd, args.join(" ")))
                    } else {
                        ("local", transport_str, true, "local command".to_string())
                    }
                }
                crate::state::ServerConfig::Remote { url, transport, .. } => {
                    let transport_str = match transport {
                        crate::state::RemoteTransport::StreamableHttp => "streamable-http",
                        crate::state::RemoteTransport::Http => "http",
                        crate::state::RemoteTransport::Sse => "sse",
                    };
                    let scope_str = if is_remote_url(url) { "remote" } else { "local" };
                    (scope_str, transport_str, false, url.clone())
                }
            };

            let container_info = state.containers.iter().find(|c| c.name == *name || c.name.trim_start_matches('/') == *name);

            let (running, status_text, ports) = if let Some(c) = container_info {
                (
                    c.running,
                    if c.running { "● RUNNING".to_string() } else { "○ STOPPED".to_string() },
                    c.ports.clone(),
                )
            } else if is_container {
                (false, "○ READY".to_string(), vec![])
            } else {
                (true, "● READY".to_string(), vec![])
            };

            items.push(UnifiedMcpItem {
                name: name.clone(),
                scope: scope.to_string(),
                transport: transport.to_string(),
                is_container,
                is_enabled_in_profile: is_enabled,
                running,
                status_text,
                image_or_cmd: img_or_cmd,
                ports,
            });
        }

        // 2. Extra managed/host containers (e.g. web-search)
        for c in &state.containers {
            let clean_name = c.name.trim_start_matches('/');
            if !items.iter().any(|item| item.name == c.name || item.name == clean_name) {
                let status_text = if c.running {
                    "● RUNNING".to_string()
                } else if !c.daemon_online {
                    "○ OFFLINE".to_string()
                } else {
                    "○ STOPPED".to_string()
                };

                let inferred_transport = if !c.ports.is_empty() {
                    "streamable-http"
                } else {
                    "stdio"
                };

                let is_enabled = enabled_in_profile
                    .map(|list| list.contains(&clean_name.to_string()) || list.contains(&c.name))
                    .unwrap_or(false);

                items.push(UnifiedMcpItem {
                    name: clean_name.to_string(),
                    scope: "local".to_string(),
                    transport: inferred_transport.to_string(),
                    is_container: true,
                    is_enabled_in_profile: is_enabled,
                    running: c.running,
                    status_text,
                    image_or_cmd: c.image.clone(),
                    ports: c.ports.clone(),
                });
            }
        }

        // 3. Static configured containers if not yet detected in live daemon list
        for (name, cfg) in &state.configured_containers {
            if !items.iter().any(|item| item.name == *name) {
                let is_enabled = enabled_in_profile
                    .map(|list| list.contains(name))
                    .unwrap_or(false);

                let inferred_transport = if !cfg.ports.is_empty() {
                    "streamable-http"
                } else {
                    "stdio"
                };

                let ports: Vec<String> = cfg
                    .ports
                    .iter()
                    .map(|p| format!("{}:{}", p.host_port, p.container_port))
                    .collect();

                items.push(UnifiedMcpItem {
                    name: name.clone(),
                    scope: "local".to_string(),
                    transport: inferred_transport.to_string(),
                    is_container: true,
                    is_enabled_in_profile: is_enabled,
                    running: false,
                    status_text: "○ STOPPED".to_string(),
                    image_or_cmd: cfg.image.clone(),
                    ports,
                });
            }
        }

        items
    }
}

impl Component for Mcps {
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

impl StatefulWidget for &Mcps {
    type State = AppState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let items = Mcps::build_items(state);
        let selected_item = items.get(self.selected_index);

        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(55), // Top half: Table & Details
                Constraint::Percentage(45), // Bottom half: Logs / Traffic Telemetry
            ])
            .split(area);

        let top_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(58), // Table
                Constraint::Percentage(42), // Detail Panel
            ])
            .split(main_chunks[0]);

        // 1. MCP Servers & Containers Table
        let rows: Vec<Row> = items
            .iter()
            .enumerate()
            .map(|(idx, item)| {
                let is_selected = idx == self.selected_index;

                let name_style = if is_selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD)
                };

                let scope_style = if item.scope == "remote" {
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Cyan)
                };

                let profile_badge = if item.is_enabled_in_profile {
                    Span::styled(
                        "● ACTIVE",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    Span::styled("○ OFF", Style::default().fg(Color::DarkGray))
                };

                let (status_badge, status_style) = if item.running {
                    (
                        "● RUNNING",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    ("○ STOPPED", Style::default().fg(Color::Red))
                };

                let transport_style = match item.transport.as_str() {
                    "stdio" => Style::default().fg(Color::Rgb(130, 180, 255)),
                    "streamable-http" | "http" => Style::default().fg(Color::Green),
                    "sse" => Style::default().fg(Color::Yellow),
                    _ => Style::default().fg(Color::DarkGray),
                };

                Row::new(vec![
                    Span::styled(format!(" {} ", item.name), name_style),
                    Span::styled(format!(" {} ", item.scope), scope_style),
                    Span::styled(&item.transport, transport_style),
                    profile_badge,
                    Span::styled(status_badge, status_style),
                    Span::styled(&item.image_or_cmd, Style::default().fg(Color::DarkGray)),
                ])
            })
            .collect();

        let active_prof = &state.settings.active_profile;
        let table_title = format!(
            " MCP Servers & Containers [Profile: '{}'] ({}) ",
            active_prof,
            items.len()
        );

        let table = Table::new(
            rows,
            [
                Constraint::Percentage(24),
                Constraint::Percentage(12),
                Constraint::Percentage(18),
                Constraint::Percentage(14),
                Constraint::Percentage(16),
                Constraint::Percentage(16),
            ],
        )
        .header(
            Row::new(vec![
                "Name",
                "Scope",
                "Transport",
                "Profile",
                "Status",
                "Target / Image",
            ])
            .style(
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan))
                .title(Span::styled(
                    table_title,
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
        );
        ratatui::prelude::Widget::render(table, top_chunks[0], buf);

        // 2. Selected Server / Container Details & Quick Actions
        let detail_lines = if let Some(item) = selected_item {
            let mut lines = vec![
                Line::from(vec![
                    Span::styled("Target Name:  ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        &item.name,
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("Scope:        ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        &item.scope,
                        if item.scope == "remote" {
                            Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::Cyan)
                        },
                    ),
                    Span::raw(" │ Transport: "),
                    Span::styled(
                        &item.transport,
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" │ Status: "),
                    Span::styled(
                        &item.status_text,
                        Style::default().fg(if item.running {
                            Color::Green
                        } else {
                            Color::Red
                        }),
                    ),
                ]),
            ];

            if let Some(srv) = state.servers.get(&item.name) {
                match srv {
                    crate::state::ServerConfig::Local {
                        command,
                        args,
                        tool_filter,
                        env,
                        container,
                        ..
                    } => {
                        if !container.image.is_empty() {
                            lines.push(Line::from(vec![
                                Span::styled(
                                    "Docker Image: ",
                                    Style::default().fg(Color::DarkGray),
                                ),
                                Span::styled(&container.image, Style::default().fg(Color::White)),
                            ]));
                        }
                        if let Some(cmd) = command {
                            lines.push(Line::from(vec![
                                Span::styled(
                                    "Command:      ",
                                    Style::default().fg(Color::DarkGray),
                                ),
                                Span::styled(
                                    format!("{} {}", cmd, args.join(" ")),
                                    Style::default().fg(Color::White),
                                ),
                            ]));
                        }
                        lines.push(Line::from(vec![
                            Span::styled("Filesystem:   ", Style::default().fg(Color::DarkGray)),
                            Span::styled(
                                if container.read_only_rootfs {
                                    "Read-Only Sandbox (Enforced)"
                                } else {
                                    "Read-Write"
                                },
                                Style::default().fg(if container.read_only_rootfs {
                                    Color::Green
                                } else {
                                    Color::Red
                                }),
                            ),
                        ]));
                        let allow_str = if tool_filter.allow.is_empty() {
                            "* (All)".to_string()
                        } else {
                            tool_filter.allow.join(", ")
                        };
                        let deny_str = if tool_filter.deny.is_empty() {
                            "None".to_string()
                        } else {
                            tool_filter.deny.join(", ")
                        };
                        lines.push(Line::from(vec![
                            Span::styled("Tool Policy:  ", Style::default().fg(Color::DarkGray)),
                            Span::styled(
                                format!("Allow: [{}] | Deny: [{}]", allow_str, deny_str),
                                Style::default().fg(Color::Green),
                            ),
                        ]));
                        if !env.is_empty() {
                            lines.push(Line::from(vec![
                                Span::styled(
                                    "Environment:  ",
                                    Style::default().fg(Color::DarkGray),
                                ),
                                Span::styled(
                                    format!("{} variables configured", env.len()),
                                    Style::default().fg(Color::Cyan),
                                ),
                            ]));
                        }
                    }
                    crate::state::ServerConfig::Remote { url, .. } => {
                        lines.push(Line::from(vec![
                            Span::styled("Remote URL:   ", Style::default().fg(Color::DarkGray)),
                            Span::styled(url, Style::default().fg(Color::Cyan)),
                        ]));
                    }
                }
            } else if let Some(c) = state
                .containers
                .iter()
                .find(|c| c.name == item.name || c.name.trim_start_matches('/') == item.name)
            {
                lines.push(Line::from(vec![
                    Span::styled("Docker Image: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(&c.image, Style::default().fg(Color::White)),
                ]));
                let ports_str = if c.ports.is_empty() {
                    "-".to_string()
                } else {
                    c.ports.join(", ")
                };
                lines.push(Line::from(vec![
                    Span::styled("Ports:        ", Style::default().fg(Color::DarkGray)),
                    Span::styled(ports_str, Style::default().fg(Color::Cyan)),
                ]));
            }

            lines.push(Line::raw(""));
            lines.push(Line::from(Span::styled(
                "─── Actions ───",
                Style::default().fg(Color::Magenta),
            )));
            lines.push(Line::from(vec![
                Span::styled(
                    " [e] ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("Toggle Profile   "),
                Span::styled(
                    " [s] ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("Start/Stop"),
            ]));
            lines.push(Line::from(vec![
                Span::styled(
                    " [a] ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("Add MCP Server   "),
                Span::styled(
                    " [d] ",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("Delete Item"),
            ]));
            lines.push(Line::from(vec![
                Span::styled(
                    " [i] ",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("Logs/Inspector   "),
                Span::styled(
                    " [c] ",
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("Clear Inspector"),
            ]));

            lines
        } else {
            vec![
                Line::from(Span::styled(
                    "No servers or containers configured.",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::raw(""),
                Line::from(vec![
                    Span::styled("Press ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        " [a] ",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        " to add an MCP server or ",
                        Style::default().fg(Color::White),
                    ),
                    Span::styled(
                        " [b] ",
                        Style::default()
                            .fg(Color::Magenta)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        " to browse the MCP Registry.",
                        Style::default().fg(Color::White),
                    ),
                ]),
            ]
        };

        let detail_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Magenta))
            .title(Span::styled(
                " Inspection & Security Policy ",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ));
        Paragraph::new(detail_lines)
            .block(detail_block)
            .render(top_chunks[1], buf);

        // 3. Bottom Pane: Folded Live Container Logs OR Live JSON-RPC Inspector
        let bottom_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Subpane Tabs: [Logs] [Live Traffic Inspector]
                Constraint::Min(4),    // Content
            ])
            .split(main_chunks[1]);

        let mode_idx = match self.bottom_mode {
            McpBottomPaneMode::Logs => 0,
            McpBottomPaneMode::Inspector => 1,
        };

        let subpane_titles = vec![
            Line::from(vec![Span::styled(
                " [1] Logs ",
                if mode_idx == 0 {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::DarkGray)
                },
            )]),
            Line::from(vec![Span::styled(
                format!(" [2] Live Inspector ({}) ", state.inspector_events.len()),
                if mode_idx == 1 {
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::DarkGray)
                },
            )]),
        ];

        let sub_tabs = Tabs::new(subpane_titles)
            .select(mode_idx)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::DarkGray))
                    .title(Span::styled(
                        " Live Telemetry & Console [i] ",
                        Style::default().fg(Color::White),
                    )),
            )
            .highlight_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            );
        sub_tabs.render(bottom_chunks[0], buf);

        // Render selected subpane
        match self.bottom_mode {
            McpBottomPaneMode::Logs => {
                let mut log_lines: Vec<Line> = Vec::new();
                let daemon_online = state.docker_status.contains("Online");

                if !daemon_online {
                    log_lines.push(Line::from(vec![
                        Span::styled(
                            " ○ DOCKER DAEMON OFFLINE ",
                            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                        ),
                        Span::raw("  Could not reach Docker or Podman runtime socket."),
                    ]));
                    log_lines.push(Line::from(Span::styled("Start Docker Desktop, OrbStack, or 'podman machine start' to stream live logs.", Style::default().fg(Color::DarkGray))));
                } else if state.container_logs.is_empty() {
                    let sel_name = selected_item.map(|i| i.name.as_str()).unwrap_or("None");
                    log_lines.push(Line::from(Span::styled(
                        format!(
                            "No active logs for '{}' (container stopped or no output yet).",
                            sel_name
                        ),
                        Style::default().fg(Color::DarkGray),
                    )));
                } else {
                    for line in state.container_logs.iter().skip(self.logs_scroll_offset) {
                        let style = if line.contains("ERROR")
                            || line.contains("error")
                            || line.contains("ERR")
                        {
                            Style::default().fg(Color::Red)
                        } else if line.contains("WARN") || line.contains("warn") {
                            Style::default().fg(Color::Magenta)
                        } else {
                            Style::default().fg(Color::White)
                        };
                        log_lines.push(Line::from(Span::styled(line.clone(), style)));
                    }
                }

                let logs_block = Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Cyan))
                    .title(Span::styled(
                        " Output Stream ",
                        Style::default().fg(Color::Cyan),
                    ));
                Paragraph::new(log_lines)
                    .block(logs_block)
                    .render(bottom_chunks[1], buf);
            }
            McpBottomPaneMode::Inspector => {
                let inspector_events: Vec<InspectorEvent> = state.inspector_events.iter().map(InspectorEvent::from_telemetry).collect();
                let inspector_split = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
                    .split(bottom_chunks[1]);

                let rows: Vec<Row> = inspector_events
                    .iter()
                    .enumerate()
                    .skip(self.inspector_scroll_offset)
                    .map(|(idx, ev)| {
                        let is_selected = idx == self.inspector_selected_index;
                        let type_style = match ev.event_type.as_str() {
                            "REQ" => Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                            "RESP" => Style::default()
                                .fg(Color::Green)
                                .add_modifier(Modifier::BOLD),
                            "ERR" => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                            "BLOCK" => Style::default()
                                .fg(Color::Black)
                                .bg(Color::Red)
                                .add_modifier(Modifier::BOLD),
                            "NOTIF" => Style::default().fg(Color::Magenta),
                            _ => Style::default().fg(Color::DarkGray),
                        };

                        let latency_str =
                            ev.latency.map(|l| format!("{}ms", l)).unwrap_or_default();
                        let row_style = if is_selected {
                            Style::default().bg(Color::Rgb(30, 40, 60))
                        } else {
                            Style::default()
                        };

                        Row::new(vec![
                            Span::styled(&ev.timestamp, Style::default().fg(Color::DarkGray)),
                            Span::styled(
                                &ev.server,
                                Style::default()
                                    .fg(Color::White)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(&ev.event_type, type_style),
                            Span::styled(&ev.summary, Style::default().fg(Color::White)),
                            Span::styled(latency_str, Style::default().fg(Color::Cyan)),
                        ])
                        .style(row_style)
                    })
                    .collect();

                let event_table = Table::new(
                    rows,
                    [
                        Constraint::Length(9),
                        Constraint::Length(14),
                        Constraint::Length(8),
                        Constraint::Min(16),
                        Constraint::Length(8),
                    ],
                )
                .header(
                    Row::new(vec!["Time", "Server", "Type", "Method / Event", "Latency"]).style(
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::BOLD),
                    ),
                )
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(Color::Cyan))
                        .title(Span::styled(
                            " Live Traffic Stream ",
                            Style::default().fg(Color::Cyan),
                        )),
                );
                ratatui::prelude::Widget::render(event_table, inspector_split[0], buf);

                let mut payload_lines: Vec<Line> = Vec::new();
                if let Some(ev) = inspector_events.get(self.inspector_selected_index) {
                    payload_lines.push(Line::from(vec![
                        Span::styled(
                            format!("[{}] ", ev.event_type),
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("Server: {} ", ev.server),
                            Style::default().fg(Color::White),
                        ),
                        Span::styled(
                            format!("Time: {}", ev.timestamp),
                            Style::default().fg(Color::Magenta),
                        ),
                    ]));
                    let pretty_json = serde_json::to_string_pretty(&ev.payload).unwrap_or_default();
                    for line in pretty_json.lines().take(20) {
                        payload_lines.push(Line::from(Span::styled(
                            line.to_string(),
                            Style::default().fg(Color::DarkGray),
                        )));
                    }
                } else {
                    payload_lines.push(Line::from(Span::styled(
                        "No traffic events yet. Run an MCP client with shim to stream RPC traffic.",
                        Style::default().fg(Color::DarkGray),
                    )));
                }

                let payload_block = Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Magenta))
                    .title(Span::styled(
                        " JSON-RPC Payload ",
                        Style::default().fg(Color::Magenta),
                    ));
                Paragraph::new(payload_lines)
                    .block(payload_block)
                    .render(inspector_split[1], buf);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{AppState, ServerConfig, LocalTransport, RemoteTransport, ContainerConfig};
    use std::collections::HashMap;

    #[test]
    fn test_is_remote_url_classification() {
        assert!(!is_remote_url("http://localhost:3000/sse"));
        assert!(!is_remote_url("http://127.0.0.1:8080/mcp"));
        assert!(!is_remote_url("http://0.0.0.0:8000"));
        assert!(!is_remote_url("http://[::1]:3000"));
        assert!(!is_remote_url("localhost:3000"));
        assert!(is_remote_url("https://mcp.github.com/v1/sse"));
        assert!(is_remote_url("https://api.context7.ai/mcp"));
        assert!(is_remote_url("http://192.168.1.100:3000/mcp"));
    }

    #[test]
    fn test_mcps_build_items_inferred_scope_and_transport() {
        let mut servers = HashMap::new();
        servers.insert(
            "local-stdio".to_string(),
            ServerConfig::Local {
                command: Some("npx".to_string()),
                args: vec![],
                env: HashMap::new(),
                tool_filter: Default::default(),
                container: ContainerConfig::default(),
                transport: LocalTransport::Stdio,
            },
        );
        servers.insert(
            "local-http".to_string(),
            ServerConfig::Remote {
                url: "http://localhost:3000/mcp".to_string(),
                transport: RemoteTransport::StreamableHttp,
                headers: HashMap::new(),
                env: HashMap::new(),
                tool_filter: Default::default(),
                shim_port: None,
            },
        );
        servers.insert(
            "remote-sse".to_string(),
            ServerConfig::Remote {
                url: "https://mcp.company.com/sse".to_string(),
                transport: RemoteTransport::Sse,
                headers: HashMap::new(),
                env: HashMap::new(),
                tool_filter: Default::default(),
                shim_port: None,
            },
        );

        let mut state = AppState {
            version: "1.0.0".to_string(),
            settings: crate::state::GlobalSettings {
                active_profile: "default".to_string(),
                docker_socket: None,
                sync_clients: vec![],
                filter_managed_containers_only: None,
            },
            servers,
            configured_containers: HashMap::new(),
            profiles: HashMap::new(),
            workspaces: HashMap::new(),
            docker_status: String::new(),
            containers: Vec::new(),
            container_logs: Vec::new(),
            inspector_events: Vec::new(),
        };

        // Add an extra running container with ports (like web-search)
        state.containers.push(crate::docker::ContainerStatusInfo {
            id: "123".to_string(),
            name: "web-search".to_string(),
            image: "ghcr.io/aas-ee/open-web-search:latest".to_string(),
            state: "running".to_string(),
            status: "Up".to_string(),
            running: true,
            ports: vec!["3000:3000".to_string()],
            configured: true,
            is_managed: true,
            labels: HashMap::new(),
            daemon_online: true,
            error: None,
            env_count: 0,
            mount_count: 0,
            network_name: "bridge".to_string(),
            auto_start: true,
        });

        let items = Mcps::build_items(&state);

        let local_stdio = items.iter().find(|i| i.name == "local-stdio").unwrap();
        assert_eq!(local_stdio.scope, "local");
        assert_eq!(local_stdio.transport, "stdio");

        let local_http = items.iter().find(|i| i.name == "local-http").unwrap();
        assert_eq!(local_http.scope, "local");
        assert_eq!(local_http.transport, "streamable-http");

        let remote_sse = items.iter().find(|i| i.name == "remote-sse").unwrap();
        assert_eq!(remote_sse.scope, "remote");
        assert_eq!(remote_sse.transport, "sse");

        let web_search = items.iter().find(|i| i.name == "web-search").unwrap();
        assert_eq!(web_search.scope, "local");
        assert_eq!(web_search.transport, "streamable-http");
        assert_ne!(web_search.transport, "docker");
    }
}

