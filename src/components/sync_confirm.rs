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
use crate::adapters::all_adapters;
use crate::components::Component;
use crate::state::AppState;
use crate::tui::Event;

#[derive(Default)]
pub struct SyncConfirm;

impl StatefulWidget for &SyncConfirm {
    type State = AppState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(
                " Confirm Client Synchronization ",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ))
            .style(Style::default().bg(Color::Rgb(15, 20, 30)));

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 8 || inner.width < 30 {
            return;
        }

        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(4), // Header / description
                Constraint::Min(5),    // Table of target clients & paths
                Constraint::Length(2), // Safety notice
                Constraint::Length(2), // Action bar
            ])
            .split(inner);

        let active_profile = &state.settings.active_profile;
        let servers_to_sync = state.get_active_profile_servers();
        let server_count = servers_to_sync.len();

        let adapters = all_adapters();
        let enabled_client_count = adapters
            .iter()
            .filter(|a| state.is_client_enabled_in_active_profile(a.name()))
            .count();

        // 1. Header description
        let header_lines = vec![
            Line::from(vec![Span::styled(
                "Synchronize MCP configuration for active profile across enabled clients?",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )]),
            Line::raw(""),
            Line::from(vec![
                Span::styled("Active Profile: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("[{}]", active_profile),
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("  │  Servers: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{} server(s)", server_count),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "  │  Target Clients: ",
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    format!("{}/{} enabled", enabled_client_count, adapters.len()),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
        ];
        Paragraph::new(header_lines).render(main_chunks[0], buf);

        // 2. Client list table
        let rows: Vec<Row> = adapters
            .iter()
            .map(|adapter| {
                let is_enabled = state.is_client_enabled_in_active_profile(adapter.name());
                let (profile_badge, profile_style) = if is_enabled {
                    (
                        "● ENABLED",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    ("○ DISABLED (Skip)", Style::default().fg(Color::Yellow))
                };

                let installed = adapter.detect_installed();
                let (host_badge, host_style) = if installed {
                    ("● DETECTED", Style::default().fg(Color::Green))
                } else {
                    ("○ NOT FOUND", Style::default().fg(Color::DarkGray))
                };

                let path_str = adapter
                    .config_path(None)
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|_| "Unavailable".to_string());

                Row::new(vec![
                    Span::styled(
                        format!(" {} ", adapter.display_name()),
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(profile_badge, profile_style),
                    Span::styled(host_badge, host_style),
                    Span::styled(path_str, Style::default().fg(Color::DarkGray)),
                ])
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Length(16),
                Constraint::Length(18),
                Constraint::Length(14),
                Constraint::Min(20),
            ],
        )
        .header(
            Row::new(vec![
                "Client Adapter",
                "Profile Status",
                "Host Status",
                "Target Config Path",
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
                .border_style(Style::default().fg(Color::DarkGray))
                .title(Span::styled(
                    " Target Clients in Active Profile ",
                    Style::default().fg(Color::White),
                )),
        );
        Widget::render(table, main_chunks[1], buf);

        // 3. Safety Notice
        let safety_line = Line::from(vec![
            Span::styled("Safety: ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled("Automatic backups are created before writing (up to 10 rolling backups retained per profile-client pair in XDG storage).", Style::default().fg(Color::DarkGray)),
        ]);
        Paragraph::new(vec![safety_line]).render(main_chunks[2], buf);

        // 4. Action bar
        let action_line = Line::from(vec![
            Span::styled(
                " [Enter] / [y] ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Confirm & Sync Enabled Clients   ",
                Style::default().fg(Color::White),
            ),
            Span::styled(
                " [Esc] / [n] ",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Cancel", Style::default().fg(Color::DarkGray)),
        ]);
        Paragraph::new(vec![action_line]).render(main_chunks[3], buf);
    }
}

impl Component for SyncConfirm {
    fn handle_events(&mut self, event: Option<Event>) -> Result<Option<Action>> {
        Ok(None)
    }
}
