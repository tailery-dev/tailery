use color_eyre::Result;
use ratatui::prelude::Widget;

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols::merge::MergeStrategy,
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, StatefulWidget, Tabs},
};
use similar::{ChangeTag, TextDiff};
use tokio::sync::mpsc::UnboundedSender;

use crate::action::Action;
use crate::adapters::all_adapters;
use crate::components::Component;
use crate::state::AppState;
use crate::tui::Event;

pub fn compute_client_diff(
    state: &AppState,
    adapter_index: usize,
) -> (String, String, Vec<(ChangeTag, String)>) {
    let adapters = all_adapters();
    if adapters.is_empty() {
        return (String::new(), String::new(), Vec::new());
    }
    let selected_idx = adapter_index % adapters.len();
    let current_adapter = &adapters[selected_idx];

    let config_path = current_adapter.config_path(None).ok();
    let on_disk_content = config_path
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();

    let generated_json = current_adapter
        .generate_config(&state.servers)
        .and_then(|v| {
            serde_json::to_string_pretty(&v).map_err(|e| {
                crate::adapters::AdapterError::Serialization {
                    adapter: current_adapter.name(),
                    source: e,
                }
            })
        })
        .unwrap_or_default();

    let diff = TextDiff::from_lines(&on_disk_content, &generated_json);
    let mut changes = Vec::new();
    for change in diff.iter_all_changes() {
        changes.push((
            change.tag(),
            change.value().trim_end_matches('\n').to_string(),
        ));
    }
    (on_disk_content, generated_json, changes)
}

pub fn highlight_matches<'a>(
    text: &'a str,
    base_style: Style,
    query: &str,
    is_focused_line: bool,
) -> Vec<Span<'a>> {
    if query.is_empty() {
        return vec![Span::styled(text, base_style)];
    }

    let q_lower = query.to_lowercase();
    let text_lower = text.to_lowercase();
    let mut spans = Vec::new();
    let mut last_idx = 0;

    let match_style = if is_focused_line {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Magenta)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    };

    let mut search_from = 0;
    while let Some(rel_pos) = text_lower[search_from..].find(&q_lower) {
        let start = search_from + rel_pos;
        let end = start + q_lower.len();

        if text.is_char_boundary(start) && text.is_char_boundary(end) {
            if start > last_idx {
                spans.push(Span::styled(&text[last_idx..start], base_style));
            }
            spans.push(Span::styled(&text[start..end], match_style));
            last_idx = end;
            search_from = end;
        } else {
            search_from += 1;
        }
    }

    if last_idx < text.len() {
        spans.push(Span::styled(&text[last_idx..], base_style));
    }

    if spans.is_empty() {
        spans.push(Span::styled(text, base_style));
    }

    spans
}

#[derive(Default)]
pub struct Clients {
    pub selected_adapter_index: usize,
    pub diff_scroll_offset: usize,
    pub search_active: bool,
    pub search_query: String,
    pub search_match_index: usize,
    pub total_matches: usize,
    pub current_match_line: Option<usize>,
    pub command_tx: Option<UnboundedSender<Action>>,
}

impl Component for Clients {
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

impl StatefulWidget for &Clients {
    type State = AppState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let constraints = if self.search_active {
            vec![
                Constraint::Length(3), // Client Adapter Tabs
                Constraint::Length(3), // Target File Path & Status Header
                Constraint::Min(6),    // Diff Viewer Box
                Constraint::Length(3), // Vim Search Prompt
            ]
        } else {
            vec![
                Constraint::Length(3), // Client Adapter Tabs
                Constraint::Length(3), // Target File Path & Status Header
                Constraint::Min(8),    // Diff Viewer Box
            ]
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);

        let adapters = all_adapters();
        if adapters.is_empty() {
            return;
        }
        let selected_idx = self.selected_adapter_index % adapters.len();
        let current_adapter = &adapters[selected_idx];

        let active_profile = &state.settings.active_profile;

        // 1. Client Adapter Tabs with styled background pills
        let unselected_bg = Color::Rgb(40, 44, 60);
        let selected_bg = Color::Rgb(130, 180, 255);
        let selected_fg = Color::Rgb(15, 20, 35);

