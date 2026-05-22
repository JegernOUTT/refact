use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::tasks::storage;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use refact_runtime_api::{ChatSessionFacade, SessionState};

const DEFAULT_LIMIT: usize = 20;
const STUCK_AFTER_MINUTES: i64 = 15;

pub(crate) async fn get_task_id(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    args: &HashMap<String, Value>,
) -> Result<String, String> {
    if let Some(id) = args.get("task_id").and_then(|v| v.as_str()) {
        return Ok(id.to_string());
    }
    let ccx_lock = ccx.lock().await;
    if let Some(ref meta) = ccx_lock.task_meta {
        return Ok(meta.task_id.clone());
    }
    storage::infer_task_id_from_chat_id(&ccx_lock.chat_id)
        .ok_or_else(|| "Missing 'task_id' (and chat is not bound to a task)".to_string())
}

pub(crate) async fn planner_bound_task_id(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    args: &HashMap<String, Value>,
) -> Result<String, String> {
    let requested_task_id = match args.get("task_id") {
        Some(value) if value.is_null() => None,
        Some(value) => Some(
            value
                .as_str()
                .ok_or_else(|| "task_id must be a string".to_string())?
                .trim()
                .to_string(),
        ),
        None => None,
    };
    let ccx_lock = ccx.lock().await;
    if let Some(meta) = &ccx_lock.task_meta {
        if meta.role != "planner" {
            return Err(
                "task observability tools can only be called by the task planner.".to_string(),
            );
        }
        if requested_task_id.as_deref().is_some_and(|task_id| task_id != meta.task_id) {
            return Err("task_id override is not allowed from this planner chat".to_string());
        }
        return Ok(meta.task_id.clone());
    }

    let inferred_task_id = storage::infer_task_id_from_chat_id(&ccx_lock.chat_id);
    match (requested_task_id, inferred_task_id) {
        (Some(requested), Some(inferred)) if requested == inferred => Ok(inferred),
        (Some(_), _) => Err("task_id override is not allowed from this planner chat".to_string()),
        (None, Some(inferred)) => Ok(inferred),
        (None, None) => Err("Missing 'task_id' (and chat is not bound to a task)".to_string()),
    }
}

pub struct ToolTaskCheckAgents;

impl ToolTaskCheckAgents {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AgentStatus {
    pub(crate) card_id: String,
    pub(crate) card_title: String,
    pub(crate) agent_chat_id: String,
    pub(crate) column: String,
    pub(crate) priority: String,
    pub(crate) session_state: Option<SessionState>,
    pub(crate) last_status_update: Option<String>,
    pub(crate) last_activity_at: Option<DateTime<Utc>>,
    pub(crate) final_report: Option<String>,
    pub(crate) last_tool_name: Option<String>,
    pub(crate) change_seq: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentReportFormat {
    Compact,
    Summary,
    Detail,
    Delta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum AgentStateKind {
    Running,
    Stuck,
    Failed,
    Done,
    Paused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentSort {
    Priority,
    LastActivity,
    CardId,
    Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentGroupBy {
    Status,
    Priority,
    None,
}

#[derive(Debug, Clone)]
pub(crate) struct AgentStatusQuery {
    format: AgentReportFormat,
    status_filter: Option<HashSet<AgentStateKind>>,
    card_ids: Option<HashSet<String>>,
    priority_filter: Option<HashSet<String>>,
    min_age_minutes: Option<i64>,
    sort: AgentSort,
    limit: usize,
    offset: usize,
    since_seq: Option<u64>,
    group_by: AgentGroupBy,
}

impl AgentStatusQuery {
    fn default_for_format(format: AgentReportFormat) -> Self {
        let default_status_filter = if matches!(
            format,
            AgentReportFormat::Summary | AgentReportFormat::Detail
        ) {
            None
        } else {
            Some(HashSet::from([
                AgentStateKind::Running,
                AgentStateKind::Stuck,
                AgentStateKind::Failed,
            ]))
        };
        Self {
            format,
            status_filter: default_status_filter,
            card_ids: None,
            priority_filter: None,
            min_age_minutes: None,
            sort: AgentSort::Priority,
            limit: DEFAULT_LIMIT,
            offset: 0,
            since_seq: None,
            group_by: AgentGroupBy::Status,
        }
    }
}

pub(crate) fn agent_status_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "task_id": {
                "type": "string",
                "description": "Task ID (optional if chat is bound to a task)"
            },
            "format": {
                "type": "string",
                "enum": ["compact", "summary", "detail", "delta"],
                "description": "Output format. Default: compact"
            },
            "status_filter": {
                "type": "array",
                "items": {"type": "string", "enum": ["running", "stuck", "failed", "done", "paused", "all"]},
                "description": "Statuses to include. Default: running, stuck, failed"
            },
            "card_ids": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Only include these card IDs"
            },
            "priority_filter": {
                "type": "array",
                "items": {"type": "string", "enum": ["P0", "P1", "P2"]},
                "description": "Only include these priorities"
            },
            "min_age_minutes": {
                "type": "number",
                "description": "Only include agents with last activity at least this many minutes ago"
            },
            "sort": {
                "type": "string",
                "enum": ["priority", "last_activity", "card_id", "status"],
                "description": "Sort key. Default: priority"
            },
            "limit": {
                "type": "number",
                "description": "Maximum rows after filtering. Default: 20"
            },
            "offset": {
                "type": "number",
                "description": "Rows to skip after filtering. Default: 0"
            },
            "since_seq": {
                "type": "number",
                "description": "Delta mode: only include cards changed after this sequence"
            },
            "group_by": {
                "type": "string",
                "enum": ["status", "priority", "none"],
                "description": "Grouping hint. Default: status"
            }
        },
        "required": []
    })
}

