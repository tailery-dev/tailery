use color_eyre::Result;
use ratatui::prelude::Widget;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Row, StatefulWidget, Table},
};
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    action::Action, adapters::all_adapters, components::Component, state::AppState, tui::Event,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToggleTarget {
    Server(String),
    Client(String),
}

#[derive(Default)]
pub struct ProfileEditor {
    pub profile_name: String,
    pub focused_pane: usize, // 0: MCP Servers, 1: AI Clients
    pub server_selected_index: usize,
    pub client_selected_index: usize,
    pub command_tx: Option<UnboundedSender<Action>>,
}

impl ProfileEditor {
    pub fn new(profile_name: &str) -> Self {
        Self {
            profile_name: profile_name.to_string(),
            focused_pane: 0,
            server_selected_index: 0,
            client_selected_index: 0,
            command_tx: None,
        }
    }

    pub fn switch_pane(&mut self) {
        self.focused_pane = if self.focused_pane == 0 { 1 } else { 0 };
    }

    pub fn move_up(&mut self) {
        if self.focused_pane == 0 {
            if self.server_selected_index > 0 {
                self.server_selected_index -= 1;
            }
        } else if self.client_selected_index > 0 {
            self.client_selected_index -= 1;
        }
    }

    pub fn move_down(&mut self, total_servers: usize, total_clients: usize) {
        if self.focused_pane == 0 {
            if total_servers > 0 && self.server_selected_index + 1 < total_servers {
                self.server_selected_index += 1;
            }
        } else if total_clients > 0 && self.client_selected_index + 1 < total_clients {
            self.client_selected_index += 1;
        }
    }

    pub fn current_target(&self, sorted_server_names: &[String]) -> Option<ToggleTarget> {
        if self.focused_pane == 0 {
            sorted_server_names
                .get(self.server_selected_index)
                .map(|s| ToggleTarget::Server(s.clone()))
        } else {
            all_adapters()
                .get(self.client_selected_index)
                .map(|a| ToggleTarget::Client(a.name().to_string()))
        }
    }
}

