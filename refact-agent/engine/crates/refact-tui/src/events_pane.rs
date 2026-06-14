use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::WorkerInfo;

const MAX_EVENTS: usize = 200;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DaemonEventRecord {
    pub ts_ms: u64,
    pub kind: String,
    pub project_id: Option<String>,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EventsPaneState {
    pub open: bool,
    events: Vec<DaemonEventRecord>,
    workers: Vec<WorkerInfo>,
}

impl EventsPaneState {
    pub fn new() -> Self {
        Self {
            open: false,
            events: Vec::new(),
            workers: Vec::new(),
        }
    }

    pub fn events(&self) -> &[DaemonEventRecord] {
        &self.events
    }

    pub fn workers(&self) -> &[WorkerInfo] {
        &self.workers
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    pub fn push_event(&mut self, event: DaemonEventRecord) {
        self.events.push(event);
        if self.events.len() > MAX_EVENTS {
            let drop_count = self.events.len() - MAX_EVENTS;
            self.events.drain(0..drop_count);
        }
    }

    pub fn set_workers(&mut self, workers: Vec<WorkerInfo>) {
        self.workers = workers;
    }
}

pub fn parse_daemon_event(data: &str) -> Result<DaemonEventRecord, serde_json::Error> {
    serde_json::from_str(data)
}

pub fn format_event(event: &DaemonEventRecord) -> String {
    let project = event.project_id.as_deref().unwrap_or("daemon");
    let payload = match &event.payload {
        Value::Null => String::new(),
        Value::Object(map) if map.is_empty() => String::new(),
        value => format!(" {}", compact_json(value, 96)),
    };
    format!("{} {}{}", project, event.kind, payload)
}

pub fn render_event_lines(events: &[DaemonEventRecord]) -> Vec<Line<'static>> {
    if events.is_empty() {
        return vec![Line::from(Span::styled(
            "No daemon events yet",
            Style::default().fg(Color::DarkGray),
        ))];
    }
    events
        .iter()
        .rev()
        .take(12)
        .map(|event| Line::from(Span::raw(format_event(event))))
        .collect()
}

pub fn render_worker_lines(workers: &[WorkerInfo]) -> Vec<Line<'static>> {
    if workers.is_empty() {
        return vec![Line::from(Span::styled(
            "No workers",
            Style::default().fg(Color::DarkGray),
        ))];
    }
    workers
        .iter()
        .map(|worker| {
            let pid = worker
                .pid
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "-".to_string());
            Line::from(format!(
                "{} pid={} http={} lsp={} state={}",
                worker.project_id,
                pid,
                worker.http_port,
                worker.lsp_port,
                crate::client::worker_state_label(Some(worker))
            ))
        })
        .collect()
}

fn compact_json(value: &Value, max_chars: usize) -> String {
    let raw = serde_json::to_string(value).unwrap_or_else(|_| value.to_string());
    if raw.chars().count() <= max_chars {
        return raw;
    }
    let mut out = raw.chars().take(max_chars).collect::<String>();
    out.push('…');
    out
}

impl Default for EventsPaneState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_formatting_includes_project_kind_and_payload() {
        let event = DaemonEventRecord {
            ts_ms: 1,
            kind: "worker_ready".to_string(),
            project_id: Some("abc".to_string()),
            payload: serde_json::json!({"pid": 42}),
        };
        assert_eq!(format_event(&event), "abc worker_ready {\"pid\":42}");
    }

    #[test]
    fn events_state_caps_tail() {
        let mut state = EventsPaneState::new();
        for idx in 0..205 {
            state.push_event(DaemonEventRecord {
                ts_ms: idx,
                kind: "tick".to_string(),
                project_id: None,
                payload: Value::Null,
            });
        }
        assert_eq!(state.events().len(), 200);
        assert_eq!(state.events()[0].ts_ms, 5);
    }
}
