use std::collections::{HashMap, HashSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::chat_types::{ChatContent, ChatMessage};

pub const COMPRESSION_REPORT_KEY: &str = "compression_report";
pub const RECONSTRUCTED_HISTORY_KIND: &str = "reconstructed_history";
pub const RECONSTRUCTION_SCHEMA_VERSION: u32 = 1;
pub const STATIC_COMPRESSION_REPORT_KIND: &str = "chat_compression_report";
pub const MAX_RECONSTRUCTION_JSON_DEPTH: usize = 64;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReconstructionMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_version: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metrics: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconstructionPayload {
    pub messages: Vec<ChatMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconstructionReport {
    pub kind: String,
    pub schema_version: u32,
    pub payload: ReconstructionPayload,
    #[serde(flatten)]
    pub metadata: ReconstructionMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageOrigin {
    ReportPayload {
        report_index: usize,
        message_index: usize,
    },
    Stored {
        message_index: usize,
    },
}

#[derive(Debug, Clone)]
pub struct ActiveContext {
    pub messages: Vec<ChatMessage>,
    pub report_index: Option<usize>,
    pub origins: Vec<MessageOrigin>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActiveContextError {
    NeedsExplicitRebuild,
    InvalidReport(String),
    UnsupportedVersion(u32),
    InvalidPayload(String),
    InvalidWriteback(String),
}

impl fmt::Display for ActiveContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NeedsExplicitRebuild => {
                f.write_str("Legacy compressed history needs an explicit rebuild")
            }
            Self::UnsupportedVersion(v) => {
                write!(f, "Unsupported reconstruction schema version {v}")
            }
            Self::InvalidReport(s) => write!(f, "Invalid reconstruction report: {s}"),
            Self::InvalidPayload(s) => write!(f, "Invalid reconstructed history: {s}"),
            Self::InvalidWriteback(s) => write!(f, "Invalid active-context writeback: {s}"),
        }
    }
}
impl std::error::Error for ActiveContextError {}

pub fn is_reconstruction_report(message: &ChatMessage) -> bool {
    message.role == COMPRESSION_REPORT_KEY
        && message
            .extra
            .get(COMPRESSION_REPORT_KEY)
            .and_then(|v| v.get("kind"))
            .and_then(Value::as_str)
            == Some(RECONSTRUCTED_HISTORY_KIND)
}

pub fn is_static_compression_report(message: &ChatMessage) -> bool {
    message.role == COMPRESSION_REPORT_KEY
        && message
            .extra
            .get(COMPRESSION_REPORT_KEY)
            .and_then(|v| v.get("kind"))
            .and_then(Value::as_str)
            == Some(STATIC_COMPRESSION_REPORT_KIND)
}

pub fn is_legacy_summary(message: &ChatMessage) -> bool {
    if is_reconstruction_report(message) || is_static_compression_report(message) {
        return false;
    }
    message.summarized_range.is_some()
        || message.extra.get("compression").is_some_and(|v| {
            v.get("kind").and_then(Value::as_str) == Some("llm_segment_summary")
                || v.get("summarized_source_message_ids").is_some()
        })
        || message.extra.contains_key("summarized_source_message_ids")
}

fn is_boundary_candidate(message: &ChatMessage) -> bool {
    message.extra.contains_key(COMPRESSION_REPORT_KEY)
        && !is_static_compression_report(message)
        && (message.role == COMPRESSION_REPORT_KEY || !is_legacy_summary(message))
}

pub fn requires_explicit_rebuild(messages: &[ChatMessage]) -> bool {
    let start = messages
        .iter()
        .rposition(is_boundary_candidate)
        .map_or(0, |i| i + 1);
    messages[start..].iter().any(is_legacy_summary)
}

fn validate_depth(value: &Value) -> Result<(), ActiveContextError> {
    let mut stack = vec![(value, 0)];
    while let Some((value, depth)) = stack.pop() {
        if depth > MAX_RECONSTRUCTION_JSON_DEPTH {
            return Err(ActiveContextError::InvalidPayload(
                "JSON nesting exceeds limit".into(),
            ));
        }
        match value {
            Value::Array(values) => stack.extend(values.iter().map(|v| (v, depth + 1))),
            Value::Object(values) => stack.extend(values.values().map(|v| (v, depth + 1))),
            _ => {}
        }
    }
    Ok(())
}

fn validate_ids(messages: &[ChatMessage], require_ids: bool) -> Result<(), ActiveContextError> {
    let mut ids = HashSet::new();
    for message in messages {
        if message.message_id.is_empty() {
            if require_ids {
                return Err(ActiveContextError::InvalidPayload(
                    "payload message has no ID".into(),
                ));
            }
        } else if !ids.insert(&message.message_id) {
            return Err(ActiveContextError::InvalidPayload(
                "duplicate active message ID".into(),
            ));
        }
    }
    Ok(())
}

pub fn validate_reconstruction_payload(messages: &[ChatMessage]) -> Result<(), ActiveContextError> {
    if messages.is_empty() {
        return Err(ActiveContextError::InvalidPayload("empty payload".into()));
    }
    validate_ids(messages, true)?;
    for message in messages {
        if is_boundary_candidate(message) || is_legacy_summary(message) {
            return Err(ActiveContextError::InvalidPayload(
                "recursive report or legacy summary".into(),
            ));
        }
        if !matches!(
            message.role.as_str(),
            "system"
                | "user"
                | "assistant"
                | "tool"
                | "diff"
                | "context_file"
                | "plain_text"
                | "cd_instruction"
                | "goal"
                | "plan"
                | "event"
        ) && !is_static_compression_report(message)
        {
            return Err(ActiveContextError::InvalidPayload(
                "unsupported message role".into(),
            ));
        }
        for key in ["_ui_only", "ui_only"] {
            if message
                .extra
                .get(key)
                .is_some_and(|v| v != &Value::Bool(false))
            {
                return Err(ActiveContextError::InvalidPayload(
                    "UI-only message in payload".into(),
                ));
            }
        }
        for key in ["goal", "plan", "event"] {
            if let Some(meta) = message.extra.get(key) {
                if message.role != key || !meta.is_object() {
                    return Err(ActiveContextError::InvalidPayload(
                        "incorrect control metadata".into(),
                    ));
                }
            }
        }
        if matches!(message.role.as_str(), "goal" | "plan") {
            let meta = message.extra.get(&message.role);
            if !meta.is_some_and(|m| {
                m.get("version")
                    .and_then(Value::as_u64)
                    .is_some_and(|v| v > 0 && v <= u32::MAX as u64)
                    && m.get("mode")
                        .and_then(Value::as_str)
                        .is_some_and(|v| !v.is_empty())
                    && m.get("active").is_none_or(Value::is_boolean)
                    && m.get("budget").is_none_or(Value::is_object)
                    && m.get("supersedes")
                        .is_none_or(|v| v.is_null() || v.is_string())
            }) {
                return Err(ActiveContextError::InvalidPayload(
                    "malformed goal or plan metadata".into(),
                ));
            }
        }
        if message.role == "event"
            && !message.extra.get("event").is_some_and(|m| {
                m.get("subkind").and_then(Value::as_str).is_some_and(|v| {
                    matches!(
                        v,
                        "mode_switch"
                            | "tool_decision"
                            | "ide_callback"
                            | "process_completed"
                            | "cron_fire"
                            | "tick"
                            | "summarization_marker"
                            | "verifier_report"
                            | "cancellation_note"
                            | "system_notice"
                            | "plan_delta"
                            | "goal_delta"
                            | "goal_pursuit"
                    )
                }) && m.get("source").is_some_and(Value::is_string)
                    && m.get("payload").is_some()
            })
        {
            return Err(ActiveContextError::InvalidPayload(
                "malformed event metadata".into(),
            ));
        }
        let value = serde_json::to_value(message).map_err(|_| {
            ActiveContextError::InvalidPayload("message serialization failed".into())
        })?;
        validate_depth(&value)?;
        if matches!(message.role.as_str(), "system" | "goal" | "plan" | "event")
            && (!message.tool_call_id.is_empty()
                || message.tool_calls.as_ref().is_some_and(|v| !v.is_empty()))
        {
            return Err(ActiveContextError::InvalidPayload(
                "control message contains tool calls".into(),
            ));
        }
    }
    Ok(())
}

pub fn parse_reconstruction_report(
    message: &ChatMessage,
) -> Result<ReconstructionReport, ActiveContextError> {
    let value = message
        .extra
        .get(COMPRESSION_REPORT_KEY)
        .ok_or_else(|| ActiveContextError::InvalidReport("missing metadata".into()))?;
    validate_depth(value)?;
    if message.role != COMPRESSION_REPORT_KEY || !is_reconstruction_report(message) {
        return Err(ActiveContextError::InvalidReport(
            "unknown kind or incorrect report role".into(),
        ));
    }
    let version = value
        .get("schema_version")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            ActiveContextError::InvalidReport("missing or invalid schema version".into())
        })?;
    if version != u64::from(RECONSTRUCTION_SCHEMA_VERSION) {
        return Err(ActiveContextError::UnsupportedVersion(
            u32::try_from(version).unwrap_or(u32::MAX),
        ));
    }
    let report: ReconstructionReport = serde_json::from_value(value.clone())
        .map_err(|_| ActiveContextError::InvalidReport("malformed metadata or payload".into()))?;
    validate_reconstruction_payload(&report.payload.messages)?;
    Ok(report)
}

