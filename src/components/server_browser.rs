use color_eyre::Result;
use ratatui::prelude::Widget;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Row, StatefulWidget, Table},
};

use crate::action::Action;
use crate::components::Component;
use crate::state::{AppState, ServerConfig};
use crate::tui::Event;

#[derive(Default)]
pub struct ServerBrowser {
    pub selected_index: usize,
}

impl StatefulWidget for &ServerBrowser {
    type State = AppState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(area);

        let server_entries: Vec<(&String, &ServerConfig)> = state.servers.iter().collect();
        let selected_entry = server_entries.get(self.selected_index);

        // 1. Left Pane: Server Inventory Table
        let active_profile = &state.settings.active_profile;
        let enabled_in_profile = state
            .profiles
            .get(active_profile)
            .map(|p| &p.enabled_servers);

        let rows: Vec<Row> = server_entries
            .iter()
            .enumerate()
            .map(|(idx, (name, s))| {
                let is_selected = idx == self.selected_index;
                let is_enabled = enabled_in_profile
                    .map(|list| list.contains(*name))
                    .unwrap_or(true);

                let status_badge = if is_enabled {
                    Span::styled(
                        "● ACTIVE",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    Span::styled("○ DISABLED", Style::default().fg(Color::DarkGray))
                };

                let (transport_badge, sandbox_badge) = match s {
                    crate::state::ServerConfig::Local {
                        transport,
                        container,
                        ..
                    } => {
                        let t_str = match transport {
                            crate::state::LocalTransport::Stdio => "stdio",
                            crate::state::LocalTransport::StreamableHttp { .. } => {
                                "streamable-http"
                            }
                            crate::state::LocalTransport::Http { .. } => "http",
                            crate::state::LocalTransport::Sse { .. } => "sse",
                        };
                        (
                            Span::styled(t_str, Style::default().fg(Color::Cyan)),
                            if container.read_only_rootfs {
                                Span::styled(
                                    "docker-ro",
                                    Style::default()
                                        .fg(Color::Green)
                                        .add_modifier(Modifier::BOLD),
                                )
                            } else {
                                Span::styled("docker-rw", Style::default().fg(Color::Yellow))
                            },
                        )
                    }
                    crate::state::ServerConfig::Remote { transport, .. } => {
                        let t_str = match transport {
                            crate::state::RemoteTransport::StreamableHttp => "streamable-http",
                            crate::state::RemoteTransport::Http => "http",
                            crate::state::RemoteTransport::Sse => "sse",
                        };
                        (
                            Span::styled(t_str, Style::default().fg(Color::Cyan)),
                            Span::styled("remote", Style::default().fg(Color::Magenta)),
                        )
                    }
                };

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

                Row::new(vec![
                    Span::styled(format!(" {} ", name), name_style),
                    transport_badge,
                    sandbox_badge,
                    status_badge,
                ])
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Percentage(40),
                Constraint::Percentage(20),
                Constraint::Percentage(20),
                Constraint::Percentage(20),
            ],
        )
        .header(
            Row::new(vec![
                Span::styled(
                    "SERVER NAME",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "TRANSPORT",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "SANDBOX",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "STATUS",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
            ])
            .bottom_margin(1),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(" Configured MCP Servers "),
        );
        Widget::render(table, chunks[0], buf);

        // 2. Right Pane: Selected Server Inspector Detail
        let detail_lines = if let Some((name, config)) = selected_entry {
            let mut lines = vec![
                Line::from(vec![
                    Span::styled("Server:        ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        *name,
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::raw(""),
            ];

            match config {
                crate::state::ServerConfig::Local {
                    command,
                    args,
                    env,
                    tool_filter,
                    container,
                    transport,
                } => {
                    let transport_str = match transport {
                        crate::state::LocalTransport::Stdio => "stdio (Docker Container with Shim)",
                        crate::state::LocalTransport::StreamableHttp { .. } => {
                            "streamable-http (Docker Container with Shim)"
                        }
                        crate::state::LocalTransport::Http { .. } => {
                            "http (Docker Container with Shim)"
                        }
                        crate::state::LocalTransport::Sse { .. } => {
                            "sse (Docker Container with Shim)"
                        }
                    };
                    lines.push(Line::from(vec![
                        Span::styled("Transport:     ", Style::default().fg(Color::DarkGray)),
                        Span::styled(
                            transport_str,
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]));
                    if !container.image.is_empty() {
                        lines.push(Line::from(vec![
                            Span::styled("Docker Image:  ", Style::default().fg(Color::DarkGray)),
                            Span::styled(&container.image, Style::default().fg(Color::White)),
                        ]));
                    }
                    if let Some(cmd) = command {
                        lines.push(Line::from(vec![
                            Span::styled("Command:       ", Style::default().fg(Color::DarkGray)),
                            Span::styled(
                                format!("{} {}", cmd, args.join(" ")),
                                Style::default().fg(Color::White),
                            ),
                        ]));
                    }
                    lines.push(Line::from(vec![
                        Span::styled("Root Filesystem:", Style::default().fg(Color::DarkGray)),
                        Span::styled(
                            if container.read_only_rootfs {
                                " Read-Only (Enforced)"
                            } else {
                                " Read-Write"
                            },
                            Style::default().fg(if container.read_only_rootfs {
                                Color::Green
                            } else {
                                Color::Red
                            }),
                        ),
                    ]));
                    lines.push(Line::from(vec![
                        Span::styled("Network Mode:  ", Style::default().fg(Color::DarkGray)),
                        Span::styled(&container.network, Style::default().fg(Color::Cyan)),
                    ]));
                    if let Some(res) = &container.resources {
                        lines.push(Line::from(vec![
                            Span::styled("Memory Limit:  ", Style::default().fg(Color::DarkGray)),
                            Span::styled(
                                res.memory_mb
                                    .map(|m| format!("{} MB", m))
                                    .unwrap_or_else(|| "unlimited".to_string()),
                                Style::default().fg(Color::White),
                            ),
                            Span::styled(" │ CPUs: ", Style::default().fg(Color::DarkGray)),
                            Span::styled(
                                res.cpus
                                    .map(|c| format!("{:.1}", c))
                                    .unwrap_or_else(|| "unlimited".to_string()),
                                Style::default().fg(Color::White),
                            ),
                        ]));
                    }
                    if !container.mounts.is_empty() {
                        lines.push(Line::raw(""));
                        lines.push(Line::from(Span::styled(
                            "Volume Mounts:",
                            Style::default().fg(Color::DarkGray),
                        )));
                        for m in &container.mounts {
                            lines.push(Line::from(vec![
                                Span::styled(
                                    format!("  {} -> {}", m.host, m.guest),
                                    Style::default().fg(Color::White),
                                ),
                                Span::styled(
                                    if m.read_only { " [ro]" } else { " [rw]" },
                                    Style::default().fg(if m.read_only {
                                        Color::Green
                                    } else {
                                        Color::Red
                                    }),
                                ),
                            ]));
                        }
                    }
                    if !env.is_empty() {
                        lines.push(Line::raw(""));
                        lines.push(Line::from(Span::styled(
                            "Environment Variables:",
                            Style::default().fg(Color::DarkGray),
                        )));
                        for (k, v) in env {
                            lines.push(Line::from(vec![
                                Span::styled(
                                    format!("  {}=", k),
                                    Style::default().fg(Color::Magenta),
                                ),
                                Span::styled(v, Style::default().fg(Color::White)),
                            ]));
                        }
                    }
                    lines.push(Line::raw(""));
                    lines.push(Line::from(Span::styled(
                        "Security Tool Policy:",
                        Style::default().fg(Color::DarkGray),
                    )));
                    let allow_str = if tool_filter.allow.is_empty() {
                        "* (All)".to_string()
                    } else {
                        tool_filter.allow.join(", ")
                    };
                    let deny_str = if tool_filter.deny.is_empty() {
                        "(None)".to_string()
                    } else {
                        tool_filter.deny.join(", ")
                    };
                    lines.push(Line::from(vec![
                        Span::styled("  Allow: ", Style::default().fg(Color::Green)),
                        Span::styled(allow_str, Style::default().fg(Color::White)),
                    ]));
                    lines.push(Line::from(vec![
                        Span::styled("  Deny:  ", Style::default().fg(Color::Red)),
                        Span::styled(deny_str, Style::default().fg(Color::White)),
                    ]));
                }
                crate::state::ServerConfig::Remote {
                    url,
                    transport,
                    headers,
                    ..
                } => {
                    let transport_str = match transport {
                        crate::state::RemoteTransport::StreamableHttp => "Streamable HTTP",
                        crate::state::RemoteTransport::Http => "HTTP",
                        crate::state::RemoteTransport::Sse => "Server-Sent Events (SSE)",
                    };
                    lines.push(Line::from(vec![
                        Span::styled("Transport:     ", Style::default().fg(Color::DarkGray)),
                        Span::styled(
                            transport_str,
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]));
                    lines.push(Line::from(vec![
                        Span::styled("Endpoint URL:  ", Style::default().fg(Color::DarkGray)),
                        Span::styled(url, Style::default().fg(Color::Cyan)),
                    ]));
                    if !headers.is_empty() {
                        lines.push(Line::raw(""));
                        lines.push(Line::from(Span::styled(
                            "Headers:",
                            Style::default().fg(Color::DarkGray),
                        )));
                        for (k, v) in headers {
                            lines.push(Line::from(vec![
                                Span::styled(
                                    format!("  {}: ", k),
                                    Style::default().fg(Color::Magenta),
                                ),
                                Span::styled(v, Style::default().fg(Color::White)),
                            ]));
                        }
                    }
                }
            }

            lines
        } else {
            vec![Line::from(Span::styled(
                "No server selected.",
                Style::default().fg(Color::DarkGray),
            ))]
        };

        let detail_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Magenta))
            .title(Span::styled(
                " Server Configuration & Security Policy ",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ));
        Paragraph::new(detail_lines)
            .block(detail_block)
            .render(chunks[1], buf);
    }
}

impl Component for ServerBrowser {
    fn handle_events(&mut self, _event: Option<Event>) -> Result<Option<Action>> {
        Ok(None)
    }
}
