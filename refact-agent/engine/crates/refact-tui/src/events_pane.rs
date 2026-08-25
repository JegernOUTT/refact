use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::client::{worker_state_label, WorkerInfo};
use crate::text_safety::{sanitize_tool_inline, truncate_graphemes};
use crate::theme::{ThemeRole, TuiTheme};
use crate::vendored::line_truncation::truncate_line_with_ellipsis_if_overflow;

const MAX_EVENTS: usize = 10_000;
const EVENTS_RETENTION_NOTICE_KIND: &str = "retention_notice";
const EVENTS_RETENTION_NOTICE_MESSAGE: &str =
    "Older daemon events dropped after reaching 10000 events";
const SOURCE_MAX_GRAPHEMES: usize = 18;
const SUMMARY_MAX_GRAPHEMES: usize = 72;
const RAW_ID_HEX_LEN: usize = 64;
const RAW_ID_PREFIX_LEN: usize = 8;

fn deserialize_default_on_null<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Option::<T>::deserialize(deserializer).map(Option::unwrap_or_default)
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DaemonEventRecord {
    pub ts_ms: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub kind: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub project_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
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
                let ts_ms = self.events.last().and_then(|event| event.ts_ms);
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
    let presentation = event_presentation(event);
    format!(
        "{} · {} · {} · {}",
        presentation.timestamp, presentation.icon, presentation.source, presentation.summary
    )
}

pub fn render_event_lines(
    events: &[DaemonEventRecord],
    theme: &TuiTheme,
    width: u16,
) -> Vec<Line<'static>> {
    if events.is_empty() {
        return vec![truncate_line_with_ellipsis_if_overflow(
            Line::from(Span::styled(
                "No daemon events yet",
                theme.style(ThemeRole::Muted).add_modifier(Modifier::ITALIC),
            )),
            width as usize,
        )];
    }
    events
        .iter()
        .rev()
        .take(12)
        .map(|event| event_line(event, theme))
        .map(|line| truncate_line_with_ellipsis_if_overflow(line, width as usize))
        .collect()
}

pub fn render_worker_lines(
    workers: &[WorkerInfo],
    theme: &TuiTheme,
    width: u16,
) -> Vec<Line<'static>> {
    if workers.is_empty() {
        return vec![truncate_line_with_ellipsis_if_overflow(
            Line::from(Span::styled(
                "No workers",
                theme.style(ThemeRole::Muted).add_modifier(Modifier::ITALIC),
            )),
            width as usize,
        )];
    }
    workers
        .iter()
        .map(|worker| worker_line(worker, theme))
        .map(|line| truncate_line_with_ellipsis_if_overflow(line, width as usize))
        .collect()
}

fn event_line(event: &DaemonEventRecord, theme: &TuiTheme) -> Line<'static> {
    let presentation = event_presentation(event);
    Line::from(vec![
        Span::styled(presentation.timestamp, theme.style(ThemeRole::Muted)),
        Span::styled(" · ", theme.style(ThemeRole::Muted)),
        Span::styled(presentation.icon, theme.style(ThemeRole::Accent)),
        Span::styled(" · ", theme.style(ThemeRole::Muted)),
        Span::styled(presentation.source, theme.style(ThemeRole::Accent)),
        Span::styled(" · ", theme.style(ThemeRole::Muted)),
        Span::raw(presentation.summary),
    ])
}

fn worker_line(worker: &WorkerInfo, theme: &TuiTheme) -> Line<'static> {
    let pid = worker
        .pid
        .map(|pid| pid.to_string())
        .unwrap_or_else(|| "-".to_string());
    Line::from(vec![
        Span::styled(
            compact_source(&worker.project_id),
            theme.style(ThemeRole::Accent),
        ),
        Span::styled(" pid=", theme.style(ThemeRole::Muted)),
        Span::raw(pid),
        Span::styled(" http=", theme.style(ThemeRole::Muted)),
        Span::raw(
            worker
                .http_port
                .map(|port| port.to_string())
                .unwrap_or_else(|| "-".to_string()),
        ),
        Span::styled(" lsp=", theme.style(ThemeRole::Muted)),
        Span::raw(
            worker
                .lsp_port
                .map(|port| port.to_string())
                .unwrap_or_else(|| "-".to_string()),
        ),
        Span::styled(" state=", theme.style(ThemeRole::Muted)),
        Span::raw(worker_state_label(Some(worker))),
    ])
}