pub(crate) fn parse_agent_status_query(
    args: &HashMap<String, Value>,
) -> Result<AgentStatusQuery, String> {
    let format = parse_format(args.get("format"))?;
    let mut query = AgentStatusQuery::default_for_format(format);

    if let Some(statuses) = parse_string_list(args, "status_filter")? {
        query.status_filter = parse_status_filter(&statuses)?;
    }
    if let Some(card_ids) = parse_string_list(args, "card_ids")? {
        query.card_ids = Some(card_ids.into_iter().collect());
    }
    if let Some(priorities) = parse_string_list(args, "priority_filter")? {
        query.priority_filter = Some(
            priorities
                .into_iter()
                .map(|p| p.to_ascii_uppercase())
                .map(|p| match p.as_str() {
                    "P0" | "P1" | "P2" => Ok(p),
                    _ => Err(format!("Invalid priority_filter value: {}", p)),
                })
                .collect::<Result<HashSet<_>, _>>()?,
        );
    }
    if let Some(min_age) = parse_i64(args, "min_age_minutes")? {
        query.min_age_minutes = Some(min_age);
    }
    if let Some(sort) = args.get("sort") {
        query.sort = parse_sort(sort)?;
    }
    if let Some(limit) = parse_usize(args, "limit")? {
        query.limit = limit;
    }
    if let Some(offset) = parse_usize(args, "offset")? {
        query.offset = offset;
    }
    if let Some(seq) = parse_u64(args, "since_seq")? {
        query.since_seq = Some(seq);
    }
    if let Some(group_by) = args.get("group_by") {
        query.group_by = parse_group_by(group_by)?;
    }
    if matches!(query.format, AgentReportFormat::Delta) && query.since_seq.is_none() {
        return Err("delta format requires since_seq".to_string());
    }

    Ok(query)
}

fn parse_format(value: Option<&Value>) -> Result<AgentReportFormat, String> {
    match value.and_then(|v| v.as_str()).unwrap_or("compact") {
        "compact" => Ok(AgentReportFormat::Compact),
        "summary" => Ok(AgentReportFormat::Summary),
        "detail" => Ok(AgentReportFormat::Detail),
        "delta" => Ok(AgentReportFormat::Delta),
        other => Err(format!("Invalid format: {}", other)),
    }
}

fn parse_sort(value: &Value) -> Result<AgentSort, String> {
    match value.as_str().unwrap_or("priority") {
        "priority" => Ok(AgentSort::Priority),
        "last_activity" => Ok(AgentSort::LastActivity),
        "card_id" => Ok(AgentSort::CardId),
        "status" => Ok(AgentSort::Status),
        other => Err(format!("Invalid sort: {}", other)),
    }
}

fn parse_group_by(value: &Value) -> Result<AgentGroupBy, String> {
    match value.as_str().unwrap_or("status") {
        "status" => Ok(AgentGroupBy::Status),
        "priority" => Ok(AgentGroupBy::Priority),
        "none" => Ok(AgentGroupBy::None),
        other => Err(format!("Invalid group_by: {}", other)),
    }
}

fn parse_string_list(
    args: &HashMap<String, Value>,
    key: &str,
) -> Result<Option<Vec<String>>, String> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    if let Some(single) = value.as_str() {
        return Ok(Some(vec![single.to_string()]));
    }
    let array = value
        .as_array()
        .ok_or_else(|| format!("{} must be an array of strings", key))?;
    let mut items = Vec::new();
    for item in array {
        let text = item
            .as_str()
            .ok_or_else(|| format!("{} must be an array of strings", key))?;
        items.push(text.to_string());
    }
    Ok(Some(items))
}

