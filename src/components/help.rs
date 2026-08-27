use color_eyre::Result;
use ratatui::prelude::Widget;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
};

use crate::action::Action;
use crate::components::Component;
use crate::tui::Event;

#[derive(Default)]
pub struct Help;

impl Widget for &Help {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(
                " Keyboard Shortcuts & Reference [?] ",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ))
            .style(Style::default().bg(Color::Rgb(12, 16, 24)));

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 10 || inner.width < 30 {
            return;
        }

        // Layout: 2 Columns of Shortcut Groups + Bottom Dismiss Bar
        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(8),    // Content columns
                Constraint::Length(2), // Bottom Action Bar
            ])
            .split(inner);

        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(main_chunks[0]);

        // Color styles: Blue & Pink-Purple primary theme
        let key_style = Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD);
        let sec_title_style = Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD);
        let desc_style = Style::default().fg(Color::White);
        let dim_style = Style::default().fg(Color::DarkGray);

        // --- Left Column ---
        let left_lines = vec![
            // 1. Global Navigation
            Line::from(Span::styled("Navigation & Views", sec_title_style)),
            Line::from(vec![
                Span::styled(" [Tab] / [S-Tab] ", key_style),
                Span::styled("Cycle active view", desc_style),
            ]),
            Line::from(vec![
                Span::styled(" [1] [2] [3]     ", key_style),
                Span::styled("Clients / MCPs / Skills", desc_style),
            ]),
            Line::from(vec![
                Span::styled(" [b]             ", key_style),
                Span::styled("Open MCP Registry Browser", desc_style),
            ]),
            Line::from(vec![
                Span::styled(" [p]             ", key_style),
                Span::styled("Open Profile Switcher", desc_style),
            ]),
            Line::from(vec![
                Span::styled(" [?]             ", key_style),
                Span::styled("Toggle this shortcuts help", desc_style),
            ]),
            Line::from(vec![
                Span::styled(" [q]             ", key_style),
                Span::styled("Quit application", desc_style),
            ]),
            Line::raw(""),
            // 2. Clients View
            Line::from(Span::styled("Clients View (Diff & Sync)", sec_title_style)),
            Line::from(vec![
                Span::styled(" [1-4] / [h] [l] ", key_style),
                Span::styled("Select target client tab", desc_style),
            ]),
            Line::from(vec![
                Span::styled(" [e] / [Space]   ", key_style),
                Span::styled("Toggle client in active profile", desc_style),
            ]),
            Line::from(vec![
                Span::styled(" [/]             ", key_style),
                Span::styled("Search diff (Vim regex mode)", desc_style),
            ]),
            Line::from(vec![
                Span::styled(" [n] / [N]       ", key_style),
                Span::styled("Next / previous search match", desc_style),
            ]),
            Line::from(vec![
                Span::styled(" [s]             ", key_style),
                Span::styled("Sync enabled clients for profile", desc_style),
            ]),
            Line::from(vec![
                Span::styled(" [B] / [R]       ", key_style),
                Span::styled("Backup / Restore (10 max/pair)", desc_style),
            ]),
            Line::from(vec![
                Span::styled(" [j] / [k]       ", key_style),
                Span::styled("Scroll diff viewer", desc_style),
            ]),
        ];

        Paragraph::new(left_lines).render(cols[0], buf);

        // --- Right Column ---
        let right_lines = vec![
            // 3. MCPs & Containers View
            Line::from(Span::styled("MCPs & Containers View", sec_title_style)),
            Line::from(vec![
                Span::styled(" [s]             ", key_style),
                Span::styled("Start / Stop selected container", desc_style),
            ]),
            Line::from(vec![
                Span::styled(" [a]             ", key_style),
                Span::styled("Add MCP server (CLI wizard)", desc_style),
            ]),
            Line::from(vec![
                Span::styled(" [i]             ", key_style),
                Span::styled("Toggle Logs / Live Inspector", desc_style),
            ]),
            Line::from(vec![
                Span::styled(" [f]             ", key_style),
                Span::styled("Filter containers (Managed / All)", desc_style),
            ]),
            Line::from(vec![
                Span::styled(" [+]             ", key_style),
                Span::styled("Register container as MCP server", desc_style),
            ]),
            Line::from(vec![
                Span::styled(" [d] / [x]       ", key_style),
                Span::styled("Delete server / container from config", desc_style),
            ]),
            Line::from(vec![
                Span::styled(" [c]             ", key_style),
                Span::styled("Clear Inspector telemetry events", desc_style),
            ]),
            Line::from(vec![
                Span::styled(" [j] / [k]       ", key_style),
                Span::styled("Select item or scroll logs", desc_style),
            ]),
            Line::raw(""),
            // 4. Modals & Dialogs
            Line::from(Span::styled("Modals & Dialogs", sec_title_style)),
            Line::from(vec![
                Span::styled(" [Enter]         ", key_style),
                Span::styled("Submit step / Confirm action", desc_style),
            ]),
            Line::from(vec![
                Span::styled(" [Esc]           ", key_style),
                Span::styled("Dismiss modal or clear filter", desc_style),
            ]),
            Line::from(vec![
                Span::styled(" [n] / [d]       ", key_style),
                Span::styled("New / Delete profile in switcher", dim_style),
            ]),
        ];

        Paragraph::new(right_lines).render(cols[1], buf);

        // --- Bottom Action Bar ---
        let action_line = Line::from(vec![
            Span::styled(" [?] ", key_style),
            Span::styled("or ", dim_style),
            Span::styled(" [Esc] ", key_style),
            Span::styled("or ", dim_style),
            Span::styled(" [Enter] ", key_style),
            Span::styled("or ", dim_style),
            Span::styled(" [q] ", key_style),
            Span::styled(" Close Shortcuts Help", desc_style),
        ]);
        Paragraph::new(action_line).render(main_chunks[1], buf);
    }
}

impl Component for Help {
    fn handle_events(&mut self, _event: Option<Event>) -> Result<Option<Action>> {
        Ok(None)
    }
}
