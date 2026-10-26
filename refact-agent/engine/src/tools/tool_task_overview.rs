use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Value};
use serde_yaml::Value as YamlValue;
use tokio::fs;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::chat::history_limit::{compute_context_budget, ContextBudgetReport, ContextPressure};
use crate::global_context::GlobalContext;
use crate::tasks::storage;
use crate::tasks::types::{BoardCard, TaskBoard};
use crate::tools::tool_task_check_agents::{
    classify_agent_status, format_card_pause, get_agent_statuses, AgentStateKind, AgentStatus,
};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use refact_runtime_api::ChatSessionFacade;

const MAX_REPORT_CHARS: usize = 4096;
const MAX_DOCUMENTS: usize = 6;
const MAX_ACTIVITY: usize = 10;

pub struct ToolTaskOverview;

impl ToolTaskOverview {
    pub fn new() -> Self {
        Self
    }
}

struct OverviewContext {
    task_id: String,
    task_meta: Option<crate::chat::types::TaskMeta>,
    gcx: Arc<GlobalContext>,
    chat_facade: Arc<dyn ChatSessionFacade>,
    messages: Vec<ChatMessage>,
    n_ctx: usize,
}

#[derive(Default)]
struct ColumnCounts {
    planned: usize,
    doing: usize,
    done: usize,
    failed: usize,
}

#[derive(Clone)]
struct ActivityEntry {
    timestamp: DateTime<Utc>,
    text: String,
}

#[derive(Default)]
struct MemoryPulse {
    total: usize,
    pinned: usize,
    archived: usize,
    superseded: usize,
    active: usize,
    kinds: HashMap<String, usize>,
    tags: HashMap<String, usize>,
    activities: Vec<ActivityEntry>,
}

#[derive(Clone)]
struct DocumentInfo {
    slug: String,
    name: String,
    kind: String,
    version: u64,
    pinned: bool,
    updated_at: Option<DateTime<Utc>>,
}

async fn overview_context(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    args: &HashMap<String, Value>,
) -> Result<OverviewContext, String> {
    let ccx_lock = ccx.lock().await;
    let is_planner = ccx_lock
        .task_meta
        .as_ref()
        .map(|meta| meta.role == "planner")
        .unwrap_or(false);
    if !is_planner {
        return Err("task_overview can only be called by the task planner. Switch to the planner chat for task situational awareness.".to_string());
    }
    let task_id = args
        .get("task_id")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .or_else(|| ccx_lock.task_meta.as_ref().map(|meta| meta.task_id.clone()))
        .ok_or_else(|| "Missing 'task_id' (and chat is not bound to a task)".to_string())?;
    Ok(OverviewContext {
        task_id,
        task_meta: ccx_lock.task_meta.clone(),
        gcx: ccx_lock.app.gcx.clone(),
        chat_facade: ccx_lock.app.chat.facade.clone(),
        messages: ccx_lock.messages.clone(),
        n_ctx: ccx_lock.n_ctx,
    })
}

fn task_overview_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "task_id": {
                "type": "string",
                "description": "Task ID (optional if chat is bound to a task)"
            }
        },
        "required": []
    })
}

fn task_overview_description() -> ToolDesc {
    ToolDesc {
        name: "task_overview".to_string(),
        display_name: "Task Overview".to_string(),
        source: ToolSource {
            source_type: ToolSourceType::Builtin,
            config_path: String::new(),
        },
        experimental: false,
        allow_parallel: true,
        description: "Planner-only one-shot situational awareness report combining board state, agent health, memory pulse, documents, recent activity, and context pressure.".to_string(),
        input_schema: task_overview_schema(),
        output_schema: None,
        annotations: None,
    }
}

fn priority_key(priority: &str) -> String {
    let priority = priority.trim().to_ascii_uppercase();
    if priority.is_empty() {
        "P1".to_string()
    } else {
        priority
    }
}

fn board_counts(board: &TaskBoard) -> Vec<(String, ColumnCounts)> {
    let mut counts: HashMap<String, ColumnCounts> = HashMap::new();
    for priority in ["P0", "P1", "P2"] {
        counts.entry(priority.to_string()).or_default();
    }
    for card in &board.cards {
        let entry = counts.entry(priority_key(&card.priority)).or_default();
        match card.column.as_str() {
            "planned" => entry.planned += 1,
            "doing" => entry.doing += 1,
            "done" => entry.done += 1,
            "failed" | "regressed" => entry.failed += 1,
            _ => entry.planned += 1,
        }
    }
    let mut rows: Vec<_> = counts.into_iter().collect();
    rows.sort_by(|(left, _), (right, _)| priority_rank(left).cmp(&priority_rank(right)));
    rows
}

