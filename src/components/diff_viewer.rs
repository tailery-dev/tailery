use color_eyre::Result;
use ratatui::prelude::Widget;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, StatefulWidget, Tabs},
};
use similar::{ChangeTag, TextDiff};

use crate::action::Action;
use crate::adapters::all_adapters;
use crate::components::Component;
use crate::state::AppState;
use crate::tui::Event;

#[derive(Default)]
pub struct DiffViewer {
    pub selected_adapter_index: usize,
    pub scroll_offset: usize,
    pub search_active: bool,
    pub search_query: String,
    pub search_match_index: usize,
}

impl StatefulWidget for &DiffViewer {
    type State = AppState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Adapter Tabs
                Constraint::Length(3), // Path & Status Header
                Constraint::Min(10),   // Diff Box
            ])
            .split(area);

        let adapters = all_adapters();
        if adapters.is_empty() {
            return;
        }
        let selected_idx = self.selected_adapter_index % adapters.len();
        let current_adapter = &adapters[selected_idx];

        // 1. Adapter Tabs with styled background pills
        let unselected_bg = Color::Rgb(40, 44, 60);
        let selected_bg = Color::Rgb(130, 180, 255);
        let selected_fg = Color::Rgb(15, 20, 35);

        let titles: Vec<Line> = adapters
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let is_selected = i == selected_idx;
                if is_selected {
                    Line::from(vec![
                        Span::styled(
                            format!(" [{}] ", i + 1),
                            Style::default()
                                .fg(selected_fg)
                                .bg(selected_bg)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("{} ", a.display_name()),
                            Style::default()
                                .fg(selected_fg)
                                .bg(selected_bg)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ])
                } else {
                    let num_color = Color::Rgb(125, 175, 235);
                    let name_color = Color::Rgb(185, 195, 215);
                    Line::from(vec![
                        Span::styled(
                            format!(" [{}] ", i + 1),
                            Style::default().fg(num_color).bg(unselected_bg),
                        ),
                        Span::styled(
                            format!("{} ", a.display_name()),
                            Style::default().fg(name_color).bg(unselected_bg),
                        ),
                    ])
                }
            })
            .collect();

        let tabs = Tabs::new(titles)
            .select(selected_idx)
            .divider(" ")
            .padding("", "")
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Cyan))
                    .title(Span::styled(
                        " Select Target AI Client ",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )),
            )
            .highlight_style(
                Style::default()
                    .fg(selected_fg)
                    .bg(selected_bg)
                    .add_modifier(Modifier::BOLD),
            );
        tabs.render(chunks[0], buf);

        // 2. Path & Status info
        let config_path = current_adapter.config_path(None).ok();
        let path_str = config_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "Unknown path".to_string());

        let exists = config_path.as_ref().map(|p| p.exists()).unwrap_or(false);
        let on_disk_content = current_adapter
            .managed_diff_content(config_path.as_deref())
            .unwrap_or_default();

        let active_profile = &state.settings.active_profile;
        let active_servers = state.get_active_profile_servers();
        let generated_json = current_adapter
            .generate_config(&active_servers)
            .and_then(|v| {
                serde_json::to_string_pretty(&v).map_err(|e| {
                    crate::adapters::AdapterError::Serialization {
                        adapter: current_adapter.name(),
                        source: e,
                    }
                })
            })
            .unwrap_or_default();

        let backup_count =
            crate::backup::list_backups(Some(active_profile), Some(current_adapter.name()))
                .map(|b| b.len())
                .unwrap_or(0);

        let header_lines = vec![Line::from(vec![
            Span::styled("Config Target: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&path_str, Style::default().fg(Color::Cyan)),
            Span::raw(" │ "),
            Span::styled("File Exists: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                if exists { "YES" } else { "NO (New File)" },
                Style::default().fg(if exists { Color::Green } else { Color::Red }),
            ),
            Span::raw(" │ "),
            Span::styled("Backups: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}/10", backup_count),
                Style::default().fg(if backup_count > 0 {
                    Color::Magenta
                } else {
                    Color::DarkGray
                }),
            ),
        ])];

        let header_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(Span::styled(
                " File Path & Backups ",
                Style::default().fg(Color::White),
            ));
        Paragraph::new(header_lines)
            .block(header_block)
            .render(chunks[1], buf);

        // 3. Diff View using `similar`
        let mut diff_lines: Vec<Line> = Vec::new();
        if on_disk_content.trim() == generated_json.trim() {
            diff_lines.push(Line::from(Span::styled(
                "No differences. Configurations are in perfect sync.",
                Style::default().fg(Color::Green),
            )));
        } else if on_disk_content.trim().is_empty() {
            for line in generated_json.lines() {
                diff_lines.push(Line::from(vec![
                    Span::styled(
                        "+ ",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        line,
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
            }
        } else if generated_json.trim().is_empty() {
            for line in on_disk_content.lines() {
                diff_lines.push(Line::from(vec![
                    Span::styled("- ", Style::default().fg(Color::Red)),
                    Span::styled(line, Style::default().fg(Color::Red)),
                ]));
            }
        } else {
            let diff = TextDiff::from_lines(&on_disk_content, &generated_json);
            for change in diff.iter_all_changes() {
                let (sign, style) = match change.tag() {
                    ChangeTag::Delete => ("-", Style::default().fg(Color::Red)),
                    ChangeTag::Insert => (
                        "+",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                    ChangeTag::Equal => (" ", Style::default().fg(Color::DarkGray)),
                };
                diff_lines.push(Line::from(vec![
                    Span::styled(format!("{} ", sign), style),
                    Span::styled(change.value().trim_end_matches('\n'), style),
                ]));
            }
        }

        let diff_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Magenta))
            .title(Span::styled(
                format!(
                    " Diff Preview: {} │ [s] Sync │ [B] Backup │ [R] Restore ",
                    current_adapter.display_name()
                ),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ));

        Paragraph::new(diff_lines)
            .block(diff_block)
            .scroll((self.scroll_offset as u16, 0))
            .render(chunks[2], buf);
    }
}

impl Component for DiffViewer {
    fn handle_events(&mut self, _event: Option<Event>) -> Result<Option<Action>> {
        Ok(None)
    }
}
