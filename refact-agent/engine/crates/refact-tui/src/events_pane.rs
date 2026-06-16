use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::{worker_state_label, WorkerInfo};
use crate::theme::{ThemeRole, TuiTheme};

const MAX_EVENTS: usize = 10_000;
const EVENTS_RETENTION_NOTICE_KIND: &str = "retention_notice";
const EVENTS_RETENTION_NOTICE_MESSAGE: &str =
    "Older daemon events dropped after reaching 10000 events";

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
            if !self
                .events
                .iter()
                .any(|event| event.kind == EVENTS_RETENTION_NOTICE_KIND)
            {
                let ts_ms = self
                    .events
                    .last()
                    .map(|event| event.ts_ms)
                    .unwrap_or_default();
                self.events.push(DaemonEventRecord {
                    ts_ms,
                    kind: EVENTS_RETENTION_NOTICE_KIND.to_string(),
                    project_id: None,
                    payload: serde_json::json!({"message": EVENTS_RETENTION_NOTICE_MESSAGE}),
                });
                if self.events.len() > MAX_EVENTS {
                    let drop_count = self.events.len() - MAX_EVENTS;
                    self.events.drain(0..drop_count);
                }
            }
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

pub fn render_event_lines(events: &[DaemonEventRecord], theme: &TuiTheme) -> Vec<Line<'static>> {
    if events.is_empty() {
        return vec![Line::from(Span::styled(
            "No daemon events yet",
            theme.style(ThemeRole::Muted).add_modifier(Modifier::ITALIC),
        ))];
    }
    events
        .iter()
        .rev()
        .take(12)
        .map(|event| event_line(event, theme))
        .collect()
}

pub fn render_worker_lines(workers: &[WorkerInfo], theme: &TuiTheme) -> Vec<Line<'static>> {
    if workers.is_empty() {
        return vec![Line::from(Span::styled(
            "No workers",
            theme.style(ThemeRole::Muted).add_modifier(Modifier::ITALIC),
        ))];
    }
    workers
        .iter()
        .map(|worker| worker_line(worker, theme))
        .collect()
}

fn event_line(event: &DaemonEventRecord, theme: &TuiTheme) -> Line<'static> {
    let project = event.project_id.as_deref().unwrap_or("daemon");
    let mut spans = vec![
        Span::styled(project.to_string(), theme.style(ThemeRole::Accent)),
        Span::raw(" "),
        Span::raw(event.kind.clone()),
    ];
    match &event.payload {
        Value::Null => {}
        Value::Object(map) if map.is_empty() => {}
        value => {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                compact_json(value, 96),
                theme.style(ThemeRole::Muted),
            ));
        }
    }
    Line::from(spans)
}

fn worker_line(worker: &WorkerInfo, theme: &TuiTheme) -> Line<'static> {
    let pid = worker
        .pid
        .map(|pid| pid.to_string())
        .unwrap_or_else(|| "-".to_string());
    Line::from(vec![
        Span::styled(worker.project_id.clone(), theme.style(ThemeRole::Accent)),
        Span::styled(" pid=", theme.style(ThemeRole::Muted)),
        Span::raw(pid),
        Span::styled(" http=", theme.style(ThemeRole::Muted)),
        Span::raw(worker.http_port.to_string()),
        Span::styled(" lsp=", theme.style(ThemeRole::Muted)),
        Span::raw(worker.lsp_port.to_string()),
        Span::styled(" state=", theme.style(ThemeRole::Muted)),
        Span::raw(worker_state_label(Some(worker))),
    ])
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
    fn events_rendering_keeps_event_and_worker_data() {
        let theme = TuiTheme::dark();
        let event = DaemonEventRecord {
            ts_ms: 1,
            kind: "worker_ready".to_string(),
            project_id: Some("abc".to_string()),
            payload: serde_json::json!({"pid": 42}),
        };
        let worker = WorkerInfo {
            project_id: "abc".to_string(),
            pid: Some(42),
            http_port: 9000,
            lsp_port: 9001,
            state: Value::String("ready".to_string()),
            last_error: None,
        };

        assert_eq!(
            render_event_lines(&[event], &theme)[0].to_string(),
            "abc worker_ready {\"pid\":42}"
        );
        assert_eq!(
            render_worker_lines(&[worker], &theme)[0].to_string(),
            "abc pid=42 http=9000 lsp=9001 state=ready"
        );
    }

    #[test]
    fn events_state_caps_tail() {
        let mut state = EventsPaneState::new();
        for idx in 0..10_005 {
            state.push_event(DaemonEventRecord {
                ts_ms: idx,
                kind: "tick".to_string(),
                project_id: None,
                payload: Value::Null,
            });
        }
        assert_eq!(state.events().len(), 10_000);
        assert!(state
            .events()
            .iter()
            .any(|event| event.kind == EVENTS_RETENTION_NOTICE_KIND));
        assert!(state.events()[0].ts_ms > 0);
    }
}
