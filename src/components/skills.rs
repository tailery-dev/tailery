use ratatui::prelude::Widget;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Row, Table},
};

use crate::action::Action;
use crate::components::Component;
use crate::tui::Event;


#[derive(Default)]
pub struct SkillPreview {
    pub name: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    pub tools_required: &'static [&'static str],
    pub triggers: &'static [&'static str],
    pub status: &'static str,
}

pub const PREVIEW_SKILLS: &[SkillPreview] = &[
    SkillPreview {
        name: "docker-orchestration",
        category: "DevOps & Infrastructure",
        description: "Zero-trust container sandboxing, automatic image building, and resource quota monitoring.",
        tools_required: &["docker_run", "docker_logs", "docker_stop"],
        triggers: &["docker", "container", "sandbox", "isolation"],
        status: "In Development",
    },
    SkillPreview {
        name: "git-flow-manager",
        category: "Version Control",
        description: "Automated atomic commits, branch management, semantic release notes, and pull request drafting.",
        tools_required: &["git_status", "git_diff", "git_commit"],
        triggers: &["git", "commit", "branch", "pr", "release"],
        status: "Planned",
    },
    SkillPreview {
        name: "database-migrator",
        category: "Data & Schema",
        description: "Safe database schema introspection, migration validation, and mock dataset synthesis.",
        tools_required: &["sql_query", "schema_inspect", "migrate_run"],
        triggers: &["sql", "postgres", "sqlite", "migration", "schema"],
        status: "Planned",
    },
    SkillPreview {
        name: "security-audit-sentinel",
        category: "Security & Compliance",
        description: "Automated secret scanning, vulnerability assessment, and per-tool RBAC policy enforcement.",
        tools_required: &["shim_filter", "audit_log", "keychain_verify"],
        triggers: &["audit", "security", "secrets", "cve", "policy"],
        status: "Planned",
    },
    SkillPreview {
        name: "api-contract-tester",
        category: "Testing & QA",
        description: "Generates OpenAPI-driven contract tests, mock servers, and payload validation harnesses.",
        tools_required: &["http_request", "json_schema_validate"],
        triggers: &["api", "openapi", "endpoint", "test", "contract"],
        status: "Planned",
    },
];

#[derive(Default)]
pub struct Skills {
    pub selected_index: usize,
}

impl Component for Skills {
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
            _ => {}
        }
        Ok(None)
    }

    fn update(&mut self, _action: Action) -> color_eyre::Result<Option<Action>> {
        Ok(None)
    }
}

impl Widget for &Skills {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(6),
                Constraint::Min(10),
            ])
            .split(area);

        let banner_lines = vec![
            Line::from(vec![
                Span::styled(" AGENT SKILLS CONTROL PLANE ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::styled("── Next-Generation Autonomous Agent Harnessing", Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(vec![
                Span::styled("Skills extend AI coding assistants with specialized instructions, scripts, prompt-injection harnesses, and tool workflows.", Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("Tailery provides decentralized skill discovery, zero-friction sync to Claude / Cursor / Zed, and sandboxed execution.", Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(vec![
                Span::styled("Status: ", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
                Span::styled("Phase 2 Core Capability (In Development)", Style::default().fg(Color::Green)),
            ]),
        ];

        let banner_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(
                " Roadmap & Overview ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));
        Paragraph::new(banner_lines)
            .block(banner_block)
            .render(chunks[0], buf);

        let main_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(chunks[1]);

        let selected_idx = self.selected_index % PREVIEW_SKILLS.len();
        let selected_skill = &PREVIEW_SKILLS[selected_idx];

        let rows: Vec<Row> = PREVIEW_SKILLS
            .iter()
            .enumerate()
            .map(|(idx, skill)| {
                let is_selected = idx == selected_idx;
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

                let status_style = match skill.status {
                    "In Development" => Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                    _ => Style::default().fg(Color::DarkGray),
                };

                Row::new(vec![
                    Span::styled(format!(" {} ", skill.name), name_style),
                    Span::styled(skill.category, Style::default().fg(Color::Magenta)),
                    Span::styled(skill.status, status_style),
                ])
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Percentage(42),
                Constraint::Percentage(38),
                Constraint::Percentage(20),
            ],
        )
        .header(
            Row::new(vec!["Skill Name", "Category", "Status"]).style(
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
                    " Skills Specification Directory [j/k] ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
        );
        table.render(main_cols[0], buf);

        let detail_lines = vec![
            Line::from(vec![
                Span::styled("Skill:        ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    selected_skill.name,
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled("Category:     ", Style::default().fg(Color::DarkGray)),
                Span::styled(selected_skill.category, Style::default().fg(Color::Magenta)),
            ]),
            Line::from(vec![
                Span::styled("Status:       ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    selected_skill.status,
                    Style::default().fg(if selected_skill.status == "In Development" {
                        Color::Green
                    } else {
                        Color::DarkGray
                    }),
                ),
            ]),
            Line::raw(""),
            Line::from(Span::styled(
                "Description:",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::styled(
                selected_skill.description,
                Style::default().fg(Color::White),
            )),
            Line::raw(""),
            Line::from(Span::styled(
                "Required Tool Bindings:",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    selected_skill.tools_required.join(", "),
                    Style::default().fg(Color::Cyan),
                ),
            ]),
            Line::raw(""),
            Line::from(Span::styled(
                "Auto-Activation Triggers:",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    selected_skill.triggers.join(", "),
                    Style::default().fg(Color::Magenta),
                ),
            ]),
            Line::raw(""),
            Line::from(Span::styled(
                "─── Skill Structure ───",
                Style::default().fg(Color::Magenta),
            )),
            Line::from(Span::styled(
                " • SKILL.md (Instructions & YAML Frontmatter)",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::styled(
                " • scripts/ (Sandboxed Python/Bash Tool Scripts)",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::styled(
                " • references/ (Domain Knowledge & Schemas)",
                Style::default().fg(Color::DarkGray),
            )),
        ];

        let detail_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Magenta))
            .title(Span::styled(
                " Skill Specification & Interface ",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ));
        Paragraph::new(detail_lines)
            .block(detail_block)
            .render(main_cols[1], buf);
    }
}
