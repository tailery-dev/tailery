use color_eyre::Result;
use ratatui::prelude::Widget;
use std::collections::HashMap;

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols::merge::MergeStrategy,
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Row, StatefulWidget, Table, Tabs},
};
use similar::{ChangeTag, TextDiff};
use tokio::sync::mpsc::UnboundedSender;

use crate::action::Action;
use crate::adapters::all_adapters;
use crate::components::Component;
use crate::state::AppState;
use crate::tui::Event;

#[derive(Clone, Debug, Default)]
pub struct CachedClientDiff {
    pub on_disk_content: String,
    pub generated_json: String,
    pub changes: Vec<(ChangeTag, String)>,
    pub file_exists: bool,
    pub is_installed: bool,
    pub backup_count: usize,
    pub is_loading: bool,
    pub discovered_mcps: Vec<crate::adapters::DiscoveredMcp>,
}

pub fn compute_client_diff_full(
    state: &AppState,
    adapter_index: usize,
) -> CachedClientDiff {
    let adapters = all_adapters();
    if adapters.is_empty() {
        return CachedClientDiff::default();
    }
    let selected_idx = adapter_index % adapters.len();
    let current_adapter = &adapters[selected_idx];

    let config_path = current_adapter.config_path(None).ok();
    let file_exists = config_path.as_ref().map(|p| p.exists()).unwrap_or(false);
    let on_disk_content = current_adapter
        .managed_diff_content(config_path.as_deref())
        .unwrap_or_default();

    let active_profile = &state.settings.active_profile;
    let is_installed = current_adapter.detect_installed();
    let backup_count =
        crate::backup::list_backups(Some(active_profile), Some(current_adapter.name()))
            .map(|b| b.len())
            .unwrap_or(0);

    let active_servers = state.get_active_profile_servers();

    let existing_json: Option<serde_json::Value> = if file_exists {
        config_path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|raw| crate::adapters::parse_json_relaxed(&raw))
    } else {
        None
    };

    let path_ref = config_path.as_deref().unwrap_or(std::path::Path::new(""));
    let merged_val = current_adapter
        .merge_managed_config(path_ref, existing_json.as_ref(), &active_servers)
        .unwrap_or_else(|_| serde_json::json!({}));
    let target_managed = current_adapter.extract_managed_config(path_ref, &merged_val);
    let generated_json = serde_json::to_string_pretty(&target_managed).unwrap_or_default();

    let discovered_mcps = current_adapter
        .discover_mcps(config_path.as_deref(), state)
        .unwrap_or_default();

    let mut changes = Vec::new();
    if on_disk_content.trim() == generated_json.trim() {
        for line in generated_json.lines() {
            changes.push((ChangeTag::Equal, line.to_string()));
        }
    } else if on_disk_content.trim().is_empty() {
        for line in generated_json.lines() {
            changes.push((ChangeTag::Insert, line.to_string()));
        }
    } else if generated_json.trim().is_empty() {
        for line in on_disk_content.lines() {
            changes.push((ChangeTag::Delete, line.to_string()));
        }
    } else {
        let diff = TextDiff::from_lines(&on_disk_content, &generated_json);
        for change in diff.iter_all_changes() {
            changes.push((
                change.tag(),
                change.value().trim_end_matches('\n').to_string(),
            ));
        }
    }

    CachedClientDiff {
        on_disk_content,
        generated_json,
        changes,
        file_exists,
        is_installed,
        backup_count,
        is_loading: false,
        discovered_mcps,
    }
}