fn parse_status_filter(values: &[String]) -> Result<Option<HashSet<AgentStateKind>>, String> {
    let mut result = HashSet::new();
    for value in values {
        match value.as_str() {
            "all" => return Ok(None),
            "running" => {
                result.insert(AgentStateKind::Running);
            }
            "stuck" => {
                result.insert(AgentStateKind::Stuck);
            }
            "failed" => {
                result.insert(AgentStateKind::Failed);
            }
            "done" => {
                result.insert(AgentStateKind::Done);
            }
            "paused" => {
                result.insert(AgentStateKind::Paused);
            }
            other => return Err(format!("Invalid status_filter value: {}", other)),
        };
    }
    Ok(Some(result))
}

fn parse_usize(args: &HashMap<String, Value>, key: &str) -> Result<Option<usize>, String> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let Some(n) = value.as_u64() else {
        return Err(format!("{} must be a non-negative number", key));
    };
    usize::try_from(n)
        .map(Some)
        .map_err(|_| format!("{} is too large", key))
}

fn parse_u64(args: &HashMap<String, Value>, key: &str) -> Result<Option<u64>, String> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_u64()
        .map(Some)
        .ok_or_else(|| format!("{} must be a non-negative number", key))
}

fn parse_i64(args: &HashMap<String, Value>, key: &str) -> Result<Option<i64>, String> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let Some(n) = value.as_i64() else {
        return Err(format!("{} must be a non-negative number", key));
    };
    if n < 0 {
        return Err(format!("{} must be a non-negative number", key));
    }
    Ok(Some(n))
}

pub(crate) async fn get_agent_statuses(
    gcx: Arc<crate::global_context::GlobalContext>,
    chat_facade: Arc<dyn ChatSessionFacade>,
    task_id: &str,
) -> Result<Vec<AgentStatus>, String> {
    let board = storage::load_board(gcx, task_id).await?;

    let mut statuses = Vec::new();

    for card in &board.cards {
        if let Some(agent_chat_id) = &card.agent_chat_id {
            let snapshot = chat_facade.session_snapshot(agent_chat_id).await.ok();
            let session_state = match snapshot.as_ref() {
                Some(snapshot) => Some(snapshot.session_state),
                None => chat_facade.session_state(agent_chat_id).await?,
            };
            let last_tool_name = snapshot
                .as_ref()
                .and_then(|snapshot| last_tool_name_from_messages(&snapshot.messages));
            let last_update = card.status_updates.last();
            let last_status_update_at =
                last_update.and_then(|update| parse_timestamp(&update.timestamp));
            let last_heartbeat_at = card.last_heartbeat_at.as_deref().and_then(parse_timestamp);
            let completed_at = card.completed_at.as_deref().and_then(parse_timestamp);
            let started_at = card.started_at.as_deref().and_then(parse_timestamp);
            let created_at = parse_timestamp(&card.created_at);
            let last_activity_at = latest_timestamp([
                last_heartbeat_at,
                last_status_update_at,
                completed_at,
                started_at,
                created_at,
            ]);
            let change_seq = last_activity_at
                .map(|ts| ts.timestamp().max(0) as u64)
                .unwrap_or(board.rev);
            let last_status_update = last_update.map(|u| format!("{}: {}", u.timestamp, u.message));

            statuses.push(AgentStatus {
                card_id: card.id.clone(),
                card_title: card.title.clone(),
                agent_chat_id: agent_chat_id.clone(),
                column: card.column.clone(),
                priority: card.priority.clone(),
                session_state,
                last_status_update,
                last_activity_at,
                final_report: card.final_report.clone(),
                last_tool_name,
                change_seq,
            });
        }
    }

    Ok(statuses)
}

fn last_tool_name_from_messages(messages: &[ChatMessage]) -> Option<String> {
    messages.iter().rev().find_map(|message| {
        message
            .tool_calls
            .as_ref()
            .and_then(|tool_calls| tool_calls.last())
            .map(|tool_call| tool_call.function.name.clone())
    })
}

fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn latest_timestamp(
    times: impl IntoIterator<Item = Option<DateTime<Utc>>>,
) -> Option<DateTime<Utc>> {
    times.into_iter().flatten().max()
}

fn classify_agent_status(status: &AgentStatus, now: DateTime<Utc>) -> AgentStateKind {
    match status.column.as_str() {
        "done" => AgentStateKind::Done,
        "failed" => AgentStateKind::Failed,
        "doing" => {
            if matches!(status.session_state, Some(SessionState::Error)) {
                return AgentStateKind::Stuck;
            }
            if matches!(status.session_state, Some(SessionState::Completed))
                && status.final_report.is_none()
            {
                return AgentStateKind::Stuck;
            }
            if matches!(
                status.session_state,
                Some(SessionState::Paused | SessionState::WaitingUserInput)
            ) {
                return AgentStateKind::Paused;
            }
            if is_stale(status, now) {
                return AgentStateKind::Stuck;
            }
            AgentStateKind::Running
        }
        _ => AgentStateKind::Running,
    }
}

