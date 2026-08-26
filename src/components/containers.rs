use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Row, Table, StatefulWidget},
};

use crate::action::Action;
use crate::components::Component;
use crate::tui::Event;
use crate::state::AppState;

#[derive(Debug, Default, PartialEq, Eq, Clone, Copy)]
pub enum ContainerFilterMode {
    #[default]
    ManagedOnly,
    All,
}

#[derive(Default)]
pub struct Containers {
    pub selected_index: usize,
    pub filter_mode: ContainerFilterMode,
}

impl Component for Containers {
    fn register_action_handler(&mut self, _tx: tokio::sync::mpsc::UnboundedSender<Action>) -> color_eyre::Result<()> {
        Ok(())
    }

    fn handle_events(&mut self, event: Option<Event>) -> color_eyre::Result<Option<Action>> {
        let Some(Event::Key(key)) = event else {
            return Ok(None);
        };
        match key.code {
            crossterm::event::KeyCode::Char('j') | crossterm::event::KeyCode::Down => {
                self.selected_index = self.selected_index.saturating_add(1);
            }
            crossterm::event::KeyCode::Char('k') | crossterm::event::KeyCode::Up => {
                self.selected_index = self.selected_index.saturating_sub(1);
            }
            crossterm::event::KeyCode::Char('f') => {
                self.filter_mode = match self.filter_mode {
                    ContainerFilterMode::ManagedOnly => ContainerFilterMode::All,
                    ContainerFilterMode::All => ContainerFilterMode::ManagedOnly,
                };
            }
            _ => {}
        }
        Ok(None)
    }

    fn update(&mut self, _action: Action) -> color_eyre::Result<Option<Action>> {
        Ok(None)
    }
}

