use color_eyre::Result;
use ratatui::prelude::Widget;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, StatefulWidget},
};
use tokio::sync::mpsc::UnboundedSender;

use crate::action::Action;
use crate::components::Component;
use crate::state::AppState;
use crate::tui::Event;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActiveView {
    #[default]
    Clients,
    Mcps,
    Skills,
}

#[derive(Default)]
pub struct Sidebar {
    pub active_view: ActiveView,
    pub command_tx: Option<UnboundedSender<Action>>,
}

impl Component for Sidebar {
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

impl StatefulWidget for &Sidebar {
    type State = AppState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(
                " TAILERY ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ))
            .style(Style::default().bg(Color::Rgb(12, 16, 24)));

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 10 || inner.width < 15 {
            return;
        }

        let mut lines = Vec::new();

        lines.push(Line::from(vec![Span::styled(
            " MCP Control Plane",
            Style::default().fg(Color::DarkGray),
        )]));
        lines.push(Line::from(vec![Span::styled(
            format!(" v{}", state.version),
            Style::default().fg(Color::DarkGray),
        )]));
        lines.push(Line::raw(""));

        lines.push(Line::from(Span::styled(
            "─── VIEWS ───",
            Style::default().fg(Color::Magenta),
        )));

        let client_count = state
            .profiles
            .get(&state.settings.active_profile)
            .map(|p| p.enabled_clients.len())
            .unwrap_or(0);

        let mcp_count = state.get_active_profile_servers().len();

        let is_clients = self.active_view == ActiveView::Clients;
        let clients_style = if is_clients {
            Style::default()
                .fg(Color::White)
                .bg(Color::Blue)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        lines.push(Line::from(vec![
            Span::styled(
                " [1] ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("Clients ({})", client_count), clients_style),
        ]));

        let is_mcps = self.active_view == ActiveView::Mcps;
        let mcps_style = if is_mcps {
            Style::default()
                .fg(Color::White)
                .bg(Color::Blue)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        lines.push(Line::from(vec![
            Span::styled(
                " [2] ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("MCPs ({})", mcp_count), mcps_style),
        ]));

        let is_skills = self.active_view == ActiveView::Skills;
        let skills_style = if is_skills {
            Style::default()
                .fg(Color::White)
                .bg(Color::Blue)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        lines.push(Line::from(vec![
            Span::styled(
                " [3] ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Skills", skills_style),
        ]));

        lines.push(Line::raw(""));

        lines.push(Line::from(Span::styled(
            "─── POPUPS ───",
            Style::default().fg(Color::Magenta),
        )));
        lines.push(Line::from(vec![
            Span::styled(
                " [b] ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("MCP Browser", Style::default().fg(Color::White)),
        ]));
        lines.push(Line::from(vec![
            Span::styled(
                " [p] ",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Profiles", Style::default().fg(Color::White)),
        ]));
        lines.push(Line::from(vec![
            Span::styled(
                " [?] ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Shortcuts", Style::default().fg(Color::White)),
        ]));

        lines.push(Line::raw(""));

        lines.push(Line::from(Span::styled(
            "─── ACTIONS ───",
            Style::default().fg(Color::Magenta),
        )));
        lines.push(Line::from(vec![
            Span::styled(
                " [s] ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Sync Clients", Style::default().fg(Color::White)),
        ]));
        lines.push(Line::from(vec![
            Span::styled(
                " [q] ",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Quit", Style::default().fg(Color::DarkGray)),
        ]));

        let current_len = lines.len();
        let footer_lines_count = 5;
        if inner.height as usize > current_len + footer_lines_count {
            for _ in 0..(inner.height as usize - current_len - footer_lines_count) {
                lines.push(Line::raw(""));
            }
        }

        lines.push(Line::from(Span::styled(
            "─────────────",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(vec![
            Span::styled("Profile:", Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled(
                format!("[{}]", state.settings.active_profile),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        let docker_badge = if state.docker_status.contains("Online") {
            Span::styled(
                "● Online",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled("○ Offline", Style::default().fg(Color::Red))
        };
        lines.push(Line::from(vec![
            Span::styled("Docker:  ", Style::default().fg(Color::DarkGray)),
            docker_badge,
        ]));

        Paragraph::new(lines).render(inner, buf);
    }
}