struct EventPresentation {
    timestamp: String,
    icon: &'static str,
    source: String,
    summary: String,
}

fn event_presentation(event: &DaemonEventRecord) -> EventPresentation {
    let subkind = event_subkind(&event.kind);
    let payload = event_payload(event);
    EventPresentation {
        timestamp: event_timestamp(event.ts_ms),
        icon: event_icon(subkind),
        source: event_source(event),
        summary: compact_summary(&event_summary(event, subkind, payload)),
    }
}

fn event_subkind(kind: &str) -> &str {
    kind.strip_prefix("chat.").unwrap_or(kind)
}

fn event_payload(event: &DaemonEventRecord) -> &Value {
    event.payload.get("payload").unwrap_or(&event.payload)
}

fn event_source(event: &DaemonEventRecord) -> String {
    event
        .payload
        .get("source")
        .and_then(Value::as_str)
        .filter(|source| !source.trim().is_empty())
        .or(event.project_id.as_deref())
        .map(compact_source)
        .unwrap_or_else(|| "daemon".to_string())
}

fn event_timestamp(ts_ms: Option<u64>) -> String {
    ts_ms
        .and_then(|ts_ms| i64::try_from(ts_ms).ok())
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|timestamp| timestamp.format("%H:%M:%S").to_string())
        .unwrap_or_else(|| "--:--:--".to_string())
}

fn event_icon(subkind: &str) -> &'static str {
    match subkind {
        "mode_switch" => "↔",
        "tool_decision" => "⚖",
        "ide_callback" => "⌘",
        "process_completed" => "■",
        "cron_fire" => "◷",
        "tick" => "·",
        "summarization_marker" => "≋",
        "verifier_report" => "☑",
        "cancellation_note" => "×",
        "system_notice" => "!",
        "plan_delta" => "+",
        "goal_delta" => "◎",
        "goal_pursuit" => "→",
        _ => "•",
    }
}

fn event_summary(event: &DaemonEventRecord, subkind: &str, payload: &Value) -> String {
    let content = event_content(event);
    match subkind {
        "mode_switch" => match (string_field(payload, "from"), string_field(payload, "to")) {
            (Some(from), Some(to)) => format!("Mode switched: {from} → {to}"),
            _ => "Mode switched".to_string(),
        },
        "tool_decision" => tool_decision_summary(payload),
        "ide_callback" => {
            string_field(payload, "summary").unwrap_or_else(|| "IDE callback completed".to_string())
        }
        "process_completed" => process_completed_summary(payload),
        "cron_fire" => string_field(payload, "description")
            .map(|summary| format!("Scheduled task fired: {summary}"))
            .unwrap_or_else(|| "Scheduled task fired".to_string()),
        "tick" => tick_summary(payload),
        "summarization_marker" => summarization_summary(payload),
        "verifier_report" => string_field(payload, "kind")
            .map(|kind| format!("Verifier: {kind}"))
            .unwrap_or_else(|| "Verifier report".to_string()),
        "cancellation_note" => "Cancellation noted".to_string(),
        "system_notice" => string_field(payload, "message")
            .or_else(|| string_field(payload, "error"))
            .unwrap_or_else(|| "System notice".to_string()),
        "plan_delta" => string_field(payload, "summary")
            .map(|summary| format!("Plan updated: {summary}"))
            .unwrap_or_else(|| "Plan updated".to_string()),
        "goal_delta" => string_field(payload, "summary")
            .map(|summary| format!("Goal updated: {summary}"))
            .unwrap_or_else(|| "Goal updated".to_string()),
        "goal_pursuit" => string_field(payload, "kind")
            .map(|kind| format!("Goal pursuit: {kind}"))
            .unwrap_or_else(|| "Goal pursuit updated".to_string()),
        _ => content.unwrap_or_else(|| humanize_kind(&event.kind)),
    }
}

fn tool_decision_summary(payload: &Value) -> String {
    let decision = string_field(payload, "decision")
        .or_else(|| {
            payload
                .get("accepted")
                .and_then(Value::as_bool)
                .map(|accepted| if accepted { "approved" } else { "denied" }.to_string())
        })
        .unwrap_or_else(|| "recorded".to_string());
    let count = payload
        .get("tool_call_ids")
        .and_then(Value::as_array)
        .map(Vec::len)
        .or_else(|| payload.get("tool_call_id").map(|_| 1));
    match count {
        Some(count) => {
            format!(
                "Tool decision: {decision} ({count} tool{})",
                plural_suffix(count)
            )
        }
        None => format!("Tool decision: {decision}"),
    }
}