fn is_stale(status: &AgentStatus, now: DateTime<Utc>) -> bool {
    status
        .last_activity_at
        .map(|last| now.signed_duration_since(last) >= Duration::minutes(STUCK_AFTER_MINUTES))
        .unwrap_or(false)
}

pub(crate) fn has_active_agent_statuses(statuses: &[AgentStatus]) -> bool {
    let now = Utc::now();
    statuses.iter().any(|status| {
        matches!(
            classify_agent_status(status, now),
            AgentStateKind::Running | AgentStateKind::Paused
        )
    })
}

fn format_agent_status_detail_at(status: &AgentStatus, now: DateTime<Utc>) -> String {
    let kind = classify_agent_status(status, now);
    let (state_emoji, state_text) = match kind {
        AgentStateKind::Done => ("✅", "Completed"),
        AgentStateKind::Failed => ("❌", "Failed"),
        AgentStateKind::Stuck => ("🔴", "Stuck / needs attention"),
        AgentStateKind::Paused => ("⏸️", "Paused / awaiting approval"),
        AgentStateKind::Running => match &status.session_state {
            Some(SessionState::Generating) => ("🔄", "Generating response"),
            Some(SessionState::ExecutingTools) => ("⚙️", "Executing tools"),
            Some(SessionState::WaitingIde) => ("⏳", "Waiting for IDE"),
            Some(SessionState::Idle) => ("💤", "Idle (waiting)"),
            None => ("❓", "Unknown/offline"),
            _ => ("🔄", "Running"),
        },
    };

    let mut result = format!(
        "### {} {} ({})\n**Status:** {} | **Column:** {} | **Priority:** {} | **Chat:** `{}`\n",
        state_emoji,
        status.card_title,
        status.card_id,
        state_text,
        status.column,
        status.priority,
        status.agent_chat_id
    );

    if let Some(last_activity) = status.last_activity_at {
        result.push_str(&format!(
            "**Last activity:** {} ({})\n",
            last_activity.to_rfc3339(),
            format_age_ago(now, last_activity)
        ));
    }
    if let Some(tool_name) = &status.last_tool_name {
        result.push_str(&format!("**Last tool:** `{}`\n", tool_name));
    }
    if let Some(report) = &status.final_report {
        let preview = truncate_chars(report, 300);
        result.push_str(&format!("\n**Final Report:**\n{}\n", preview));
    } else if let Some(update) = &status.last_status_update {
        result.push_str(&format!("\n**Last Update:** {}\n", update));
    }

    result
}

pub(crate) fn format_agent_statuses(
    statuses: &[AgentStatus],
    query: &AgentStatusQuery,
) -> Result<String, String> {
    format_agent_statuses_at(statuses, query, Utc::now())
}

fn format_agent_statuses_at(
    statuses: &[AgentStatus],
    query: &AgentStatusQuery,
    now: DateTime<Utc>,
) -> Result<String, String> {
    if matches!(query.format, AgentReportFormat::Delta) && query.since_seq.is_none() {
        return Err("delta format requires since_seq".to_string());
    }

    let alerts = AgentAlerts::from_statuses(statuses, now);
    let mut filtered = filtered_statuses(statuses, query, now);
    sort_statuses(&mut filtered, query.sort, query.group_by, now);

    let total_after_filter = filtered.len();
    let page = paginate_statuses(&filtered, query.offset, query.limit);
    let mut result = format_alerts(&alerts);

    if statuses.is_empty() {
        result.push_str("# Agent Status\n\nNo agents have been spawned yet for this task.\n\nUse `task_spawn_agent(card_id)` to spawn an agent for a card.");
        return Ok(result);
    }

    match query.format {
        AgentReportFormat::Summary => {
            result.push_str(&format_summary(&filtered, now));
        }
        AgentReportFormat::Detail => {
            result.push_str(&format_detail(&page, total_after_filter, now));
            result.push_str(&format_pagination(
                total_after_filter,
                page.len(),
                query.offset,
                query.limit,
            ));
        }
        AgentReportFormat::Compact => {
            result.push_str(&format_compact(&page, total_after_filter, now, None));
            result.push_str(&format_pagination(
                total_after_filter,
                page.len(),
                query.offset,
                query.limit,
            ));
        }
        AgentReportFormat::Delta => {
            let since_seq = query.since_seq.unwrap_or_default();
            result.push_str(&format_compact(
                &page,
                total_after_filter,
                now,
                Some(since_seq),
            ));
            result.push_str(&format_pagination(
                total_after_filter,
                page.len(),
                query.offset,
                query.limit,
            ));
        }
    }

    Ok(result)
}

