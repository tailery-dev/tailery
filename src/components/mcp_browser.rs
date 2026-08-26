use ratatui::prelude::Widget;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Row, Table, StatefulWidget},
};

use crate::state::{ContainerConfig, MountConfig, ResourceLimits, ServerConfig, ToolFilter, AppState};
use crate::action::Action;
use crate::components::Component;
use crate::tui::Event;

#[derive(Debug, Clone)]

#[derive(Default)]
pub struct RegistryMcpEntry {
    pub name: &'static str,
    pub display_title: &'static str,
    pub category: &'static str,
    pub transport: &'static str,
    pub description: &'static str,
    pub default_config: ServerConfig,
}

pub fn get_mcp_registry_entries() -> Vec<RegistryMcpEntry> {
    vec![
        RegistryMcpEntry {
            name: "filesystem-sandbox",
            display_title: "Local Filesystem Sandbox",
            category: "File Operations",
            transport: "stdio",
            description: "Secure, read-only Docker isolated filesystem server for navigating and reading local workspace files.",
            default_config: ServerConfig::Local {
                command: Some("mcp/filesystem:latest".to_string()),
                args: vec![],
                container: ContainerConfig {
                    auto_start: false,
                    image: "mcp/filesystem:latest".to_string(),
                    read_only_rootfs: true,
                    mounts: vec![MountConfig {
                        host: "/Users/vlad.fratila/code".to_string(),
                        guest: "/workspace".to_string(),
                        read_only: true,
                    }],
                    ports: Vec::new(),
                    network: "none".to_string(),
                    resources: Some(ResourceLimits {
                        memory_mb: Some(512),
                        cpus: Some(1.0),
                    }),
                },
                env: std::collections::HashMap::new(),
                tool_filter: ToolFilter {
                    allow: vec!["read_file".to_string(), "list_directory".to_string()],
                    deny: vec!["write_file".to_string(), "delete_file".to_string()],
                    auto_approve: vec![],
                },
                transport: crate::state::LocalTransport::Stdio,
            },
        },
        RegistryMcpEntry {
            name: "github-context",
            display_title: "GitHub Repository & PR Context",
            category: "Code Hosting",
            transport: "stdio",
            description: "Inspect repositories, search code, read issues, review pull requests, and query commit history.",
            default_config: ServerConfig::Local {
                command: Some("npx".to_string()),
                args: vec!["-y".to_string(), "@modelcontextprotocol/server-github".to_string()],
                env: std::collections::HashMap::from([
                    ("GITHUB_PERSONAL_ACCESS_TOKEN".to_string(), "${keychain:github-pat}".to_string()),
                ]),
                tool_filter: ToolFilter::default(),
                container: ContainerConfig::default(),
                transport: crate::state::LocalTransport::Stdio,
            },
        },
        RegistryMcpEntry {
            name: "postgres-mcp",
            display_title: "PostgreSQL Database Inspector",
            category: "Databases",
            transport: "stdio",
            description: "Read-only schema inspection and SQL query runner with parameterized safety guardrails.",
            default_config: ServerConfig::Local {
                command: Some("npx".to_string()),
                args: vec!["-y".to_string(), "@modelcontextprotocol/server-postgres".to_string(), "postgresql://localhost/mydb".to_string()],
                env: std::collections::HashMap::new(),
                tool_filter: ToolFilter {
                    allow: vec!["query".to_string(), "list_tables".to_string(), "describe_table".to_string()],
                    deny: vec![],
                    auto_approve: vec![],
                },
                container: ContainerConfig::default(),
                transport: crate::state::LocalTransport::Stdio,
            },
        },
        RegistryMcpEntry {
            name: "brave-search",
            display_title: "Brave Web Search",
            category: "Search & Web",
            transport: "stdio",
            description: "Privacy-preserving web search and local result indexing without tracking or ad injects.",
            default_config: ServerConfig::Local {
                command: Some("npx".to_string()),
                args: vec!["-y".to_string(), "@modelcontextprotocol/server-brave-search".to_string()],
                env: std::collections::HashMap::from([
                    ("BRAVE_API_KEY".to_string(), "${keychain:brave-api-key}".to_string()),
                ]),
                tool_filter: ToolFilter::default(),
                container: ContainerConfig::default(),
                transport: crate::state::LocalTransport::Stdio,
            },
        },
        RegistryMcpEntry {
            name: "memory-graph",
            display_title: "Knowledge Graph & Memory",
            category: "Knowledge & Memory",
            transport: "stdio",
            description: "Persistent knowledge graph memory service that tracks entity relationships across long-running sessions.",
            default_config: ServerConfig::Local {
                command: Some("npx".to_string()),
                args: vec!["-y".to_string(), "@modelcontextprotocol/server-memory".to_string()],
                env: std::collections::HashMap::new(),
                tool_filter: ToolFilter::default(),
                container: ContainerConfig::default(),
                transport: crate::state::LocalTransport::Stdio,
            },
        },
        RegistryMcpEntry {
            name: "fetch-web",
            display_title: "Fetch HTML to Markdown",
            category: "Web & Scraping",
            transport: "stdio",
            description: "Lightweight web page fetcher that converts HTML directly to clean token-efficient Markdown.",
            default_config: ServerConfig::Local {
                command: Some("npx".to_string()),
                args: vec!["-y".to_string(), "@modelcontextprotocol/server-fetch".to_string()],
                env: std::collections::HashMap::new(),
                tool_filter: ToolFilter::default(),
                container: ContainerConfig::default(),
                transport: crate::state::LocalTransport::Stdio,
            },
        },
        RegistryMcpEntry {
            name: "sqlite-explorer",
            display_title: "SQLite Database Explorer",
            category: "Databases",
            transport: "stdio",
            description: "Local SQLite file reader, schema analyzer, and interactive query harness.",
            default_config: ServerConfig::Local {
                command: Some("npx".to_string()),
                args: vec!["-y".to_string(), "@modelcontextprotocol/server-sqlite".to_string(), "./data.db".to_string()],
                env: std::collections::HashMap::new(),
                tool_filter: ToolFilter::default(),
                container: ContainerConfig::default(),
                transport: crate::state::LocalTransport::Stdio,
            },
        },
        RegistryMcpEntry {
            name: "docker-agent",
            display_title: "Docker Runtime Controller",
            category: "DevOps & Containers",
            transport: "stdio",
            description: "Container daemon harness for building images, checking container health, and streaming logs.",
            default_config: ServerConfig::Local {
                command: Some("mcp/docker:latest".to_string()),
                args: vec![],
                container: ContainerConfig {
                    auto_start: false,
                    image: "mcp/docker:latest".to_string(),
                    read_only_rootfs: false,
                    mounts: vec![MountConfig {
                        host: "/var/run/docker.sock".to_string(),
                        guest: "/var/run/docker.sock".to_string(),
                        read_only: false,
                    }],
                    ports: Vec::new(),
                    network: "host".to_string(),
                    resources: None,
                },
                env: std::collections::HashMap::new(),
                tool_filter: ToolFilter::default(),
                transport: crate::state::LocalTransport::Stdio,
            },
        },
        RegistryMcpEntry {
            name: "linear-sync",
            display_title: "Linear Project Management",
            category: "Productivity",
            transport: "streamable-http",
            description: "Direct streamable HTTP bridge for issues, projects, cycles, and task tracking.",
            default_config: ServerConfig::Remote {
                url: "https://mcp.linear.app/stream".to_string(),
                transport: crate::state::RemoteTransport::StreamableHttp,
                headers: std::collections::HashMap::from([
                    ("Authorization".to_string(), "Bearer ${keychain:linear-key}".to_string()),
                ]),
                env: std::collections::HashMap::new(),
                tool_filter: ToolFilter::default(),
                shim_port: None,
            },
        },
    ]
}