pub fn make_reconstruction_report(
    messages: Vec<ChatMessage>,
    metadata: ReconstructionMetadata,
) -> Result<ChatMessage, ActiveContextError> {
    validate_reconstruction_payload(&messages)?;
    let report = ReconstructionReport {
        kind: RECONSTRUCTED_HISTORY_KIND.into(),
        schema_version: RECONSTRUCTION_SCHEMA_VERSION,
        payload: ReconstructionPayload { messages },
        metadata,
    };
    let value = serde_json::to_value(report)
        .map_err(|_| ActiveContextError::InvalidReport("serialization failed".into()))?;
    validate_depth(&value)?;
    let mut message = ChatMessage {
        message_id: uuid::Uuid::new_v4().to_string(),
        role: COMPRESSION_REPORT_KEY.into(),
        content: ChatContent::SimpleText("Conversation context reconstructed.".into()),
        ..Default::default()
    };
    message.extra.insert(COMPRESSION_REPORT_KEY.into(), value);
    Ok(message)
}

pub fn active_context(stored: &[ChatMessage]) -> Result<ActiveContext, ActiveContextError> {
    let report_index = stored.iter().rposition(is_boundary_candidate);
    let mut messages = Vec::new();
    let mut origins = Vec::new();
    let start = if let Some(index) = report_index {
        let report = parse_reconstruction_report(&stored[index])?;
        for (message_index, message) in report.payload.messages.into_iter().enumerate() {
            messages.push(message);
            origins.push(MessageOrigin::ReportPayload {
                report_index: index,
                message_index,
            });
        }
        index + 1
    } else {
        0
    };
    for (message_index, message) in stored.iter().enumerate().skip(start) {
        if is_legacy_summary(message) {
            return Err(ActiveContextError::NeedsExplicitRebuild);
        }
        messages.push(message.clone());
        origins.push(MessageOrigin::Stored { message_index });
    }
    validate_ids(&messages, false)?;
    Ok(ActiveContext {
        messages,
        report_index,
        origins,
    })
}