fn priority_rank(priority: &str) -> u8 {
    match priority {
        "P0" => 0,
        "P1" => 1,
        "P2" => 2,
        _ => 3,
    }
}

fn render_board(board: &TaskBoard) -> String {
    let mut output = String::from("## Board\n");
    for (priority, count) in board_counts(board) {
        output.push_str(&format!(
            "{}: {} planned, {} doing, {} done, {} failed\n",
            priority, count.planned, count.doing, count.done, count.failed
        ));
    }
    if board.cards.is_empty() {
        output.push_str("_No cards yet._\n");
    }
    output
}

fn render_agent_health(statuses: &[AgentStatus], now: DateTime<Utc>) -> String {
    let mut running = 0usize;
    let mut stuck = 0usize;
    let mut failed = 0usize;
    let mut paused = 0usize;
    let mut done_last_hour = 0usize;
    let mut running_labels = Vec::new();
    let mut paused_labels = Vec::new();

    for status in statuses {
        match classify_agent_status(status, now) {
            AgentStateKind::Running | AgentStateKind::Queued | AgentStateKind::Parked => {
                running += 1;
                if running_labels.len() < 4 {
                    let age = status
                        .last_activity_at
                        .map(|timestamp| format_duration_short(now, timestamp))
                        .unwrap_or_else(|| "?".to_string());
                    running_labels.push(format!(
                        "{}: {} {}",
                        priority_key(&status.priority),
                        status.card_id,
                        age
                    ));
                }
            }
            AgentStateKind::Paused => {
                paused += 1;
                if paused_labels.len() < 4 {
                    paused_labels.push(format!(
                        "{}: {}",
                        status.status_id(),
                        format_card_pause(status)
                    ));
                }
            }
            AgentStateKind::Stuck => stuck += 1,
            AgentStateKind::Failed => failed += 1,
            AgentStateKind::Done => {
                if status
                    .last_activity_at
                    .map(|timestamp| now.signed_duration_since(timestamp) <= Duration::hours(1))
                    .unwrap_or(false)
                {
                    done_last_hour += 1;
                }
            }
        }
    }

    let running_detail = if running_labels.is_empty() {
        String::new()
    } else {
        format!("   ({})", running_labels.join(", "))
    };
    let paused_detail = if paused_labels.is_empty() {
        String::new()
    } else {
        format!("   ({})", paused_labels.join(", "))
    };
    format!(
        "## Agent Health\n🔄 {} running{}\n🔴 {} stuck\n❌ {} failed\n⏸️ Paused: {}{}\n✅ {} done in last hour\n",
        running, running_detail, stuck, failed, paused, paused_detail, done_last_hour
    )
}

fn split_frontmatter(raw: &str) -> Option<&str> {
    let rest = raw
        .strip_prefix("---\n")
        .or_else(|| raw.strip_prefix("---\r\n"))?;
    if let Some(index) = rest.find("\n---\n") {
        return Some(&rest[..index]);
    }
    if let Some(index) = rest.find("\r\n---\r\n") {
        return Some(&rest[..index]);
    }
    None
}

fn yaml_field<'a>(mapping: &'a serde_yaml::Mapping, key: &str) -> Option<&'a YamlValue> {
    mapping.get(&YamlValue::String(key.to_string()))
}