fn filtered_statuses<'a>(
    statuses: &'a [AgentStatus],
    query: &AgentStatusQuery,
    now: DateTime<Utc>,
) -> Vec<&'a AgentStatus> {
    let mut seen = HashSet::new();
    statuses
        .iter()
        .filter(|status| {
            if !seen.insert(status.card_id.as_str()) {
                return false;
            }
            if matches!(query.format, AgentReportFormat::Delta)
                && status.change_seq <= query.since_seq.unwrap_or_default()
            {
                return false;
            }
            if let Some(card_ids) = &query.card_ids {
                if !card_ids.contains(&status.card_id) {
                    return false;
                }
            }
            if let Some(priorities) = &query.priority_filter {
                if !priorities.contains(&status.priority.to_ascii_uppercase()) {
                    return false;
                }
            }
            if let Some(min_age) = query.min_age_minutes {
                let Some(age) = age_minutes(status, now) else {
                    return false;
                };
                if age < min_age {
                    return false;
                }
            }
            if let Some(status_filter) = &query.status_filter {
                if !status_filter.contains(&classify_agent_status(status, now)) {
                    return false;
                }
            }
            true
        })
        .collect()
}

fn sort_statuses(
    statuses: &mut Vec<&AgentStatus>,
    sort: AgentSort,
    group_by: AgentGroupBy,
    now: DateTime<Utc>,
) {
    statuses.sort_by(|a, b| {
        let group_cmp = match group_by {
            AgentGroupBy::Status => status_rank(classify_agent_status(a, now))
                .cmp(&status_rank(classify_agent_status(b, now))),
            AgentGroupBy::Priority => priority_rank(&a.priority).cmp(&priority_rank(&b.priority)),
            AgentGroupBy::None => std::cmp::Ordering::Equal,
        };
        if !group_cmp.is_eq() {
            return group_cmp;
        }
        let cmp = match sort {
            AgentSort::Priority => priority_rank(&a.priority).cmp(&priority_rank(&b.priority)),
            AgentSort::LastActivity => last_activity_sort_key(a).cmp(&last_activity_sort_key(b)),
            AgentSort::CardId => a.card_id.cmp(&b.card_id),
            AgentSort::Status => status_rank(classify_agent_status(a, now))
                .cmp(&status_rank(classify_agent_status(b, now))),
        };
        cmp.then_with(|| a.card_id.cmp(&b.card_id))
    });
}

fn paginate_statuses<'a>(
    statuses: &[&'a AgentStatus],
    offset: usize,
    limit: usize,
) -> Vec<&'a AgentStatus> {
    statuses.iter().skip(offset).take(limit).copied().collect()
}

fn last_activity_sort_key(status: &AgentStatus) -> i64 {
    status
        .last_activity_at
        .map(|ts| ts.timestamp())
        .unwrap_or_default()
}

fn status_rank(kind: AgentStateKind) -> u8 {
    match kind {
        AgentStateKind::Stuck => 0,
        AgentStateKind::Failed => 1,
        AgentStateKind::Paused => 2,
        AgentStateKind::Running => 3,
        AgentStateKind::Done => 4,
    }
}

fn priority_rank(priority: &str) -> u8 {
    match priority.to_ascii_uppercase().as_str() {
        "P0" => 0,
        "P1" => 1,
        "P2" => 2,
        _ => 3,
    }
}

#[derive(Default)]
struct AgentAlerts {
    stuck: usize,
    failed: usize,
    paused: usize,
}

impl AgentAlerts {
    fn from_statuses(statuses: &[AgentStatus], now: DateTime<Utc>) -> Self {
        let mut alerts = AgentAlerts::default();
        for status in statuses {
            match classify_agent_status(status, now) {
                AgentStateKind::Stuck => alerts.stuck += 1,
                AgentStateKind::Failed => alerts.failed += 1,
                AgentStateKind::Paused => alerts.paused += 1,
                _ => {}
            }
        }
        alerts
    }
}

fn format_alerts(alerts: &AgentAlerts) -> String {
    format!(
        "⚠️  Alerts: {} stuck (>15min), {} failed, {} needing approval\n\n",
        alerts.stuck, alerts.failed, alerts.paused
    )
}