pub fn legacy_rebuild_input(
    stored: &[ChatMessage],
) -> Result<Vec<ChatMessage>, ActiveContextError> {
    let input = if let Some(index) = stored.iter().rposition(is_boundary_candidate) {
        let mut messages = parse_reconstruction_report(&stored[index])?
            .payload
            .messages;
        messages.extend_from_slice(&stored[index + 1..]);
        messages
    } else {
        stored.to_vec()
    };
    validate_ids(&input, false)?;
    Ok(input
        .iter()
        .map(|message| {
            let mut result = message.clone();
            if is_legacy_summary(message) {
                result.content = ChatContent::SimpleText(format!(
                    "[Legacy summary — lossy historical context, not original evidence]\n{}",
                    message.content.content_text_only()
                ));
                result.summarized_range = None;
                result.summarization_tier = None;
                result.summarized_token_estimate = None;
                result.extra.remove("compression");
                result.extra.remove("summarized_source_message_ids");
                result
                    .extra
                    .insert("legacy_summary_context".into(), Value::Bool(true));
            }
            result
        })
        .collect())
}

fn same_messages(left: &[ChatMessage], right: &[ChatMessage]) -> bool {
    serde_json::to_value(left).ok() == serde_json::to_value(right).ok()
}

fn store_payload_messages(
    report: &mut ChatMessage,
    payload: Vec<ChatMessage>,
) -> Result<(), ActiveContextError> {
    validate_reconstruction_payload(&payload)?;
    let value = serde_json::to_value(payload)
        .map_err(|_| ActiveContextError::InvalidWriteback("serialization failed".into()))?;
    report
        .extra
        .get_mut(COMPRESSION_REPORT_KEY)
        .and_then(|v| v.get_mut("payload"))
        .and_then(Value::as_object_mut)
        .ok_or_else(|| ActiveContextError::InvalidWriteback("missing payload".into()))?
        .insert("messages".into(), value);
    Ok(())
}

