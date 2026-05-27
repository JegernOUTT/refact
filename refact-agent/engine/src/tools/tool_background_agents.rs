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
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

const DEFAULT_LIMIT: usize = 20;
const DEFAULT_TERMINAL_HOURS: i64 = 24;
const DEFAULT_WAIT_MS: u64 = 60_000;
const MIN_WAIT_MS: u64 = 1_000;
const MAX_WAIT_MS: u64 = 30 * 60 * 1_000;
const RESULT_DEFAULT_LIMIT: usize = 4_000;
const RESULT_DETAILS_LIMIT: usize = 16_000;

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
    agent_id_schema(serde_json::Map::from_iter([(
        "timeout_ms".to_string(),
        json!({
            "type": "integer",
            "minimum": MIN_WAIT_MS,
            "maximum": MAX_WAIT_MS,
            "description": "How long to wait for completion. Default: 60000"
        }),
    )]))
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
    agent_id_schema(serde_json::Map::from_iter([(
        "reason".to_string(),
        json!({
            "type": "string",
            "description": "Optional cancellation reason"
        }),
    )]))
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
        Some(Value::Bool(value)) => Ok(*value),
        Some(Value::Null) | None => Ok(default),
        Some(value) => Err(format!("{key} must be a boolean, got {value}")),
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

fn parse_wait_timeout_ms(args: &HashMap<String, Value>) -> Result<u64, String> {
    let timeout_ms = parse_optional_u64(args, "timeout_ms", DEFAULT_WAIT_MS)?;
    if timeout_ms < MIN_WAIT_MS {
        return Err(format!("timeout_ms must be at least {MIN_WAIT_MS}ms"));
    }
    if timeout_ms > MAX_WAIT_MS {
        return Err(format!("timeout_ms must be at most {MAX_WAIT_MS}ms"));
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
    result.push_str("| ID | Kind | Status | Title | Age | Last activity |\n");
    result.push_str("|---|---|---|---|---|---|\n");
    for row in rows {
        result.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            markdown_cell(&row.agent_id, 80),
            kind_label(row.kind),
            format_status_chip(row.status),
            markdown_cell(&row.title, 80),
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
            result.push_str(&format!(
                "- {label}: {}\n",
                truncate_chars(value.trim(), 500)
            ));
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

    let limit = if include_details {
        RESULT_DETAILS_LIMIT
    } else {
        RESULT_DEFAULT_LIMIT
    };
    let summary = record
        .result_summary
        .as_deref()
        .or(record.error.as_deref())
        .unwrap_or("No result summary was recorded.");
    let mut result = format!("# Agent Result: {}\n", record.title);
    result.push_str(&format!(
        "- Status: {}\n",
        format_status_chip(record.status)
    ));
    result.push_str(&format!("- Agent ID: {}\n", record.agent_id));
    result.push_str(&format!("- Kind: {}\n", kind_label(record.kind)));
    if let Some(child_chat_id) = &record.child_chat_id {
        result.push_str(&format!(
            "- Child trajectory: [view](refact://chat/{child_chat_id})\n"
        ));
    }
    result.push_str("\n");
    result.push_str(&truncate_chars(summary, limit));
    result.push('\n');
    format_list_line(&mut result, "Edited files", &record.edited_files);
    format_optional_line(&mut result, "Diff summary", record.diff_summary.as_deref());
    format_optional_line(
        &mut result,
        "Conflict summary",
        record.conflict_summary.as_deref(),
    );
    format_optional_line(&mut result, "Error", record.error.as_deref());

    if include_details {
        if let Some(path) = &record.result_payload_path {
            let payload = tokio::fs::read_to_string(path).await.map_err(|error| {
                format!(
                    "Failed to read background agent result payload {}: {error}",
                    path.display()
                )
            })?;
            result.push_str("\n## Details\n");
            result.push_str(&truncate_chars(&payload, RESULT_DETAILS_LIMIT));
            result.push('\n');
        }
    }

    Ok(result)
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
            "Wait for one background agent spawned by this chat to reach a terminal state, or return its current status after a timeout.",
            agent_wait_schema(),
        )
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let agent_id = parse_required_string(args, "agent_id")?;
        let timeout_ms = parse_wait_timeout_ms(args)?;
        let (app, parent_chat_id) = background_agent_context(&ccx).await;
        let record = app
            .agents
            .wait(
                &parent_chat_id,
                &agent_id,
                Duration::from_millis(timeout_ms),
            )
            .await
            .map_err(|error| map_registry_error(&agent_id, error))?;
        let content = if record.status.is_terminal() {
            format_agent_result(&record, false).await?
        } else {
            format!(
                "⏰ Timed out after {timeout_ms}ms — agent still running.\n\n{}",
                format_agent_status_block(&record)
            )
        };
        Ok(tool_message(tool_call_id, content))
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
        let (app, parent_chat_id) = background_agent_context(&ccx).await;
        let current = app
            .agents
            .get(&parent_chat_id, &agent_id)
            .await
            .map_err(|error| map_registry_error(&agent_id, error))?;
        if current.status.is_terminal() {
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
            .cancel(&parent_chat_id, &agent_id, reason)
            .await
            .map_err(|error| map_registry_error(&agent_id, error))?;
        crate::agents::spawn::emit_background_agent_update(app, &updated).await;
        Ok(tool_message(
            tool_call_id,
            format!("✓ Cancel requested for {agent_id}. Status is now cancelled."),
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
        }
    }

    fn completion(summary: &str) -> AgentCompletion {
        AgentCompletion {
            result_summary: summary.to_string(),
            edited_files: vec!["src/frog.rs".to_string()],
            diff_summary: Some("one frog changed".to_string()),
            conflict_summary: Some("no conflicts".to_string()),
            child_chat_id: Some("subchat-frog".to_string()),
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
        app.agents
            .update_progress(
                &running.agent_id,
                "searching call sites in src/".to_string(),
                5,
                Some("t_shell".to_string()),
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
        assert!(output.contains("| ID | Kind | Status | Title | Age | Last activity |"));
        assert!(output.contains(&running.agent_id));
        assert!(output.contains("delegate"));
        assert!(output.contains("🟢 running"));
        assert!(output.contains("t_shell"));
        assert!(output.contains("Total: 2"));
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
                Some("regex_search".to_string()),
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
        assert!(output.contains("- Last activity: regex_search"));
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

        assert!(output.contains("✓ Cancel requested for bgagent-"));
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
    async fn agent_result_truncates_default_and_expands_details() {
        let (_temp, app, ccx) = test_context(PARENT).await;
        let record = create_agent(&app, PARENT, BgAgentKind::Delegate, "Long result").await;
        let summary = "frog".repeat(5000);
        let completed = app
            .agents
            .mark_completed(&record.agent_id, completion(&summary))
            .await
            .unwrap();

        let default_output = output_text(
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
        let detailed_output = output_text(
            ToolAgentResult {
                config_path: String::new(),
            }
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[
                    ("agent_id", json!(completed.agent_id)),
                    ("include_details", json!(true)),
                ]),
            )
            .await
            .unwrap(),
        );

        assert!(default_output.contains('…'));
        assert!(default_output.chars().count() < detailed_output.chars().count());
        assert!(detailed_output.contains("## Details"));
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

    #[test]
    fn human_age_formats_short_durations() {
        let now = Utc::now();
        assert_eq!(human_age(now - chrono::Duration::minutes(2), now), "2m");
        assert_eq!(human_age(now - chrono::Duration::hours(3), now), "3h");
        assert_eq!(human_age(now - chrono::Duration::days(1), now), "1d");
    }
}