        let titles: Vec<Line> = adapters
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let is_selected = i == selected_idx;
                let is_detected = a.detect_installed();
                let is_enabled_in_profile = state.is_client_enabled_in_active_profile(a.name());

                if is_selected {
                    let badge = if is_enabled_in_profile { "●" } else { "○" };
                    let badge_color = if is_enabled_in_profile {
                        Color::Rgb(0, 75, 30)
                    } else {
                        Color::Rgb(85, 95, 115)
                    };

                    Line::from(vec![
                        Span::styled(
                            format!(" [{}] ", i + 1),
                            Style::default()
                                .fg(selected_fg)
                                .bg(selected_bg)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("{} ", badge),
                            Style::default()
                                .fg(badge_color)
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
                    let (badge, badge_color) = if is_enabled_in_profile {
                        if is_detected {
                            ("●", Color::Green)
                        } else {
                            ("●", Color::Yellow)
                        }
                    } else {
                        ("○", Color::DarkGray)
                    };

                    let num_color = Color::Rgb(125, 175, 235);
                    let name_color = if is_enabled_in_profile {
                        Color::Rgb(185, 195, 215)
                    } else {
                        Color::Rgb(115, 125, 145)
                    };

                    Line::from(vec![
                        Span::styled(
                            format!(" [{}] ", i + 1),
                            Style::default().fg(num_color).bg(unselected_bg),
                        ),
                        Span::styled(
                            format!("{} ", badge),
                            Style::default().fg(badge_color).bg(unselected_bg),
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
                    .borders(Borders::LEFT | Borders::RIGHT | Borders::TOP)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Cyan))
                    .merge_borders(MergeStrategy::Exact),
            )
            .highlight_style(
                Style::default()
                    .fg(selected_fg)
                    .bg(selected_bg)
                    .add_modifier(Modifier::BOLD),
            );
        tabs.render(chunks[0], buf);

        // 2. Path & Status info Header
        let config_path = current_adapter.config_path(None).ok();
        let exists = config_path.as_ref().map(|p| p.exists()).unwrap_or(false);
        let is_installed = current_adapter.detect_installed();
        let install_badge = if is_installed {
            Span::styled(
                "● INSTALLED",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled("○ NOT DETECTED", Style::default().fg(Color::DarkGray))
        };

        let is_client_enabled = state.is_client_enabled_in_active_profile(current_adapter.name());
        let profile_badge = if is_client_enabled {
            Span::styled(
                "● ENABLED",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(
                "○ DISABLED",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        };

        let backup_count =
            crate::backup::list_backups(Some(active_profile), Some(current_adapter.name()))
                .map(|b| b.len())
                .unwrap_or(0);

        let header_lines = vec![Line::from(vec![
            Span::styled("Profile: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("[{}]", active_profile),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" │ "),
            Span::styled("Client: ", Style::default().fg(Color::DarkGray)),
            profile_badge,
            Span::styled(" [e: toggle]", Style::default().fg(Color::DarkGray)),
            Span::raw(" │ "),
            Span::styled("Host: ", Style::default().fg(Color::DarkGray)),
            install_badge,
            Span::raw(" │ "),
            Span::styled("File Exists: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                if exists { "YES" } else { "NO (Will Create)" },
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
                " Client Profile Status & Rolling Backups (XDG) ",
                Style::default().fg(Color::White),
            ));
        Paragraph::new(header_lines)
            .block(header_block)
            .render(chunks[1], buf);

        // 3. Diff View using `similar` & search highlighting
        let (_, _, changes) = compute_client_diff(state, self.selected_adapter_index);
        let mut diff_lines: Vec<Line> = Vec::new();
        let mut insertions = 0;
        let mut deletions = 0;

        for (idx, (tag, val)) in changes.iter().enumerate() {
            let (sign, style) = match tag {
                ChangeTag::Delete => {
                    deletions += 1;
                    ("-", Style::default().fg(Color::Red))
                }
                ChangeTag::Insert => {
                    insertions += 1;
                    (
                        "+",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    )
                }
                ChangeTag::Equal => (" ", Style::default().fg(Color::DarkGray)),
            };

            let is_focused = self.current_match_line == Some(idx);
            let mut line_spans = vec![Span::styled(format!("{} ", sign), style)];
            line_spans.extend(highlight_matches(
                val,
                style,
                &self.search_query,
                is_focused,
            ));
            diff_lines.push(Line::from(line_spans));
        }

        if !is_client_enabled {
            diff_lines = vec![
                Line::raw(""),
                Line::from(vec![
                    Span::styled(
                        " ○ ",
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(
                            "{} is currently DISABLED for profile '{}'.",
                            current_adapter.display_name(),
                            active_profile
                        ),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::raw(""),
                Line::from(Span::styled(
                    "When disabled, this client is skipped during batch synchronization.",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::raw(""),
                Line::from(vec![
                    Span::styled("Press ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        " [e] ",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(
                            " to ENABLE {} for profile '{}'.",
                            current_adapter.display_name(),
                            active_profile
                        ),
                        Style::default().fg(Color::White),
                    ),
                ]),
            ];
        } else if diff_lines.is_empty() || (insertions == 0 && deletions == 0) {
            diff_lines = vec![
                Line::raw(""),
                Line::from(vec![
                    Span::styled(
                        " ● ",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(
                            "{} configuration is in sync with Tailery state for profile '{}'.",
                            current_adapter.display_name(),
                            active_profile
                        ),
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::raw(""),
                Line::from(Span::styled(
                    "No differences between on-disk configuration and generated MCP servers.",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::raw(""),
                Line::from(vec![
                    Span::styled("Press ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        " [s] ",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        " to force re-synchronize client configuration files.",
                        Style::default().fg(Color::White),
                    ),
                ]),
            ];
        }

        // Apply scroll offset
        let visible_lines: Vec<Line> = diff_lines
            .into_iter()
            .skip(self.diff_scroll_offset)
            .collect();

        let base_title = if !is_client_enabled {
            format!(
                " Diff Preview: {} (DISABLED in [{}]) │ [e] Enable │ [s] Sync ",
                current_adapter.display_name(),
                active_profile
            )
        } else if insertions > 0 || deletions > 0 {
            format!(
                " Diff Preview: {} (+{} -{}) │ [e] Disable │ [s] Sync │ [B] Backup ",
                current_adapter.display_name(),
                insertions,
                deletions
            )
        } else {
            format!(
                " Diff Preview: {} (In Sync) │ [e] Disable │ [s] Sync │ [B] Backup ",
                current_adapter.display_name()
            )
        };

        let search_title_suffix = if !self.search_query.is_empty() {
            if self.total_matches > 0 {
                format!(
                    "│ \"{}\" [{}/{}] ([n]/[N] match, [Esc] clear) ",
                    self.search_query,
                    self.search_match_index + 1,
                    self.total_matches
                )
            } else {
                format!("│ \"{}\" [Not found] ([Esc] clear) ", self.search_query)
            }
        } else {
            "│ [/] Search (vim) ".to_string()
        };

        let diff_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Magenta))
            .title(Span::styled(
                format!("{}{}", base_title, search_title_suffix),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ));
        Paragraph::new(visible_lines)
            .block(diff_block)
            .render(chunks[2], buf);

        // 4. Search Prompt Bar (if actively searching)
        if self.search_active {
            let search_block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan))
                .title(Span::styled(
                    " Vim Search [Enter: Confirm, Esc: Cancel] ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));

            let match_count_info = if !self.search_query.is_empty() {
                if self.total_matches > 0 {
                    format!(
                        " [{} match{}] ",
                        self.total_matches,
                        if self.total_matches == 1 { "" } else { "es" }
                    )
                } else {
                    " [Pattern not found] ".to_string()
                }
            } else {
                String::new()
            };

            let search_line = Line::from(vec![
                Span::styled(
                    " / ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    &self.search_query,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("█", Style::default().fg(Color::Magenta)),
                Span::styled(
                    match_count_info,
                    Style::default().fg(if self.total_matches > 0 {
                        Color::Green
                    } else {
                        Color::Red
                    }),
                ),
            ]);
            Paragraph::new(search_line)
                .block(search_block)
                .render(chunks[3], buf);
        }
    }
}