pub fn writeback_active_context(
    stored: &[ChatMessage],
    original: &ActiveContext,
    transformed: &[ChatMessage],
) -> Result<Vec<ChatMessage>, ActiveContextError> {
    let current = active_context(stored)?;
    if current.report_index != original.report_index
        || current.origins != original.origins
        || !same_messages(&current.messages, &original.messages)
    {
        return Err(ActiveContextError::InvalidWriteback(
            "stale projection".into(),
        ));
    }
    validate_ids(transformed, false)?;
    // Preserve control identity/content/order. Goal runtime metadata is mutable
    // through authorized ownership updates which also use this generic seam.
    let controls = |messages: &[ChatMessage]| {
        messages
            .iter()
            .filter(|m| matches!(m.role.as_str(), "system" | "goal" | "plan" | "event"))
            .map(|m| {
                let mut m = m.clone();
                if m.role == "goal" {
                    m.extra.remove("goal");
                }
                m
            })
            .collect::<Vec<_>>()
    };
    if !same_messages(&controls(&original.messages), &controls(transformed)) {
        return Err(ActiveContextError::InvalidWriteback(
            "transform changed control messages".into(),
        ));
    }
    let by_id: HashMap<_, _> = original
        .messages
        .iter()
        .zip(&original.origins)
        .filter(|(m, _)| !m.message_id.is_empty())
        .map(|(m, origin)| (m.message_id.as_str(), origin))
        .collect();
    let mut payload = Vec::new();
    let mut suffix = Vec::new();
    let mut used_idless = HashSet::new();
    let mut in_suffix = original.report_index.is_none();
    for message in transformed {
        if is_boundary_candidate(message) || is_legacy_summary(message) {
            return Err(ActiveContextError::InvalidWriteback(
                "transform introduced a boundary or legacy summary".into(),
            ));
        }
        let origin = by_id.get(message.message_id.as_str()).copied().or_else(|| {
            if !message.message_id.is_empty() {
                return None;
            }
            original
                .messages
                .iter()
                .zip(&original.origins)
                .enumerate()
                .find(|(index, (old, _))| {
                    !used_idless.contains(index)
                        && old.message_id.is_empty()
                        && same_messages(std::slice::from_ref(*old), std::slice::from_ref(message))
                })
                .map(|(index, (_, origin))| {
                    used_idless.insert(index);
                    origin
                })
        });
        match origin {
            Some(MessageOrigin::ReportPayload { .. }) if in_suffix => {
                return Err(ActiveContextError::InvalidWriteback(
                    "payload moved after raw suffix".into(),
                ));
            }
            Some(MessageOrigin::Stored { .. }) => in_suffix = true,
            None if !is_static_compression_report(message) => {
                return Err(ActiveContextError::InvalidWriteback(
                    "new or re-identified non-diagnostic message".into(),
                ));
            }
            _ => {}
        }
        if in_suffix {
            suffix.push(message.clone());
        } else {
            payload.push(message.clone());
        }
    }
    let result = if let Some(index) = original.report_index {
        let mut result = stored[..=index].to_vec();
        store_payload_messages(&mut result[index], payload)?;
        result.extend(suffix);
        result
    } else {
        suffix
    };
    let projected = active_context(&result)?;
    if !same_messages(&projected.messages, transformed) {
        return Err(ActiveContextError::InvalidWriteback(
            "projection changed during writeback".into(),
        ));
    }
    Ok(result)
}

