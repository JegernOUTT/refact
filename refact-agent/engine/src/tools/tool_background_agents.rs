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

pub struct ToolAgentWait {
    pub config_path: String,
}

pub struct ToolAgentResult {
    pub config_path: String,
}

pub struct ToolAgentCancel {
    pub config_path: String,
}

pub struct ToolAgentResume {
    pub config_path: String,
}

#[derive(Debug, PartialEq, Eq)]
enum AgentResumeAction {
    Inspect,
    Resume {
        expected_change_seq: u64,
        instruction: Option<String>,
        reconcile_incomplete: bool,
    },
    RaiseAndResume {
        max_steps: usize,
        expected_change_seq: u64,
    },
}

fn agent_resume_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "agent_id": {"type": "string", "minLength": 1},
            "action": {"type": "string", "enum": ["inspect", "raise_and_resume", "resume"]},
            "instruction": {"type": "string", "minLength": 1, "description": "Optional exact parent follow-up for resume; appended after preserved child history."},
            "reconcile_incomplete": {"type": "boolean", "default": false, "description": "For resume only: explicitly acknowledge missing tool outcomes as unknown without replaying calls. Inspect side effects before enabling."},
            "max_steps": {
                "type": "integer", "minimum": 1,
                "description": "New per-child total step cap, required for raise_and_resume. Must increase the stopped cap without exceeding the Settings ceiling."
            },
            "expected_change_seq": {
                "type": "integer", "minimum": 1,
                "description": "Exact change_seq returned by inspect, required for any resume."
            }
        },
        "required": ["agent_id", "action"],
        "oneOf": [
            {"properties": {"action": {"const": "inspect"}}, "not": {"anyOf": [{"required": ["max_steps"]}, {"required": ["expected_change_seq"]}, {"required": ["instruction"]}, {"required": ["reconcile_incomplete"]}]}},
            {"properties": {"action": {"const": "raise_and_resume"}}, "required": ["max_steps", "expected_change_seq"], "not": {"anyOf": [{"required": ["instruction"]}, {"required": ["reconcile_incomplete"]}]}},
            {"properties": {"action": {"const": "resume"}}, "required": ["expected_change_seq"], "not": {"required": ["max_steps"]}}
        ]
    })
}

