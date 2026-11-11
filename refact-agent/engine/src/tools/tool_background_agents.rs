use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::agents::types::{AgentListFilter, BackgroundAgent, BgAgentKind, BgAgentStatus};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

const DEFAULT_LIMIT: usize = 20;
const DEFAULT_TERMINAL_HOURS: i64 = 24;
pub struct ToolAgentList {
    pub config_path: String,
}

pub struct ToolAgentStatus {
    pub config_path: String,
}

pub struct ToolAgentResult {
    pub config_path: String,
}

pub struct ToolAgentCancel {
    pub config_path: String,
}

#[derive(Debug, PartialEq, Eq)]
enum AgentLifecycleAction {
    Inspect,
    Close {
        expected_run_generation: Option<u64>,
        reason: Option<String>,
    },
    Resume {
        expected_change_seq: u64,
        instruction: Option<String>,
        reconcile_incomplete: bool,
    },
    ContinueTurn {
        expected_change_seq: u64,
        instruction: Option<String>,
        reconcile_incomplete: bool,
    },
    RaiseAndResume {
        max_steps: usize,
        expected_change_seq: u64,
    },
}

/// What a lifecycle call actually did. Kept separate from the record's status
/// so a request to continue can never be reported as a completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LifecycleOutcome {
    Inspected,
    Closed,
    AlreadyClosed,
    Queued,
}

impl LifecycleOutcome {
    /// True only when the call changed the child's lifecycle right now.
    fn accepted(self) -> bool {
        matches!(self, LifecycleOutcome::Closed | LifecycleOutcome::Queued)
    }

    /// The state to report. An accepted continuation is queued: the agent is
    /// running again only once its new generation starts, and the previous
    /// completion is never presented as this call's result.
    fn state(self, status: BgAgentStatus) -> &'static str {
        match self {
            LifecycleOutcome::Queued => BgAgentStatus::Queued.as_str(),
            LifecycleOutcome::Closed | LifecycleOutcome::AlreadyClosed => {
                BgAgentStatus::Closed.as_str()
            }
            LifecycleOutcome::Inspected => status.as_str(),
        }
    }

    fn summary(self) -> &'static str {
        match self {
            LifecycleOutcome::Inspected => {
                "Child inspected; no limits, lifecycle or execution state changed."
            }
            LifecycleOutcome::Closed => {
                "Child closed. It holds no execution slot; its history, goal and plan stay resumable."
            }
            LifecycleOutcome::AlreadyClosed => {
                "Child was already closed. Nothing changed."
            }
            LifecycleOutcome::Queued => {
                "Continuation accepted and queued on the same child. This is NOT a completion: the child has not run yet, and any earlier result stands unchanged until the new generation reports."
            }
        }
    }
}

fn agent_lifecycle_schema() -> Value {
    json!({
        "type": "object", "additionalProperties": false,
        "properties": {
            "agent_id": {"type": "string", "minLength": 1},
            "action": {"type": "string", "enum": ["inspect", "close", "raise_and_recover", "recover", "continue"], "description": "inspect accepts only agent_id and action. close additionally accepts reason and expected_run_generation. raise_and_recover requires max_steps and expected_change_seq only. recover and continue require expected_change_seq and optionally accept instruction and reconcile_incomplete."},
            "instruction": {"type": "string", "minLength": 1, "description": "Optional exact parent follow-up for recover or continue; appended after preserved child history."},
            "reconcile_incomplete": {"type": "boolean", "default": false, "description": "For recover or continue only: explicitly acknowledge missing tool outcomes as unknown without replaying calls. Inspect side effects before enabling."},
            "reason": {"type": "string", "minLength": 1, "description": "For close only: why this child is being closed."},
            "expected_run_generation": {
                "type": "integer", "minimum": 1,
                "description": "For close only: refuse if the child already moved to another generation."
            },
            "max_steps": {
                "type": "integer", "minimum": 1,
                "description": "New per-child total step cap, required for raise_and_recover. Must increase the stopped cap without exceeding the Settings ceiling."
            },
            "expected_change_seq": {
                "type": "integer", "minimum": 1,
                "description": "Exact change_seq returned by inspect, required for raise_and_recover, recover and continue."
            }
        },
        "required": ["agent_id", "action"]
    })
}

fn parse_agent_lifecycle_args(
    args: &HashMap<String, Value>,
) -> Result<(String, AgentLifecycleAction), String> {
    let action = parse_required_string(args, "action")?;
    let allowed: &[&str] = match action.as_str() {
        "inspect" => &[],
        "close" => &["reason", "expected_run_generation"],
        "recover" | "continue" => &["instruction", "reconcile_incomplete", "expected_change_seq"],
        "raise_and_recover" => &["max_steps", "expected_change_seq"],
        _ => {
            return Err(
                "action must be inspect, close, recover, continue or raise_and_recover".into(),
            )
        }
    };
    for key in args.keys() {
        if !matches!(key.as_str(), "agent_id" | "action") && !allowed.contains(&key.as_str()) {
            return Err(format!("{action} does not accept '{key}'"));
        }
    }
    let agent_id = parse_required_string(args, "agent_id")?;
    let positive = |key: &str| {
        args.get(key)
            .and_then(Value::as_u64)
            .filter(|n| *n > 0)
            .ok_or_else(|| format!("{key} must be a positive integer"))
    };
    let text = |key: &str| {
        args.get(key)
            .map(|v| {
                v.as_str()
                    .filter(|s| !s.trim().is_empty())
                    .map(str::to_string)
                    .ok_or_else(|| format!("{key} must be a non-empty string"))
            })
            .transpose()
    };
    let action = match action.as_str() {
        "inspect" => AgentLifecycleAction::Inspect,
        "close" => AgentLifecycleAction::Close {
            expected_run_generation: args
                .get("expected_run_generation")
                .map(|_| positive("expected_run_generation"))
                .transpose()?,
            reason: text("reason")?,
        },
        "raise_and_recover" => AgentLifecycleAction::RaiseAndResume {
            max_steps: usize::try_from(positive("max_steps")?)
                .map_err(|_| "max_steps is too large".to_string())?,
            expected_change_seq: positive("expected_change_seq")?,
        },
        action => {
            let expected_change_seq = positive("expected_change_seq")?;
            let instruction = text("instruction")?;
            let reconcile_incomplete = args
                .get("reconcile_incomplete")
                .map(|v| {
                    v.as_bool()
                        .ok_or_else(|| "reconcile_incomplete must be a boolean".to_string())
                })
                .transpose()?
                .unwrap_or(false);
            if action == "recover" {
                AgentLifecycleAction::Resume {
                    expected_change_seq,
                    instruction,
                    reconcile_incomplete,
                }
            } else {
                AgentLifecycleAction::ContinueTurn {
                    expected_change_seq,
                    instruction,
                    reconcile_incomplete,
                }
            }
        }
    };
    Ok((agent_id, action))
}

fn background_agent_tool_desc(
    config_path: String,
    name: &str,
    display_name: &str,
    description: &str,
    input_schema: Value,
) -> ToolDesc {
    ToolDesc {
        name: name.to_string(),
        display_name: display_name.to_string(),
        source: ToolSource {
            source_type: ToolSourceType::Builtin,
            config_path,
        },
        experimental: false,
        allow_parallel: true,
        description: description.to_string(),
        input_schema,
        output_schema: None,
        annotations: None,
    }
}

fn agent_list_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "status": {
                "type": "string",
                "enum": ["queued", "running", "waiting_for_approval", "completed", "failed", "cancelled", "interrupted", "closed", "all"],
                "description": "Only include agents with this status. Default: all"
            },
            "kind": {
                "type": "string",
                "enum": ["subagent", "delegate", "all"],
                "description": "Only include this kind of background agent. Default: all"
            },
            "include_terminal_within_hours": {
                "type": "integer",
                "minimum": 0,
                "description": "Include terminal agents finished within this many hours. Default: 24"
            },
            "limit": {
                "type": "integer",
                "minimum": 0,
                "description": "Maximum rows to return. Default: 20"
            }
        },
        "required": []
    })
}

fn agent_id_schema(extra: serde_json::Map<String, Value>) -> Value {
    let mut properties = serde_json::Map::new();
    properties.insert(
        "agent_id".to_string(),
        json!({
            "type": "string",
            "description": "Background agent ID, for example bgagent-..."
        }),
    );
    properties.extend(extra);
    json!({
        "type": "object",
        "properties": properties,
        "required": ["agent_id"]
    })
}

fn agent_status_schema() -> Value {
    agent_id_schema(serde_json::Map::new())
}

fn parse_wait_targets(
    args: &HashMap<String, Value>,
    _max_agents: usize,
) -> Result<Vec<String>, String> {
    let mut ids: Vec<String> = match args.get("agent_ids") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .filter(|id| !id.trim().is_empty())
                    .map(|id| id.trim().to_string())
                    .ok_or_else(|| "argument `agent_ids` must contain agent id strings".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err("argument `agent_ids` must be an array of agent ids".to_string()),
    };
    for key in args.keys() {
        if !matches!(key.as_str(), "agent_ids" | "timeout_ms") {
            return Err(format!("Unknown argument '{key}'"));
        }
    }
    ids.dedup();
    let mut seen = std::collections::HashSet::new();
    ids.retain(|id| seen.insert(id.clone()));
    if ids.is_empty() {
        return Err("provide `agent_id` or a nonempty `agent_ids` list".to_string());
    }
    Ok(ids)
}

fn agent_result_schema() -> Value {
    agent_id_schema(serde_json::Map::from_iter([(
        "include_details".to_string(),
        json!({
            "type": "boolean",
            "description": "Include expanded result details and payload. Default: false"
        }),
    )]))
}

fn agent_cancel_schema() -> Value {
    agent_id_schema(serde_json::Map::from_iter([
        (
            "reason".to_string(),
            json!({
                "type": "string",
                "description": "Optional cancellation reason"
            }),
        ),
        (
            "subtree".to_string(),
            json!({
                "type": "boolean",
                "default": true,
                "description": "Cancel descendants too. Default: true"
            }),
        ),
    ]))
}

fn parse_required_string(args: &HashMap<String, Value>, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("Missing '{key}'"))
}

fn parse_optional_string(args: &HashMap<String, Value>, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn parse_optional_bool(
    args: &HashMap<String, Value>,
    key: &str,
    default: bool,
) -> Result<bool, String> {
    match args.get(key) {
        Some(Value::Null) | None => Ok(default),
        Some(value) => refact_tool_api::coerce_bool(value)
            .ok_or_else(|| format!("{key} must be a boolean, got {value}")),
    }
}

fn parse_optional_u64(
    args: &HashMap<String, Value>,
    key: &str,
    default: u64,
) -> Result<u64, String> {
    match args.get(key) {
        Some(Value::Null) | None => Ok(default),
        Some(value) => value
            .as_u64()
            .ok_or_else(|| format!("{key} must be a non-negative integer")),
    }
}

fn parse_optional_i64(
    args: &HashMap<String, Value>,
    key: &str,
    default: i64,
) -> Result<i64, String> {
    match args.get(key) {
        Some(Value::Null) | None => Ok(default),
        Some(value) => {
            let number = value
                .as_i64()
                .ok_or_else(|| format!("{key} must be a non-negative integer"))?;
            if number < 0 {
                return Err(format!("{key} must be a non-negative integer"));
            }
            Ok(number)
        }
    }
}

fn parse_optional_usize(
    args: &HashMap<String, Value>,
    key: &str,
    default: usize,
) -> Result<usize, String> {
    let value = parse_optional_u64(args, key, default as u64)?;
    usize::try_from(value).map_err(|_| format!("{key} is too large"))
}

fn parse_wait_timeout_ms(
    args: &HashMap<String, Value>,
    default_ms: u64,
    _max_ms: u64,
) -> Result<u64, String> {
    let timeout_ms = parse_optional_u64(args, "timeout_ms", default_ms)?;
    if timeout_ms < 1 {
        return Err("timeout_ms must be at least 1ms".to_string());
    }
    if tokio::time::Instant::now()
        .checked_add(std::time::Duration::from_millis(timeout_ms))
        .is_none()
    {
        return Err("timeout_ms cannot be represented by the system clock".into());
    }
    Ok(timeout_ms)
}

fn parse_optional_enum<'a>(
    args: &'a HashMap<String, Value>,
    key: &str,
    default: &'a str,
) -> Result<&'a str, String> {
    match args.get(key) {
        Some(Value::Null) | None => Ok(default),
        Some(value) => value
            .as_str()
            .ok_or_else(|| format!("{key} must be a string")),
    }
}