/// Insert one message at `active_index` of the active view without touching the archive.
/// Positions inside the reconstructed payload land in the report payload; positions past
/// it land in the raw suffix after the boundary.
pub fn insert_active_message(
    stored: &[ChatMessage],
    active_index: usize,
    mut message: ChatMessage,
) -> Result<(Vec<ChatMessage>, MessageOrigin), ActiveContextError> {
    if is_boundary_candidate(&message) || is_legacy_summary(&message) {
        return Err(ActiveContextError::InvalidWriteback(
            "insert introduced a boundary or legacy summary".into(),
        ));
    }
    let active = active_context(stored)?;
    if message.message_id.is_empty() {
        message.message_id = uuid::Uuid::new_v4().to_string();
    } else if active
        .messages
        .iter()
        .any(|m| m.message_id == message.message_id)
    {
        return Err(ActiveContextError::InvalidWriteback(
            "duplicate active message ID".into(),
        ));
    }
    let index = active_index.min(active.messages.len());
    let payload_len = active
        .origins
        .iter()
        .filter(|origin| matches!(origin, MessageOrigin::ReportPayload { .. }))
        .count();
    let mut result = stored.to_vec();
    let origin = match active.report_index {
        Some(report_index) if index < payload_len => {
            let mut payload = parse_reconstruction_report(&stored[report_index])?
                .payload
                .messages;
            payload.insert(index, message.clone());
            store_payload_messages(&mut result[report_index], payload)?;
            MessageOrigin::ReportPayload {
                report_index,
                message_index: index,
            }
        }
        Some(report_index) => {
            let message_index = report_index + 1 + (index - payload_len);
            result.insert(message_index, message.clone());
            MessageOrigin::Stored { message_index }
        }
        None => {
            result.insert(index, message.clone());
            MessageOrigin::Stored {
                message_index: index,
            }
        }
    };
    let mut expected = active.messages;
    expected.insert(index, message);
    if !same_messages(&active_context(&result)?.messages, &expected) {
        return Err(ActiveContextError::InvalidWriteback(
            "projection changed during insert".into(),
        ));
    }
    Ok((result, origin))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn message(id: &str, role: &str, text: &str) -> ChatMessage {
        ChatMessage {
            message_id: id.to_string(),
            role: role.to_string(),
            content: ChatContent::SimpleText(text.to_string()),
            ..Default::default()
        }
    }

    fn report(messages: Vec<ChatMessage>) -> ChatMessage {
        make_reconstruction_report(messages, ReconstructionMetadata::default()).unwrap()
    }

    fn diagnostic(id: &str) -> ChatMessage {
        let mut result = message(id, COMPRESSION_REPORT_KEY, "Static trim");
        result.extra.insert(
            COMPRESSION_REPORT_KEY.into(),
            json!({"kind": STATIC_COMPRESSION_REPORT_KIND}),
        );
        result
    }

    fn legacy(id: &str) -> ChatMessage {
        let mut result = message(id, "assistant", "Existing summary");
        result.extra.insert(
            "compression".into(),
            json!({
                "kind": "llm_segment_summary", "summarized_source_message_ids": ["missing-source"]
            }),
        );
        result
    }

    #[test]
    fn malformed_payloads_fail_closed_without_mutating_transcript() {
        for role in ["arbitrary", "error", "ui_only", ""] {
            assert!(validate_reconstruction_payload(&[message("id", role, "x")]).is_err());
        }
        for (role, key, value) in [
            ("user", "_ui_only", json!(true)),
            ("user", "goal", json!({"version": 1, "mode": "agent"})),
            ("plan", "plan", json!({"version": "bad", "mode": "agent"})),
            (
                "event",
                "event",
                json!({"subkind": "unknown", "source": "test"}),
            ),
        ] {
            let mut boundary = report(vec![message("id", "user", "x")]);
            let payload = &mut boundary.extra.get_mut(COMPRESSION_REPORT_KEY).unwrap()["payload"]
                ["messages"][0];
            payload["role"] = json!(role);
            payload[key] = value;
            let stored = vec![message("archive", "user", "browse me"), boundary];
            let before = serde_json::to_value(&stored).unwrap();
            assert!(active_context(&stored).is_err());
            assert_eq!(serde_json::to_value(&stored).unwrap(), before);
        }
    }

    #[test]
    fn payload_depth_is_checked_directly_and_on_read() {
        let mut deep = json!(0);
        for _ in 0..MAX_RECONSTRUCTION_JSON_DEPTH + 1 {
            deep = json!([deep]);
        }
        let mut msg = message("deep", "user", "x");
        msg.extra.insert("nested".into(), deep.clone());
        assert!(validate_reconstruction_payload(&[msg]).is_err());
        let mut boundary = report(vec![message("id", "user", "x")]);
        boundary.extra.get_mut(COMPRESSION_REPORT_KEY).unwrap()["nested"] = deep;
        assert!(active_context(&[boundary]).is_err());
    }

    #[test]
    fn static_writeback_cannot_remove_or_change_controls() {
        let mut plan = message("plan", "plan", "keep exactly");
        plan.extra
            .insert("plan".into(), json!({"version": 1, "mode": "agent"}));
        let stored = vec![report(vec![plan, message("user", "user", "task")])];
        let view = active_context(&stored).unwrap();
        assert!(writeback_active_context(&stored, &view, &view.messages[1..]).is_err());
        let mut changed = view.messages.clone();
        changed[0].content = ChatContent::SimpleText("rewritten".into());
        assert!(writeback_active_context(&stored, &view, &changed).is_err());
        assert!(writeback_active_context(&stored, &view, &view.messages).is_ok());
    }

    #[test]
    fn latest_payload_and_suffix_exclude_archive_and_prior_reports() {
        let original = message("original", "user", "ARCHIVE_ONLY");
        let first = report(vec![message("rebuilt-1", "user", "first context")]);
        let next = message("next", "assistant", "after first report");
        let mut stored = vec![original, first, next];
        let before = active_context(&stored).unwrap();
        assert_eq!(
            before
                .messages
                .iter()
                .map(|m| m.message_id.as_str())
                .collect::<Vec<_>>(),
            vec!["rebuilt-1", "next"]
        );
        let archive = serde_json::to_value(&stored).unwrap();
        stored.push(report(vec![message("rebuilt-2", "user", "second context")]));
        stored.push(diagnostic("static"));
        stored.push(message("tail", "user", "continue"));
        let active = active_context(&stored).unwrap();
        assert_eq!(active.report_index, Some(3));
        assert_eq!(
            active
                .messages
                .iter()
                .map(|m| m.message_id.as_str())
                .collect::<Vec<_>>(),
            vec!["rebuilt-2", "static", "tail"]
        );
        assert_eq!(serde_json::to_value(&stored[..3]).unwrap(), archive);
        assert!(!serde_json::to_string(&active.messages)
            .unwrap()
            .contains("ARCHIVE_ONLY"));
    }

    #[test]
    fn static_reports_do_not_require_rebuild() {
        let stored = vec![message("user", "user", "hello"), diagnostic("static")];
        assert!(!requires_explicit_rebuild(&stored));
        assert_eq!(active_context(&stored).unwrap().report_index, None);
    }

    #[test]
    fn legacy_requires_explicit_rebuild_without_mutating_available_history() {
        let stored = vec![message("user", "user", "task"), legacy("summary")];
        let before = serde_json::to_value(&stored).unwrap();
        assert!(requires_explicit_rebuild(&stored));
        assert!(matches!(
            active_context(&stored),
            Err(ActiveContextError::NeedsExplicitRebuild)
        ));
        let input = legacy_rebuild_input(&stored).unwrap();
        assert!(input[1]
            .content
            .content_text_only()
            .contains("Existing summary"));
        assert!(input[1]
            .content
            .content_text_only()
            .contains("Legacy summary"));
        assert!(!is_legacy_summary(&input[1]));
        assert_eq!(serde_json::to_value(&stored).unwrap(), before);
    }

    #[test]
    fn valid_checkpoint_supersedes_legacy_but_not_new_legacy_suffix() {
        let mut stored = vec![
            legacy("old"),
            report(vec![message("current", "user", "rebuilt")]),
        ];
        assert!(!requires_explicit_rebuild(&stored));
        assert!(active_context(&stored).is_ok());
        stored.push(legacy("later"));
        assert!(requires_explicit_rebuild(&stored));
        assert!(matches!(
            active_context(&stored),
            Err(ActiveContextError::NeedsExplicitRebuild)
        ));
    }

    #[test]
    fn corrupt_latest_report_never_falls_back_to_old_context() {
        let good = report(vec![message("good", "user", "valid")]);
        let mut bad = report(vec![message("bad", "user", "new")]);
        bad.extra.get_mut(COMPRESSION_REPORT_KEY).unwrap()["schema_version"] = json!(999);
        let stored = vec![good, bad];
        assert!(matches!(
            active_context(&stored),
            Err(ActiveContextError::UnsupportedVersion(999))
        ));
        assert!(legacy_rebuild_input(&stored).is_err());
    }

    #[test]
    fn unknown_authoritative_kind_blocks_context() {
        let mut unknown = diagnostic("future");
        unknown.extra.get_mut(COMPRESSION_REPORT_KEY).unwrap()["kind"] = json!("future_history");
        assert!(active_context(&[message("archive", "user", "secret"), unknown]).is_err());
    }

    #[test]
    fn nested_history_reports_and_duplicate_payload_ids_are_rejected() {
        let inner = report(vec![message("inner", "user", "x")]);
        assert!(
            make_reconstruction_report(vec![inner], ReconstructionMetadata::default()).is_err()
        );
        assert!(make_reconstruction_report(
            vec![
                message("same", "user", "x"),
                message("same", "assistant", "y")
            ],
            ReconstructionMetadata::default()
        )
        .is_err());
        assert!(make_reconstruction_report(Vec::new(), ReconstructionMetadata::default()).is_err());
    }

    #[test]
    fn archived_ids_can_be_preserved_in_payload() {
        let original = message("preserved", "user", "same ID");
        let stored = vec![original.clone(), report(vec![original])];
        assert_eq!(active_context(&stored).unwrap().messages.len(), 1);
    }

    #[test]
    fn serialization_preserves_inner_control_and_privacy_metadata() {
        let mut plan = message("plan", "plan", "exact plan bytes\n");
        plan.extra
            .insert("plan".into(), json!({"version": 1, "mode": "agent"}));
        plan.extra.insert("privacy".into(), json!({"files": [{"path": "private.rs", "zone": "private", "attribution": "declared"}]}));
        let value = serde_json::to_value(report(vec![plan.clone()])).unwrap();
        assert!(value.get(COMPRESSION_REPORT_KEY).is_some());
        let restored: ChatMessage = serde_json::from_value(value).unwrap();
        let active = active_context(&[restored]).unwrap();
        assert_eq!(
            serde_json::to_value(&active.messages[0]).unwrap(),
            serde_json::to_value(plan).unwrap()
        );
    }

    #[test]
    fn static_writeback_preserves_archive_and_updates_payload_and_suffix() {
        let archived = message("archive", "user", "never mutate");
        let mut boundary = report(vec![
            message("payload", "user", "large input"),
            message("drop", "context_file", "file"),
        ]);
        boundary.extra.get_mut(COMPRESSION_REPORT_KEY).unwrap()["future_diagnostic"] = json!(true);
        let stored = vec![
            archived.clone(),
            boundary,
            message("suffix", "tool", "long output"),
        ];
        let original = active_context(&stored).unwrap();
        let mut transformed = original.messages.clone();
        transformed.remove(1);
        transformed[0].content = ChatContent::SimpleText("short input".into());
        transformed[1].content = ChatContent::SimpleText("short output".into());
        transformed.push(diagnostic("stats"));
        let result = writeback_active_context(&stored, &original, &transformed).unwrap();
        assert_eq!(
            serde_json::to_value(&result[0]).unwrap(),
            serde_json::to_value(archived).unwrap()
        );
        assert_eq!(result[1].message_id, stored[1].message_id);
        assert_eq!(
            result[1].extra[COMPRESSION_REPORT_KEY]["future_diagnostic"],
            json!(true)
        );
        assert!(same_messages(
            &active_context(&result).unwrap().messages,
            &transformed
        ));
    }

    #[test]
    fn insert_at_active_head_lands_in_payload_and_leaves_archive_intact() {
        let archived = message("archive", "user", "never mutate");
        let stored = vec![
            archived.clone(),
            report(vec![
                message("files", "context_file", "rebuilt files"),
                message("payload", "user", "rebuilt request"),
            ]),
            message("suffix", "user", "follow-up"),
        ];
        let system = message("system", "system", "prompt");
        let (result, origin) = insert_active_message(&stored, 0, system).unwrap();
        assert_eq!(
            origin,
            MessageOrigin::ReportPayload {
                report_index: 1,
                message_index: 0
            }
        );
        assert_eq!(result.len(), stored.len());
        assert_eq!(
            serde_json::to_value(&result[0]).unwrap(),
            serde_json::to_value(&archived).unwrap()
        );
        assert_eq!(result[1].message_id, stored[1].message_id);
        let active = active_context(&result).unwrap();
        assert_eq!(
            active
                .messages
                .iter()
                .map(|m| m.message_id.as_str())
                .collect::<Vec<_>>(),
            vec!["system", "files", "payload", "suffix"]
        );
        let (result, origin) = insert_active_message(
            &result,
            1,
            message("project", "context_file", "instructions"),
        )
        .unwrap();
        assert_eq!(
            origin,
            MessageOrigin::ReportPayload {
                report_index: 1,
                message_index: 1
            }
        );
        assert_eq!(
            active_context(&result)
                .unwrap()
                .messages
                .iter()
                .map(|m| m.message_id.as_str())
                .collect::<Vec<_>>(),
            vec!["system", "project", "files", "payload", "suffix"]
        );
    }

    #[test]
    fn insert_past_payload_lands_in_raw_suffix() {
        let stored = vec![
            report(vec![message("payload", "user", "rebuilt request")]),
            message("suffix", "user", "follow-up"),
        ];
        let (result, origin) =
            insert_active_message(&stored, 1, message("hint", "cd_instruction", "hint")).unwrap();
        assert_eq!(origin, MessageOrigin::Stored { message_index: 1 });
        assert_eq!(result.len(), 3);
        assert_eq!(result[1].message_id, "hint");
        assert_eq!(
            serde_json::to_value(&result[0]).unwrap(),
            serde_json::to_value(&stored[0]).unwrap()
        );
        let (result, origin) =
            insert_active_message(&stored, 99, message("tail", "user", "appended")).unwrap();
        assert_eq!(origin, MessageOrigin::Stored { message_index: 2 });
        assert_eq!(result[2].message_id, "tail");
        let plain = vec![message("only", "user", "hello")];
        let (result, origin) =
            insert_active_message(&plain, 0, message("system", "system", "prompt")).unwrap();
        assert_eq!(origin, MessageOrigin::Stored { message_index: 0 });
        assert_eq!(result[0].message_id, "system");
    }

    #[test]
    fn insert_assigns_ids_and_rejects_duplicates_boundaries_and_invalid_context() {
        let stored = vec![report(vec![message("payload", "user", "rebuilt request")])];
        let (result, _) =
            insert_active_message(&stored, 0, message("", "system", "prompt")).unwrap();
        let active = active_context(&result).unwrap();
        assert!(!active.messages[0].message_id.is_empty());
        assert!(insert_active_message(&stored, 0, message("payload", "system", "dup")).is_err());
        assert!(
            insert_active_message(&stored, 0, report(vec![message("inner", "user", "x")])).is_err()
        );
        assert!(insert_active_message(&stored, 0, legacy("legacy")).is_err());
        assert!(insert_active_message(&stored, 0, message("ui", "error", "boom")).is_err());
        let legacy_history = vec![message("user", "user", "task"), legacy("summary")];
        assert!(matches!(
            insert_active_message(&legacy_history, 0, message("system", "system", "prompt")),
            Err(ActiveContextError::NeedsExplicitRebuild)
        ));
    }

    #[test]
    fn writeback_rejects_stale_and_reordered_views() {
        let stored = vec![
            report(vec![message("payload", "user", "context")]),
            message("suffix", "assistant", "reply"),
        ];
        let original = active_context(&stored).unwrap();
        let mut changed = stored.clone();
        changed.push(message("new", "user", "arrival"));
        assert!(writeback_active_context(&changed, &original, &original.messages).is_err());
        let transformed = vec![original.messages[1].clone(), original.messages[0].clone()];
        assert!(writeback_active_context(&stored, &original, &transformed).is_err());
    }
}