#[allow(dead_code)]
pub fn compute_client_diff(
    state: &AppState,
    adapter_index: usize,
) -> (String, String, Vec<(ChangeTag, String)>) {
    let full = compute_client_diff_full(state, adapter_index);
    (full.on_disk_content, full.generated_json, full.changes)
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

pub struct Clients {
    pub selected_adapter_index: usize,
    pub focused_pane: usize, // 0: Discovered MCPs, 1: Diff Preview
    pub mcp_selected_index: usize,
    pub diff_scroll_offset: usize,
    pub search_active: bool,
    pub search_query: String,
    pub search_match_index: usize,
    pub total_matches: usize,
    pub current_match_line: Option<usize>,
    pub command_tx: Option<UnboundedSender<Action>>,
    pub diff_cache: HashMap<usize, CachedClientDiff>,
    pub installed_cache: HashMap<usize, bool>,
}

impl Default for Clients {
    fn default() -> Self {
        Self {
            selected_adapter_index: 0,
            focused_pane: 0,
            mcp_selected_index: 0,
            diff_scroll_offset: 0,
            search_active: false,
            search_query: String::new(),
            search_match_index: 0,
            total_matches: 0,
            current_match_line: None,
            command_tx: None,
            diff_cache: HashMap::new(),
            installed_cache: HashMap::new(),
        }
    }
}

impl Clients {
    pub fn move_up(&mut self) {
        if self.focused_pane == 0 {
            if self.mcp_selected_index > 0 {
                self.mcp_selected_index -= 1;
            }
        } else if self.diff_scroll_offset > 0 {
            self.diff_scroll_offset -= 1;
        }
    }

    pub fn move_down(&mut self, total_discovered: usize, total_diff_lines: usize) {
        if self.focused_pane == 0 {
            if total_discovered > 0 && self.mcp_selected_index + 1 < total_discovered {
                self.mcp_selected_index += 1;
            }
        } else if self.diff_scroll_offset + 1 < total_diff_lines {
            self.diff_scroll_offset += 1;
        }
    }

    pub fn switch_pane(&mut self) {
        self.focused_pane = if self.focused_pane == 0 { 1 } else { 0 };
    }
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
                let is_detected = self.installed_cache.get(&i).copied().unwrap_or(true);
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

        // 2. Path & Status info Header from Cache
        let cached_opt = self.diff_cache.get(&selected_idx);
        let exists = cached_opt.map(|c| c.file_exists).unwrap_or(false);
        let is_installed = cached_opt.map(|c| c.is_installed).unwrap_or(true);
        let backup_count = cached_opt.map(|c| c.backup_count).unwrap_or(0);
        let is_loading = cached_opt.map(|c| c.is_loading).unwrap_or(false);

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

        // 3. Diff View using cached changes
        let mut diff_lines: Vec<Line> = Vec::new();
        let mut insertions = 0;
        let mut deletions = 0;

        if is_loading || cached_opt.is_none() {
            diff_lines = vec![
                Line::raw(""),
                Line::from(vec![
                    Span::styled(
                        " ⏳ ",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(
                            "Computing configuration diff for {}...",
                            current_adapter.display_name()
                        ),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
            ];
        } else if let Some(cached) = cached_opt {
            for (idx, (tag, val)) in cached.changes.iter().enumerate() {
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

            if diff_lines.is_empty() || (insertions == 0 && deletions == 0) {
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
                ];

                if !is_client_enabled {
                    diff_lines.push(Line::raw(""));
                    diff_lines.push(Line::from(vec![
                        Span::styled(
                            "Notice: ",
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!(
                                "{} is DISABLED in profile '{}' (sync is disabled).",
                                current_adapter.display_name(),
                                active_profile
                            ),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ]));
                    diff_lines.push(Line::from(vec![
                        Span::styled("Press ", Style::default().fg(Color::DarkGray)),
                        Span::styled(
                            " [e] ",
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!(
                                "to ENABLE {} for profile '{}'.",
                                current_adapter.display_name(),
                                active_profile
                            ),
                            Style::default().fg(Color::White),
                        ),
                    ]));
                } else {
                    diff_lines.push(Line::raw(""));
                    diff_lines.push(Line::from(vec![
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
                    ]));
                }
            }
        }

        // Apply scroll offset
        let visible_lines: Vec<Line> = diff_lines
            .into_iter()
            .skip(self.diff_scroll_offset)
            .collect();

        let base_title = if is_loading {
            format!(
                " Diff Preview: {} (⏳ Loading...) │ [e] Toggle │ [r] Refresh ",
                current_adapter.display_name()
            )
        } else if !is_client_enabled {
            if insertions > 0 || deletions > 0 {
                format!(
                    " Diff Preview: {} (+{} -{} - DISABLED) │ [e] Enable │ Sync (disabled) │ [B] Backup │ [r] Refresh ",
                    current_adapter.display_name(),
                    insertions,
                    deletions
                )
            } else {
                format!(
                    " Diff Preview: {} (In Sync - DISABLED) │ [e] Enable │ Sync (disabled) │ [B] Backup │ [r] Refresh ",
                    current_adapter.display_name()
                )
            }
        } else if insertions > 0 || deletions > 0 {
            format!(
                " Diff Preview: {} (+{} -{}) │ [e] Disable │ [s] Sync │ [B] Backup │ [r] Refresh ",
                current_adapter.display_name(),
                insertions,
                deletions
            )
        } else {
            format!(
                " Diff Preview: {} (In Sync) │ [e] Disable │ [s] Sync │ [B] Backup │ [r] Refresh ",
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

        let main_panes = if chunks[2].width >= 80 {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(45), // Discovered MCPs
                    Constraint::Percentage(55), // Diff Preview
                ])
                .split(chunks[2])
        } else {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Percentage(40),
                    Constraint::Percentage(60),
                ])
                .split(chunks[2])
        };

        // Render Discovered MCPs pane (Left)
        let is_mcp_pane_focused = self.focused_pane == 0;
        let is_diff_pane_focused = self.focused_pane == 1;

        let discovered_list = cached_opt.map(|c| &c.discovered_mcps[..]).unwrap_or(&[]);
        let total_discovered = discovered_list.len();

        let mcp_rows: Vec<Row> = discovered_list
            .iter()
            .enumerate()
            .map(|(idx, mcp)| {
                let is_selected = is_mcp_pane_focused && idx == self.mcp_selected_index;
                let (status_badge, status_style) = match mcp.status {
                    crate::adapters::DiscoveredMcpStatus::ManagedEnabled => (
                        "● ACTIVE",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                    crate::adapters::DiscoveredMcpStatus::ManagedDisabled => (
                        "○ DISABLED",
                        Style::default().fg(Color::DarkGray),
                    ),
                    crate::adapters::DiscoveredMcpStatus::ManagedDiff => (
                        "▲ DIFF",
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                    crate::adapters::DiscoveredMcpStatus::Unmanaged => (
                        "+ UNMANAGED",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                };

                let name_style = if is_selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
                };

                let scope_style = match &mcp.scope {
                    crate::adapters::McpSourceScope::User => Style::default().fg(Color::DarkGray),
                    crate::adapters::McpSourceScope::Project(_) => Style::default().fg(Color::Magenta),
                };

                Row::new(vec![
                    Span::styled(format!(" {}", status_badge), status_style),
                    Span::styled(format!(" {}", mcp.name), name_style),
                    Span::styled(format!(" {}", mcp.scope), scope_style),
                    Span::styled(format!(" {}", mcp.transport_label), Style::default().fg(Color::DarkGray)),
                ])
            })
            .collect();

        let mcp_border_color = if is_mcp_pane_focused {
            Color::Cyan
        } else {
            Color::DarkGray
        };

        let mcp_table_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(mcp_border_color))
            .title(Span::styled(
                format!(" Discovered MCPs ({}) │ [Tab] Focus │ [i] Import │ [e] Toggle ", total_discovered),
                if is_mcp_pane_focused {
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                },
            ));

        if total_discovered == 0 {
            let empty_text = vec![
                Line::raw(""),
                Line::from(Span::styled(
                    " No MCP servers discovered in configuration.",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::raw(""),
                Line::from(Span::styled(
                    " Enable servers in Tailery to sync them to this client.",
                    Style::default().fg(Color::DarkGray),
                )),
            ];
            Paragraph::new(empty_text)
                .block(mcp_table_block)
                .render(main_panes[0], buf);
        } else {
            let mcp_table = Table::new(
                mcp_rows,
                [
                    Constraint::Length(13),
                    Constraint::Percentage(42),
                    Constraint::Percentage(25),
                    Constraint::Percentage(18),
                ],
            )
            .header(
                Row::new(vec!["Status", "MCP Name", "Scope", "Transport"]).style(
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                ),
            )
            .block(mcp_table_block);
            Widget::render(mcp_table, main_panes[0], buf);
        }

        // Render Diff Preview pane (Right)
        let diff_border_color = if is_diff_pane_focused {
            Color::Cyan
        } else if !is_client_enabled {
            Color::Yellow
        } else {
            Color::DarkGray
        };

        let diff_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(diff_border_color))
            .title(Span::styled(
                format!("{}{}", base_title, search_title_suffix),
                Style::default()
                    .fg(diff_border_color)
                    .add_modifier(Modifier::BOLD),
            ));
        Paragraph::new(visible_lines)
            .block(diff_block)
            .render(main_panes[1], buf);

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

#[cfg(test)]
mod tests {
    use crate::adapters::ZedAdapter;
    use crate::adapters::ClientAdapter;
    use crate::state::*;
    use std::collections::HashMap;
    use serde_json::json;

    #[test]
    fn test_diff_isolates_managed_keys_only() {
        let adapter = ZedAdapter::default();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("tailery_test_diff_iso_{}", nanos));
        let _ = std::fs::create_dir_all(&temp_dir);
        let path = temp_dir.join("settings.json");

        // Seed settings.json with many unmanaged user settings + empty context_servers
        let seed = json!({
            "theme": "Solarized Dark",
            "vim_mode": true,
            "font_size": 14,
            "telemetry": false,
            "context_servers": {}
        });
        std::fs::write(&path, serde_json::to_string_pretty(&seed).unwrap()).unwrap();

        // Check managed_diff_content produces only context_servers (or empty if empty)
        let managed_diff_str = adapter.managed_diff_content(Some(&path)).unwrap();
        // Since context_servers is empty, managed is empty or `{}`
        assert!(!managed_diff_str.contains("Solarized Dark"));
        assert!(!managed_diff_str.contains("vim_mode"));
        assert!(!managed_diff_str.contains("font_size"));

        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn test_diff_reflects_enabled_and_disabled_servers() {
        use crate::state::{AppState, GlobalSettings, ProfileConfig, ServerConfig, ToolFilter};
        use std::collections::HashMap;

        let mut servers = HashMap::new();
        servers.insert(
            "active-server".to_string(),
            ServerConfig::Local {
                command: Some("node".to_string()),
                args: vec!["index.js".to_string()],
                env: HashMap::new(),
                tool_filter: ToolFilter::default(),
                container: crate::state::ContainerConfig::default(),
                transport: crate::state::LocalTransport::Stdio,
            },
        );
        servers.insert(
            "disabled-server".to_string(),
            ServerConfig::Local {
                command: Some("python3".to_string()),
                args: vec!["server.py".to_string()],
                env: HashMap::new(),
                tool_filter: ToolFilter::default(),
                container: crate::state::ContainerConfig::default(),
                transport: crate::state::LocalTransport::Stdio,
            },
        );

        let mut profiles = HashMap::new();
        profiles.insert(
            "default".to_string(),
            ProfileConfig {
                enabled_servers: vec!["active-server".to_string()],
                enabled_clients: vec!["cursor".to_string()], include_project_mcps: false, project_search_paths: Vec::new(), project_search_paths: Vec::new(), project_search_paths: Vec::new() },
        );

        let mut state = AppState {
            version: "1.0.0".to_string(),
            settings: GlobalSettings {
                active_profile: "default".to_string(),
                docker_socket: None,
                sync_clients: vec![],
                filter_managed_containers_only: None,
            },
            servers,
            configured_containers: HashMap::new(),
            profiles,
            workspaces: HashMap::new(),
            docker_status: String::new(),
            containers: Vec::new(),
            container_logs: Vec::new(),
            inspector_events: Vec::new(),
            managed_servers: HashMap::new(),
        };

        // 1. Cursor adapter index = 0
        let diff_initial = super::compute_client_diff_full(&state, 0);
        assert!(diff_initial.generated_json.contains("active-server"));
        assert!(!diff_initial.generated_json.contains("disabled-server"));

        // 2. Enable "disabled-server" in active profile
        state.profiles.get_mut("default").unwrap().enable_server("disabled-server");
        let diff_enabled = super::compute_client_diff_full(&state, 0);
        assert!(diff_enabled.generated_json.contains("active-server"));
        assert!(diff_enabled.generated_json.contains("disabled-server"));

        // 3. Disable "active-server" in active profile
        state.profiles.get_mut("default").unwrap().disable_server("active-server");
        let diff_disabled = super::compute_client_diff_full(&state, 0);
        assert!(!diff_disabled.generated_json.contains("active-server"));
        assert!(diff_disabled.generated_json.contains("disabled-server"));
    }

    #[test]
    fn test_diff_preserves_unmanaged_servers_without_false_deletions() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("tailery_test_diff_unmanaged_{}", nanos));
        let _ = std::fs::create_dir_all(&temp_dir);
        let path = temp_dir.join(".claude.json");

        // Seed file with unmanaged SuperhumanDocs
        let seed = serde_json::json!({
            "mcpServers": {
                "SuperhumanDocs": {
                    "type": "http",
                    "url": "https://docs.superhuman.com/apis/mcp"
                }
            }
        });
        std::fs::write(&path, serde_json::to_string_pretty(&seed).unwrap()).unwrap();

        let mut state = AppState::default();
        state.profiles.insert(
            "default".to_string(),
            ProfileConfig::new(vec!["active-mcp".to_string()], vec!["claude_code".to_string()]),
        );
        state.servers.insert(
            "active-mcp".to_string(),
            ServerConfig::Remote {
                url: "http://localhost:8080".to_string(),
                headers: HashMap::new(),
                env: HashMap::new(),
                transport: crate::state::RemoteTransport::StreamableHttp,
                shim_port: None,
                tool_filter: ToolFilter::default(),
            },
        );

        let claude_adapter = crate::adapters::claude_code::ClaudeCodeAdapter::default();
        let existing_json = crate::adapters::parse_json_relaxed(&std::fs::read_to_string(&path).unwrap());
        let active_servers = state.get_active_profile_servers();

        let merged = claude_adapter.merge_managed_config(&path, Some(&existing_json), &active_servers).unwrap();
        let target_managed = claude_adapter.extract_managed_config(&path, &merged);

        // Verify target config contains both SuperhumanDocs (preserved) and active-mcp (added)
        assert!(target_managed["mcpServers"].get("SuperhumanDocs").is_some());
        assert!(target_managed["mcpServers"].get("active-mcp").is_some());

        let on_disk_managed = claude_adapter.extract_managed_config(&path, &existing_json);
        let old_str = serde_json::to_string_pretty(&on_disk_managed).unwrap();
        let new_str = serde_json::to_string_pretty(&target_managed).unwrap();

        let diff = similar::TextDiff::from_lines(&old_str, &new_str);
        // There should be NO deletion lines (`-`) for SuperhumanDocs
        for change in diff.iter_all_changes() {
            if change.tag() == similar::ChangeTag::Delete {
                assert!(!change.value().contains("SuperhumanDocs"), "Unmanaged SuperhumanDocs was falsely marked for deletion!");
            }
        }

        let _ = std::fs::remove_dir_all(temp_dir);
    }
}