impl Component for ProfileEditor {
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

impl StatefulWidget for &ProfileEditor {
    type State = AppState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(
                format!(" Edit Profile: [{}] ", self.profile_name),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ))
            .style(Style::default().bg(Color::Rgb(15, 20, 30)));

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 8 || inner.width < 35 {
            return;
        }

        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Profile summary status banner
                Constraint::Min(6),    // Columns: Servers (Left) & Clients (Right)
                Constraint::Length(3), // Action bar / shortcuts
            ])
            .split(inner);

        let mut sorted_server_names: Vec<String> = state.servers.keys().cloned().collect();
        for k in state.configured_containers.keys() {
            if !sorted_server_names.contains(k) {
                sorted_server_names.push(k.clone());
            }
        }
        sorted_server_names.sort();

        let profile_cfg = state.profiles.get(&self.profile_name);
        let is_active = state.settings.active_profile == self.profile_name;
        let server_count = profile_cfg.map(|p| p.enabled_servers.len()).unwrap_or(0);
        let total_servers = sorted_server_names.len();
        let client_count = profile_cfg.map(|p| p.enabled_clients.len()).unwrap_or(0);
        let adapters = all_adapters();
        let total_clients = adapters.len();

        let (status_badge, status_style) = if is_active {
            (
                "● CURRENT ACTIVE",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            ("○ INACTIVE", Style::default().fg(Color::DarkGray))
        };

        // 1. Top Summary Banner
        let summary_lines = vec![Line::from(vec![
            Span::styled("Profile: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("[{}] ", self.profile_name),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(status_badge, status_style),
            Span::raw(" │ "),
            Span::styled("Active Servers: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}/{}", server_count, total_servers),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" │ "),
            Span::styled("Enabled Clients: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}/{}", client_count, total_clients),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ])];

        let summary_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(Span::styled(
                " Configuration Scope ",
                Style::default().fg(Color::White),
            ));
        Paragraph::new(summary_lines)
            .block(summary_block)
            .render(main_chunks[0], buf);

        // 2. Split Columns: Left = MCP Servers, Right = Clients
        let col_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(55), // MCP Servers
                Constraint::Percentage(45), // Clients
            ])
            .split(main_chunks[1]);

        let is_server_pane_focused = self.focused_pane == 0;
        let is_client_pane_focused = self.focused_pane == 1;

        // Left Pane: MCP Servers Table
        let server_rows: Vec<Row> = sorted_server_names
            .iter()
            .enumerate()
            .map(|(idx, name)| {
                let is_selected = is_server_pane_focused && idx == self.server_selected_index;
                let is_enabled = profile_cfg
                    .map(|p| p.is_server_enabled(name))
                    .unwrap_or(false);

                let (check_badge, check_style) = if is_enabled {
                    (
                        "[x]",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    ("[ ]", Style::default().fg(Color::DarkGray))
                };

                let name_style = if is_selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else if is_enabled {
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::DarkGray)
                };

                let transport = state
                    .servers
                    .get(name)
                    .map(|s| match s {
                        crate::state::ServerConfig::Local { transport, .. } => match transport {
                            crate::state::LocalTransport::Stdio => "stdio",
                            crate::state::LocalTransport::StreamableHttp { .. } => {
                                "streamable-http"
                            }
                            crate::state::LocalTransport::Http { .. } => "http",
                            crate::state::LocalTransport::Sse { .. } => "sse",
                        },
                        crate::state::ServerConfig::Remote { transport, .. } => match transport {
                            crate::state::RemoteTransport::StreamableHttp => "streamable-http",
                            crate::state::RemoteTransport::Http => "http",
                            crate::state::RemoteTransport::Sse => "sse",
                        },
                    })
                    .unwrap_or("custom");

                Row::new(vec![
                    Span::styled(format!(" {} ", check_badge), check_style),
                    Span::styled(format!(" {} ", name), name_style),
                    Span::styled(transport, Style::default().fg(Color::DarkGray)),
                ])
            })
            .collect();

        let server_table = Table::new(
            server_rows,
            [
                Constraint::Length(5),
                Constraint::Percentage(65),
                Constraint::Percentage(25),
            ],
        )
        .header(
            Row::new(vec!["State", "Server Identifier", "Transport"]).style(
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(if is_server_pane_focused {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default().fg(Color::DarkGray)
                })
                .title(Span::styled(
                    format!(" [1] MCP Servers ({}/{}) ", server_count, total_servers),
                    if is_server_pane_focused {
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    },
                )),
        );
        Widget::render(server_table, col_chunks[0], buf);

        // Right Pane: Clients Table
        let client_rows: Vec<Row> = adapters
            .iter()
            .enumerate()
            .map(|(idx, adapter)| {
                let is_selected = is_client_pane_focused && idx == self.client_selected_index;
                let is_enabled = profile_cfg
                    .map(|p| p.is_client_enabled(adapter.name()))
                    .unwrap_or(false);
                let is_detected = adapter.detect_installed();

                let (check_badge, check_style) = if is_enabled {
                    (
                        "[x]",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    ("[ ]", Style::default().fg(Color::DarkGray))
                };

                let name_style = if is_selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else if is_enabled {
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::DarkGray)
                };

                let (detect_badge, detect_style) = if is_detected {
                    ("● INSTALLED", Style::default().fg(Color::Green))
                } else {
                    ("○ NOT FOUND", Style::default().fg(Color::DarkGray))
                };

                Row::new(vec![
                    Span::styled(format!(" {} ", check_badge), check_style),
                    Span::styled(format!(" {} ", adapter.display_name()), name_style),
                    Span::styled(detect_badge, detect_style),
                ])
            })
            .collect();

        let client_table = Table::new(
            client_rows,
            [
                Constraint::Length(5),
                Constraint::Percentage(55),
                Constraint::Percentage(35),
            ],
        )
        .header(
            Row::new(vec!["State", "Client Adapter", "Host Status"]).style(
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(if is_client_pane_focused {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default().fg(Color::DarkGray)
                })
                .title(Span::styled(
                    format!(" [2] AI Clients ({}/{}) ", client_count, total_clients),
                    if is_client_pane_focused {
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    },
                )),
        );
        Widget::render(client_table, col_chunks[1], buf);

        // 3. Action Bar
        let action_lines = vec![
            Line::from(vec![
                Span::styled(
                    " [Space/Enter] ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("Toggle Highlighted Item   "),
                Span::styled(
                    " [Tab / h / l] ",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("Switch Pane (Servers ↔ Clients)"),
            ]),
            Line::from(vec![
                Span::styled(
                    " [a] ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("Toggle All in Current Pane   "),
                Span::styled(
                    " [j/k or ↑/↓] ",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("Navigate List   "),
                Span::styled(
                    " [Esc] ",
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("Done & Save"),
            ]),
        ];
        Paragraph::new(action_lines).render(main_chunks[2], buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_editor_state_toggle_and_navigation() {
        let mut editor = ProfileEditor::new("test-prof");
        assert_eq!(editor.focused_pane, 0);

        // Test navigation
        editor.move_down(5, 4);
        assert_eq!(editor.server_selected_index, 1);
        editor.move_up();
        assert_eq!(editor.server_selected_index, 0);

        // Switch pane
        editor.switch_pane();
        assert_eq!(editor.focused_pane, 1);
        editor.move_down(5, 4);
        assert_eq!(editor.client_selected_index, 1);

        let sorted_servers = vec!["srv-1".to_string(), "srv-2".to_string()];

        // In clients pane, target index 1 should be client "claude_code"
        let target = editor.current_target(&sorted_servers);
        assert_eq!(
            target,
            Some(ToggleTarget::Client("claude_code".to_string()))
        );

        // Switch back to servers pane
        editor.switch_pane();
        assert_eq!(editor.focused_pane, 0);
        let target_srv = editor.current_target(&sorted_servers);
        assert_eq!(target_srv, Some(ToggleTarget::Server("srv-1".to_string())));
    }
}