fn parse_kind(value: &str) -> Result<Option<BgAgentKind>, String> {
    match value {
        "all" => Ok(None),
        "subagent" => Ok(Some(BgAgentKind::Subagent)),
        "delegate" => Ok(Some(BgAgentKind::Delegate)),
        other => Err(format!("Invalid kind: {other}")),
    }
}

fn parse_status(value: &str) -> Result<Option<Vec<BgAgentStatus>>, String> {
    match value {
        "all" => Ok(None),
        "queued" => Ok(Some(vec![BgAgentStatus::Queued])),
        "running" => Ok(Some(vec![BgAgentStatus::Running])),
        "waiting_for_approval" => Ok(Some(vec![BgAgentStatus::WaitingForApproval])),
        "completed" => Ok(Some(vec![BgAgentStatus::Completed])),
        "failed" => Ok(Some(vec![BgAgentStatus::Failed])),
        "cancelled" => Ok(Some(vec![BgAgentStatus::Cancelled])),
        "interrupted" => Ok(Some(vec![BgAgentStatus::Interrupted])),
        "closed" => Ok(Some(vec![BgAgentStatus::Closed])),
        other => Err(format!("Invalid status: {other}")),
    }
}

fn parse_agent_list_filter(args: &HashMap<String, Value>) -> Result<AgentListFilter, String> {
    Ok(AgentListFilter {
        status: parse_status(parse_optional_enum(args, "status", "all")?)?,
        kind: parse_kind(parse_optional_enum(args, "kind", "all")?)?,
        include_terminal_within_hours: Some(parse_optional_i64(
            args,
            "include_terminal_within_hours",
            DEFAULT_TERMINAL_HOURS,
        )?),
        limit: Some(parse_optional_usize(args, "limit", DEFAULT_LIMIT)?),
    })
}

fn get_parent_chat_id(ccx_lock: &AtCommandsContext) -> String {
    ccx_lock.chat_id.clone()
}

async fn background_agent_context(
    ccx: &Arc<AMutex<AtCommandsContext>>,
) -> (crate::app_state::AppState, String) {
    let ccx_lock = ccx.lock().await;
    (ccx_lock.app.clone(), get_parent_chat_id(&ccx_lock))
}

async fn authorize_agent_control(
    app: &crate::app_state::AppState,
    caller_chat_id: &str,
    caller_root_id: &str,
    caller_agent_id: Option<&str>,
    agent_id: &str,
) -> Result<BackgroundAgent, String> {
    if let Ok(record) = app.agents.get(caller_chat_id, agent_id).await {
        return Ok(record);
    }
    if let Some(caller_agent_id) = caller_agent_id {
        if let Some(record) = app
            .agents
            .list_descendants(caller_agent_id)
            .await
            .into_iter()
            .find(|record| record.agent_id == agent_id)
        {
            return Ok(record);
        }
    }
    let record = app
        .agents
        .get_any(agent_id)
        .await
        .map_err(|error| map_registry_error(agent_id, error))?;
    let target_root = record
        .parent_root_chat_id
        .as_deref()
        .unwrap_or(record.parent_chat_id.as_str());
    let source_root = app
        .agent_executions
        .saved_scope(agent_id)
        .ok_or_else(|| not_found_error(agent_id))?;
    let project = crate::coordination::roots::project_identity(&source_root);
    if app.peer_grants.allows(
        &project,
        caller_root_id,
        target_root,
        crate::coordination::peer_authority::PeerAuthority::Control,
        chrono::Utc::now().timestamp_millis().max(0) as u64,
    ) {
        return Ok(record);
    }
    Err(not_found_error(agent_id))
}

fn not_found_error(agent_id: &str) -> String {
    format!("Background agent '{agent_id}' not found in this chat.")
}

fn map_registry_error(agent_id: &str, error: String) -> String {
    if error == "agent not found" {
        not_found_error(agent_id)
    } else {
        error
    }
}

fn tool_message(tool_call_id: &str, content: String) -> (bool, Vec<ContextEnum>) {
    (
        false,
        vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(content),
            tool_calls: None,
            tool_call_id: tool_call_id.to_string(),
            preserve: Some(true),
            output_filter: Some(OutputFilter::no_limits()),
            ..Default::default()
        })],
    )
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    if max == 0 {
        return String::new();
    }
    format!(
        "{}…",
        text.chars().take(max.saturating_sub(1)).collect::<String>()
    )
}

fn markdown_cell(text: &str, max: usize) -> String {
    truncate_chars(&text.replace('\n', " ").replace('|', "\\|"), max)
}

fn kind_label(kind: BgAgentKind) -> &'static str {
    match kind {
        BgAgentKind::Subagent => "subagent",
        BgAgentKind::Delegate => "delegate",
    }
}

fn kind_title(kind: BgAgentKind) -> &'static str {
    match kind {
        BgAgentKind::Subagent => "Subagent",
        BgAgentKind::Delegate => "Delegate",
    }
}

fn agent_heading(record: &BackgroundAgent) -> String {
    let kind = kind_title(record.kind);
    let prefixed = record
        .title
        .get(..kind.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(kind))
        && record.title[kind.len()..].starts_with(':');
    if prefixed {
        record.title.clone()
    } else {
        format!("{kind}: {}", record.title)
    }
}

fn format_status_chip(status: BgAgentStatus) -> &'static str {
    match status {
        BgAgentStatus::Queued => "⏳ queued",
        BgAgentStatus::Running => "🟢 running",
        BgAgentStatus::WaitingForApproval => "✋ waiting_for_approval",
        BgAgentStatus::Completed => "✅ completed",
        BgAgentStatus::Failed => "❌ failed",
        BgAgentStatus::Cancelled => "⏹ cancelled",
        BgAgentStatus::Interrupted => "⚠ interrupted",
        BgAgentStatus::Closed => "🔒 closed",
    }
}

pub(crate) fn format_agent_status_chip(record: &BackgroundAgent) -> String {
    if record.status == BgAgentStatus::Failed
        && crate::agents::push::budget_stop_feedback(record).is_some()
    {
        return "⏸ budget stopped (stored status: failed)".into();
    }
    record
        .pause_label()
        .map(|label| format!("⏸ {label}"))
        .unwrap_or_else(|| format_status_chip(record.status).to_string())
}

fn human_age(then: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let seconds = now.signed_duration_since(then).num_seconds().max(0);
    if seconds == 0 {
        "now".to_string()
    } else if seconds < 60 {
        "0m".to_string()
    } else if seconds < 60 * 60 {
        format!("{}m", seconds / 60)
    } else if seconds < 60 * 60 * 24 {
        format!("{}h", seconds / (60 * 60))
    } else {
        format!("{}d", seconds / (60 * 60 * 24))
    }
}

fn human_age_ago(then: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let age = human_age(then, now);
    if age == "now" {
        age
    } else {
        format!("{age} ago")
    }
}

fn format_agent_last_activity(record: &BackgroundAgent) -> String {
    if record.status == BgAgentStatus::Failed {
        if let Some(error) = &record.error {
            return format!("error: {}", truncate_chars(error, 80));
        }
    }
    if record.status == BgAgentStatus::Closed {
        return record
            .close_reason
            .as_deref()
            .map(|reason| format!("closed: {}", truncate_chars(reason, 80)))
            .unwrap_or_else(|| "(closed, resumable)".to_string());
    }
    if record.status.is_terminal() {
        return "(finished)".to_string();
    }
    record
        .last_activity
        .clone()
        .unwrap_or_else(|| "-".to_string())
}

fn format_agent_table(rows: &[BackgroundAgent]) -> String {
    let now = Utc::now();
    let mut result = String::from("# Background Agents\n\n");
    result.push_str("| ID | Kind | Status | Title | Tool | Tokens | Age | Last activity |\n");
    result.push_str("|---|---|---|---|---|---|---|---|\n");
    for row in rows {
        result.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} |\n",
            markdown_cell(&row.agent_id, 80),
            kind_label(row.kind),
            format_agent_status_chip(row),
            markdown_cell(&row.title, 80),
            markdown_cell(row.current_tool.as_deref().unwrap_or("-"), 60),
            row.tokens_used,
            human_age(row.created_at, now),
            markdown_cell(&format_agent_last_activity(row), 80),
        ));
    }
    result.push('\n');
    result.push_str(&format_agent_counts(rows));
    result.push_str(
        "\nUse `agent_status(agent_id=...)`, `wait with target.kind agents`, or `agent_result(...)` to follow up.",
    );
    result
}

fn format_agent_counts(rows: &[BackgroundAgent]) -> String {
    if rows.is_empty() {
        return "Total: 0.".to_string();
    }
    let statuses = [
        BgAgentStatus::Queued,
        BgAgentStatus::Running,
        BgAgentStatus::WaitingForApproval,
        BgAgentStatus::Completed,
        BgAgentStatus::Failed,
        BgAgentStatus::Cancelled,
        BgAgentStatus::Interrupted,
        BgAgentStatus::Closed,
    ];
    let counts = statuses
        .into_iter()
        .filter_map(|status| {
            let count = rows
                .iter()
                .filter(|row| row.status == status && !row.is_paused())
                .count();
            (count > 0).then(|| format!("{}: {count}", status.as_str()))
        })
        .collect::<Vec<_>>()
        .join(", ");
    let paused = rows.iter().filter(|row| row.is_paused()).count();
    let counts = if paused > 0 {
        if counts.is_empty() {
            format!("paused: {paused}")
        } else {
            format!("{counts}, paused: {paused}")
        }
    } else {
        counts
    };
    format!("Total: {} ({}).", rows.len(), counts)
}

fn format_optional_line(result: &mut String, label: &str, value: Option<&str>) {
    if let Some(value) = value {
        if !value.trim().is_empty() {
            result.push_str(&format!("- {label}: {}\n", value.trim()));
        }
    }
}

fn format_list_line(result: &mut String, label: &str, values: &[String]) {
    if !values.is_empty() {
        result.push_str(&format!("- {label}: {}\n", values.join(", ")));
    }
}

fn format_agent_status_block(record: &BackgroundAgent) -> String {
    let now = Utc::now();
    let mut result = format!("# {}\n", agent_heading(record));
    result.push_str(&format!("- Status: {}\n", format_agent_status_chip(record)));
    result.push_str(&format!("- Agent ID: {}\n", record.agent_id));
    result.push_str(&format!("- Kind: {}\n", kind_label(record.kind)));
    let started = record
        .started_at
        .map(|started_at| human_age_ago(started_at, now))
        .unwrap_or_else(|| "not started".to_string());
    result.push_str(&format!("- Started: {started}\n"));
    result.push_str(&format!("- Step count: {}\n", record.step_count));
    if let Some(feedback) = crate::agents::push::budget_stop_feedback(record) {
        result.push_str(&format!("- Stop reason: {feedback}\n"));
    }
    format_optional_line(&mut result, "Current tool", record.current_tool.as_deref());
    format_optional_line(&mut result, "Model type", record.model_type.as_deref());
    result.push_str(&format!("- Model: {}\n", record.model));
    format_optional_line(
        &mut result,
        "Worktree branch",
        record.worktree_branch.as_deref(),
    );
    format_optional_line(&mut result, "Merge status", record.merge_status.as_deref());
    format_optional_line(&mut result, "Goal", record.goal_summary.as_deref());
    result.push_str(&format!("- Tokens: {}\n", record.tokens_used));
    if let Some(cost) = record.cost_usd {
        result.push_str(&format!("- Cost: ${cost:.4}\n"));
    }
    if !record.questions.is_empty() {
        let questions = record
            .questions
            .iter()
            .map(|question| match &question.answer {
                Some(answer) => {
                    format!("{}: answered ({})", question.id, truncate_chars(answer, 80))
                }
                None => format!(
                    "{}: pending ({})",
                    question.id,
                    truncate_chars(&question.text, 80)
                ),
            })
            .collect::<Vec<_>>();
        format_list_line(&mut result, "Questions", &questions);
    }
    result.push_str(&format!(
        "- Last activity: {}\n",
        record.last_activity.as_deref().unwrap_or("-")
    ));
    format_optional_line(&mut result, "Progress", record.progress.as_deref());
    if let Some(child_chat_id) = &record.child_chat_id {
        result.push_str(&format!(
            "\nChild trajectory: [view](refact://chat/{child_chat_id})\n"
        ));
    }
    if record.status.is_terminal() {
        format_list_line(&mut result, "Edited files", &record.edited_files);
        format_optional_line(&mut result, "Diff summary", record.diff_summary.as_deref());
        format_optional_line(
            &mut result,
            "Conflict summary",
            record.conflict_summary.as_deref(),
        );
        format_optional_line(&mut result, "Error", record.error.as_deref());
        format_optional_line(&mut result, "Close reason", record.close_reason.as_deref());
        if record.status == BgAgentStatus::Closed {
            result.push_str(
                "\n(Closed, not finished. agent_lifecycle with action inspect, then recover, continues this same child.)\n",
            );
        }
    } else {
        result.push_str("\n(Pull final result with agent_result after completion.)\n");
    }
    result
}