fn process_completed_summary(payload: &Value) -> String {
    let description =
        string_field(payload, "short_description").unwrap_or_else(|| "Process".to_string());
    let status = string_field(payload, "status").unwrap_or_else(|| "completed".to_string());
    match payload.get("exit_code").and_then(Value::as_i64) {
        Some(exit_code) => format!("{description} {status} (exit {exit_code})"),
        None => format!("{description} {status}"),
    }
}

fn tick_summary(payload: &Value) -> String {
    let elapsed = payload.get("elapsed_ms").and_then(Value::as_u64);
    let remaining = payload.get("remaining_ms").and_then(Value::as_u64);
    match (elapsed, remaining) {
        (Some(elapsed), Some(remaining)) => format!(
            "Waiting: {} elapsed · {} remaining",
            format_duration(elapsed),
            format_duration(remaining)
        ),
        _ => "Waiting".to_string(),
    }
}

fn summarization_summary(payload: &Value) -> String {
    let before = payload.get("tokens_before").and_then(Value::as_u64);
    let after = payload.get("tokens_after").and_then(Value::as_u64);
    match (before, after) {
        (Some(before), Some(after)) => format!(
            "Context compressed: {} → {} tokens",
            format_count(before),
            format_count(after)
        ),
        _ => "Context summarized".to_string(),
    }
}

fn event_content(event: &DaemonEventRecord) -> Option<String> {
    event
        .payload
        .get("content")
        .or_else(|| event.payload.get("message"))
        .and_then(Value::as_str)
        .filter(|content| !content.trim().is_empty())
        .map(str::to_string)
}

fn string_field(payload: &Value, field: &str) -> Option<String> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
}

fn plural_suffix(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

fn format_duration(ms: u64) -> String {
    if ms >= 60_000 {
        format!("{}m {:02}s", ms / 60_000, (ms / 1_000) % 60)
    } else {
        format!("{}s", ms / 1_000)
    }
}

fn format_count(count: u64) -> String {
    if count >= 1_000 {
        format!("{:.1}K", count as f64 / 1_000.0)
    } else {
        count.to_string()
    }
}

fn humanize_kind(kind: &str) -> String {
    let kind = kind.strip_prefix("chat.").unwrap_or(kind).replace('_', " ");
    let mut chars = kind.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "Event".to_string(),
    }
}

fn compact_source(source: &str) -> String {
    compact_text(source, SOURCE_MAX_GRAPHEMES)
}

fn compact_summary(summary: &str) -> String {
    compact_text(summary, SUMMARY_MAX_GRAPHEMES)
}

fn compact_text(text: &str, max_graphemes: usize) -> String {
    let sanitized = sanitize_tool_inline(text);
    let redacted = redact_hex_ids(&sanitized);
    truncate_graphemes(&redacted, max_graphemes).0
}

fn redact_hex_ids(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut hex_run = String::new();
    for ch in text.chars() {
        if ch.is_ascii_hexdigit() {
            hex_run.push(ch);
            continue;
        }
        append_hex_run(&mut output, &mut hex_run);
        output.push(ch);
    }
    append_hex_run(&mut output, &mut hex_run);
    output
}

fn append_hex_run(output: &mut String, hex_run: &mut String) {
    if hex_run.len() >= RAW_ID_HEX_LEN {
        output.extend(hex_run.chars().take(RAW_ID_PREFIX_LEN));
        output.push('…');
    } else {
        output.push_str(hex_run);
    }
    hex_run.clear();
}