impl RegistryMcpEntry {
    pub fn matches_query(&self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        let q = query.to_lowercase();
        self.name.to_lowercase().contains(&q)
            || self.display_title.to_lowercase().contains(&q)
            || self.category.to_lowercase().contains(&q)
            || self.description.to_lowercase().contains(&q)
    }
}

#[derive(Default)]
pub struct McpBrowser {
    pub selected_index: usize,
    pub search_query: String,
    pub search_focused: bool,
}

impl Component for McpBrowser {
    fn register_action_handler(&mut self, _tx: tokio::sync::mpsc::UnboundedSender<Action>) -> color_eyre::Result<()> {
        Ok(())
    }

    fn handle_events(&mut self, event: Option<Event>) -> color_eyre::Result<Option<Action>> {
        let Some(Event::Key(key)) = event else {
            return Ok(None);
        };
        match key.code {
            crossterm::event::KeyCode::Char('j') | crossterm::event::KeyCode::Down => {
                if !self.search_focused {
                    self.selected_index = self.selected_index.saturating_add(1);
                } else if key.code == crossterm::event::KeyCode::Char('j') {
                    self.search_query.push('j');
                    self.selected_index = 0;
                }
            }
            crossterm::event::KeyCode::Char('k') | crossterm::event::KeyCode::Up => {
                if !self.search_focused {
                    self.selected_index = self.selected_index.saturating_sub(1);
                } else if key.code == crossterm::event::KeyCode::Char('k') {
                    self.search_query.push('k');
                    self.selected_index = 0;
                }
            }
            crossterm::event::KeyCode::Char('/') => {
                if !self.search_focused {
                    self.search_focused = true;
                } else {
                    self.search_query.push('/');
                    self.selected_index = 0;
                }
            }
            crossterm::event::KeyCode::Esc => {
                if self.search_focused {
                    self.search_focused = false;
                    self.search_query.clear();
                    self.selected_index = 0;
                }
            }
            crossterm::event::KeyCode::Enter => {
                if self.search_focused {
                    self.search_focused = false;
                }
            }
            crossterm::event::KeyCode::Backspace => {
                if self.search_focused {
                    self.search_query.pop();
                    self.selected_index = 0;
                }
            }
            crossterm::event::KeyCode::Char(c) => {
                if self.search_focused {
                    self.search_query.push(c);
                    self.selected_index = 0;
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn update(&mut self, _action: Action) -> color_eyre::Result<Option<Action>> {
        Ok(None)
    }
}

impl StatefulWidget for &McpBrowser {
    type State = AppState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(
                " MCP Server Registry & Discovery ",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ))
            .style(Style::default().bg(Color::Rgb(15, 20, 30)));

        let inner = block.inner(area);
        Widget::render(block, area, buf);

        if inner.height < 10 || inner.width < 30 {
            return;
        }

        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(8),
                Constraint::Length(2),
            ])
            .split(inner);

        let (search_border_style, search_title) = if self.search_focused {
            (
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
                Span::styled(
                    " Search Filter (Typing...) [Enter/Esc: Browse] ",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
            )
        } else {
            (
                Style::default().fg(Color::DarkGray),
                Span::styled(
                    " Search Filter (Press [/] to Search) ",
                    Style::default().fg(Color::Cyan),
                ),
            )
        };

        let search_text = if self.search_query.is_empty() {
            if self.search_focused {
                vec![
                    Span::styled("█", Style::default().fg(Color::Magenta)),
                    Span::styled(
                        " (type to filter by name, category, or description)",
                        Style::default().fg(Color::DarkGray),
                    ),
                ]
            } else {
                vec![Span::styled(
                    "Press [/] to search or [j]/[k] to browse...",
                    Style::default().fg(Color::DarkGray),
                )]
            }
        } else if self.search_focused {
            vec![
                Span::styled(
                    &self.search_query,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("█", Style::default().fg(Color::Magenta)),
            ]
        } else {
            vec![Span::styled(
                &self.search_query,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )]
        };

        let mut search_line_spans = vec![Span::styled(
            " / ",
            if self.search_focused {
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Cyan)
            },
        )];
        search_line_spans.extend(search_text);

        let search_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(search_border_style)
            .title(search_title);
        Widget::render(Paragraph::new(Line::from(search_line_spans)).block(search_block), main_chunks[0], buf);

        let middle_split = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(main_chunks[1]);

        let active_prof = &state.settings.active_profile;
        let enabled_in_profile = state
            .profiles
            .get(active_prof)
            .map(|p| &p.enabled_servers);

        let all_entries = get_mcp_registry_entries();
        let filtered_entries: Vec<_> = all_entries
            .into_iter()
            .filter(|e| e.matches_query(&self.search_query))
            .collect();

        let selected_idx = if filtered_entries.is_empty() {
            0
        } else {
            self.selected_index % filtered_entries.len()
        };

        let rows: Vec<Row> = filtered_entries
            .iter()
            .enumerate()
            .map(|(idx, entry)| {
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

                let is_installed_in_profile = enabled_in_profile
                    .map(|list| list.contains(&entry.name.to_string()))
                    .unwrap_or(false);

                let is_configured_globally = state.servers.contains_key(entry.name);

                let (status_badge, status_style) = if is_installed_in_profile {
                    (
                        "● INSTALLED",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    )
                } else if is_configured_globally {
                    ("● READY", Style::default().fg(Color::Cyan))
                } else {
                    ("+ AVAILABLE", Style::default().fg(Color::Magenta))
                };

                let transport_style = match entry.transport {
                    "stdio" => Style::default().fg(Color::Cyan),
                    "stdio-in-container" => Style::default().fg(Color::Magenta),
                    "streamable-http" => Style::default().fg(Color::Cyan),
                    _ => Style::default().fg(Color::DarkGray),
                };

                Row::new(vec![
                    Span::styled(format!(" {} ", entry.name), name_style),
                    Span::styled(entry.category, Style::default().fg(Color::Magenta)),
                    Span::styled(entry.transport, transport_style),
                    Span::styled(status_badge, status_style),
                ])
            })
            .collect();

        let table_border_style = if !self.search_focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let table = Table::new(
            rows,
            [
                Constraint::Percentage(35),
                Constraint::Percentage(25),
                Constraint::Percentage(22),
                Constraint::Percentage(18),
            ],
        )
        .header(
            Row::new(vec!["Server Name", "Category", "Transport", "Status"]).style(
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(table_border_style)
                .title(Span::styled(
                    " Registry Catalog ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
        );
        Widget::render(table, middle_split[0], buf);

        let detail_lines = if let Some(entry) = filtered_entries.get(selected_idx) {
            let is_installed = enabled_in_profile
                .map(|list| list.contains(&entry.name.to_string()))
                .unwrap_or(false);

            let mut lines = vec![
                Line::from(vec![
                    Span::styled("Title:        ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        entry.display_title,
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("Identifier:   ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        entry.name,
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("Category:     ", Style::default().fg(Color::DarkGray)),
                    Span::styled(entry.category, Style::default().fg(Color::Magenta)),
                ]),
                Line::from(vec![
                    Span::styled("Transport:    ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        entry.transport,
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::raw(""),
                Line::from(Span::styled(
                    "Description:",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(Span::styled(
                    entry.description,
                    Style::default().fg(Color::White),
                )),
                Line::raw(""),
            ];

            match &entry.default_config {
                crate::state::ServerConfig::Local {
                    command,
                    args,
                    container,
                    ..
                } => {
                    if !container.image.is_empty() {
                        lines.push(Line::from(vec![
                            Span::styled("Docker Image: ", Style::default().fg(Color::DarkGray)),
                            Span::styled(&container.image, Style::default().fg(Color::White)),
                        ]));
                        lines.push(Line::from(vec![
                            Span::styled("Rootfs Mode:  ", Style::default().fg(Color::DarkGray)),
                            Span::styled(
                                if container.read_only_rootfs {
                                    "Read-Only Sandbox"
                                } else {
                                    "Read-Write"
                                },
                                Style::default().fg(if container.read_only_rootfs {
                                    Color::Green
                                } else {
                                    Color::Red
                                }),
                            ),
                        ]));
                    } else if let Some(cmd) = command {
                        lines.push(Line::from(vec![
                            Span::styled("Exec Command: ", Style::default().fg(Color::DarkGray)),
                            Span::styled(
                                format!("{} {}", cmd, args.join(" ")),
                                Style::default().fg(Color::White),
                            ),
                        ]));
                    }
                }
                crate::state::ServerConfig::Remote { url, .. } => {
                    lines.push(Line::from(vec![
                        Span::styled("Remote URL:   ", Style::default().fg(Color::DarkGray)),
                        Span::styled(url, Style::default().fg(Color::Cyan)),
                    ]));
                }
            }

            lines.push(Line::raw(""));
            if is_installed {
                lines.push(Line::from(vec![
                    Span::styled(
                        " ● Active in current profile '",
                        Style::default().fg(Color::Green),
                    ),
                    Span::styled(
                        active_prof,
                        Style::default()
                            .fg(Color::Magenta)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("'", Style::default().fg(Color::Green)),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::styled(" Press ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        " [Enter] ",
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        " to install & enable in profile '",
                        Style::default().fg(Color::White),
                    ),
                    Span::styled(
                        active_prof,
                        Style::default()
                            .fg(Color::Magenta)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("'", Style::default().fg(Color::White)),
                ]));
            }

            lines
        } else {
            vec![Line::from(Span::styled(
                "No registry entries found.",
                Style::default().fg(Color::DarkGray),
            ))]
        };

        let detail_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Magenta))
            .title(Span::styled(
                " Server Specification & Install ",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ));
        Widget::render(Paragraph::new(detail_lines).block(detail_block), middle_split[1], buf);

        let action_line = if self.search_focused {
            Line::from(vec![
                Span::styled(
                    " [Enter] / [Esc] ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("Browse Filtered Results   "),
                Span::styled(
                    " [Backspace] ",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("Delete Character   "),
            ])
        } else {
            Line::from(vec![
                Span::styled(
                    " [/] ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("Search   "),
                Span::styled(
                    " [Enter] / [a] ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("Install to Profile   "),
                Span::styled(
                    " [j]/[k] ",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("Navigate Catalog   "),
                Span::styled(
                    " [Esc] ",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("Close Browser"),
            ])
        };
        Widget::render(Paragraph::new(action_line), main_chunks[2], buf);
    }
}