async fn format_agent_result(
    record: &BackgroundAgent,
    include_details: bool,
) -> Result<String, String> {
    if !record.status.is_terminal() {
        return Ok(format!(
            "Agent has not finished yet. Current status: {}. wait with target.kind agents to block.",
            format_agent_status_chip(record)
        ));
    }

    let wants_payload = include_details
        || record
            .result_summary
            .as_deref()
            .map_or(true, |summary| summary.trim().is_empty());
    let mut payload_error = None;
    let payload = if wants_payload {
        match load_result_payload(record).await {
            Ok(payload) => payload,
            Err(error) => {
                payload_error = Some(error);
                None
            }
        }
    } else {
        None
    };
    let summary = record
        .result_summary
        .as_deref()
        .filter(|summary| !summary.trim().is_empty())
        .map(str::to_string)
        .or_else(|| {
            payload
                .as_ref()
                .and_then(|(_, value)| value.as_ref().and_then(format_payload_fallback))
        })
        .or_else(|| {
            payload
                .as_ref()
                .map(|(payload_text, _)| payload_text.clone())
        })
        .or_else(|| {
            record
                .error
                .as_deref()
                .filter(|error| !error.trim().is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| {
            if record.status == BgAgentStatus::Closed {
                "This child was closed by its parent, not finished. Nothing was produced by this generation; its saved history, goal and plan remain resumable.".to_string()
            } else {
                format!("Agent finished with status {}.", record.status.as_str())
            }
        });
    let mut result = format!("# Agent Result: {}\n", record.title);
    result.push_str(&format!("- Status: {}\n", format_agent_status_chip(record)));
    if let Some(feedback) = crate::agents::push::budget_stop_feedback(record) {
        result.push_str(&format!("- Stop reason: {feedback}\n"));
    }
    result.push_str(&format!("- Agent ID: {}\n", record.agent_id));
    result.push_str(&format!("- Kind: {}\n", kind_label(record.kind)));
    format_optional_line(&mut result, "Current tool", record.current_tool.as_deref());
    result.push_str(&format!("- Model: {}\n", record.model));
    format_optional_line(&mut result, "Model type", record.model_type.as_deref());
    format_optional_line(
        &mut result,
        "Worktree branch",
        record.worktree_branch.as_deref(),
    );
    format_optional_line(&mut result, "Merge status", record.merge_status.as_deref());
    format_optional_line(&mut result, "Goal", record.goal_summary.as_deref());
    result.push_str(&format!("- Tokens: {}\n", record.tokens_used));
    if let Some(cost) = record.cost_usd {
        result.push_str(&format!("- Cost: ${cost:.4}\n"));
    }
    if !record.questions.is_empty() {
        let questions = record
            .questions
            .iter()
            .map(|question| match &question.answer {
                Some(answer) => {
                    format!("{}: answered ({})", question.id, truncate_chars(answer, 80))
                }
                None => format!(
                    "{}: pending ({})",
                    question.id,
                    truncate_chars(&question.text, 80)
                ),
            })
            .collect::<Vec<_>>();
        format_list_line(&mut result, "Questions", &questions);
    }
    if let Some(child_chat_id) = &record.child_chat_id {
        result.push_str(&format!(
            "- Child trajectory: [view](refact://chat/{child_chat_id})\n"
        ));
    }
    result.push_str("\n");
    result.push_str(&summary);
    result.push('\n');
    format_list_line(&mut result, "Edited files", &record.edited_files);
    format_optional_line(&mut result, "Diff summary", record.diff_summary.as_deref());
    format_optional_line(
        &mut result,
        "Conflict summary",
        record.conflict_summary.as_deref(),
    );
    format_optional_line(&mut result, "Error", record.error.as_deref());
    format_optional_line(&mut result, "Result payload", payload_error.as_deref());

    if include_details {
        if let Some((payload_text, _)) = payload {
            result.push_str("\n## Details\n");
            result.push_str(&payload_text);
            result.push('\n');
        } else if let Some(payload_error) = payload_error {
            result.push_str("\n## Details\n");
            result.push_str(&payload_error);
            result.push('\n');
        }
    }

    Ok(result)
}

async fn load_result_payload(
    record: &BackgroundAgent,
) -> Result<Option<(String, Option<Value>)>, String> {
    let Some(path) = &record.result_payload_path else {
        return Ok(None);
    };
    let payload_text = tokio::fs::read_to_string(path).await.map_err(|error| {
        format!(
            "Failed to read background agent result payload {}: {error}",
            path.display()
        )
    })?;
    let payload = serde_json::from_str(&payload_text).ok();
    Ok(Some((payload_text, payload)))
}

fn format_payload_fallback(payload: &Value) -> Option<String> {
    let mut lines = Vec::new();
    if let Some(summary) = payload_string(payload, "result_summary") {
        lines.push(summary);
    }
    if let Some(status) = payload_string(payload, "status") {
        lines.push(format!("Status: {status}"));
    }
    if let Some(error) = payload_string(payload, "error") {
        lines.push(format!("Error: {error}"));
    }
    if let Some(files) = payload_string_list(payload, "edited_files") {
        lines.push(format!("Edited files: {}", files.join(", ")));
    }
    if let Some(diff_summary) = payload_string(payload, "diff_summary") {
        lines.push(format!("Diff summary: {diff_summary}"));
    }
    if let Some(conflict_summary) = payload_string(payload, "conflict_summary") {
        lines.push(format!("Conflict summary: {conflict_summary}"));
    }
    if let Some(child_chat_id) = payload_string(payload, "child_chat_id") {
        lines.push(format!("Child trajectory: refact://chat/{child_chat_id}"));
    }
    if lines.is_empty() {
        serde_json::to_string_pretty(payload).ok()
    } else {
        Some(lines.join("\n"))
    }
}

fn payload_string(payload: &Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn payload_string_list(payload: &Value, key: &str) -> Option<Vec<String>> {
    let values = payload.get(key)?.as_array()?;
    let values = values
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    (!values.is_empty()).then_some(values)
}

pub(crate) fn agent_lifecycle_description(config_path: &str) -> ToolDesc {
    let mut description = background_agent_tool_desc(
        config_path.to_string(), "agent_lifecycle", "Agent Lifecycle",
        "Inspect, close, recover or continue a direct child on its saved history, plan and goal. Recover restarts a stopped or closed child; continue asks a completed child for another turn. raise_and_recover increases a stopped per-child step cap within Settings. Mutations require the inspected revision; incomplete outcomes require explicit reconciliation and are never replayed. Accepted continuations are queued, never completed. Cannot change global Settings, goal budgets, policy or provider quotas.",
        agent_lifecycle_schema(),
    );
    description.allow_parallel = false;
    description
}

pub(crate) async fn child_pause_resume(
    ccx: Arc<AMutex<AtCommandsContext>>,
    tool_call_id: &String,
    args: &HashMap<String, Value>,
    resume: bool,
) -> Result<(bool, Vec<ContextEnum>), String> {
    let agent_id = parse_required_string(args, "agent_id")?;
    let (app, chat_id) = background_agent_context(&ccx).await;
    let record = app
        .agents
        .get_any(&agent_id)
        .await
        .map_err(|e| map_registry_error(&agent_id, e))?;
    if record.parent_chat_id != chat_id
        && record.parent_root_chat_id.as_deref() != Some(chat_id.as_str())
    {
        return Err(not_found_error(&agent_id));
    }
    {
        let expected = args
            .get("expected_run_generation")
            .ok_or_else(|| "expected_run_generation is required".to_string())?;
        let expected = expected
            .as_u64()
            .filter(|n| *n > 0)
            .ok_or_else(|| "expected_run_generation must be a positive integer".to_string())?;
        if expected != record.run_generation {
            return Err("child moved to another generation".into());
        }
    }
    let record = app
        .agents
        .set_parent_pause_checked(&agent_id, &chat_id, !resume, record.run_generation)
        .await
        .map_err(|e| map_registry_error(&agent_id, e))?;
    crate::agents::spawn::emit_background_agent_update(app.clone(), &record).await;
    let pause = app.agents.pause_status(&agent_id).await;
    Ok(tool_message(tool_call_id, json!({
        "summary": if resume { "Parent pause released; other gates remain in effect." } else { "Parent pause requested; child parks at its next safe boundary." },
        "accepted": true, "agent_id": record.agent_id, "child_chat_id": record.child_chat_id,
        "status": if record.is_paused() { "paused" } else { record.status.as_str() },
        "paused_by": record.paused_by, "paused_at": record.paused_at, "pause": pause,
        "change_seq": record.change_seq, "run_generation": record.run_generation, "step_count": record.step_count
    }).to_string()))
}

pub struct ToolAgentLifecycle {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolAgentLifecycle {
    fn tool_description(&self) -> ToolDesc {
        agent_lifecycle_description(&self.config_path)
    }
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        agent_lifecycle_execute(ccx, tool_call_id, args).await
    }
    async fn match_against_confirm_deny(
        &self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
    ) -> Result<crate::tools::tools_description::MatchConfirmDeny, String> {
        agent_lifecycle_match_against_confirm_deny(ccx, args).await
    }
    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

pub(crate) async fn agent_lifecycle_execute(
    ccx: Arc<AMutex<AtCommandsContext>>,
    tool_call_id: &String,
    args: &HashMap<String, Value>,
) -> Result<(bool, Vec<ContextEnum>), String> {
    let (agent_id, action) = parse_agent_lifecycle_args(args)?;
    let (app, chat_id) = background_agent_context(&ccx).await;
    let mut record = app
        .agents
        .get_any(&agent_id)
        .await
        .map_err(|error| map_registry_error(&agent_id, error))?;
    if record.parent_chat_id != chat_id {
        return Err(not_found_error(&agent_id));
    }
    let outcome = match action {
        AgentLifecycleAction::Inspect => LifecycleOutcome::Inspected,
        AgentLifecycleAction::Close {
            expected_run_generation,
            reason,
        } => {
            let (closed, already_closed) = crate::agents::lifecycle::close_agent(
                app.clone(),
                &chat_id,
                &agent_id,
                expected_run_generation,
                reason,
            )
            .await
            .map_err(|error| map_registry_error(&agent_id, error))?;
            record = closed;
            if already_closed {
                LifecycleOutcome::AlreadyClosed
            } else {
                LifecycleOutcome::Closed
            }
        }
        AgentLifecycleAction::Resume {
            expected_change_seq,
            instruction,
            reconcile_incomplete,
        } => {
            if record.status == BgAgentStatus::Completed {
                return Err(
                    "this child already completed; use action continue to ask it for another turn"
                        .to_string(),
                );
            }
            record = crate::agents::spawn::resume_agent(
                app.clone(),
                &chat_id,
                &agent_id,
                None,
                expected_change_seq,
                instruction,
                reconcile_incomplete,
            )
            .await?;
            LifecycleOutcome::Queued
        }
        AgentLifecycleAction::ContinueTurn {
            expected_change_seq,
            instruction,
            reconcile_incomplete,
        } => {
            if record.status != BgAgentStatus::Completed {
                return Err(format!(
                        "continue applies to a completed child; this one is {}. Use action recover instead.",
                        record.status.as_str()
                    ));
            }
            record = crate::agents::spawn::resume_agent(
                app.clone(),
                &chat_id,
                &agent_id,
                None,
                expected_change_seq,
                instruction,
                reconcile_incomplete,
            )
            .await?;
            LifecycleOutcome::Queued
        }
        AgentLifecycleAction::RaiseAndResume {
            max_steps,
            expected_change_seq,
        } => {
            record = crate::agents::spawn::resume_capped_agent(
                app.clone(),
                &chat_id,
                &agent_id,
                max_steps,
                expected_change_seq,
            )
            .await?;
            LifecycleOutcome::Queued
        }
    };
    let recovery = crate::agents::spawn::recover_run_config(&app, &chat_id, &agent_id).await;
    let mut recovery_unavailable_reason = recovery.as_ref().err().cloned();
    let recovered = recovery.ok();
    if record.status.is_terminal() {
        if let Some(run) = &recovered {
            recovery_unavailable_reason =
                crate::agents::spawn::recovery_unavailable_reason(&app, &record, run).await;
        }
    }
    let max_steps = recovered.as_ref().map(|run| run.config.max_steps);
    let tool_authority = recovered.as_ref().map(|run| {
        json!({
            "tools": run.config.tools,
            "migrated_from_legacy_all": run.authority_migrated,
        })
    });
    let cap_stop = if record.status.is_terminal() {
        record
            .error
            .as_deref()
            .and_then(crate::agents::types::CapStop::from_error)
    } else {
        None
    };
    let scope_available = recovered.is_some();
    let settings_ceiling = if let Some(run) = &recovered {
        crate::runtime_settings::coordination_for_project(&run.root)
            .await
            .ok()
            .map(|limits| limits.subagent_max_steps)
    } else {
        None
    };
    let recovery_available = scope_available && recovery_unavailable_reason.is_none();
    let unlimited_steps = recovered
        .as_ref()
        .is_some_and(|run| run.config.ordinary_background_has_unlimited_steps());
    let steps_left = recovery_available
        && (unlimited_steps || max_steps.is_some_and(|limit| limit > record.step_count as usize))
        && (cap_stop.is_none()
            || unlimited_steps
                && cap_stop
                    .as_ref()
                    .is_some_and(|cap| cap.parent_can_raise_steps()));
    Ok(tool_message(tool_call_id, json!({
            "summary": outcome.summary(),
            "action": args.get("action"),
            "accepted": outcome.accepted(),
            "agent_id": record.agent_id,
            "child_chat_id": record.child_chat_id,
            "status": if record.is_paused() { "paused" } else { outcome.state(record.status) },
            "paused_by": record.paused_by,
            "paused_at": record.paused_at,
            "change_seq": record.change_seq,
            "run_generation": record.run_generation,
            "step_count": record.step_count,
            "cap_stop": cap_stop,
            "close_reason": record.close_reason,
            "previous_runs": record.previous_runs,
            "can_close": !record.status.is_terminal(),
            "can_resume": steps_left && record.status.is_terminal() && record.status != BgAgentStatus::Completed,
            "can_continue": steps_left && record.status == BgAgentStatus::Completed,
            "can_raise_and_resume": !unlimited_steps && recovery_available && record.status == BgAgentStatus::Failed
                && cap_stop.as_ref().is_some_and(|stop| stop.parent_can_raise_steps()
                    && settings_ceiling.is_some_and(|ceiling| ceiling > stop.used.max(record.step_count as usize).max(stop.limit))),
            "max_steps": max_steps,
            "settings_subagent_max_steps_ceiling": settings_ceiling,
            "resume_configuration_available": max_steps.is_some(),
            "recovery_unavailable_reason": recovery_unavailable_reason,
            "tool_authority": tool_authority,
            "original_scope_available": scope_available,
        }).to_string()))
}

pub(crate) async fn agent_lifecycle_match_against_confirm_deny(
    _ccx: Arc<AMutex<AtCommandsContext>>,
    args: &HashMap<String, Value>,
) -> Result<crate::tools::tools_description::MatchConfirmDeny, String> {
    use crate::tools::tools_description::{MatchConfirmDeny, MatchConfirmDenyResult};
    let (agent_id, action) = parse_agent_lifecycle_args(args)?;
    Ok(MatchConfirmDeny {
        result: match action {
            AgentLifecycleAction::Inspect => MatchConfirmDenyResult::PASS,
            AgentLifecycleAction::Close {
                ..
            }
            | AgentLifecycleAction::RaiseAndResume {
                ..
            }
            | AgentLifecycleAction::Resume {
                ..
            }
            | AgentLifecycleAction::ContinueTurn {
                ..
            } => MatchConfirmDenyResult::CONFIRMATION,
        },
        command: format!(
            "agent_lifecycle {agent_id} {}",
            args.get("action")
                .and_then(Value::as_str)
                .unwrap_or_default()
        ),
        rule: "default".to_string(),
    })
}

#[async_trait]
impl Tool for ToolAgentList {
    fn tool_description(&self) -> ToolDesc {
        background_agent_tool_desc(
            self.config_path.clone(),
            "agent_list",
            "Agent List",
            "List background subagents and delegates spawned by this chat, with status, kind, terminal-window, and limit filters.",
            agent_list_schema(),
        )
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let filter = parse_agent_list_filter(args)?;
        let (app, parent_chat_id) = background_agent_context(&ccx).await;
        let records = app.agents.list_for_parent(&parent_chat_id, filter).await;
        let mut content = format_agent_table(&records);
        if let Some(report) = app.agents.quarantine_report() {
            content = format!("{report}\n\n{content}");
        }
        Ok(tool_message(tool_call_id, content))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolAgentStatus {
    fn tool_description(&self) -> ToolDesc {
        background_agent_tool_desc(
            self.config_path.clone(),
            "agent_status",
            "Agent Status",
            "Show current status for one background agent spawned by this chat.",
            agent_status_schema(),
        )
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let agent_id = parse_required_string(args, "agent_id")?;
        let (app, parent_chat_id) = background_agent_context(&ccx).await;
        let record = app
            .agents
            .get(&parent_chat_id, &agent_id)
            .await
            .map_err(|error| map_registry_error(&agent_id, error))?;
        Ok(tool_message(
            tool_call_id,
            format_agent_status_block(&record),
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

pub(crate) async fn execute_agents_wait(
    ccx: Arc<AMutex<AtCommandsContext>>,
    tool_call_id: &String,
    args: &HashMap<String, Value>,
) -> Result<(bool, Vec<ContextEnum>), String> {
    let settings = refact_work_core::coordination_settings();
    let targets = parse_wait_targets(args, settings.agent_wait_max_agents)?;
    let timeout_ms = parse_wait_timeout_ms(
        args,
        settings.agent_wait_default_ms,
        settings.agent_wait_max_ms,
    )?;
    let (app, parent_chat_id, background_agent_id) = {
        let context = ccx.lock().await;
        (
            context.app.clone(),
            context.chat_id.clone(),
            context.background_agent_id.clone(),
        )
    };
    let interrupt = crate::tools::tools_description::WaitInterrupt::from_context(&ccx).await;
    if targets.len() == 1 {
        let agent_id = &targets[0];
        app.agents
            .get(&parent_chat_id, agent_id)
            .await
            .map_err(|error| map_registry_error(agent_id, error))?;
        let waiting = crate::coordination::deadlock::declare_agent_wait(
            &app,
            &parent_chat_id,
            background_agent_id.as_deref(),
            &targets,
        );
        let record = tokio::select! {
            biased;
            _ = interrupt.wait() => {
                let mut statuses = Vec::new();
                for agent_id in &targets {
                    let record = app.agents.get(&parent_chat_id, agent_id).await.map_err(|e| map_registry_error(agent_id, e))?;
                    statuses.push(json!({"agent_id": agent_id, "status": record.status.as_str()}));
                }
                return Ok(tool_message(tool_call_id, json!({"target":"agents", "outcome":"interrupted", "statuses":statuses, "evidence":"Wait interrupted; no child was cancelled."}).to_string()));
            },
            settled = app
                .agents
                .wait(&parent_chat_id, agent_id, Duration::from_millis(timeout_ms)) => {
                settled.map_err(|error| map_registry_error(agent_id, error))?
            }
            report = waiting.refused() => return Err(report.to_string()),

        };
        let content = if record.status.is_terminal() {
            format_agent_result(&record, false).await?
        } else {
            format!(
                "⏰ Timed out after {timeout_ms}ms — agent still running.\n\n{}",
                format_agent_status_block(&record)
            )
        };
        return Ok(tool_message(tool_call_id, json!({"target": "agents", "outcome": if record.status.is_terminal() { "completed" } else { "timed_out" }, "evidence": content}).to_string()));
    }
    // Every id is authorized before any wait starts, so a guessed id can
    // neither leak a peer's status nor keep the caller waiting.
    let mut already_terminal = None;
    for agent_id in &targets {
        let record = app
            .agents
            .get(&parent_chat_id, agent_id)
            .await
            .map_err(|error| map_registry_error(agent_id, error))?;
        // Argument order is the tie-break, so several already-finished
        // agents always settle on the same one.
        if already_terminal.is_none() && record.status.is_terminal() {
            already_terminal = Some((agent_id.clone(), record));
        }
    }
    let waiting = crate::coordination::deadlock::declare_agent_wait(
        &app,
        &parent_chat_id,
        background_agent_id.as_deref(),
        &targets,
    );
    let waits = targets.iter().map(|agent_id| {
        let app = app.clone();
        let parent_chat_id = parent_chat_id.clone();
        let agent_id = agent_id.clone();
        Box::pin(async move {
            let record = app
                .agents
                .wait(
                    &parent_chat_id,
                    &agent_id,
                    Duration::from_millis(timeout_ms),
                )
                .await
                .map_err(|error| map_registry_error(&agent_id, error));
            (agent_id, record)
        })
    });
    let mut first_terminal = already_terminal;
    if first_terminal.is_none() || interrupt.interrupted() {
        let mut pending: Vec<_> = waits.collect();
        let refused = waiting.refused();
        tokio::pin!(refused);
        while !pending.is_empty() {
            let ((agent_id, result), _, rest) = tokio::select! {
                biased;
            _ = interrupt.wait() => {
                let mut statuses = Vec::new();
                for agent_id in &targets {
                    let record = app.agents.get(&parent_chat_id, agent_id).await.map_err(|e| map_registry_error(agent_id, e))?;
                    statuses.push(json!({"agent_id": agent_id, "status": record.status.as_str()}));
                }
                return Ok(tool_message(tool_call_id, json!({"target":"agents", "outcome":"interrupted", "statuses":statuses, "evidence":"Wait interrupted; no child was cancelled."}).to_string()));
            },
                settled = futures::future::select_all(pending) => settled,
                report = &mut refused => return Err(report.to_string()),

            };
            pending = rest;
            match result {
                Ok(record) if record.status.is_terminal() => {
                    first_terminal = Some((agent_id, record));
                    break;
                }
                Ok(_) => {}
                Err(error) => return Err(error),
            }
        }
    }
    let mut lines = Vec::new();
    match &first_terminal {
        Some((agent_id, record)) => {
            lines.push(format!("First terminal agent: {agent_id}"));
            lines.push(String::new());
            lines.push(format_agent_result(record, false).await?);
        }
        None => {
            lines.push(format!(
                "⏰ Timed out after {timeout_ms}ms — none of {} agents finished.",
                targets.len()
            ));
        }
    }
    lines.push(String::new());
    lines.push("## Other waited agents".to_string());
    for agent_id in &targets {
        if first_terminal
            .as_ref()
            .is_some_and(|(done, _)| done == agent_id)
        {
            continue;
        }
        let record = app
            .agents
            .get(&parent_chat_id, agent_id)
            .await
            .map_err(|error| map_registry_error(agent_id, error))?;
        lines.push(format!("- {agent_id}: {}", record.status.as_str()));
    }
    Ok(tool_message(tool_call_id, json!({"target": "agents", "outcome": if first_terminal.is_some() { "completed" } else { "timed_out" }, "evidence": lines.join("\n")}).to_string()))
}

#[async_trait]
impl Tool for ToolAgentResult {
    fn tool_description(&self) -> ToolDesc {
        background_agent_tool_desc(
            self.config_path.clone(),
            "agent_result",
            "Agent Result",
            "Return the final result for a completed, failed, cancelled, or interrupted background agent spawned by this chat.",
            agent_result_schema(),
        )
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let agent_id = parse_required_string(args, "agent_id")?;
        let include_details = parse_optional_bool(args, "include_details", false)?;
        let (app, parent_chat_id) = background_agent_context(&ccx).await;
        let record = app
            .agents
            .get(&parent_chat_id, &agent_id)
            .await
            .map_err(|error| map_registry_error(&agent_id, error))?;
        let content = format_agent_result(&record, include_details).await?;
        let mut result = tool_message(tool_call_id, content);
        let model = ccx.lock().await.current_model.clone();
        if let Some(partial) =
            crate::agents::push::saved_budget_partial(&app, &record, &model).await
        {
            if let Some(ContextEnum::ChatMessage(message)) = result.1.first_mut() {
                if let ChatContent::SimpleText(text) = &mut message.content {
                    text.push_str("\n\n");
                    text.push_str(&partial.content.content_text_only());
                }
                crate::privacy::records::carry_records_into(message, &[partial])
                    .map_err(|_| "partial findings provenance unavailable")?;
            }
        }
        Ok(result)
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolAgentCancel {
    fn tool_description(&self) -> ToolDesc {
        background_agent_tool_desc(
            self.config_path.clone(),
            "agent_cancel",
            "Agent Cancel",
            "Request cancellation of one running background agent spawned by this chat.",
            agent_cancel_schema(),
        )
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let agent_id = parse_required_string(args, "agent_id")?;
        let reason = parse_optional_string(args, "reason");
        let subtree = parse_optional_bool(args, "subtree", true)?;
        let (app, parent_chat_id, root_chat_id, caller_agent_id) = {
            let context = ccx.lock().await;
            (
                context.app.clone(),
                context.chat_id.clone(),
                context.root_chat_id.clone(),
                context.background_agent_id.clone(),
            )
        };
        let current = authorize_agent_control(
            &app,
            &parent_chat_id,
            &root_chat_id,
            caller_agent_id.as_deref(),
            &agent_id,
        )
        .await?;
        let (updated, changed) = app
            .agents
            .cancel_subtree_with_outcome(&current.parent_chat_id, &agent_id, subtree, reason)
            .await
            .map_err(|error| map_registry_error(&agent_id, error))?;
        if changed.is_empty() {
            return Ok(tool_message(
                tool_call_id,
                format!(
                    "Agent already in terminal state: {}. No action.",
                    format_status_chip(updated[0].status)
                ),
            ));
        }
        for record in &updated {
            crate::agents::spawn::emit_background_agent_update(app.clone(), record).await;
        }
        let cancelled = updated
            .iter()
            .filter(|record| changed.contains(&record.agent_id))
            .map(|record| record.agent_id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        Ok(tool_message(
            tool_call_id,
            format!("✓ Cancel requested for: {cancelled}."),
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    use crate::agents::types::{AgentCompletion, CreateAgentRequest};
    use crate::app_state::AppState;
    use crate::chat::types::ChatSession;
    use crate::tools::tools_description::Tool;
    use serde_json::json;
    use tempfile::TempDir;

    const PARENT: &str = "parent-chat";
    const OTHER_PARENT: &str = "other-parent-chat";

    fn args(items: &[(&str, Value)]) -> HashMap<String, Value> {
        items
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect()
    }

    fn create_request(parent_chat_id: &str, kind: BgAgentKind, title: &str) -> CreateAgentRequest {
        CreateAgentRequest {
            parent_chat_id: parent_chat_id.to_string(),
            parent_root_chat_id: Some(parent_chat_id.to_string()),
            parent_tool_call_id: Some("tool-call".to_string()),
            kind,
            config_name: kind_label(kind).to_string(),
            title: title.to_string(),
            prompt: format!("Prompt for {title}"),
            target_files: vec!["src/frog.rs".to_string()],
            model: "test-model".to_string(),
            model_type: None,
            goal_summary: None,
            plan_present: false,
            worktree_id: None,
            worktree_branch: None,
        }
    }

    fn completion(summary: &str) -> AgentCompletion {
        AgentCompletion {
            result_summary: summary.to_string(),
            edited_files: vec!["src/frog.rs".to_string()],
            diff_summary: Some("one frog changed".to_string()),
            conflict_summary: Some("no conflicts".to_string()),
            child_chat_id: Some("subchat-frog".to_string()),
            deliverable_status: None,
            handoff: None,
        }
    }

    async fn test_context(chat_id: &str) -> (TempDir, AppState, Arc<AMutex<AtCommandsContext>>) {
        let temp = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            temp.path().to_path_buf(),
            config.path().to_path_buf(),
        )
        .await;
        let app = AppState::from_gcx(gcx).await;
        let ccx = Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                app.clone(),
                4096,
                20,
                false,
                vec![],
                chat_id.to_string(),
                None,
                "model".to_string(),
                None,
                None,
            )
            .await,
        ));
        (temp, app, ccx)
    }

    async fn create_agent(
        app: &AppState,
        parent_chat_id: &str,
        kind: BgAgentKind,
        title: &str,
    ) -> BackgroundAgent {
        app.agents
            .create(create_request(parent_chat_id, kind, title))
            .await
            .unwrap()
            .0
    }

    fn output_text(result: (bool, Vec<ContextEnum>)) -> String {
        match result.1.into_iter().next().unwrap() {
            ContextEnum::ChatMessage(message) => match message.content {
                ChatContent::SimpleText(text) => serde_json::from_str::<Value>(&text)
                    .ok()
                    .and_then(|v| {
                        v.get("evidence")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .unwrap_or(text),
                _ => panic!("expected text output"),
            },
            _ => panic!("expected chat message"),
        }
    }

    #[tokio::test]
    async fn capped_result_returns_saved_partial_and_withholds_blocked_destination() {
        let (temp, app, ccx) = test_context(PARENT).await;
        *app.gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![temp.path().canonicalize().unwrap()];
        let record = create_agent(&app, PARENT, BgAgentKind::Subagent, "Capped research").await;
        app.agents
            .mark_running(&record.agent_id, "subchat-partial".into())
            .await
            .unwrap();
        let stop = crate::agents::types::CapStop {
            key: refact_work_core::CapKind::GoalTurns,
            scope: refact_work_core::CapScope::Child,
            limit: 3,
            used: 3,
            resumable: false,
            run_generation: 1,
        };
        app.agents
            .mark_failed(&record.agent_id, stop.as_error().unwrap())
            .await
            .unwrap();
        let mut caps = crate::caps::CodeAssistantCaps::default();
        for provider in ["trusted", "blocked"] {
            let id = format!("{provider}/model");
            caps.chat_models.insert(
                id.clone(),
                Arc::new(crate::caps::ChatModelRecord {
                    base: crate::caps::BaseModelRecord {
                        id,
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            );
        }
        {
            let mut state = app.gcx.caps_state.write().await;
            state.caps = Some(Arc::new(caps));
            state.last_attempted_ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
        }
        let mut session = ChatSession::new("subchat-partial".into());
        session.thread.parent_id = Some(PARENT.into());
        let mut source = ChatMessage::new("context_file".into(), "source evidence".into());
        crate::privacy::records::merge_records(
            &mut source,
            [refact_privacy::FileRecord {
                path: "src/private.rs".into(),
                zone: "private".into(),
                attribution: refact_privacy::Attribution::Declared,
            }],
        );
        let partial_text = format!("Observed parser finding: {}", "é".repeat(5000));
        assert_eq!(partial_text.chars().count(), 5025);
        let messages = vec![
            source,
            ChatMessage::new("assistant".into(), partial_text.clone()),
        ];
        crate::chat::trajectories::save_trajectory_as_with_intent(
            app.gcx.clone(),
            &session.thread,
            &messages,
            crate::chat::types::TrajectoryCommitIntent::Required,
        )
        .await;
        let saved =
            crate::chat::trajectories::load_trajectory_for_chat(app.gcx.clone(), "subchat-partial")
                .await
                .expect("saved partial trajectory must be discoverable in the test workspace");
        assert_eq!(saved.thread.parent_id.as_deref(), Some(PARENT));
        assert_eq!(
            saved.messages.last().unwrap().content.content_text_only(),
            partial_text
        );
        app.gcx.privacy_policy_load.write().unwrap().policy =
            Arc::new(refact_privacy::PrivacyPolicy {
                zones: vec![refact_privacy::Zone {
                    name: "private".into(),
                    patterns: vec!["**/private.rs".into()],
                    send_to: vec!["trusted".into()],
                    on_shell_read: refact_privacy::ShellBehavior::Withhold,
                }],
                ..Default::default()
            });
        let mut tool = ToolAgentResult {
            config_path: String::new(),
        };
        ccx.lock().await.current_model = "trusted/model".into();
        let allowed = tool
            .tool_execute(
                ccx.clone(),
                &"allowed".into(),
                &args(&[("agent_id", json!(record.agent_id))]),
            )
            .await
            .unwrap();
        let ContextEnum::ChatMessage(message) = &allowed.1[0] else {
            panic!("tool output")
        };
        let text = message.content.content_text_only();
        assert!(
            text.contains("Budget stop, not a crash or completion: goal.max_turns"),
            "{text}"
        );
        assert!(text.contains("Saved partial findings (not verified completion"));
        assert!(text.contains("Observed parser finding:"));
        assert!(text.contains("[truncated, 5025 chars]"));
        assert!(!text.contains(&partial_text));
        assert!(text.contains("cannot widen"));
        assert_eq!(
            crate::privacy::records::records_to_carry(&[message.clone()])
                .unwrap()
                .len(),
            1
        );
        let saved_record = app.agents.get_any(&record.agent_id).await.unwrap();
        let mut non_cap = saved_record.clone();
        non_cap.error = Some("provider crash".into());
        assert!(
            crate::agents::push::saved_budget_partial(&app, &non_cap, "trusted/model")
                .await
                .is_none()
        );
        let mut wrong_parent = saved_record.clone();
        wrong_parent.parent_chat_id = "foreign-parent".into();
        assert!(
            crate::agents::push::saved_budget_partial(&app, &wrong_parent, "trusted/model")
                .await
                .is_none()
        );
        let mut absent = saved_record.clone();
        absent.child_chat_id = Some("subchat-missing".into());
        assert!(
            crate::agents::push::saved_budget_partial(&app, &absent, "trusted/model")
                .await
                .is_none()
        );
        ccx.lock().await.current_model = "blocked/model".into();
        let denied = output_text(
            tool.tool_execute(
                ccx,
                &"denied".into(),
                &args(&[("agent_id", json!(record.agent_id))]),
            )
            .await
            .unwrap(),
        );
        assert!(!denied.contains("Observed parser finding:"));
        assert!(!denied.contains("Saved partial findings"));
        assert_eq!(
            app.agents.get_any(&record.agent_id).await.unwrap().status,
            BgAgentStatus::Failed
        );
    }

    #[tokio::test]
    async fn quarantine_is_visible_in_agent_list() {
        let temp = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let root = temp.path().join(".refact").join("background_agents");
        tokio::fs::create_dir_all(&root).await.unwrap();
        let bad = json!({"schema_version": 999, "agent_id": "unsupported"});
        tokio::fs::write(root.join("records.json"), json!([bad]).to_string())
            .await
            .unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            temp.path().to_path_buf(),
            config.path().to_path_buf(),
        )
        .await;
        let report = gcx.agents.quarantine_report().unwrap().clone();
        let app = AppState::from_gcx(gcx).await;
        let ccx = Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                app,
                4096,
                20,
                false,
                vec![],
                PARENT.to_string(),
                None,
                "model".to_string(),
                None,
                None,
            )
            .await,
        ));
        let output = output_text(
            ToolAgentList {
                config_path: String::new(),
            }
            .tool_execute(ccx, &"list".to_string(), &HashMap::new())
            .await
            .unwrap(),
        );
        assert!(output.starts_with("WARNING: Background agent data quarantined"));
        assert!(output.contains(&report.path.display().to_string()));
        assert!(output.contains("Record count: 1"));
        assert!(output.contains("Unsupported background agent schema version: 999"));
        assert!(output.contains("Total: 0"));
        let sidecar: Value =
            serde_json::from_slice(&tokio::fs::read(&report.path).await.unwrap()).unwrap();
        assert_eq!(sidecar[0]["record"], bad);
    }

    #[tokio::test]
    async fn peer_cancel_requires_directional_control_not_metadata_or_mail() {
        let (_temp, app, ccx) = test_context("root-a").await;
        ccx.lock().await.root_chat_id = "root-a".to_string();
        let mut peer_request = create_request("root-b", BgAgentKind::Subagent, "Peer");
        peer_request.parent_root_chat_id = Some("root-b".to_string());
        let (peer, abort, _) = app.agents.create(peer_request).await.unwrap();
        app.agents
            .mark_running(&peer.agent_id, "peer-chat".to_string())
            .await
            .unwrap();
        app.agent_executions
            .remember_scope(&peer.agent_id, std::env::temp_dir());
        let project = crate::coordination::roots::project_identity(&std::env::temp_dir());
        let request = args(&[("agent_id", json!(peer.agent_id.clone()))]);
        for authority in [
            crate::coordination::peer_authority::PeerAuthority::Metadata,
            crate::coordination::peer_authority::PeerAuthority::Mail,
        ] {
            app.peer_grants
                .grant(
                    crate::coordination::peer_authority::PeerGrant {
                        project: project.clone(),
                        sender_root: "root-a".to_string(),
                        recipient_root: "root-b".to_string(),
                        authority,
                        granted_by: "user".to_string(),
                        expires_at_ms: chrono::Utc::now().timestamp_millis().max(0) as u64 + 10_000,
                    },
                    chrono::Utc::now().timestamp_millis().max(0) as u64,
                )
                .unwrap();
            let error = ToolAgentCancel {
                config_path: String::new(),
            }
            .tool_execute(ccx.clone(), &"denied".to_string(), &request)
            .await
            .unwrap_err();
            assert_eq!(error, not_found_error(&peer.agent_id));
            assert!(!abort.load(std::sync::atomic::Ordering::SeqCst));
        }
        app.peer_grants
            .grant(
                crate::coordination::peer_authority::PeerGrant {
                    project,
                    sender_root: "root-a".to_string(),
                    recipient_root: "root-b".to_string(),
                    authority: crate::coordination::peer_authority::PeerAuthority::Control,
                    granted_by: "user".to_string(),
                    expires_at_ms: chrono::Utc::now().timestamp_millis().max(0) as u64 + 10_000,
                },
                chrono::Utc::now().timestamp_millis().max(0) as u64,
            )
            .unwrap();
        ToolAgentCancel {
            config_path: String::new(),
        }
        .tool_execute(ccx, &"allowed".to_string(), &request)
        .await
        .unwrap();
        assert!(abort.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn parent_scoping_is_enforced_for_all_agent_tools() {
        let (_temp, app, ccx) = test_context(PARENT).await;
        let record = create_agent(&app, OTHER_PARENT, BgAgentKind::Delegate, "Hidden").await;

        let list_output = output_text(
            ToolAgentList {
                config_path: String::new(),
            }
            .tool_execute(ccx.clone(), &"call".to_string(), &HashMap::new())
            .await
            .unwrap(),
        );
        assert!(!list_output.contains(&record.agent_id));
        assert!(list_output.contains("Total: 0"));

        let status_err = ToolAgentStatus {
            config_path: String::new(),
        }
        .tool_execute(
            ccx.clone(),
            &"call".to_string(),
            &args(&[("agent_id", json!(record.agent_id.clone()))]),
        )
        .await
        .unwrap_err();
        assert_eq!(status_err, not_found_error(&record.agent_id));

        let wait_err = execute_agents_wait(
            ccx.clone(),
            &"call".to_string(),
            &args(&[
                ("agent_ids", json!([record.agent_id.clone()])),
                ("timeout_ms", json!(1000)),
            ]),
        )
        .await
        .unwrap_err();
        assert_eq!(wait_err, not_found_error(&record.agent_id));

        let result_err = ToolAgentResult {
            config_path: String::new(),
        }
        .tool_execute(
            ccx.clone(),
            &"call".to_string(),
            &args(&[("agent_id", json!(record.agent_id.clone()))]),
        )
        .await
        .unwrap_err();
        assert_eq!(result_err, not_found_error(&record.agent_id));

        let cancel_err = ToolAgentCancel {
            config_path: String::new(),
        }
        .tool_execute(
            ccx,
            &"call".to_string(),
            &args(&[("agent_id", json!(record.agent_id.clone()))]),
        )
        .await
        .unwrap_err();
        assert_eq!(cancel_err, not_found_error(&record.agent_id));
    }

    #[tokio::test]
    async fn agent_wait_timeout_returns_status_with_prefix() {
        let (_temp, app, ccx) = test_context(PARENT).await;
        let record = create_agent(&app, PARENT, BgAgentKind::Delegate, "Slow agent").await;
        app.agents
            .mark_running(&record.agent_id, "subchat-slow".to_string())
            .await
            .unwrap();

        let output = output_text(
            execute_agents_wait(
                ccx,
                &"call".to_string(),
                &args(&[
                    ("agent_ids", json!([record.agent_id])),
                    ("timeout_ms", json!(1000)),
                ]),
            )
            .await
            .unwrap(),
        );

        assert!(output.starts_with("⏰ Timed out after 1000ms"));
        assert!(output.contains("# Delegate: Slow agent"));
        let mut prefixed = record.clone();
        prefixed.kind = BgAgentKind::Subagent;
        prefixed.title = "subagent: Fix it".to_string();
        assert!(format_agent_status_block(&prefixed).starts_with("# subagent: Fix it\n"));
        assert!(output.contains("- Status: 🟢 running"));
    }

    #[tokio::test]
    async fn agent_wait_completion_returns_result() {
        let (_temp, app, ccx) = test_context(PARENT).await;
        let record = create_agent(&app, PARENT, BgAgentKind::Subagent, "Finish soon").await;
        app.agents
            .mark_running(&record.agent_id, "subchat-finish".to_string())
            .await
            .unwrap();
        let agent_id = record.agent_id.clone();
        let registry = app.agents.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(25)).await;
            registry
                .mark_completed(&agent_id, completion("finished from wait"))
                .await
                .unwrap();
        });

        let output = output_text(
            execute_agents_wait(
                ccx,
                &"call".to_string(),
                &args(&[
                    ("agent_ids", json!([record.agent_id])),
                    ("timeout_ms", json!(1000)),
                ]),
            )
            .await
            .unwrap(),
        );

        assert!(output.contains("# Agent Result: Finish soon"));
        assert!(output.contains("finished from wait"));
    }

    #[tokio::test]
    async fn agent_wait_any_returns_the_first_terminal_agent_and_reports_the_rest() {
        let (_temp, app, ccx) = test_context(PARENT).await;
        let slow = create_agent(&app, PARENT, BgAgentKind::Subagent, "Slow one").await;
        let fast = create_agent(&app, PARENT, BgAgentKind::Subagent, "Fast one").await;
        for record in [&slow, &fast] {
            app.agents
                .mark_running(&record.agent_id, format!("subchat-{}", record.agent_id))
                .await
                .unwrap();
        }
        let foreign = create_agent(&app, "other-parent", BgAgentKind::Subagent, "Foreign").await;
        let denied = execute_agents_wait(
            ccx.clone(),
            &"denied".to_string(),
            &args(&[
                (
                    "agent_ids",
                    json!([slow.agent_id.clone(), foreign.agent_id.clone()]),
                ),
                ("timeout_ms", json!(1000)),
            ]),
        )
        .await;
        assert!(
            denied.is_err(),
            "a foreign id must fail before any wait starts"
        );
        let fast_id = fast.agent_id.clone();
        let registry = app.agents.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(25)).await;
            registry
                .mark_completed(&fast_id, completion("fast finished"))
                .await
                .unwrap();
        });
        let output = output_text(
            execute_agents_wait(
                ccx.clone(),
                &"any".to_string(),
                &args(&[
                    (
                        "agent_ids",
                        json!([slow.agent_id.clone(), fast.agent_id.clone()]),
                    ),
                    ("timeout_ms", json!(2000)),
                ]),
            )
            .await
            .unwrap(),
        );
        assert!(output.contains(&format!("First terminal agent: {}", fast.agent_id)));
        assert!(output.contains("fast finished"));
        assert!(output.contains(&format!("- {}: running", slow.agent_id)));
        let timed_out = output_text(
            execute_agents_wait(
                ccx,
                &"timeout".to_string(),
                &args(&[
                    ("agent_ids", json!([slow.agent_id.clone()])),
                    ("timeout_ms", json!(1)),
                ]),
            )
            .await
            .unwrap(),
        );
        assert!(timed_out.contains("Timed out"));
        assert!(timed_out.contains("still running"));
    }

    #[test]
    fn agent_wait_targets_dedupe_without_an_agent_count_or_timeout_ceiling() {
        let max = 3;
        assert_eq!(
            parse_wait_targets(&args(&[("agent_ids", json!(["a", " a ", "b"])),]), max).unwrap(),
            vec!["a".to_string(), "b".to_string()],
        );
        assert!(parse_wait_targets(&args(&[("agent_ids", json!([]))]), max).is_err());
        assert!(parse_wait_targets(&args(&[]), max).is_err());
        assert!(parse_wait_targets(&args(&[("agent_ids", json!(["a", 7]))]), max).is_err());
        assert!(parse_wait_targets(&args(&[("agent_ids", json!("a"))]), max).is_err());
        assert_eq!(
            parse_wait_targets(&args(&[("agent_ids", json!(["a", "b", "c", "d"]))]), max)
                .unwrap()
                .len(),
            4
        );
        let many = (0..300)
            .map(|index| format!("agent-{index}"))
            .collect::<Vec<_>>();
        assert_eq!(
            parse_wait_targets(&args(&[("agent_ids", json!(many))]), max)
                .unwrap()
                .len(),
            300
        );

        let settings = refact_work_core::coordination_settings();
        assert_eq!(
            parse_wait_timeout_ms(
                &args(&[("timeout_ms", json!(settings.agent_wait_max_ms + 1))]),
                settings.agent_wait_default_ms,
                settings.agent_wait_max_ms,
            )
            .unwrap(),
            settings.agent_wait_max_ms + 1
        );
        assert!(parse_wait_timeout_ms(
            &args(&[("timeout_ms", json!(0))]),
            settings.agent_wait_default_ms,
            settings.agent_wait_max_ms,
        )
        .is_err());
        assert_eq!(
            parse_wait_timeout_ms(
                &args(&[]),
                settings.agent_wait_default_ms,
                settings.agent_wait_max_ms
            )
            .unwrap(),
            settings.agent_wait_default_ms
        );
    }

    #[test]
    fn agent_wait_is_a_recognized_runner_wait_so_no_permit_is_held() {
        let desc = crate::tools::tool_wait::ToolWait {
            config_path: String::new(),
        }
        .tool_description();
        assert_eq!(desc.name, "wait");
        assert_eq!(desc.input_schema["required"], json!(["target"]));
        assert!(desc.input_schema["properties"].get("description").is_none());
        assert_eq!(
            desc.input_schema["properties"]["target"]["oneOf"][0]["required"],
            json!(["kind", "duration_ms", "description"])
        );
        assert!(
            crate::chat::tools::is_recognized_runner_wait_name(&desc.name),
            "agent_wait must park the runner's execution permit instead of holding it"
        );
        assert!(matches!(
            desc.source.source_type,
            crate::tools::tools_description::ToolSourceType::Builtin
        ));
    }

    #[tokio::test]
    async fn agent_wait_any_rejects_every_unauthorized_id_before_disclosing_anything() {
        let (_temp, app, ccx) = test_context(PARENT).await;
        let mine = create_agent(&app, PARENT, BgAgentKind::Subagent, "Mine").await;
        let foreign = create_agent(&app, OTHER_PARENT, BgAgentKind::Subagent, "Foreign").await;
        app.agents
            .mark_completed(&foreign.agent_id, completion("foreign secret"))
            .await
            .unwrap();

        for ids in [
            json!([foreign.agent_id.clone(), mine.agent_id.clone()]),
            json!([mine.agent_id.clone(), foreign.agent_id.clone()]),
            json!([mine.agent_id.clone(), "bgagent-guessed".to_string()]),
        ] {
            let error = execute_agents_wait(
                ccx.clone(),
                &"denied".to_string(),
                &args(&[("agent_ids", ids.clone()), ("timeout_ms", json!(1000))]),
            )
            .await
            .unwrap_err();
            assert!(!error.contains("foreign secret"), "{error}");
            assert!(!error.contains("Foreign"), "{error}");
        }
    }

    #[tokio::test]
    async fn agent_wait_any_settles_once_on_argument_order_when_several_are_terminal() {
        let (_temp, app, ccx) = test_context(PARENT).await;
        let first = create_agent(&app, PARENT, BgAgentKind::Subagent, "First done").await;
        let second = create_agent(&app, PARENT, BgAgentKind::Subagent, "Second done").await;
        for (record, summary) in [(&first, "first result"), (&second, "second result")] {
            app.agents
                .mark_completed(&record.agent_id, completion(summary))
                .await
                .unwrap();
        }

        for _ in 0..5 {
            let output = output_text(
                execute_agents_wait(
                    ccx.clone(),
                    &"any".to_string(),
                    &args(&[
                        (
                            "agent_ids",
                            json!([first.agent_id.clone(), second.agent_id.clone()]),
                        ),
                        ("timeout_ms", json!(1000)),
                    ]),
                )
                .await
                .unwrap(),
            );
            assert!(output.contains(&format!("First terminal agent: {}", first.agent_id)));
            assert!(output.contains("first result"));
            assert!(!output.contains("Timed out"));
            assert_eq!(
                output.matches("First terminal agent:").count(),
                1,
                "a wait settles exactly once"
            );
            assert!(output.contains(&format!("- {}: completed", second.agent_id)));
        }
    }

    #[tokio::test]
    async fn agent_wait_any_timeout_lists_still_pending_ids_truthfully() {
        let (_temp, app, ccx) = test_context(PARENT).await;
        let running = create_agent(&app, PARENT, BgAgentKind::Subagent, "Running").await;
        let queued = create_agent(&app, PARENT, BgAgentKind::Subagent, "Queued").await;
        app.agents
            .mark_running(&running.agent_id, "subchat-running".to_string())
            .await
            .unwrap();

        let output = output_text(
            execute_agents_wait(
                ccx,
                &"timeout".to_string(),
                &args(&[
                    (
                        "agent_ids",
                        json!([running.agent_id.clone(), queued.agent_id.clone()]),
                    ),
                    ("timeout_ms", json!(20)),
                ]),
            )
            .await
            .unwrap(),
        );

        assert!(output.contains("⏰ Timed out after 20ms — none of 2 agents finished."));
        assert!(!output.contains("First terminal agent:"));
        assert!(output.contains(&format!("- {}: running", running.agent_id)));
        assert!(output.contains(&format!("- {}: queued", queued.agent_id)));
    }

    #[tokio::test]
    async fn agent_wait_any_cancellation_racing_a_timeout_settles_exactly_once() {
        for delay_ms in [0, 10, 20, 30] {
            let (_temp, app, ccx) = test_context(PARENT).await;
            let racer = create_agent(&app, PARENT, BgAgentKind::Subagent, "Racer").await;
            let other = create_agent(&app, PARENT, BgAgentKind::Subagent, "Other").await;
            for record in [&racer, &other] {
                app.agents
                    .mark_running(&record.agent_id, format!("subchat-{}", record.agent_id))
                    .await
                    .unwrap();
            }
            let racer_id = racer.agent_id.clone();
            let registry = app.agents.clone();
            let canceller = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                registry
                    .cancel(PARENT, &racer_id, Some("raced".to_string()))
                    .await
                    .unwrap();
            });

            let output = output_text(
                execute_agents_wait(
                    ccx,
                    &"race".to_string(),
                    &args(&[
                        (
                            "agent_ids",
                            json!([racer.agent_id.clone(), other.agent_id.clone()]),
                        ),
                        ("timeout_ms", json!(20)),
                    ]),
                )
                .await
                .unwrap(),
            );
            canceller.await.unwrap();

            let settled = output.contains("First terminal agent:");
            let timed_out = output.contains("⏰ Timed out after 20ms");
            assert!(settled ^ timed_out, "delay {delay_ms}: {output}");
            if settled {
                assert!(output.contains(&format!("First terminal agent: {}", racer.agent_id)));
                assert!(output.contains(&format!("- {}: running", other.agent_id)));
            } else {
                assert!(output.contains(&format!("- {}: ", racer.agent_id)));
                assert!(output.contains(&format!("- {}: running", other.agent_id)));
            }
        }
    }

    #[tokio::test]
    async fn agent_cancel_on_terminal_returns_no_action() {
        let (_temp, app, ccx) = test_context(PARENT).await;
        let record = create_agent(&app, PARENT, BgAgentKind::Delegate, "Already done").await;
        let completed = app
            .agents
            .mark_completed(&record.agent_id, completion("done"))
            .await
            .unwrap();

        let output = output_text(
            ToolAgentCancel {
                config_path: String::new(),
            }
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[("agent_id", json!(completed.agent_id.clone()))]),
            )
            .await
            .unwrap(),
        );
        let after = app.agents.get(PARENT, &completed.agent_id).await.unwrap();

        assert!(output.contains("Agent already in terminal state: ✅ completed. No action."));
        assert_eq!(after.status, BgAgentStatus::Completed);
    }

    #[tokio::test]
    async fn agent_cancel_on_running_flips_abort_flag() {
        let (_temp, app, ccx) = test_context(PARENT).await;
        let session_arc = Arc::new(AMutex::new(ChatSession::new(PARENT.to_string())));
        let mut rx = session_arc.lock().await.subscribe();
        app.chat
            .sessions
            .write()
            .await
            .insert(PARENT.to_string(), session_arc);
        let (record, abort_flag, _) = app
            .agents
            .create(create_request(PARENT, BgAgentKind::Delegate, "Abort me"))
            .await
            .unwrap();
        app.agents
            .mark_running(&record.agent_id, "subchat-abort".to_string())
            .await
            .unwrap();

        let output = output_text(
            ToolAgentCancel {
                config_path: String::new(),
            }
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[("agent_id", json!(record.agent_id.clone()))]),
            )
            .await
            .unwrap(),
        );
        let after = app.agents.get(PARENT, &record.agent_id).await.unwrap();

        assert!(abort_flag.load(Ordering::SeqCst));
        assert_eq!(after.status, BgAgentStatus::Cancelled);
        assert!(output.contains("Cancel requested"));
        let json = rx.try_recv().expect("background agent update");
        let envelope: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(envelope["chat_id"], PARENT);
        assert_eq!(envelope["seq"], 1);
        assert_eq!(envelope["type"], "background_agent_updated");
        assert_eq!(envelope["agent"]["agentId"], record.agent_id);
        assert_eq!(
            envelope["agent"]["status"],
            BgAgentStatus::Cancelled.as_str()
        );
    }

    #[tokio::test]
    async fn agent_cancel_subtree_cancels_descendants() {
        let (_temp, app, ccx) = test_context(PARENT).await;
        let (parent, parent_abort, _) = app
            .agents
            .create(create_request(PARENT, BgAgentKind::Subagent, "Parent"))
            .await
            .unwrap();
        app.agents
            .mark_running(&parent.agent_id, "child-chat".to_string())
            .await
            .unwrap();
        let parent_run = app.agents.parent_run("child-chat").await.unwrap().unwrap();
        assert!(!parent.agent_id.is_empty());
        assert_eq!(parent.run_generation, 1);
        let completed = app
            .agents
            .mark_completed(
                &parent.agent_id,
                AgentCompletion {
                    child_chat_id: Some("child-chat".into()),
                    ..completion("parent finished while launch was preparing")
                },
            )
            .await
            .unwrap();
        assert_eq!(completed.status, BgAgentStatus::Completed);
        let (child, child_abort, _) = app
            .agents
            .create_for_parent_run(
                create_request("child-chat", BgAgentKind::Subagent, "Child"),
                Some(&parent_run),
            )
            .await
            .unwrap();
        app.agents
            .mark_running(&child.agent_id, "grandchild-chat".to_string())
            .await
            .unwrap();

        let output = output_text(
            ToolAgentCancel {
                config_path: String::new(),
            }
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[("agent_id", json!(parent.agent_id.clone()))]),
            )
            .await
            .unwrap(),
        );

        assert!(!parent_abort.load(Ordering::SeqCst));
        assert!(child_abort.load(Ordering::SeqCst));
        assert_eq!(
            app.agents.get_any(&parent.agent_id).await.unwrap().status,
            BgAgentStatus::Completed
        );
        assert_eq!(
            app.agents.get_any(&child.agent_id).await.unwrap().status,
            BgAgentStatus::Cancelled
        );
        assert!(!output.contains(&parent.agent_id));
        assert!(output.contains(&child.agent_id));
        assert!(app
            .agents
            .create_for_parent_run(
                create_request("child-chat", BgAgentKind::Subagent, "Late"),
                Some(&parent_run)
            )
            .await
            .is_err());
        assert_eq!(
            app.agents.get_any(&parent.agent_id).await.unwrap().status,
            BgAgentStatus::Completed
        );
    }

    #[test]
    fn agent_lifecycle_parser_requires_exact_actions_and_positive_integers() {
        let inspect = args(&[
            ("agent_id", json!("bgagent-test")),
            ("action", json!("inspect")),
        ]);
        assert_eq!(
            parse_agent_lifecycle_args(&inspect).unwrap().1,
            AgentLifecycleAction::Inspect
        );
        let valid = args(&[
            ("agent_id", json!("bgagent-test")),
            ("action", json!("raise_and_recover")),
            ("max_steps", json!(80)),
            ("expected_change_seq", json!(5)),
        ]);
        assert_eq!(
            parse_agent_lifecycle_args(&valid).unwrap().1,
            AgentLifecycleAction::RaiseAndResume {
                max_steps: 80,
                expected_change_seq: 5
            }
        );
        for key in ["max_steps", "expected_change_seq"] {
            for value in [
                json!(0),
                json!(-1),
                json!(1.5),
                json!(80.0),
                json!("80"),
                Value::Null,
                json!(true),
            ] {
                let mut invalid = valid.clone();
                invalid.insert(key.to_string(), value);
                assert!(
                    parse_agent_lifecycle_args(&invalid).is_err(),
                    "accepted {key}: {invalid:?}"
                );
            }
            let mut missing = valid.clone();
            missing.remove(key);
            assert!(parse_agent_lifecycle_args(&missing).is_err());
            let mut mutation_on_inspect = inspect.clone();
            mutation_on_inspect.insert(key.to_string(), json!(1));
            assert!(parse_agent_lifecycle_args(&mutation_on_inspect).is_err());
        }
        for key in ["action", "agent_id"] {
            let mut missing = valid.clone();
            missing.remove(key);
            assert!(parse_agent_lifecycle_args(&missing).is_err());
            for value in [json!(""), Value::Null, json!(42)] {
                let mut invalid = valid.clone();
                invalid.insert(key.to_string(), value);
                assert!(parse_agent_lifecycle_args(&invalid).is_err());
            }
        }
        let mut unknown = valid;
        unknown.insert("scope".to_string(), json!("all"));
        assert!(parse_agent_lifecycle_args(&unknown).is_err());
    }

    #[tokio::test]
    async fn agent_lifecycle_inspection_is_read_only_and_direct_parent_scoped() {
        let (_temp, app, ccx) = test_context(PARENT).await;
        let own = create_agent(&app, PARENT, BgAgentKind::Subagent, "Capped").await;
        let cap = crate::agents::types::CapStop {
            key: refact_work_core::CapKind::Steps,
            scope: refact_work_core::CapScope::Child,
            limit: 10,
            used: 10,
            resumable: true,
            run_generation: 0,
        };
        let failed = app
            .agents
            .mark_failed(&own.agent_id, cap.as_error().unwrap())
            .await
            .unwrap();
        assert_eq!(
            crate::agents::types::CapStop::from_error(failed.error.as_deref().unwrap()),
            Some(cap.clone())
        );
        let output = output_text(
            agent_lifecycle_execute(
                ccx.clone(),
                &"inspect".to_string(),
                &args(&[
                    ("agent_id", json!(own.agent_id)),
                    ("action", json!("inspect")),
                ]),
            )
            .await
            .unwrap(),
        );
        let payload: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(payload["cap_stop"], serde_json::to_value(&cap).unwrap());
        assert_eq!(
            serde_json::from_value::<crate::agents::types::CapStop>(payload["cap_stop"].clone())
                .unwrap(),
            cap
        );
        assert_eq!(payload["action"], "inspect");
        assert_eq!(payload["agent_id"], own.agent_id);
        assert_eq!(payload["step_count"], failed.step_count);
        assert_eq!(payload["change_seq"], failed.change_seq);
        assert_eq!(payload["status"], "failed");
        assert_eq!(payload["can_resume"], false);
        assert_eq!(payload["resume_configuration_available"], false);
        assert!(payload["settings_subagent_max_steps_ceiling"].is_null());
        assert_eq!(
            app.agents.get_any(&own.agent_id).await.unwrap().change_seq,
            failed.change_seq
        );
        assert_eq!(
            app.agents
                .get_any(&own.agent_id)
                .await
                .unwrap()
                .error
                .as_deref(),
            failed.error.as_deref()
        );
        let foreign = create_agent(&app, OTHER_PARENT, BgAgentKind::Subagent, "Hidden").await;
        for action in ["inspect", "raise_and_recover"] {
            let mut request = args(&[
                ("agent_id", json!(foreign.agent_id)),
                ("action", json!(action)),
            ]);
            if action == "raise_and_recover" {
                request.insert("max_steps".to_string(), json!(80));
                request.insert("expected_change_seq".to_string(), json!(1));
            }
            let error = agent_lifecycle_execute(ccx.clone(), &"foreign".to_string(), &request)
                .await
                .unwrap_err();
            assert_eq!(error, not_found_error(&foreign.agent_id));
        }
    }

    #[tokio::test]
    async fn agent_lifecycle_raise_requires_confirmation_by_default() {
        use crate::tools::tools_description::MatchConfirmDenyResult;
        let (_temp, _app, ccx) = test_context(PARENT).await;
        let tool = ToolAgentLifecycle {
            config_path: String::new(),
        };
        let inspect = args(&[
            ("agent_id", json!("bgagent-test")),
            ("action", json!("inspect")),
        ]);
        let result = tool
            .match_against_confirm_deny(ccx.clone(), &inspect)
            .await
            .unwrap();
        assert!(matches!(result.result, MatchConfirmDenyResult::PASS));
        let mutation = args(&[
            ("agent_id", json!("bgagent-test")),
            ("action", json!("raise_and_recover")),
            ("max_steps", json!(80)),
            ("expected_change_seq", json!(5)),
        ]);
        let result = tool
            .match_against_confirm_deny(ccx, &mutation)
            .await
            .unwrap();
        assert!(matches!(
            result.result,
            MatchConfirmDenyResult::CONFIRMATION
        ));
    }

    #[test]
    fn a_queued_continuation_is_never_reported_as_a_completion() {
        assert_eq!(
            LifecycleOutcome::Queued.state(BgAgentStatus::Completed),
            "queued",
            "asking a completed child for another turn returns queued, not completed"
        );
        assert!(LifecycleOutcome::Queued.accepted());
        let summary = LifecycleOutcome::Queued.summary();
        assert!(summary.contains("NOT a completion"), "{summary}");
        assert!(!summary.contains("completed"), "{summary}");

        for status in [
            BgAgentStatus::Completed,
            BgAgentStatus::Failed,
            BgAgentStatus::Cancelled,
            BgAgentStatus::Interrupted,
            BgAgentStatus::Closed,
            BgAgentStatus::Running,
        ] {
            assert_eq!(LifecycleOutcome::Queued.state(status), "queued");
            assert_eq!(LifecycleOutcome::Closed.state(status), "closed");
            assert_eq!(LifecycleOutcome::AlreadyClosed.state(status), "closed");
            assert_eq!(LifecycleOutcome::Inspected.state(status), status.as_str());
        }

        assert!(LifecycleOutcome::Closed.accepted());
        assert!(!LifecycleOutcome::AlreadyClosed.accepted());
        assert!(!LifecycleOutcome::Inspected.accepted());
        assert!(LifecycleOutcome::AlreadyClosed
            .summary()
            .contains("Nothing changed"));
    }

    #[test]
    fn agent_lifecycle_parser_scopes_close_and_continue_fields() {
        let base = |action: &str| {
            args(&[
                ("agent_id", json!("bgagent-test")),
                ("action", json!(action)),
            ])
        };
        assert_eq!(
            parse_agent_lifecycle_args(&base("close")).unwrap().1,
            AgentLifecycleAction::Close {
                expected_run_generation: None,
                reason: None
            }
        );
        let mut close = base("close");
        close.insert("reason".into(), json!("parked"));
        close.insert("expected_run_generation".into(), json!(3));
        assert_eq!(
            parse_agent_lifecycle_args(&close).unwrap().1,
            AgentLifecycleAction::Close {
                expected_run_generation: Some(3),
                reason: Some("parked".into())
            }
        );
        for (key, value) in [
            ("expected_run_generation", json!(0)),
            ("expected_run_generation", json!(-1)),
            ("expected_run_generation", json!("3")),
            ("reason", json!("")),
            ("reason", json!(7)),
            ("expected_change_seq", json!(2)),
            ("max_steps", json!(2)),
            ("instruction", json!("go")),
        ] {
            let mut invalid = base("close");
            invalid.insert(key.to_string(), value.clone());
            assert!(
                parse_agent_lifecycle_args(&invalid).is_err(),
                "close accepted {key}={value}"
            );
        }

        let mut continued = base("continue");
        continued.insert("expected_change_seq".into(), json!(9));
        continued.insert("instruction".into(), json!("keep going"));
        continued.insert("reconcile_incomplete".into(), json!(true));
        assert_eq!(
            parse_agent_lifecycle_args(&continued).unwrap().1,
            AgentLifecycleAction::ContinueTurn {
                expected_change_seq: 9,
                instruction: Some("keep going".into()),
                reconcile_incomplete: true,
            }
        );
        let mut without_revision = base("continue");
        without_revision.insert("instruction".into(), json!("keep going"));
        assert!(parse_agent_lifecycle_args(&without_revision).is_err());
        let mut with_steps = continued.clone();
        with_steps.insert("max_steps".into(), json!(40));
        assert!(parse_agent_lifecycle_args(&with_steps).is_err());
        let mut reason_on_inspect = base("inspect");
        reason_on_inspect.insert("reason".into(), json!("nope"));
        assert!(parse_agent_lifecycle_args(&reason_on_inspect).is_err());

        let schema = ToolAgentLifecycle {
            config_path: String::new(),
        }
        .tool_description()
        .input_schema;
        assert_eq!(
            schema["properties"]["action"]["enum"],
            json!([
                "inspect",
                "close",
                "raise_and_recover",
                "recover",
                "continue"
            ])
        );
    }

    #[tokio::test]
    async fn agent_lifecycle_closes_a_live_child_and_settles_repeat_closes_once() {
        let (_temp, app, ccx) = test_context(PARENT).await;
        let record = create_agent(&app, PARENT, BgAgentKind::Subagent, "Closable").await;
        app.agents
            .mark_running(&record.agent_id, "subchat-closable".to_string())
            .await
            .unwrap();
        let close = args(&[
            ("agent_id", json!(record.agent_id)),
            ("action", json!("close")),
            ("reason", json!("parent stepped away")),
        ]);

        let first: Value = serde_json::from_str(&output_text(
            agent_lifecycle_execute(ccx.clone(), &"close".to_string(), &close)
                .await
                .unwrap(),
        ))
        .unwrap();
        assert_eq!(first["status"], "closed");
        assert_eq!(first["accepted"], true);
        assert_eq!(first["close_reason"], "parent stepped away");
        assert_eq!(first["can_close"], false);
        assert_eq!(first["can_continue"], false);

        let second: Value = serde_json::from_str(&output_text(
            agent_lifecycle_execute(ccx.clone(), &"close-again".to_string(), &close)
                .await
                .unwrap(),
        ))
        .unwrap();
        assert_eq!(second["status"], "closed");
        assert_eq!(second["accepted"], false);
        assert_eq!(second["change_seq"], first["change_seq"]);

        let stale = agent_lifecycle_execute(
            ccx.clone(),
            &"stale".to_string(),
            &args(&[
                ("agent_id", json!(record.agent_id)),
                ("action", json!("close")),
                ("expected_run_generation", json!(9)),
            ]),
        )
        .await
        .unwrap_err();
        assert!(stale.contains("another generation"), "{stale}");

        let wrong_action = agent_lifecycle_execute(
            ccx.clone(),
            &"continue".to_string(),
            &args(&[
                ("agent_id", json!(record.agent_id)),
                ("action", json!("continue")),
                ("expected_change_seq", json!(first["change_seq"].clone())),
            ]),
        )
        .await
        .unwrap_err();
        assert!(wrong_action.contains("continue applies to a completed child"));

        let inspected: Value = serde_json::from_str(&output_text(
            agent_lifecycle_execute(
                ccx.clone(),
                &"inspect".to_string(),
                &args(&[
                    ("agent_id", json!(record.agent_id)),
                    ("action", json!("inspect")),
                ]),
            )
            .await
            .unwrap(),
        ))
        .unwrap();
        assert_eq!(inspected["status"], "closed");
        assert_eq!(inspected["accepted"], false);
        assert_eq!(
            inspected["resume_configuration_available"], false,
            "a closed child without saved configuration stays deny-all"
        );
        assert!(inspected["tool_authority"].is_null());
        assert_eq!(inspected["can_resume"], false);

        let listed = output_text(
            ToolAgentList {
                config_path: String::new(),
            }
            .tool_execute(
                ccx,
                &"list".to_string(),
                &args(&[("status", json!("closed"))]),
            )
            .await
            .unwrap(),
        );
        assert!(listed.contains("🔒 closed"));
        assert!(listed.contains("closed: parent stepped away"));
        assert!(listed.contains("Total: 1"));
    }

    #[tokio::test]
    async fn agent_lifecycle_refuses_plain_resume_on_a_completed_child() {
        let (_temp, app, ccx) = test_context(PARENT).await;
        let record = create_agent(&app, PARENT, BgAgentKind::Subagent, "Finished").await;
        let completed = app
            .agents
            .mark_completed(&record.agent_id, completion("the first answer"))
            .await
            .unwrap();

        let error = agent_lifecycle_execute(
            ccx.clone(),
            &"recover".to_string(),
            &args(&[
                ("agent_id", json!(record.agent_id)),
                ("action", json!("recover")),
                ("expected_change_seq", json!(completed.change_seq)),
            ]),
        )
        .await
        .unwrap_err();
        assert!(error.contains("use action continue"), "{error}");
        assert_eq!(
            app.agents.get_any(&record.agent_id).await.unwrap().status,
            BgAgentStatus::Completed
        );

        let closing = agent_lifecycle_execute(
            ccx,
            &"close".to_string(),
            &args(&[
                ("agent_id", json!(record.agent_id)),
                ("action", json!("close")),
            ]),
        )
        .await
        .unwrap_err();
        assert!(
            closing.contains("already finished as completed"),
            "{closing}"
        );
    }
}
