use color_eyre::Result;
use ratatui::prelude::Widget;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Row, Table, StatefulWidget},
};

use crate::action::Action;
use crate::components::Component;
use crate::state::AppState;
use crate::tui::Event;

#[derive(Default)]
pub struct Greeting {
    pub docker_status: String,
    pub detected_clients: Vec<(String, bool)>,
    pub socket_path: String,
}

impl StatefulWidget for &Greeting {
    type State = AppState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        // Vertical layout: Compact Header (Logo + Title) -> 2x2 Dashboard Grid
        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(6), // Compact Logo + Greeting Header
                Constraint::Min(12),   // 2x2 Dashboard Grid
            ])
            .split(area);

        // 1. Compact Logo & Title Header
        let cyan_bold = Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD);
        let magenta_bold = Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD);
        let white_bold = Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD);
        let gray_style = Style::default().fg(Color::DarkGray);

        let header_lines = vec![
            Line::from(vec![Span::styled("  ┌───────┐", cyan_bold)]),
            Line::from(vec![
                Span::styled("  │ ", cyan_bold),
                Span::styled("TAILERY", magenta_bold),
                Span::styled(" │  ", cyan_bold),
                Span::styled("tailery ", white_bold),
                Span::styled(format!("v{}", env!("CARGO_PKG_VERSION")), gray_style),
            ]),
            Line::from(vec![
                Span::styled("  │       │  ", cyan_bold),
                Span::styled(
                    "MCP Control Plane & Sandbox Harness",
                    Style::default().fg(Color::Cyan),
                ),
            ]),
            Line::from(vec![
                Span::styled("  │       │  ", cyan_bold),
                Span::styled(
                    "Decentralized, local-first endpoint agent & multi-client sync",
                    Style::default().fg(Color::DarkGray),
                ),
            ]),
            Line::from(vec![Span::styled("  └───────┘", cyan_bold)]),
        ];

        let header_widget = Paragraph::new(header_lines);
        header_widget.render(main_chunks[0], buf);

        // 2. Generic 2x2 Grid
        let grid_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(main_chunks[1]);

        let top_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(grid_rows[0]);

        let bottom_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(grid_rows[1]);

        // Panel 1: Security & Sandboxing Policies
        let sec_lines = vec![
            Line::from(vec![
                Span::styled("Sandboxing:       ", white_bold),
                Span::styled(
                    "Docker Container (stdio-in-container)",
                    Style::default().fg(Color::Green),
                ),
            ]),
            Line::from(vec![
                Span::styled("Root Filesystem:  ", white_bold),
                Span::styled(
                    "Enforced Read-Only by Default",
                    Style::default().fg(Color::Green),
                ),
            ]),
            Line::from(vec![
                Span::styled("Network Egress:   ", white_bold),
                Span::styled(
                    "Isolated ('none') Default",
                    Style::default().fg(Color::Magenta),
                ),
            ]),
            Line::from(vec![
                Span::styled("Secret Storage:   ", white_bold),
                Span::styled(
                    "Zero-Trust OS Keychain / 1Password",
                    Style::default().fg(Color::Cyan),
                ),
            ]),
            Line::from(vec![
                Span::styled("Interception:     ", white_bold),
                Span::styled(
                    "Per-Server Headless Process Shim",
                    Style::default().fg(Color::Magenta),
                ),
            ]),
        ];

        let sec_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(
                " Security & Sandbox Architecture ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));
        Paragraph::new(sec_lines)
            .block(sec_block)
            .render(top_cols[0], buf);

        // Panel 2: Detected IDE & Client Adapters
        let mut client_lines = Vec::new();
        for (client, installed) in &self.detected_clients {
            let (icon, color, text) = if *installed {
                ("● Detected", Color::Green, "Ready to Sync")
            } else {
                ("○ Not Found", Color::DarkGray, "Not Detected")
            };
            client_lines.push(Line::from(vec![
                Span::styled(format!("{:<14}", client), white_bold),
                Span::styled(format!("{:<14}", icon), Style::default().fg(color)),
                Span::styled(text, gray_style),
            ]));
        }

        let clients_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Magenta))
            .title(Span::styled(
                " Multi-Client Sync Adapters ",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ));
        Paragraph::new(client_lines)
            .block(clients_block)
            .render(top_cols[1], buf);

        // Panel 3: Keyboard Shortcuts & Navigation
        let shortcut_lines = vec![
            Line::from(vec![
                Span::styled(
                    " [Tab] ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" Switch Views           "),
                Span::styled(
                    " [p] ",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" Profiles"),
            ]),
            Line::from(vec![
                Span::styled(
                    " [b]   ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" MCP Registry Browser   "),
                Span::styled(
                    " [?] ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" Shortcuts Help"),
            ]),
            Line::from(vec![
                Span::styled(
                    " [s]   ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" Sync Config to Clients "),
                Span::styled(
                    " [q] ",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" Quit Tailery"),
            ]),
        ];

        let shortcuts_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(
                " Navigation & Shortcuts ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));
        Paragraph::new(shortcut_lines)
            .block(shortcuts_block)
            .render(bottom_cols[0], buf);

        // Panel 4: Quick Server Overview
        let rows: Vec<Row> = state
            .servers
            .iter()
            .take(5)
            .map(|(name, s)| {
                let transport_str = match s {
                    crate::state::ServerConfig::Local { transport, .. } => match transport {
                        crate::state::LocalTransport::Stdio => "stdio",
                        crate::state::LocalTransport::StreamableHttp { .. } => "streamable-http",
                        crate::state::LocalTransport::Http { .. } => "http",
                        crate::state::LocalTransport::Sse { .. } => "sse",
                    },
                    crate::state::ServerConfig::Remote { transport, .. } => match transport {
                        crate::state::RemoteTransport::StreamableHttp => "streamable-http",
                        crate::state::RemoteTransport::Http => "http",
                        crate::state::RemoteTransport::Sse => "sse",
                    },
                };
                Row::new(vec![
                    Span::styled(name.clone(), white_bold),
                    Span::styled(transport_str, Style::default().fg(Color::Magenta)),
                    Span::styled("● Ready", Style::default().fg(Color::Green)),
                ])
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Percentage(40),
                Constraint::Percentage(30),
                Constraint::Percentage(30),
            ],
        )
        .header(
            Row::new(vec!["Server", "Transport", "Status"]).style(
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Magenta))
                .title(Span::styled(
                    " Active MCP Inventory ",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                )),
        );
        Widget::render(table, bottom_cols[1], buf);
    }
}

impl Component for Greeting {
    fn handle_events(&mut self, _event: Option<Event>) -> Result<Option<Action>> {
        Ok(None)
    }
}