fn yaml_string(mapping: &serde_yaml::Mapping, key: &str) -> Option<String> {
    match yaml_field(mapping, key)? {
        YamlValue::String(value) => Some(value.clone()),
        YamlValue::Number(value) => Some(value.to_string()),
        YamlValue::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn yaml_bool(mapping: &serde_yaml::Mapping, key: &str) -> bool {
    match yaml_field(mapping, key) {
        Some(YamlValue::Bool(value)) => *value,
        Some(YamlValue::String(value)) => value.trim().eq_ignore_ascii_case("true"),
        _ => false,
    }
}

fn yaml_u64(mapping: &serde_yaml::Mapping, key: &str) -> u64 {
    match yaml_field(mapping, key) {
        Some(YamlValue::Number(value)) => value.as_u64().unwrap_or(0),
        Some(YamlValue::String(value)) => value.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

fn yaml_string_list(mapping: &serde_yaml::Mapping, key: &str) -> Vec<String> {
    match yaml_field(mapping, key) {
        Some(YamlValue::Sequence(values)) => values
            .iter()
            .filter_map(|value| match value {
                YamlValue::String(value) => Some(value.trim().to_string()),
                YamlValue::Number(value) => Some(value.to_string()),
                _ => None,
            })
            .filter(|value| !value.is_empty())
            .collect(),
        Some(YamlValue::String(value)) => value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_yaml_mapping(raw: &str) -> Option<serde_yaml::Mapping> {
    let frontmatter = split_frontmatter(raw)?;
    match serde_yaml::from_str::<YamlValue>(frontmatter).ok()? {
        YamlValue::Mapping(mapping) => Some(mapping),
        _ => None,
    }
}

fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.with_timezone(&Utc))
}

async fn file_modified_at(path: &Path) -> Option<DateTime<Utc>> {
    fs::metadata(path)
        .await
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .map(system_time_to_utc)
}

fn system_time_to_utc(time: SystemTime) -> DateTime<Utc> {
    DateTime::<Utc>::from(time)
}

fn display_name_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("untitled")
        .to_string()
}

async fn markdown_files_in(dir: &Path) -> Result<Vec<PathBuf>, String> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    let mut entries = fs::read_dir(dir)
        .await
        .map_err(|error| format!("failed to read {}: {}", dir.display(), error))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| format!("failed to read {}: {}", dir.display(), error))?
    {
        let path = entry.path();
        if path.is_file()
            && matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("md" | "mdx")
            )
        {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

async fn load_memory_pulse(
    task_dir: &Path,
    access: &crate::tools::tool_task_memory::MemoryAccessContext,
) -> Result<MemoryPulse, String> {
    let memories_dir = task_dir.join("memories");
    let archived_dir = memories_dir.join("archived");
    let mut pulse = MemoryPulse::default();

    for (dir, force_archived) in [(&memories_dir, false), (&archived_dir, true)] {
        for path in markdown_files_in(dir).await? {
            let raw = fs::read_to_string(&path).await.unwrap_or_default();
            let Some(mapping) = parse_yaml_mapping(&raw) else {
                continue;
            };
            let namespace: crate::tools::tool_task_memory::MemoryNamespace =
                match yaml_field(&mapping, "namespace") {
                    Some(YamlValue::String(value)) => match value.parse() {
                        Ok(namespace) => namespace,
                        Err(_) => continue,
                    },
                    Some(_) => continue,
                    None => crate::tools::tool_task_memory::MemoryNamespace::default(),
                };
            match access.decide(&namespace) {
                crate::tools::tool_task_memory::MemoryAccessDecision::Allowed => {}
                crate::tools::tool_task_memory::MemoryAccessDecision::SupervisorOverride => {
                    access
                        .audit_supervisor_grant("task_overview", &path, &namespace)
                        .await;
                }
                crate::tools::tool_task_memory::MemoryAccessDecision::Denied => continue,
            }
            let status = if force_archived {
                "archived".to_string()
            } else {
                yaml_string(&mapping, "status").unwrap_or_else(|| "active".to_string())
            };
            let kind = yaml_string(&mapping, "kind").unwrap_or_else(|| "freeform".to_string());
            let tags = yaml_string_list(&mapping, "tags");
            let title =
                yaml_string(&mapping, "title").unwrap_or_else(|| display_name_from_path(&path));
            let created_at = yaml_string(&mapping, "created_at")
                .and_then(|timestamp| parse_timestamp(&timestamp));
            let timestamp = created_at.or(file_modified_at(&path).await);

            pulse.total += 1;
            if yaml_bool(&mapping, "pinned") {
                pulse.pinned += 1;
            }
            match status.as_str() {
                "archived" => pulse.archived += 1,
                "superseded" => pulse.superseded += 1,
                _ => pulse.active += 1,
            }
            *pulse.kinds.entry(kind).or_default() += 1;
            for tag in tags {
                *pulse.tags.entry(tag).or_default() += 1;
            }
            if let Some(timestamp) = timestamp {
                pulse.activities.push(ActivityEntry {
                    timestamp,
                    text: format!("memory created: {}", truncate_chars(&title, 80)),
                });
            }
        }
    }

    Ok(pulse)
}

async fn load_documents(
    task_dir: &Path,
) -> Result<(Vec<DocumentInfo>, Vec<ActivityEntry>), String> {
    let documents_dir = task_dir.join("documents");
    let mut documents = Vec::new();
    let mut activities = Vec::new();

    for path in markdown_files_in(&documents_dir).await? {
        let raw = fs::read_to_string(&path).await.unwrap_or_default();
        let mapping = parse_yaml_mapping(&raw).unwrap_or_default();
        let slug = yaml_string(&mapping, "slug").unwrap_or_else(|| display_name_from_path(&path));
        let name = yaml_string(&mapping, "name").unwrap_or_else(|| slug.clone());
        let kind = yaml_string(&mapping, "kind").unwrap_or_else(|| "document".to_string());
        let updated_at = yaml_string(&mapping, "updated_at")
            .and_then(|timestamp| parse_timestamp(&timestamp))
            .or(file_modified_at(&path).await);
        let document = DocumentInfo {
            slug: slug.clone(),
            name,
            kind,
            version: yaml_u64(&mapping, "version"),
            pinned: yaml_bool(&mapping, "pinned"),
            updated_at,
        };
        if let Some(timestamp) = updated_at {
            activities.push(ActivityEntry {
                timestamp,
                text: format!("document updated: {}", truncate_chars(&slug, 80)),
            });
        }
        documents.push(document);
    }

    documents.sort_by(|left, right| {
        right
            .pinned
            .cmp(&left.pinned)
            .then_with(|| right.updated_at.cmp(&left.updated_at))
            .then_with(|| left.slug.cmp(&right.slug))
    });
    Ok((documents, activities))
}

fn top_counts(counts: &HashMap<String, usize>, limit: usize) -> String {
    if counts.is_empty() {
        return "none".to_string();
    }
    let mut rows: Vec<_> = counts.iter().collect();
    rows.sort_by(|(left_key, left_count), (right_key, right_count)| {
        right_count
            .cmp(left_count)
            .then_with(|| left_key.cmp(right_key))
    });
    rows.into_iter()
        .take(limit)
        .map(|(key, count)| format!("{}={}", key, count))
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_memory_pulse(pulse: &MemoryPulse) -> String {
    let counts = if pulse.superseded > 0 {
        format!(
            "{} memories total ({} pinned, {} archived, {} superseded, {} active)",
            pulse.total, pulse.pinned, pulse.archived, pulse.superseded, pulse.active
        )
    } else {
        format!(
            "{} memories total ({} pinned, {} archived, {} active)",
            pulse.total, pulse.pinned, pulse.archived, pulse.active
        )
    };
    format!(
        "## Memory Pulse\n{}\nRecent kinds: {}\nTop tags: {}\n",
        counts,
        top_counts(&pulse.kinds, 5),
        top_counts(&pulse.tags, 5)
    )
}

fn render_documents(documents: &[DocumentInfo], now: DateTime<Utc>) -> String {
    let mut output = String::from("## Documents\n");
    if documents.is_empty() {
        output.push_str("- No task documents found.\n");
        return output;
    }
    for document in documents.iter().take(MAX_DOCUMENTS) {
        let updated = document
            .updated_at
            .map(|timestamp| format_age_ago(now, timestamp))
            .unwrap_or_else(|| "unknown".to_string());
        let pinned = if document.pinned { ", pinned" } else { "" };
        let version = if document.version > 0 {
            format!("v{}", document.version)
        } else {
            "v?".to_string()
        };
        output.push_str(&format!(
            "- {} ({}, {}, updated {}{})\n",
            truncate_chars(&document.name, 60),
            document.kind,
            version,
            updated,
            pinned
        ));
    }
    if documents.len() > MAX_DOCUMENTS {
        output.push_str(&format!(
            "- … {} more documents\n",
            documents.len() - MAX_DOCUMENTS
        ));
    }
    output
}

fn card_activity_entries(board: &TaskBoard) -> Vec<ActivityEntry> {
    let mut entries = Vec::new();
    for card in &board.cards {
        for update in &card.status_updates {
            if let Some(timestamp) = parse_timestamp(&update.timestamp) {
                entries.push(ActivityEntry {
                    timestamp,
                    text: format_card_update(card, &update.message),
                });
            }
        }
    }
    entries
}

fn format_card_update(card: &BoardCard, message: &str) -> String {
    let message = message.trim();
    let lower = message.to_ascii_lowercase();
    let text = if lower.contains("auto-committed") {
        "merged/committed work".to_string()
    } else if lower.contains("merged") || lower.contains("merge") {
        format!("merge: {}", message)
    } else {
        message.to_string()
    };
    format!("{} {}", card.id, truncate_chars(&text, 100))
}

fn render_recent_activity(activities: &[ActivityEntry], now: DateTime<Utc>) -> String {
    let mut recent: Vec<_> = activities
        .iter()
        .filter(|entry| now.signed_duration_since(entry.timestamp) <= Duration::hours(1))
        .collect();
    recent.sort_by(|left, right| right.timestamp.cmp(&left.timestamp));

    let mut output = String::from("## Recent Activity (last 1h)\n");
    if recent.is_empty() {
        output.push_str("- No activity in the last hour.\n");
        return output;
    }
    for entry in recent.into_iter().take(MAX_ACTIVITY) {
        output.push_str(&format!(
            "- {} ({})\n",
            entry.text,
            format_age_ago(now, entry.timestamp)
        ));
    }
    output
}

fn render_context_pressure(budget: &ContextBudgetReport) -> String {
    let pct_used = if budget.effective_n_ctx > 0 {
        budget.used_tokens_estimate.saturating_mul(100) / budget.effective_n_ctx
    } else {
        0
    };
    format!(
        "## Context Pressure\nPlanner chat: {}% of {} tokens used ({})\n",
        pct_used,
        budget.effective_n_ctx,
        pressure_label(&budget.pressure)
    )
}

fn pressure_label(pressure: &ContextPressure) -> &'static str {
    match pressure {
        ContextPressure::Low => "Low",
        ContextPressure::Medium => "Medium",
        ContextPressure::High => "High",
        ContextPressure::Critical => "Critical",
    }
}

fn render_task_overview(
    task_name: &str,
    board: &TaskBoard,
    statuses: &[AgentStatus],
    memory_pulse: &MemoryPulse,
    documents: &[DocumentInfo],
    activities: &[ActivityEntry],
    budget: &ContextBudgetReport,
    now: DateTime<Utc>,
) -> String {
    let mut output = format!("# Task Overview: {}\n\n", task_name.trim());
    output.push_str(&render_board(board));
    output.push('\n');
    output.push_str(&render_agent_health(statuses, now));
    output.push('\n');
    output.push_str(&render_memory_pulse(memory_pulse));
    output.push('\n');
    output.push_str(&render_documents(documents, now));
    output.push('\n');
    output.push_str(&render_recent_activity(activities, now));
    output.push('\n');
    output.push_str(&render_context_pressure(budget));
    truncate_report(output)
}

fn truncate_report(output: String) -> String {
    if output.chars().count() <= MAX_REPORT_CHARS {
        return output;
    }
    let note = "\n\n_Overview truncated to fit 4KB._";
    let keep = MAX_REPORT_CHARS.saturating_sub(note.chars().count());
    let mut truncated = output.chars().take(keep).collect::<String>();
    if let Some(index) = truncated.rfind('\n') {
        truncated.truncate(index);
    }
    truncated.push_str(note);
    truncated
}

fn truncate_chars(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let take = limit.saturating_sub(1);
    format!("{}…", text.chars().take(take).collect::<String>())
}

fn format_age_ago(now: DateTime<Utc>, timestamp: DateTime<Utc>) -> String {
    let duration = now.signed_duration_since(timestamp);
    let minutes = duration.num_minutes().max(0);
    if minutes == 0 {
        "now".to_string()
    } else if minutes < 60 {
        format!("{}m ago", minutes)
    } else if minutes < 60 * 24 {
        format!("{}h ago", minutes / 60)
    } else {
        format!("{}d ago", minutes / (60 * 24))
    }
}

fn format_duration_short(now: DateTime<Utc>, timestamp: DateTime<Utc>) -> String {
    let duration = now.signed_duration_since(timestamp);
    let minutes = duration.num_minutes().max(0);
    if minutes == 0 {
        "now".to_string()
    } else if minutes < 60 {
        format!("{}m", minutes)
    } else if minutes < 60 * 24 {
        format!("{}h", minutes / 60)
    } else {
        format!("{}d", minutes / (60 * 24))
    }
}

fn tool_output(tool_call_id: &String, result: String) -> (bool, Vec<ContextEnum>) {
    (
        false,
        vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(result),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        })],
    )
}

