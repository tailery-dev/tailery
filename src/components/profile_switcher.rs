use color_eyre::Result;
use ratatui::prelude::Widget;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Row, Table, StatefulWidget},
};
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    action::Action,
    components::Component,
    state::AppState,
    tui::Event,
};

#[derive(Default)]
pub struct ProfileSwitcher {
    pub selected_index: usize,
    pub command_tx: Option<UnboundedSender<Action>>,
}

impl Component for ProfileSwitcher {
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

impl StatefulWidget for &ProfileSwitcher {
    type State = AppState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(
                " Switch Active Profile ",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ))
            .style(Style::default().bg(Color::Rgb(15, 20, 30)));

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 6 || inner.width < 25 {
            return;
        }

        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(4),    // Table of profiles
                Constraint::Length(3), // Action bar
            ])
            .split(inner);

        let active_profile = &state.settings.active_profile;

        let mut profile_names: Vec<String> = state.profiles.keys().cloned().collect();
        profile_names.sort();

        let rows: Vec<Row> = profile_names
            .iter()
            .enumerate()
            .map(|(idx, name)| {
                let is_selected = idx == self.selected_index;
                let is_active = name == active_profile;

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

                let profile_cfg = state.profiles.get(name);
                let server_count = profile_cfg.map(|p| p.enabled_servers.len()).unwrap_or(0);
                let client_count = profile_cfg.map(|p| p.enabled_clients.len()).unwrap_or(0);
                let client_summary = if let Some(p) = profile_cfg {
                    if p.enabled_clients.len() == 4 {
                        "All (4)".to_string()
                    } else if p.enabled_clients.is_empty() {
                        "None (0)".to_string()
                    } else {
                        format!("{} client(s)", client_count)
                    }
                } else {
                    "All (4)".to_string()
                };

                let (status_badge, status_style) = if is_active {
                    (
                        "● CURRENT",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    ("○ INACTIVE", Style::default().fg(Color::DarkGray))
                };

                Row::new(vec![
                    Span::styled(format!(" {} ", name), name_style),
                    Span::styled(
                        format!("{} servers", server_count),
                        Style::default().fg(Color::Magenta),
                    ),
                    Span::styled(client_summary, Style::default().fg(Color::Cyan)),
                    Span::styled(status_badge, status_style),
                ])
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Percentage(32),
                Constraint::Percentage(22),
                Constraint::Percentage(26),
                Constraint::Percentage(20),
            ],
        )
        .header(
            Row::new(vec!["Profile Name", "Servers", "Enabled Clients", "Status"]).style(
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
                    " Available Profiles ",
                    Style::default().fg(Color::White),
                )),
        );
        ratatui::prelude::Widget::render(table, main_chunks[0], buf);

        // Action bar
        let action_lines = vec![
            Line::from(vec![
                Span::styled(
                    " [Enter] ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("Select & Switch   "),
                Span::styled(
                    " [e] ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("Edit Profile (Servers & Clients)   "),
                Span::styled(
                    " [n] ",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("New Profile"),
            ]),
            Line::from(vec![
                Span::styled(
                    " [d] ",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("Delete Profile   "),
                Span::styled(
                    " [Esc] ",
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("Cancel"),
            ]),
        ];
        Paragraph::new(action_lines).render(main_chunks[1], buf);
    }
}
