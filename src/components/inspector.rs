use color_eyre::Result;
use ratatui::prelude::Widget;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Row, Table, StatefulWidget},
};
use serde_json::Value;

use crate::action::Action;
use crate::components::Component;
use crate::shim::TelemetryMessage;
use crate::state::AppState;
use crate::tui::Event;

#[derive(Debug, Clone)]

#[derive(Default)]
pub struct InspectorEvent {
    pub timestamp: String,
    pub server: String,
    pub event_type: String,
    pub summary: String,
    pub latency: Option<u64>,
    pub payload: Value,
}

impl InspectorEvent {
    pub fn from_telemetry(msg: &TelemetryMessage) -> Self {
        match msg {
            TelemetryMessage::Request {
                server,
                id,
                method,
                params,
                timestamp,
            } => Self {
                timestamp: timestamp.chars().skip(11).take(8).collect(),
                server: server.clone(),
                event_type: "REQ".to_string(),
                summary: format!("{} (id: {})", method, id),
                latency: None,
                payload: serde_json::json!({
                    "id": id,
                    "method": method,
                    "params": params,
                }),
            },
            TelemetryMessage::Response {
                server,
                id,
                result,
                error,
                timestamp,
                duration_ms,
            } => Self {
                timestamp: timestamp.chars().skip(11).take(8).collect(),
                server: server.clone(),
                event_type: if error.is_some() {
                    "ERR".to_string()
                } else {
                    "RESP".to_string()
                },
                summary: format!("id: {}", id),
                latency: *duration_ms,
                payload: serde_json::json!({
                    "id": id,
                    "result": result,
                    "error": error,
                }),
            },
            TelemetryMessage::Notification {
                server,
                method,
                params,
                timestamp,
            } => Self {
                timestamp: timestamp.chars().skip(11).take(8).collect(),
                server: server.clone(),
                event_type: "NOTIF".to_string(),
                summary: method.clone(),
                latency: None,
                payload: serde_json::json!({
                    "method": method,
                    "params": params,
                }),
            },
            TelemetryMessage::Blocked {
                server,
                tool,
                reason,
                timestamp,
            } => Self {
                timestamp: timestamp.chars().skip(11).take(8).collect(),
                server: server.clone(),
                event_type: "BLOCK".to_string(),
                summary: format!("tool: {}", tool),
                latency: None,
                payload: serde_json::json!({
                    "blocked_tool": tool,
                    "reason": reason,
                }),
            },
            TelemetryMessage::Log {
                server,
                level,
                message,
                timestamp,
            } => Self {
                timestamp: timestamp.chars().skip(11).take(8).collect(),
                server: server.clone(),
                event_type: "LOG".to_string(),
                summary: format!("[{}] {}", level, message),
                latency: None,
                payload: serde_json::json!({
                    "level": level,
                    "message": message,
                }),
            },
            TelemetryMessage::Status {
                server,
                status,
                pid,
                timestamp,
            } => Self {
                timestamp: timestamp.chars().skip(11).take(8).collect(),
                server: server.clone(),
                event_type: "STATUS".to_string(),
                summary: format!("status: {} (pid: {:?})", status, pid),
                latency: None,
                payload: serde_json::json!({
                    "status": status,
                    "pid": pid,
                }),
            },
        }
    }
}

#[derive(Default)]
pub struct Inspector {
    pub selected_index: usize,
    pub scroll_offset: usize,
}

impl StatefulWidget for &Inspector {
    type State = AppState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(area);

        let events: Vec<InspectorEvent> = state.inspector_events.iter().map(InspectorEvent::from_telemetry).collect();

        // 1. Left Table: Live Event Feed
        let rows: Vec<Row> = events
            .iter()
            .enumerate()
            .map(|(idx, ev)| {
                let is_selected = idx == self.selected_index;
                let type_style = match ev.event_type.as_str() {
                    "REQ" => Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                    "RESP" => Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                    "ERR" => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    "BLOCK" => Style::default()
                        .fg(Color::Black)
                        .bg(Color::Red)
                        .add_modifier(Modifier::BOLD),
                    "NOTIF" => Style::default().fg(Color::Magenta),
                    _ => Style::default().fg(Color::DarkGray),
                };

                let latency_str = ev.latency.map(|l| format!("{}ms", l)).unwrap_or_default();
                let row_style = if is_selected {
                    Style::default().bg(Color::Rgb(30, 40, 60))
                } else {
                    Style::default()
                };

                Row::new(vec![
                    Span::styled(&ev.timestamp, Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        &ev.server,
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(&ev.event_type, type_style),
                    Span::styled(&ev.summary, Style::default().fg(Color::White)),
                    Span::styled(latency_str, Style::default().fg(Color::Cyan)),
                ])
                .style(row_style)
            })
            .collect();

        let event_table = Table::new(
            rows,
            [
                Constraint::Length(9),
                Constraint::Length(14),
                Constraint::Length(8),
                Constraint::Min(20),
                Constraint::Length(8),
            ],
        )
        .header(
            Row::new(vec!["Time", "Server", "Type", "Method / Detail", "Latency"]).style(
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
                    format!(" Live MCP Telemetry Stream ({} events) ", events.len()),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
        );
        Widget::render(event_table, chunks[0], buf);

        // 2. Right Pane: Detailed JSON Payload Inspector
        let mut payload_lines: Vec<Line> = Vec::new();
        if let Some(ev) = events.get(self.selected_index) {
            payload_lines.push(Line::from(vec![
                Span::styled("Event:  ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    ev.event_type.clone(),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" │ "),
                Span::styled("Server: ", Style::default().fg(Color::DarkGray)),
                Span::styled(ev.server.clone(), Style::default().fg(Color::White)),
                Span::raw(" │ "),
                Span::styled("Time: ", Style::default().fg(Color::DarkGray)),
                Span::styled(ev.timestamp.clone(), Style::default().fg(Color::Magenta)),
            ]));
            payload_lines.push(Line::raw(""));
            payload_lines.push(Line::from(Span::styled(
                "─── JSON-RPC Payload ───",
                Style::default().fg(Color::Magenta),
            )));

            let pretty_json = serde_json::to_string_pretty(&ev.payload).unwrap_or_default();
            for line in pretty_json.lines() {
                payload_lines.push(Line::from(Span::styled(
                    line.to_string(),
                    Style::default().fg(Color::White),
                )));
            }
        } else {
            payload_lines.push(Line::from(Span::styled(
                "Select a traffic event to inspect payload.",
                Style::default().fg(Color::DarkGray),
            )));
        }

        let detail_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Magenta))
            .title(Span::styled(
                " JSON-RPC Inspector ",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ));
        Paragraph::new(payload_lines)
            .block(detail_block)
            .scroll((self.scroll_offset as u16, 0))
            .render(chunks[1], buf);
    }
}

impl Component for Inspector {
    fn handle_events(&mut self, _event: Option<Event>) -> Result<Option<Action>> {
        Ok(None)
    }
}