fn format_summary(statuses: &[&AgentStatus], now: DateTime<Utc>) -> String {
    let mut running = 0;
    let mut stuck = 0;
    let mut failed = 0;
    let mut done = 0;
    let mut paused = 0;
    let mut priorities: HashMap<String, usize> = HashMap::new();

    for status in statuses {
        match classify_agent_status(status, now) {
            AgentStateKind::Running => running += 1,
            AgentStateKind::Stuck => stuck += 1,
            AgentStateKind::Failed => failed += 1,
            AgentStateKind::Done => done += 1,
            AgentStateKind::Paused => paused += 1,
        }
        *priorities
            .entry(status.priority.to_ascii_uppercase())
            .or_default() += 1;
    }

    let mut priority_counts = priorities.into_iter().collect::<Vec<_>>();
    priority_counts.sort_by(|(a, _), (b, _)| priority_rank(a).cmp(&priority_rank(b)));
    let priority_text = priority_counts
        .into_iter()
        .map(|(priority, count)| format!("{}={}", priority, count))
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        "Total: {} | Running: {} | Stuck: {} | Failed: {} | Done: {} | Paused: {}\nBy priority: {}\n",
        statuses.len(), running, stuck, failed, done, paused, priority_text
    )
}

fn format_detail(
    statuses: &[&AgentStatus],
    total_after_filter: usize,
    now: DateTime<Utc>,
) -> String {
    if statuses.is_empty() {
        return "# Agent Status Detail\n\nNo agent statuses match the requested filters.\n"
            .to_string();
    }

    let mut result = format!(
        "# Agent Status Detail\n\nShowing {} of {} agents.\n\n",
        statuses.len(),
        total_after_filter
    );
    for status in statuses {
        result.push_str(&format_agent_status_detail_at(status, now));
        result.push_str("\n---\n\n");
    }
    result
}

fn format_compact(
    statuses: &[&AgentStatus],
    total_after_filter: usize,
    now: DateTime<Utc>,
    since_seq: Option<u64>,
) -> String {
    let mut result = String::new();
    if let Some(seq) = since_seq {
        result.push_str(&format!("Delta since seq {}\n\n", seq));
    }
    if statuses.is_empty() {
        if let Some(seq) = since_seq {
            result.push_str(&format!("No agent changes since seq {}.\n", seq));
        } else {
            result.push_str("No agent statuses match the requested filters.\n");
        }
        return result;
    }
    for status in statuses {
        result.push_str(&format_compact_line(status, now));
        result.push('\n');
    }
    if total_after_filter == 0 {
        result.push_str("No agent statuses match the requested filters.\n");
    }
    result
}

fn format_compact_line(status: &AgentStatus, now: DateTime<Utc>) -> String {
    let kind = classify_agent_status(status, now);
    let title = compact_title(&status.card_title, 22);
    let age = status
        .last_activity_at
        .map(|ts| format_age_ago(now, ts))
        .unwrap_or_else(|| "unknown".to_string());
    let short_age = status
        .last_activity_at
        .map(|ts| format_duration_short(now, ts))
        .unwrap_or_else(|| "?".to_string());
    let priority = status.priority.to_ascii_uppercase();
    let last_tool = status.last_tool_name.as_deref().unwrap_or("?");

    match kind {
        AgentStateKind::Stuck => format!(
            "{:<2} 🔴  {:<5} {:<22} | STUCK {:<5} | needs attention",
            priority, status.card_id, title, short_age
        ),
        AgentStateKind::Failed => format!(
            "{:<2} ❌  {:<5} {:<22} | failed     | {}",
            priority, status.card_id, title, age
        ),
        AgentStateKind::Done => format!(
            "{:<2} ✅  {:<5} {:<22} | done       | {}",
            priority, status.card_id, title, age
        ),
        AgentStateKind::Paused => format!(
            "{:<2} ⏸️  {:<5} {:<22} | paused     | {} | approval",
            priority, status.card_id, title, age
        ),
        AgentStateKind::Running => {
            let (emoji, state_text) = match status.session_state {
                Some(SessionState::ExecutingTools) => ("⚙️", "exec tools"),
                Some(SessionState::Generating) => ("🔄", "generating"),
                Some(SessionState::WaitingIde) => ("⏳", "waiting ide"),
                Some(SessionState::Idle) => ("💤", "idle"),
                None => ("❓", "offline"),
                _ => ("🔄", "running"),
            };
            format!(
                "{:<2} {}  {:<5} {:<22} | {:<10} | {:>7} | last: {}",
                priority, emoji, status.card_id, title, state_text, age, last_tool
            )
        }
    }
}

fn compact_title(title: &str, limit: usize) -> String {
    let slug = title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
        .to_ascii_lowercase();
    truncate_chars(&slug, limit)
}

fn truncate_chars(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let take = limit.saturating_sub(1);
    format!("{}…", text.chars().take(take).collect::<String>())
}