#[async_trait]
impl Tool for ToolTaskOverview {
    fn tool_description(&self) -> ToolDesc {
        task_overview_description()
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let context = overview_context(&ccx, args).await?;
        let meta = storage::load_task_meta(context.gcx.clone(), &context.task_id).await?;
        let board = storage::load_board(context.gcx.clone(), &context.task_id).await?;
        let statuses = get_agent_statuses(
            context.gcx.clone(),
            context.chat_facade.clone(),
            &context.task_id,
        )
        .await?;
        let task_dir = storage::find_task_dir(context.gcx.clone(), &context.task_id).await?;
        let access = crate::tools::tool_task_memory::memory_access_context(
            context.gcx.clone(),
            context.task_meta.as_ref(),
            &context.task_id,
        )
        .await;
        let memory_pulse = load_memory_pulse(&task_dir, &access).await?;
        let (documents, document_activities) = load_documents(&task_dir).await?;
        let mut activities = card_activity_entries(&board);
        activities.extend(memory_pulse.activities.clone());
        activities.extend(document_activities);
        let budget = compute_context_budget(&context.messages, context.n_ctx);
        let report = render_task_overview(
            &meta.name,
            &board,
            &statuses,
            &memory_pulse,
            &documents,
            &activities,
            &budget,
            Utc::now(),
        );
        Ok(tool_output(tool_call_id, report))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use crate::chat::types::TaskMeta as ThreadTaskMeta;
    use crate::tasks::types::{TaskMeta, TaskStatus};
    use crate::tools::tools_description::Tool;
    use refact_runtime_api::SessionState;

    fn fixed_now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-05-22T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn task_meta() -> TaskMeta {
        TaskMeta {
            schema_version: 1,
            id: "task-1".to_string(),
            name: "Empty Task".to_string(),
            status: TaskStatus::Active,
            created_at: "2026-05-22T00:00:00Z".to_string(),
            updated_at: "2026-05-22T00:00:00Z".to_string(),
            cards_total: 0,
            cards_done: 0,
            cards_failed: 0,
            agents_active: 0,
            base_branch: None,
            base_commit: None,
            default_agent_model: None,
            is_name_generated: false,
            last_agents_summary_at: None,
            planner_session_state: None,
        }
    }

    async fn planner_ccx(
        gcx: Arc<GlobalContext>,
        role: &str,
        messages: Vec<ChatMessage>,
        n_ctx: usize,
    ) -> Arc<AMutex<AtCommandsContext>> {
        let app = AppState::from_gcx(gcx).await;
        Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                app,
                n_ctx,
                20,
                false,
                messages,
                "planner-chat".to_string(),
                None,
                "model".to_string(),
                Some(ThreadTaskMeta {
                    task_id: "task-1".to_string(),
                    role: role.to_string(),
                    agent_id: None,
                    card_id: None,
                    planner_chat_id: None,
                }),
                None,
            )
            .await,
        ))
    }

    fn status(id: &str, session_state: Option<SessionState>, minutes_ago: i64) -> AgentStatus {
        let ts = fixed_now() - Duration::minutes(minutes_ago);
        AgentStatus {
            card_id: id.to_string(),
            variant_key: None,
            card_title: format!("{} title", id),
            agent_chat_id: format!("agent-{}", id),
            column: "doing".to_string(),
            priority: "P0".to_string(),
            session_state,
            last_status_update: None,
            last_activity_at: Some(ts),
            started_at: Some(ts),
            paused_at: None,
            paused_by: None,
            final_report: None,
            last_tool_name: None,
            attention: None,
            change_seq: ts.timestamp() as u64,
        }
    }

    #[test]
    fn task_overview_counts_paused_cards_separately_but_keeps_real_failures() {
        let mut statuses = vec![
            status("T-PAUSED", Some(SessionState::Idle), 10),
            status("T-ERROR", Some(SessionState::Error), 0),
            status("T-COMPLETED", Some(SessionState::Completed), 0),
        ];
        for status in &mut statuses {
            status.paused_at = Some((fixed_now() - Duration::minutes(5)).to_rfc3339());
            status.paused_by = Some("planner-test".to_string());
        }
        let output = render_agent_health(&statuses, fixed_now());

        assert!(output.contains("🔄 0 running"));
        assert!(output.contains("🔴 2 stuck"));
        assert!(output.contains("Paused: 1"));
        assert!(output.contains("T-PAUSED: paused by planner-test since 2026-05-21T23:55:00+00:00"));
    }

    #[tokio::test]
    async fn tool_task_overview_rejects_non_planner() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let ccx = planner_ccx(gcx, "agents", vec![], 200_000).await;
        let err = ToolTaskOverview::new()
            .tool_execute(ccx, &"call".to_string(), &HashMap::new())
            .await
            .unwrap_err();

        assert!(err.contains("can only be called by the task planner"));
    }
}

#[cfg(test)]
#[path = "tool_task_overview_memory_tests.rs"]
mod memory_pulse_tests;