impl StatefulWidget for &Containers {
    type State = AppState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(55),
                Constraint::Percentage(45),
            ])
            .split(area);

        let top_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(58),
                Constraint::Percentage(42),
            ])
            .split(main_chunks[0]);

        let containers = &state.containers;
        // In the old code, containers were filtered before being passed to the widget.
        // We'll filter them here based on the filter mode.
        let filtered_containers: Vec<_> = containers.iter().filter(|c| {
            match self.filter_mode {
                ContainerFilterMode::ManagedOnly => c.is_managed,
                ContainerFilterMode::All => true,
            }
        }).collect();

        // Clamp selected index
        let clamped_index = if filtered_containers.is_empty() {
            0
        } else {
            self.selected_index.min(filtered_containers.len() - 1)
        };
        
        let selected_container = filtered_containers.get(clamped_index).copied();
        let daemon_online = state.docker_status.contains("Online");

        let rows: Vec<Row> = filtered_containers
            .iter()
            .enumerate()
            .map(|(idx, c)| {
                let is_selected = idx == clamped_index;
                let (status_badge, status_style) = if c.running {
                    (
                        "● RUNNING",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    )
                } else if !c.daemon_online {
                    (
                        "○ OFFLINE",
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    )
                } else if c.configured {
                    ("○ STOPPED", Style::default().fg(Color::Red))
                } else {
                    ("● EXTERNAL", Style::default().fg(Color::Cyan))
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

                let ports_str = if c.ports.is_empty() {
                    "-".to_string()
                } else {
                    c.ports.join(", ")
                };

                let origin_badge = if c.configured {
                    Span::styled(" [CONFIG] ", Style::default().fg(Color::Cyan))
                } else if c.is_managed {
                    Span::styled(" [MANAGED] ", Style::default().fg(Color::Magenta))
                } else {
                    Span::styled(" [EXT] ", Style::default().fg(Color::DarkGray))
                };

                Row::new(vec![
                    Span::styled(format!(" {} ", c.name), name_style),
                    Span::styled(status_badge, status_style),
                    Span::styled(&c.image, Style::default().fg(Color::Magenta)),
                    Span::styled(ports_str, Style::default().fg(Color::Cyan)),
                    origin_badge,
                ])
            })
            .collect();

        let daemon_status_str = if daemon_online { "Online" } else { "Offline" };
        let filter_str = match self.filter_mode {
            ContainerFilterMode::ManagedOnly => format!(
                "Filter: Managed Only ({} shown) │ [f] All",
                filtered_containers.len()
            ),
            ContainerFilterMode::All => format!(
                "Filter: All ({} total) │ [f] Filter",
                containers.len()
            ),
        };

        let table_title = format!(
            " Docker & Podman Containers ({} │ Engine: {}) ",
            filter_str, daemon_status_str
        );

        let table = Table::new(
            rows,
            [
                Constraint::Percentage(28),
                Constraint::Percentage(18),
                Constraint::Percentage(30),
                Constraint::Percentage(14),
                Constraint::Percentage(10),
            ],
        )
        .header(
            Row::new(vec!["Container Name", "Status", "Image", "Ports", "Origin"]).style(
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
        ratatui::widgets::Widget::render(table, top_chunks[0], buf);

        let config_path = crate::state::resolve_config_path(None).to_string_lossy().to_string();

        let detail_lines = if let Some(c) = selected_container {
            let (status_text, status_color) = if c.running {
                ("● Running in container", Color::Green)
            } else if !c.daemon_online {
                ("○ Offline (Docker daemon not running)", Color::Red)
            } else {
                ("○ Stopped (Saved in config, ready to launch)", Color::Red)
            };

            let ports_str = if c.ports.is_empty() {
                "None (Host isolated)".to_string()
            } else {
                c.ports.join(", ")
            };

            let mut lines = vec![
                Line::from(vec![
                    Span::styled("Container:    ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        &c.name,
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("Status:       ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        status_text,
                        Style::default()
                            .fg(status_color)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("Image:        ", Style::default().fg(Color::DarkGray)),
                    Span::styled(&c.image, Style::default().fg(Color::White)),
                ]),
                Line::from(vec![
                    Span::styled("Published:    ", Style::default().fg(Color::DarkGray)),
                    Span::styled(ports_str, Style::default().fg(Color::Cyan)),
                ]),
                Line::from(vec![
                    Span::styled("Config Store: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(&config_path, Style::default().fg(Color::DarkGray)),
                ]),
            ];

            if let Some(ref err) = c.error {
                lines.push(Line::from(vec![
                    Span::styled("Daemon State: ", Style::default().fg(Color::Red)),
                    Span::styled(err, Style::default().fg(Color::Red)),
                ]));
            }

            lines.push(Line::raw(""));
            lines.push(Line::from(Span::styled(
                "─── Actions ───",
                Style::default().fg(Color::Magenta),
            )));

            if c.daemon_online {
                lines.push(Line::from(vec![
                    Span::styled(
                        " [s] ",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(if c.running {
                        " Stop Container"
                    } else {
                        " Start Container"
                    }),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::styled(
                        " [!] ",
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        " Start Docker runtime to launch",
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
            }

            lines.push(Line::from(vec![
                Span::styled(
                    " [f] ",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(match self.filter_mode {
                    ContainerFilterMode::ManagedOnly => " Show All Host Containers",
                    ContainerFilterMode::All => " Filter to Managed Only",
                }),
            ]));
            lines.push(Line::from(vec![
                Span::styled(
                    " [+] ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" Register Port as MCP HTTP Server"),
            ]));
            lines.push(Line::from(vec![
                Span::styled(
                    " [d] ",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" Delete Container from Config"),
            ]));

            lines
        } else {
            vec![
                Line::from(Span::styled(
                    "No containers configured or found.",
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
                        " to configure and add an MCP server or container.",
                        Style::default().fg(Color::White),
                    ),
                ]),
                Line::raw(""),
                Line::from(vec![
                    Span::styled("Config location: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(&config_path, Style::default().fg(Color::DarkGray)),
                ]),
            ]
        };

        let detail_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Magenta))
            .title(Span::styled(
                " Container Inspection ",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ));
        ratatui::widgets::Widget::render(Paragraph::new(detail_lines).block(detail_block), top_chunks[1], buf);

        let selected_name = selected_container
            .map(|c| c.name.as_str())
            .unwrap_or("None");
        let log_title = format!(" Container Output Logs for '{}' ", selected_name);

        let mut log_lines: Vec<Line> = Vec::new();
        if !daemon_online {
            log_lines.push(Line::from(vec![
                Span::styled(
                    " ○ DOCKER DAEMON OFFLINE ",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                Span::raw("  Could not reach /var/run/docker.sock or podman.sock"),
            ]));
            log_lines.push(Line::raw(""));
            log_lines.push(Line::from(Span::styled(
                " • Configured containers are safely stored in your XDG config file:",
                Style::default().fg(Color::White),
            )));
            log_lines.push(Line::from(vec![
                Span::raw("   "),
                Span::styled(&config_path, Style::default().fg(Color::Cyan)),
            ]));
            log_lines.push(Line::raw(""));
            log_lines.push(Line::from(Span::styled(
                " • To launch and stream logs:",
                Style::default().fg(Color::White),
            )));
            log_lines.push(Line::from(Span::styled(
                "   1. Launch Docker Desktop, OrbStack, or run 'podman machine start'",
                Style::default().fg(Color::DarkGray),
            )));
            log_lines.push(Line::from(Span::styled("   2. Tailery will auto-discover the runtime socket and enable start/stop controls", Style::default().fg(Color::DarkGray))));
        } else if state.container_logs.is_empty() {
            log_lines.push(Line::from(Span::styled(
                "No logs available (container is stopped or has produced no output yet).",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            for line in state.container_logs.iter().rev().take(50).rev() {
                let style =
                    if line.contains("ERROR") || line.contains("error") || line.contains("ERR") {
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
                log_title,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));
        ratatui::widgets::Widget::render(Paragraph::new(log_lines).block(logs_block), main_chunks[1], buf);
    }
}