impl Default for EventsPaneState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat_event(subkind: &str, payload: Value) -> DaemonEventRecord {
        DaemonEventRecord {
            ts_ms: Some(1_700_000_000_000),
            kind: format!("chat.{subkind}"),
            project_id: Some("project".to_string()),
            payload: serde_json::json!({
                "source": "chat.events",
                "content": "",
                "payload": payload,
            }),
        }
    }

    #[test]
    fn event_rows_are_single_line_with_timestamp_and_summary() {
        let event = chat_event(
            "process_completed",
            serde_json::json!({"short_description": "cargo test", "status": "exited", "exit_code": 0}),
        );

        let formatted = format_event(&event);

        assert_eq!(
            formatted,
            "22:13:20 · ■ · chat.events · cargo test exited (exit 0)"
        );
        assert!(!formatted.contains('\n'));
    }

    #[test]
    fn each_event_subkind_has_an_icon_and_summary() {
        let cases = [
            (
                "mode_switch",
                serde_json::json!({"from": "ask", "to": "agent"}),
                "↔",
                "Mode switched: ask → agent",
            ),
            (
                "tool_decision",
                serde_json::json!({"decision": "approved", "tool_call_ids": ["one"]}),
                "⚖",
                "Tool decision: approved (1 tool)",
            ),
            (
                "ide_callback",
                serde_json::json!({"summary": "Applied edit"}),
                "⌘",
                "Applied edit",
            ),
            (
                "process_completed",
                serde_json::json!({"short_description": "cargo test", "status": "exited", "exit_code": 0}),
                "■",
                "cargo test exited (exit 0)",
            ),
            (
                "cron_fire",
                serde_json::json!({"description": "Daily digest"}),
                "◷",
                "Scheduled task fired: Daily digest",
            ),
            (
                "tick",
                serde_json::json!({"elapsed_ms": 5_000, "remaining_ms": 25_000}),
                "·",
                "Waiting: 5s elapsed · 25s remaining",
            ),
            (
                "summarization_marker",
                serde_json::json!({"tokens_before": 1_200, "tokens_after": 300}),
                "≋",
                "Context compressed: 1.2K → 300 tokens",
            ),
            (
                "verifier_report",
                serde_json::json!({"kind": "passed"}),
                "☑",
                "Verifier: passed",
            ),
            (
                "cancellation_note",
                serde_json::json!({}),
                "×",
                "Cancellation noted",
            ),
            (
                "system_notice",
                serde_json::json!({"message": "Daemon reconnected"}),
                "!",
                "Daemon reconnected",
            ),
            (
                "plan_delta",
                serde_json::json!({"summary": "Add tests"}),
                "+",
                "Plan updated: Add tests",
            ),
            (
                "goal_delta",
                serde_json::json!({"summary": "Ship UI"}),
                "◎",
                "Goal updated: Ship UI",
            ),
            (
                "goal_pursuit",
                serde_json::json!({"kind": "resumed"}),
                "→",
                "Goal pursuit: resumed",
            ),
        ];

        for (subkind, payload, icon, summary) in cases {
            let formatted = format_event(&chat_event(subkind, payload));
            assert!(
                formatted.contains(&format!("· {icon} ·")),
                "{subkind}: {formatted}"
            );
            assert!(formatted.ends_with(summary), "{subkind}: {formatted}");
        }
    }

    #[test]
    fn generic_events_use_content_as_their_summary() {
        let event = DaemonEventRecord {
            ts_ms: Some(1_700_000_000_000),
            kind: "worker_ready".to_string(),
            project_id: Some("project".to_string()),
            payload: serde_json::json!({"content": "Worker ready"}),
        };

        assert!(format_event(&event).ends_with("Worker ready"));
    }

    #[test]
    fn event_rows_shorten_raw_hex_identifiers_without_changing_payload() {
        let raw_id = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let event = chat_event("system_notice", serde_json::json!({"message": raw_id}));

        let formatted = format_event(&event);

        assert!(!formatted.contains(raw_id));
        assert!(formatted.contains("01234567…"));
        assert_eq!(event.payload["payload"]["message"], raw_id);
    }

    #[test]
    fn events_rendering_keeps_worker_data_in_one_line() {
        let theme = TuiTheme::dark();
        let worker = WorkerInfo {
            project_id: "abc".to_string(),
            pid: Some(42),
            http_port: Some(9000),
            lsp_port: Some(9001),
            state: Value::String("ready".to_string()),
            last_error: None,
            ..WorkerInfo::default()
        };

        assert_eq!(
            render_worker_lines(&[worker], &theme, 80)[0].to_string(),
            "abc pid=42 http=9000 lsp=9001 state=ready"
        );
    }

    #[test]
    fn events_state_caps_tail() {
        let mut state = EventsPaneState::new();
        for idx in 0..10_005 {
            state.push_event(DaemonEventRecord {
                ts_ms: Some(idx),
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
        assert!(state.events()[0].ts_ms.is_some_and(|ts_ms| ts_ms > 0));
    }
}