fn age_minutes(status: &AgentStatus, now: DateTime<Utc>) -> Option<i64> {
    status
        .last_activity_at
        .map(|last| now.signed_duration_since(last).num_minutes().max(0))
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

fn format_pagination(total: usize, shown: usize, offset: usize, limit: usize) -> String {
    if total == 0 {
        return String::new();
    }
    let next_offset = offset.saturating_add(shown);
    if limit == 0 || next_offset >= total {
        format!("\nshowing {} of {}; no more pages\n", shown, total)
    } else {
        format!(
            "\nshowing {} of {}; pass offset={} for next page\n",
            shown, total, next_offset
        )
    }
}

#[async_trait]
impl Tool for ToolTaskCheckAgents {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_check_agents".to_string(),
            display_name: "Task Check Agents".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Check spawned task agents with compact, summary, detail, or delta output. Supports filters, sorting, pagination, and sticky alerts for stuck, failed, or approval-blocked agents.".to_string(),
            input_schema: agent_status_input_schema(),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let ccx_lock = ccx.lock().await;

        let is_planner = ccx_lock
            .task_meta
            .as_ref()
            .map(|m| m.role == "planner")
            .unwrap_or(false);

        if !is_planner {
            return Err("task_check_agents can only be called by the task planner. \
                 Switch to the planner chat to check agent status."
                .to_string());
        }

        drop(ccx_lock);

        let query = parse_agent_status_query(args)?;
        let task_id = planner_bound_task_id(&ccx, args).await?;
        let (gcx, chat_facade) = {
            let ccx_lock = ccx.lock().await;
            (ccx_lock.app.gcx.clone(), ccx_lock.app.chat.facade.clone())
        };

        let statuses = get_agent_statuses(gcx, chat_facade, &task_id).await?;
        let result = format_agent_statuses(&statuses, &query)?;

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(result),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })],
        ))
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
    use crate::tasks::types::{TaskBoard, TaskMeta as StoredTaskMeta, TaskStatus};
    use crate::tools::tools_description::Tool;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-05-22T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn status(
        id: &str,
        priority: &str,
        column: &str,
        session_state: Option<SessionState>,
        minutes_ago: i64,
    ) -> AgentStatus {
        let ts = now() - Duration::minutes(minutes_ago);
        AgentStatus {
            card_id: id.to_string(),
            card_title: format!("{} title", id),
            agent_chat_id: format!("agent-{}", id),
            column: column.to_string(),
            priority: priority.to_string(),
            session_state,
            last_status_update: Some(format!("{}: last: cat", ts.to_rfc3339())),
            last_activity_at: Some(ts),
            final_report: if column == "done" {
                Some("done".to_string())
            } else {
                None
            },
            last_tool_name: Some("cat".to_string()),
            change_seq: ts.timestamp() as u64,
        }
    }

    fn query(format: AgentReportFormat) -> AgentStatusQuery {
        let mut query = AgentStatusQuery::default_for_format(format);
        query.status_filter = None;
        query.group_by = AgentGroupBy::None;
        query
    }

    fn card_lines(output: &str) -> Vec<&str> {
        output.lines().filter(|line| line.contains("T-")).collect()
    }

    fn args(items: &[(&str, Value)]) -> HashMap<String, Value> {
        items
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect()
    }

    fn task_meta() -> StoredTaskMeta {
        let now = Utc::now().to_rfc3339();
        StoredTaskMeta {
            schema_version: 1,
            id: "task-1".to_string(),
            name: "Task".to_string(),
            status: TaskStatus::Active,
            created_at: now.clone(),
            updated_at: now,
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

    async fn write_empty_task(
        root: &std::path::Path,
    ) -> Arc<crate::global_context::GlobalContext> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let task_dir = root.join(".refact").join("tasks").join("task-1");
        tokio::fs::create_dir_all(&task_dir).await.unwrap();
        tokio::fs::write(
            task_dir.join("meta.yaml"),
            serde_yaml::to_string(&task_meta()).unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::write(
            task_dir.join("board.yaml"),
            serde_yaml::to_string(&TaskBoard::default()).unwrap(),
        )
        .await
        .unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.to_path_buf()];
        gcx
    }

    async fn planner_ccx(
        gcx: Arc<crate::global_context::GlobalContext>,
        role: &str,
    ) -> Arc<AMutex<AtCommandsContext>> {
        Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                AppState::from_gcx(gcx).await,
                4096,
                20,
                false,
                vec![],
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

    fn tool_output_text(result: (bool, Vec<ContextEnum>)) -> String {
        match result.1.into_iter().next().unwrap() {
            ContextEnum::ChatMessage(message) => match message.content {
                ChatContent::SimpleText(text) => text,
                _ => panic!("expected text output"),
            },
            _ => panic!("expected chat message"),
        }
    }

    #[test]
    fn compact_format_renders_one_line_per_agent() {
        let statuses = vec![
            status("T-1", "P0", "doing", Some(SessionState::Generating), 3),
            status("T-2", "P1", "doing", Some(SessionState::ExecutingTools), 4),
        ];
        let output =
            format_agent_statuses_at(&statuses, &query(AgentReportFormat::Compact), now()).unwrap();

        let lines = card_lines(&output);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("T-1"));
        assert!(lines[1].contains("T-2"));
    }

    #[test]
    fn sticky_alerts_always_render_when_attention_agents_exist() {
        let statuses = vec![
            status("T-1", "P0", "doing", Some(SessionState::Idle), 18),
            status("T-2", "P1", "failed", Some(SessionState::Error), 2),
        ];
        let mut query = query(AgentReportFormat::Compact);
        query.card_ids = Some(HashSet::from(["missing".to_string()]));
        let output = format_agent_statuses_at(&statuses, &query, now()).unwrap();

        assert!(output.starts_with("⚠️  Alerts: 1 stuck (>15min), 1 failed, 0 needing approval"));
        assert!(card_lines(&output).is_empty());
    }

    #[test]
    fn filters_by_status_priority_and_card_ids() {
        let statuses = vec![
            status("T-1", "P0", "doing", Some(SessionState::Generating), 2),
            status("T-2", "P1", "done", Some(SessionState::Completed), 4),
            status("T-3", "P1", "failed", Some(SessionState::Error), 6),
        ];
        let mut query = query(AgentReportFormat::Compact);
        query.status_filter = Some(HashSet::from([
            AgentStateKind::Done,
            AgentStateKind::Failed,
        ]));
        query.priority_filter = Some(HashSet::from(["P1".to_string()]));
        query.card_ids = Some(HashSet::from(["T-2".to_string()]));
        let output = format_agent_statuses_at(&statuses, &query, now()).unwrap();

        let lines = card_lines(&output);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("T-2"));
    }

    #[test]
    fn sort_by_priority_orders_p0_before_p1_before_p2() {
        let statuses = vec![
            status("T-2", "P2", "doing", Some(SessionState::Generating), 2),
            status("T-0", "P0", "doing", Some(SessionState::Generating), 2),
            status("T-1", "P1", "doing", Some(SessionState::Generating), 2),
        ];
        let mut query = query(AgentReportFormat::Compact);
        query.sort = AgentSort::Priority;
        let output = format_agent_statuses_at(&statuses, &query, now()).unwrap();
        let lines = card_lines(&output);

        assert!(lines[0].contains("T-0"));
        assert!(lines[1].contains("T-1"));
        assert!(lines[2].contains("T-2"));
    }

    #[test]
    fn pagination_limit_and_offset_work() {
        let statuses = vec![
            status("T-1", "P1", "doing", Some(SessionState::Generating), 2),
            status("T-2", "P1", "doing", Some(SessionState::Generating), 2),
            status("T-3", "P1", "doing", Some(SessionState::Generating), 2),
        ];
        let mut query = query(AgentReportFormat::Compact);
        query.sort = AgentSort::CardId;
        query.limit = 2;
        query.offset = 1;
        let output = format_agent_statuses_at(&statuses, &query, now()).unwrap();
        let lines = card_lines(&output);

        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("T-2"));
        assert!(lines[1].contains("T-3"));
        assert!(output.contains("showing 2 of 3; no more pages"));
    }

    #[test]
    fn delta_mode_returns_only_changed_since_seq() {
        let mut old = status("T-1", "P1", "doing", Some(SessionState::Generating), 30);
        old.change_seq = 10;
        let mut changed = status("T-2", "P1", "doing", Some(SessionState::Generating), 2);
        changed.change_seq = 20;
        let mut query = query(AgentReportFormat::Delta);
        query.since_seq = Some(15);
        let output = format_agent_statuses_at(&[old, changed], &query, now()).unwrap();
        let lines = card_lines(&output);

        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("T-2"));
        assert!(!output.contains("T-1"));
    }

    #[tokio::test]
    async fn task_check_agents_rejects_mismatched_task_id() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let ccx = planner_ccx(gcx, "planner").await;

        let err = ToolTaskCheckAgents::new()
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[("task_id", json!("task-2"))]),
            )
            .await
            .unwrap_err();

        assert_eq!(err, "task_id override is not allowed from this planner chat");
    }

    #[tokio::test]
    async fn task_check_agents_allows_same_task_id() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = write_empty_task(temp.path()).await;
        let ccx = planner_ccx(gcx, "planner").await;

        let output = tool_output_text(
            ToolTaskCheckAgents::new()
                .tool_execute(
                    ccx,
                    &"call".to_string(),
                    &args(&[("task_id", json!("task-1"))]),
                )
                .await
                .unwrap(),
        );

        assert!(output.contains("No agents have been spawned yet for this task"));
    }
}