fn parse_agent_resume_args(
    args: &HashMap<String, Value>,
) -> Result<(String, AgentResumeAction), String> {
    for key in args.keys() {
        if !matches!(
            key.as_str(),
            "agent_id"
                | "action"
                | "max_steps"
                | "expected_change_seq"
                | "instruction"
                | "reconcile_incomplete"
        ) {
            return Err(format!("Unknown argument '{key}'"));
        }
    }
    let agent_id = parse_required_string(args, "agent_id")?;
    let action = match args.get("action").and_then(Value::as_str) {
        Some("inspect") => {
            if [
                "max_steps",
                "expected_change_seq",
                "instruction",
                "reconcile_incomplete",
            ]
            .iter()
            .any(|key| args.contains_key(*key))
            {
                return Err("inspect does not accept mutation fields".to_string());
            }
            AgentResumeAction::Inspect
        }
        Some("raise_and_resume") => {
            if args.contains_key("instruction") || args.contains_key("reconcile_incomplete") {
                return Err(
                    "raise_and_resume does not accept instruction or reconcile_incomplete".into(),
                );
            }
            let positive_integer = |key: &str| {
                args.get(key)
                    .and_then(Value::as_u64)
                    .filter(|value| *value > 0)
                    .ok_or_else(|| format!("{key} must be a positive integer"))
            };
            AgentResumeAction::RaiseAndResume {
                max_steps: usize::try_from(positive_integer("max_steps")?)
                    .map_err(|_| "max_steps is too large".to_string())?,
                expected_change_seq: positive_integer("expected_change_seq")?,
            }
        }
        Some("resume") => {
            if args.contains_key("max_steps") {
                return Err(
                    "resume does not change limits; use raise_and_resume for a stopped cap".into(),
                );
            }
            let expected_change_seq = args
                .get("expected_change_seq")
                .and_then(Value::as_u64)
                .filter(|value| *value > 0)
                .ok_or("expected_change_seq must be a positive integer")?;
            let instruction = args
                .get("instruction")
                .map(|value| {
                    value
                        .as_str()
                        .filter(|text| !text.trim().is_empty())
                        .map(str::to_string)
                        .ok_or_else(|| "instruction must be a non-empty string".to_string())
                })
                .transpose()?;
            let reconcile_incomplete = args
                .get("reconcile_incomplete")
                .map(|value| {
                    value
                        .as_bool()
                        .ok_or("reconcile_incomplete must be a boolean")
                })
                .transpose()?
                .unwrap_or(false);
            AgentResumeAction::Resume {
                expected_change_seq,
                instruction,
                reconcile_incomplete,
            }
        }
        _ => return Err("action must be inspect, resume or raise_and_resume".to_string()),
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
                "enum": ["queued", "running", "waiting_for_approval", "completed", "failed", "cancelled", "interrupted", "all"],
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

fn agent_wait_schema() -> Value {
    let settings = refact_work_core::coordination_settings();
    json!({
        "type": "object",
        "properties": {
            "agent_id": {
                "type": "string",
                "description": "Background agent ID, for example bgagent-... Use agent_ids to wait for the first of several."
            },
            "agent_ids": {
                "type": "array",
                "items": {"type": "string"},
                "minItems": 1,
                "maxItems": settings.agent_wait_max_agents,
                "description": "Wait for whichever of these agents (spawned by this chat) reaches a terminal state first."
            },
            "timeout_ms": {
                "type": "integer",
                "minimum": 1,
                "maximum": settings.agent_wait_max_ms,
                "description": format!("How long to wait for completion. Default: {}", settings.agent_wait_default_ms)
            }
        }
    })
}

fn parse_wait_targets(
    args: &HashMap<String, Value>,
    max_agents: usize,
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
    if let Some(single) = args.get("agent_id").and_then(Value::as_str) {
        if !single.trim().is_empty() {
            ids.push(single.trim().to_string());
        }
    }
    ids.dedup();
    let mut seen = std::collections::HashSet::new();
    ids.retain(|id| seen.insert(id.clone()));
    if ids.is_empty() {
        return Err("provide `agent_id` or a nonempty `agent_ids` list".to_string());
    }
    if ids.len() > max_agents {
        return Err(format!(
            "`agent_ids` supports at most {max_agents} agents per wait"
        ));
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
    max_ms: u64,
) -> Result<u64, String> {
    let timeout_ms = parse_optional_u64(args, "timeout_ms", default_ms)?;
    if timeout_ms < 1 {
        return Err("timeout_ms must be at least 1ms".to_string());
    }
    if timeout_ms > max_ms {
        return Err(format!("timeout_ms must be at most {max_ms}ms"));
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

fn format_status_chip(status: BgAgentStatus) -> &'static str {
    match status {
        BgAgentStatus::Queued => "⏳ queued",
        BgAgentStatus::Running => "🟢 running",
        BgAgentStatus::WaitingForApproval => "✋ waiting_for_approval",
        BgAgentStatus::Completed => "✅ completed",
        BgAgentStatus::Failed => "❌ failed",
        BgAgentStatus::Cancelled => "⏹ cancelled",
        BgAgentStatus::Interrupted => "⚠ interrupted",
    }
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
            format_status_chip(row.status),
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
        "\nUse `agent_status(agent_id=...)`, `agent_wait(...)`, or `agent_result(...)` to follow up.",
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
    ];
    let counts = statuses
        .into_iter()
        .filter_map(|status| {
            let count = rows.iter().filter(|row| row.status == status).count();
            (count > 0).then(|| format!("{}: {count}", status.as_str()))
        })
        .collect::<Vec<_>>()
        .join(", ");
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
    let mut result = format!("# {}: {}\n", kind_title(record.kind), record.title);
    result.push_str(&format!(
        "- Status: {}\n",
        format_status_chip(record.status)
    ));
    result.push_str(&format!("- Agent ID: {}\n", record.agent_id));
    result.push_str(&format!("- Kind: {}\n", kind_label(record.kind)));
    let started = record
        .started_at
        .map(|started_at| human_age_ago(started_at, now))
        .unwrap_or_else(|| "not started".to_string());
    result.push_str(&format!("- Started: {started}\n"));
    result.push_str(&format!("- Step count: {}\n", record.step_count));
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
            "Agent has not finished yet. Current status: {}. Use agent_wait to block.",
            format_status_chip(record.status)
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
        .unwrap_or_else(|| format!("Agent finished with status {}.", record.status.as_str()));
    let mut result = format!("# Agent Result: {}\n", record.title);
    result.push_str(&format!(
        "- Status: {}\n",
        format_status_chip(record.status)
    ));
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

#[async_trait]
impl Tool for ToolAgentResume {
    fn tool_description(&self) -> ToolDesc {
        let mut description = background_agent_tool_desc(
            self.config_path.clone(),
            "agent_resume",
            "Agent Resume",
            "Inspect or explicitly resume a stopped direct child on its saved history, plan and goal. Resume may append a parent instruction; incomplete tool outcomes require explicit reconciliation and are never replayed. raise_and_resume increases a stopped per-child step cap. Mutations require expected_change_seq from inspect. Cannot change global Settings, goal budgets, policy or provider quotas. Saved runtime configuration must still be available.",
            agent_resume_schema(),
        );
        description.allow_parallel = false;
        description
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (agent_id, action) = parse_agent_resume_args(args)?;
        let (app, chat_id) = background_agent_context(&ccx).await;
        let mut record = app
            .agents
            .get_any(&agent_id)
            .await
            .map_err(|error| map_registry_error(&agent_id, error))?;
        if record.parent_chat_id != chat_id {
            return Err(not_found_error(&agent_id));
        }
        let resumed = match action {
            AgentResumeAction::Inspect => false,
            AgentResumeAction::Resume {
                expected_change_seq,
                instruction,
                reconcile_incomplete,
            } => {
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
                true
            }
            AgentResumeAction::RaiseAndResume {
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
                true
            }
        };
        let recovered = crate::agents::spawn::recover_run_config(&app, &chat_id, &agent_id)
            .await
            .ok();
        let max_steps = recovered.as_ref().map(|run| run.config.max_steps);
        let tool_authority = recovered.as_ref().map(|run| {
            json!({
                "tools": run.config.tools,
                "migrated_from_legacy_all": run.authority_migrated,
            })
        });
        let cap_stop = if record.status == BgAgentStatus::Failed {
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
        let summary = if resumed {
            "The same child trajectory is queued to resume."
        } else {
            "Child recovery inspected; no limits or execution state changed."
        };
        Ok(tool_message(tool_call_id, json!({
            "summary": summary,
            "action": args.get("action"),
            "agent_id": record.agent_id,
            "child_chat_id": record.child_chat_id,
            "status": record.status.as_str(),
            "change_seq": record.change_seq,
            "run_generation": record.run_generation,
            "step_count": record.step_count,
            "cap_stop": cap_stop,
            "can_resume": scope_available && record.status.is_terminal() && max_steps.is_some_and(|limit| limit > record.step_count as usize) && cap_stop.is_none(),
            "can_raise_and_resume": scope_available && cap_stop.as_ref().is_some_and(|stop| stop.parent_can_raise_steps()),
            "max_steps": max_steps,
            "settings_subagent_max_steps_ceiling": settings_ceiling,
            "resume_configuration_available": max_steps.is_some(),
            "tool_authority": tool_authority,
            "original_scope_available": scope_available,
        }).to_string()))
    }

    async fn match_against_confirm_deny(
        &self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
    ) -> Result<crate::tools::tools_description::MatchConfirmDeny, String> {
        use crate::tools::tools_description::{MatchConfirmDeny, MatchConfirmDenyResult};
        let (agent_id, action) = parse_agent_resume_args(args)?;
        Ok(MatchConfirmDeny {
            result: match action {
                AgentResumeAction::Inspect => MatchConfirmDenyResult::PASS,
                AgentResumeAction::RaiseAndResume { .. } | AgentResumeAction::Resume { .. } => {
                    MatchConfirmDenyResult::CONFIRMATION
                }
            },
            command: format!("agent_resume {agent_id} {action:?}"),
            rule: "default".to_string(),
        })
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
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
        Ok(tool_message(tool_call_id, format_agent_table(&records)))
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

#[async_trait]
impl Tool for ToolAgentWait {
    fn tool_description(&self) -> ToolDesc {
        background_agent_tool_desc(
            self.config_path.clone(),
            "agent_wait",
            "Agent Wait",
            "Wait for one background agent spawned by this chat (agent_id), or for the first of several (agent_ids), to reach a terminal state; on timeout return current statuses instead. Waiting parks this runner's execution slot.",
            agent_wait_schema(),
        )
    }

    async fn tool_execute(
        &mut self,
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
        let (app, parent_chat_id) = background_agent_context(&ccx).await;
        if targets.len() == 1 {
            let agent_id = &targets[0];
            let record = app
                .agents
                .wait(&parent_chat_id, agent_id, Duration::from_millis(timeout_ms))
                .await
                .map_err(|error| map_registry_error(agent_id, error))?;
            let content = if record.status.is_terminal() {
                format_agent_result(&record, false).await?
            } else {
                format!(
                    "⏰ Timed out after {timeout_ms}ms — agent still running.\n\n{}",
                    format_agent_status_block(&record)
                )
            };
            return Ok(tool_message(tool_call_id, content));
        }
        // Every id is authorized before any wait starts, so a guessed id can
        // neither leak a peer's status nor keep the caller waiting.
        for agent_id in &targets {
            app.agents
                .get(&parent_chat_id, agent_id)
                .await
                .map_err(|error| map_registry_error(agent_id, error))?;
        }
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
        let mut pending: Vec<_> = waits.collect();
        let mut first_terminal = None;
        while !pending.is_empty() {
            let ((agent_id, result), _, rest) = futures::future::select_all(pending).await;
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
        Ok(tool_message(tool_call_id, lines.join("\n")))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
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
        Ok(tool_message(tool_call_id, content))
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
        let subtree_has_running_descendant = subtree
            && app
                .agents
                .list_descendants(&agent_id)
                .await
                .iter()
                .any(|record| !record.status.is_terminal());
        if current.status.is_terminal() && !subtree_has_running_descendant {
            return Ok(tool_message(
                tool_call_id,
                format!(
                    "Agent already in terminal state: {}. No action.",
                    format_status_chip(current.status)
                ),
            ));
        }
        let updated = app
            .agents
            .cancel_subtree(&current.parent_chat_id, &agent_id, subtree, reason)
            .await
            .map_err(|error| map_registry_error(&agent_id, error))?;
        for record in &updated {
            crate::agents::spawn::emit_background_agent_update(app.clone(), record).await;
        }
        let cancelled = updated
            .iter()
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
                ChatContent::SimpleText(text) => text,
                _ => panic!("expected text output"),
            },
            _ => panic!("expected chat message"),
        }
    }

    #[tokio::test]
    async fn agent_list_happy_path_renders_table() {
        let (_temp, app, ccx) = test_context(PARENT).await;
        let running = create_agent(&app, PARENT, BgAgentKind::Delegate, "Fix retry").await;
        app.agents
            .mark_running(&running.agent_id, "subchat-running".to_string())
            .await
            .unwrap();
        let updated = app
            .agents
            .update_progress(
                &running.agent_id,
                "searching call sites in src/".to_string(),
                5,
            )
            .await
            .unwrap();
        let completed =
            create_agent(&app, PARENT, BgAgentKind::Subagent, "List callers of X").await;
        app.agents
            .mark_completed(&completed.agent_id, completion("all callers listed"))
            .await
            .unwrap();

        let output = output_text(
            ToolAgentList {
                config_path: String::new(),
            }
            .tool_execute(ccx, &"call".to_string(), &HashMap::new())
            .await
            .unwrap(),
        );

        assert!(output.contains("# Background Agents"));
        assert!(
            output.contains("| ID | Kind | Status | Title | Tool | Tokens | Age | Last activity |")
        );
        assert!(output.contains(&running.agent_id));
        assert!(output.contains("delegate"));
        assert!(output.contains("🟢 running"));
        assert!(output.contains("Total: 2"));
        assert!(output.contains(updated.last_activity.as_deref().unwrap()));
    }

    #[tokio::test]
    async fn agent_status_happy_path_renders_status_block() {
        let (_temp, app, ccx) = test_context(PARENT).await;
        let record = create_agent(&app, PARENT, BgAgentKind::Subagent, "List callers of X").await;
        app.agents
            .mark_running(&record.agent_id, "subchat-status".to_string())
            .await
            .unwrap();
        let updated = app
            .agents
            .update_progress(
                &record.agent_id,
                "searching call sites in src/".to_string(),
                5,
            )
            .await
            .unwrap();

        let output = output_text(
            ToolAgentStatus {
                config_path: String::new(),
            }
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[("agent_id", json!(updated.agent_id))]),
            )
            .await
            .unwrap(),
        );

        assert!(output.contains("# Subagent: List callers of X"));
        assert!(output.contains("- Status: 🟢 running"));
        assert!(output.contains("- Step count: 5"));
        assert!(output.contains(updated.last_activity.as_deref().unwrap()));
        assert!(output.contains("- Progress: searching call sites in src/"));
        assert!(output.contains("Child trajectory: [view](refact://chat/subchat-status)"));
    }

    #[tokio::test]
    async fn agent_result_happy_path_returns_terminal_summary() {
        let (_temp, app, ccx) = test_context(PARENT).await;
        let record = create_agent(&app, PARENT, BgAgentKind::Delegate, "Fix retry").await;
        let completed = app
            .agents
            .mark_completed(&record.agent_id, completion("fixed retry logic"))
            .await
            .unwrap();

        let output = output_text(
            ToolAgentResult {
                config_path: String::new(),
            }
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[("agent_id", json!(completed.agent_id))]),
            )
            .await
            .unwrap(),
        );

        assert!(output.contains("# Agent Result: Fix retry"));
        assert!(output.contains("fixed retry logic"));
        assert!(output.contains("- Edited files: src/frog.rs"));
    }

    #[tokio::test]
    async fn agent_cancel_happy_path_requests_cancel() {
        let (_temp, app, ccx) = test_context(PARENT).await;
        let record = create_agent(&app, PARENT, BgAgentKind::Delegate, "Stop me").await;
        app.agents
            .mark_running(&record.agent_id, "subchat-cancel".to_string())
            .await
            .unwrap();

        let output = output_text(
            ToolAgentCancel {
                config_path: String::new(),
            }
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[("agent_id", json!(record.agent_id))]),
            )
            .await
            .unwrap(),
        );

        assert!(output.contains("✓ Cancel requested for: bgagent-"));
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

        let wait_err = ToolAgentWait {
            config_path: String::new(),
        }
        .tool_execute(
            ccx.clone(),
            &"call".to_string(),
            &args(&[
                ("agent_id", json!(record.agent_id.clone())),
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
    async fn agent_list_filters_status_kind_limit_and_terminal_window() {
        let (_temp, app, ccx) = test_context(PARENT).await;
        let running_delegate = create_agent(&app, PARENT, BgAgentKind::Delegate, "Running").await;
        app.agents
            .mark_running(&running_delegate.agent_id, "subchat-running".to_string())
            .await
            .unwrap();
        let completed_delegate =
            create_agent(&app, PARENT, BgAgentKind::Delegate, "Completed").await;
        app.agents
            .mark_completed(&completed_delegate.agent_id, completion("done"))
            .await
            .unwrap();
        let subagent = create_agent(&app, PARENT, BgAgentKind::Subagent, "Queued subagent").await;

        let running_output = output_text(
            ToolAgentList {
                config_path: String::new(),
            }
            .tool_execute(
                ccx.clone(),
                &"call".to_string(),
                &args(&[("status", json!("running"))]),
            )
            .await
            .unwrap(),
        );
        assert!(running_output.contains(&running_delegate.agent_id));
        assert!(!running_output.contains(&completed_delegate.agent_id));

        let subagent_output = output_text(
            ToolAgentList {
                config_path: String::new(),
            }
            .tool_execute(
                ccx.clone(),
                &"call".to_string(),
                &args(&[("kind", json!("subagent"))]),
            )
            .await
            .unwrap(),
        );
        assert!(subagent_output.contains(&subagent.agent_id));
        assert!(!subagent_output.contains(&running_delegate.agent_id));

        let limited_output = output_text(
            ToolAgentList {
                config_path: String::new(),
            }
            .tool_execute(
                ccx.clone(),
                &"call".to_string(),
                &args(&[("limit", json!(1))]),
            )
            .await
            .unwrap(),
        );
        assert!(limited_output.contains("Total: 1"));

        let terminal_window_output = output_text(
            ToolAgentList {
                config_path: String::new(),
            }
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[("include_terminal_within_hours", json!(0))]),
            )
            .await
            .unwrap(),
        );
        assert!(!terminal_window_output.contains(&completed_delegate.agent_id));
        assert!(terminal_window_output.contains(&running_delegate.agent_id));
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
            ToolAgentWait {
                config_path: String::new(),
            }
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[
                    ("agent_id", json!(record.agent_id)),
                    ("timeout_ms", json!(1000)),
                ]),
            )
            .await
            .unwrap(),
        );

        assert!(output.starts_with("⏰ Timed out after 1000ms"));
        assert!(output.contains("# Delegate: Slow agent"));
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
            ToolAgentWait {
                config_path: String::new(),
            }
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[
                    ("agent_id", json!(record.agent_id)),
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
        let denied = ToolAgentWait {
            config_path: String::new(),
        }
        .tool_execute(
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
            ToolAgentWait {
                config_path: String::new(),
            }
            .tool_execute(
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
            ToolAgentWait {
                config_path: String::new(),
            }
            .tool_execute(
                ccx,
                &"timeout".to_string(),
                &args(&[
                    ("agent_ids", json!([slow.agent_id.clone()])),
                    ("agent_id", json!(slow.agent_id.clone())),
                    ("timeout_ms", json!(1)),
                ]),
            )
            .await
            .unwrap(),
        );
        assert!(timed_out.contains("Timed out"));
        assert!(timed_out.contains("still running"));
    }

    #[tokio::test]
    async fn agent_result_reports_not_finished_for_running_agent() {
        let (_temp, app, ccx) = test_context(PARENT).await;
        let record = create_agent(&app, PARENT, BgAgentKind::Subagent, "Still running").await;
        app.agents
            .mark_running(&record.agent_id, "subchat-running".to_string())
            .await
            .unwrap();

        let output = output_text(
            ToolAgentResult {
                config_path: String::new(),
            }
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[("agent_id", json!(record.agent_id))]),
            )
            .await
            .unwrap(),
        );

        assert!(output.contains("Agent has not finished yet"));
        assert!(output.contains("🟢 running"));
    }

    #[tokio::test]
    async fn agent_result_returns_full_default_output() {
        let (_temp, app, ccx) = test_context(PARENT).await;
        let record = create_agent(&app, PARENT, BgAgentKind::Delegate, "Long result").await;
        let summary = "frog".repeat(5000);
        let completed = app
            .agents
            .mark_completed(&record.agent_id, completion(&summary))
            .await
            .unwrap();

        let output = output_text(
            ToolAgentResult {
                config_path: String::new(),
            }
            .tool_execute(
                ccx.clone(),
                &"call".to_string(),
                &args(&[("agent_id", json!(completed.agent_id.clone()))]),
            )
            .await
            .unwrap(),
        );

        assert!(output.contains(&summary));
        assert!(!output.contains('…'));
        assert!(output.contains("- Diff summary: one frog changed"));

        let (_, contexts) = ToolAgentResult {
            config_path: String::new(),
        }
        .tool_execute(
            ccx,
            &"call-preserve".to_string(),
            &args(&[("agent_id", json!(completed.agent_id))]),
        )
        .await
        .unwrap();
        let ContextEnum::ChatMessage(message) = contexts.into_iter().next().unwrap() else {
            panic!("expected chat message")
        };
        assert_eq!(message.preserve, Some(true));
        assert_eq!(
            message
                .output_filter
                .as_ref()
                .map(|filter| filter.limit_chars),
            Some(usize::MAX)
        );
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
        let (child, child_abort, _) = app
            .agents
            .create(create_request("child-chat", BgAgentKind::Subagent, "Child"))
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

        assert!(parent_abort.load(Ordering::SeqCst));
        assert!(child_abort.load(Ordering::SeqCst));
        assert_eq!(
            app.agents.get_any(&parent.agent_id).await.unwrap().status,
            BgAgentStatus::Cancelled
        );
        assert_eq!(
            app.agents.get_any(&child.agent_id).await.unwrap().status,
            BgAgentStatus::Cancelled
        );
        assert!(output.contains(&parent.agent_id));
        assert!(output.contains(&child.agent_id));
    }

    #[test]
    fn agent_resume_parser_requires_exact_actions_and_positive_integers() {
        let inspect = args(&[
            ("agent_id", json!("bgagent-test")),
            ("action", json!("inspect")),
        ]);
        assert_eq!(
            parse_agent_resume_args(&inspect).unwrap().1,
            AgentResumeAction::Inspect
        );
        let valid = args(&[
            ("agent_id", json!("bgagent-test")),
            ("action", json!("raise_and_resume")),
            ("max_steps", json!(80)),
            ("expected_change_seq", json!(5)),
        ]);
        assert_eq!(
            parse_agent_resume_args(&valid).unwrap().1,
            AgentResumeAction::RaiseAndResume {
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
                    parse_agent_resume_args(&invalid).is_err(),
                    "accepted {key}: {invalid:?}"
                );
            }
            let mut missing = valid.clone();
            missing.remove(key);
            assert!(parse_agent_resume_args(&missing).is_err());
            let mut mutation_on_inspect = inspect.clone();
            mutation_on_inspect.insert(key.to_string(), json!(1));
            assert!(parse_agent_resume_args(&mutation_on_inspect).is_err());
        }
        for key in ["action", "agent_id"] {
            let mut missing = valid.clone();
            missing.remove(key);
            assert!(parse_agent_resume_args(&missing).is_err());
            for value in [json!(""), Value::Null, json!(42)] {
                let mut invalid = valid.clone();
                invalid.insert(key.to_string(), value);
                assert!(parse_agent_resume_args(&invalid).is_err());
            }
        }
        let mut unknown = valid;
        unknown.insert("scope".to_string(), json!("all"));
        assert!(parse_agent_resume_args(&unknown).is_err());
    }

    #[test]
    fn agent_resume_schema_is_scoped_and_requires_mutation_revision() {
        let description = ToolAgentResume {
            config_path: String::new(),
        }
        .tool_description();
        assert_eq!(description.name, "agent_resume");
        assert!(!description.allow_parallel);
        let schema = description.input_schema;
        assert_eq!(schema["required"], json!(["agent_id", "action"]));
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(
            schema["properties"]["action"]["enum"],
            json!(["inspect", "raise_and_resume", "resume"])
        );
        assert_eq!(
            schema["oneOf"][1]["required"],
            json!(["max_steps", "expected_change_seq"])
        );
        for field in ["max_steps", "expected_change_seq"] {
            assert_eq!(schema["properties"][field]["type"], "integer");
            assert_eq!(schema["properties"][field]["minimum"], 1);
        }
    }

    #[tokio::test]
    async fn agent_resume_inspection_is_read_only_and_direct_parent_scoped() {
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
        let mut tool = ToolAgentResume {
            config_path: String::new(),
        };
        let output = output_text(
            tool.tool_execute(
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
        for action in ["inspect", "raise_and_resume"] {
            let mut request = args(&[
                ("agent_id", json!(foreign.agent_id)),
                ("action", json!(action)),
            ]);
            if action == "raise_and_resume" {
                request.insert("max_steps".to_string(), json!(80));
                request.insert("expected_change_seq".to_string(), json!(1));
            }
            let error = tool
                .tool_execute(ccx.clone(), &"foreign".to_string(), &request)
                .await
                .unwrap_err();
            assert_eq!(error, not_found_error(&foreign.agent_id));
        }
    }

    #[tokio::test]
    async fn agent_resume_raise_requires_confirmation_by_default() {
        use crate::tools::tools_description::MatchConfirmDenyResult;
        let (_temp, _app, ccx) = test_context(PARENT).await;
        let tool = ToolAgentResume {
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
            ("action", json!("raise_and_resume")),
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
    fn human_age_formats_short_durations() {
        let now = Utc::now();
        assert_eq!(human_age(now - chrono::Duration::minutes(2), now), "2m");
        assert_eq!(human_age(now - chrono::Duration::hours(3), now), "3h");
        assert_eq!(human_age(now - chrono::Duration::days(1), now), "1d");
    }
}
