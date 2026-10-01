use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;
use tokio::task::JoinSet;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::global_context::GlobalContext;
use crate::tasks::storage;
use crate::tasks::types::{BoardCard, TaskBoard};
use crate::tasks::overlap_order::WaveOrder;
use crate::tasks::waves::{
    plan_schedule_with_overlap, Lane, LaneKind, LaneOutcome, LaneStage, Wave, WaveLimits,
    WaveSchedule,
};
#[cfg(test)]
use crate::tasks::waves::plan_schedule;
use crate::tools::tool_task_board::ToolTaskBoardCreateCard;
use crate::tools::tool_task_mark_card::{ToolTaskMarkCardDone, ToolTaskMarkCardFailed};
use crate::tools::tool_task_merge_agent::ToolTaskMergeAgent;
use crate::tools::tool_task_spawn_agent::ToolTaskSpawnAgent;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

const MAX_SPAWN_BATCH: usize = 10;
const MAX_MERGE_BATCH: usize = 10;
const MAX_CREATE_BATCH: usize = 30;

fn make_source() -> ToolSource {
    ToolSource {
        source_type: ToolSourceType::Builtin,
        config_path: String::new(),
    }
}

async fn planner_context(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    args: &HashMap<String, Value>,
    tool_name: &str,
) -> Result<(Arc<GlobalContext>, String), String> {
    let ccx_lock = ccx.lock().await;
    let is_planner = ccx_lock
        .task_meta
        .as_ref()
        .map(|m| m.role == "planner")
        .unwrap_or(false);
    if !is_planner {
        return Err(format!(
            "{} can only be called by the task planner.",
            tool_name
        ));
    }
    let explicit_task_id = match args.get("task_id") {
        None => None,
        Some(Value::String(task_id)) if !task_id.trim().is_empty() => Some(task_id.clone()),
        Some(Value::String(_)) => return Err("'task_id' must be a non-empty string".to_string()),
        Some(_) => return Err("'task_id' must be a string".to_string()),
    };
    let bound_task_id = ccx_lock
        .task_meta
        .as_ref()
        .map(|m| m.task_id.as_str())
        .filter(|s| !s.is_empty());
    if let (Some(explicit), Some(bound)) = (explicit_task_id.as_deref(), bound_task_id) {
        if explicit != bound {
            return Err(format!(
                "task_id '{}' does not match planner's bound task '{}'",
                explicit, bound
            ));
        }
    }
    let task_id = explicit_task_id
        .or_else(|| bound_task_id.map(str::to_string))
        .or_else(|| storage::infer_task_id_from_chat_id(&ccx_lock.chat_id))
        .ok_or_else(|| "Missing 'task_id' (and chat is not bound to a task)".to_string())?;
    Ok((ccx_lock.app.gcx.clone(), task_id))
}

fn batch_tool_message<T: Serialize>(
    tool_call_id: &str,
    value: &T,
) -> Result<(bool, Vec<ContextEnum>), String> {
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    Ok((
        false,
        vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(text),
            tool_calls: None,
            tool_call_id: tool_call_id.to_string(),
            ..Default::default()
        })],
    ))
}

fn array_arg<'a>(args: &'a HashMap<String, Value>, key: &str) -> Result<&'a Vec<Value>, String> {
    args.get(key)
        .and_then(|v| v.as_array())
        .ok_or_else(|| format!("Missing '{}' array", key))
}

fn item_name(value: &Value, index: usize) -> String {
    value
        .get("card_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("<item {}>", index + 1))
}

fn required_string(value: &Value, key: &str) -> Result<String, String> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("Missing '{}'", key))
}

fn optional_string(value: &Value, key: &str, default: &str) -> Result<String, String> {
    match value.get(key) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(Value::Null) | None => Ok(default.to_string()),
        Some(_) => Err(format!("'{}' must be a string", key)),
    }
}

fn optional_usize(value: &Value, key: &str) -> Result<Option<usize>, String> {
    match value.get(key) {
        Some(Value::Number(n)) => n
            .as_u64()
            .map(|n| Some(n as usize))
            .ok_or_else(|| format!("'{}' must be a non-negative integer", key)),
        Some(Value::String(s)) => s
            .parse::<usize>()
            .map(Some)
            .map_err(|_| format!("'{}' must be a non-negative integer", key)),
        Some(Value::Null) | None => Ok(None),
        Some(_) => Err(format!("'{}' must be a non-negative integer", key)),
    }
}

fn optional_string_vec(value: Option<&Value>, key: &str) -> Result<Vec<String>, String> {
    match value {
        Some(Value::Array(arr)) => arr
            .iter()
            .map(|v| {
                v.as_str()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| format!("'{}' entries must be non-empty strings", key))
            })
            .collect(),
        Some(Value::String(s)) => Ok(s
            .split([',', '\n'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()),
        Some(Value::Null) | None => Ok(Vec::new()),
        Some(_) => Err(format!("'{}' must be an array of strings or a string", key)),
    }
}

fn usize_arg(args: &HashMap<String, Value>, key: &str, default: usize) -> Result<usize, String> {
    match args.get(key) {
        Some(Value::Number(n)) => n
            .as_u64()
            .map(|n| n as usize)
            .ok_or_else(|| format!("'{}' must be a non-negative integer", key)),
        Some(Value::String(s)) => s
            .parse::<usize>()
            .map_err(|_| format!("'{}' must be a non-negative integer", key)),
        Some(Value::Null) | None => Ok(default),
        Some(_) => Err(format!("'{}' must be a non-negative integer", key)),
    }
}

fn auto_revert_arg(args: &HashMap<String, Value>) -> Result<bool, String> {
    match args.get("auto_revert") {
        None => Ok(false),
        Some(value) => refact_tool_api::coerce_bool(value)
            .ok_or_else(|| "'auto_revert' must be a boolean".to_string()),
    }
}

fn auto_revert_timeout_secs_arg(args: &HashMap<String, Value>) -> Result<u64, String> {
    let Some(value) = args.get("auto_revert_timeout_secs") else {
        return Ok(crate::chat::post_merge_check::DEFAULT_POST_MERGE_CHECK_TIMEOUT_SECS);
    };
    let secs = match value {
        Value::Number(number) => number
            .as_u64()
            .ok_or_else(|| "auto_revert_timeout_secs must be a positive integer".to_string())?,
        _ => return Err("auto_revert_timeout_secs must be a positive integer".to_string()),
    };
    if secs == 0 {
        return Err("auto_revert_timeout_secs must be greater than zero".to_string());
    }
    Ok(secs)
}

#[derive(Clone, Debug)]
struct SpawnBatchItem {
    card_id: String,
    suggested_steps: Option<usize>,
    files_to_open: Vec<String>,
    base_branch: Option<String>,
    base_commit: Option<String>,
}

#[derive(Clone, Debug)]
struct BoardCreateBatchItem {
    card_id: String,
    title: String,
    priority: String,
    instructions: String,
    depends_on: Vec<String>,
    target_files: Vec<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
struct SpawnBatchResult {
    card_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_chat_id: Option<String>,
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
struct ScheduledSpawnBatchResult {
    waves: Vec<Wave>,
    #[serde(skip_serializing_if = "Option::is_none")]
    admitted_wave: Option<usize>,
    lanes: Vec<LaneOutcome>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    overlap_order: Vec<WaveOrder>,
    results: Vec<SpawnBatchResult>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
struct MergeBatchResult {
    card_id: String,
    merged: bool,
    conflict: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
struct SimpleBatchResult {
    card_id: String,
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn parse_spawn_item(value: &Value) -> Result<SpawnBatchItem, String> {
    if !value.is_object() {
        return Err("spawn item must be an object".to_string());
    }
    Ok(SpawnBatchItem {
        card_id: required_string(value, "card_id")?,
        suggested_steps: optional_usize(value, "suggested_steps")?,
        files_to_open: optional_string_vec(value.get("files_to_open"), "files_to_open")?,
        base_branch: optional_nonempty_string(value, "base_branch")?,
        base_commit: optional_nonempty_string(value, "base_commit")?,
    })
}

fn optional_nonempty_string(value: &Value, key: &str) -> Result<Option<String>, String> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.trim().is_empty() => {
            Ok(Some(value.trim().to_string()))
        }
        Some(Value::String(_)) => Err(format!("'{}' must be a non-empty string", key)),
        Some(_) => Err(format!("'{}' must be a string", key)),
    }
}

fn parse_board_create_item(value: &Value) -> Result<BoardCreateBatchItem, String> {
    if !value.is_object() {
        return Err("card item must be an object".to_string());
    }
    Ok(BoardCreateBatchItem {
        card_id: required_string(value, "card_id")?,
        title: required_string(value, "title")?,
        priority: optional_string(value, "priority", "P1")?,
        instructions: optional_string(value, "instructions", "")?,
        depends_on: optional_string_vec(value.get("depends_on"), "depends_on")?,
        target_files: optional_string_vec(value.get("target_files"), "target_files")?,
    })
}

fn validate_spawn_agent_cards(
    board: &TaskBoard,
    items: &[Result<SpawnBatchItem, String>],
) -> Vec<Option<String>> {
    let ready: HashSet<String> = board.get_ready_cards().ready.into_iter().collect();
    let mut errors: Vec<Option<String>> = items
        .iter()
        .map(|item| item.as_ref().err().cloned())
        .collect();
    let mut seen: HashMap<String, usize> = HashMap::new();

    for (idx, item) in items.iter().enumerate() {
        let Ok(item) = item else {
            continue;
        };
        if let Some(prev) = seen.insert(item.card_id.clone(), idx) {
            let message = format!("Duplicate card_id {} in batch", item.card_id);
            if errors[prev].is_none() {
                errors[prev] = Some(message.clone());
            }
            errors[idx] = Some(message);
            continue;
        }
        let Some(card) = board.get_card(&item.card_id) else {
            errors[idx] = Some(format!("Card {} not found", item.card_id));
            continue;
        };
        if card.column != "planned" {
            errors[idx] = Some(format!(
                "Card {} is in column '{}', expected ready planned card",
                item.card_id, card.column
            ));
            continue;
        }
        if !ready.contains(&item.card_id) {
            errors[idx] = Some(format!(
                "Card {} dependencies are not satisfied",
                item.card_id
            ));
        }
    }

    errors
}

fn optional_bool(value: &Value, key: &str) -> Result<bool, String> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(false),
        Some(value) => refact_tool_api::coerce_bool(value)
            .ok_or_else(|| format!("'{}' must be a boolean", key)),
    }
}

fn parse_lane_stage(value: &Value) -> Result<LaneStage, String> {
    if !value.is_object() {
        return Err("lane stage must be an object".to_string());
    }
    Ok(LaneStage {
        card_id: required_string(value, "card_id")?,
        agent_id: optional_nonempty_string(value, "agent_id")?,
        optional: optional_bool(value, "optional")?,
    })
}

fn parse_lane(value: &Value) -> Result<Lane, String> {
    if !value.is_object() {
        return Err("lane must be an object".to_string());
    }
    let stages = value
        .get("stages")
        .and_then(|value| value.as_array())
        .ok_or_else(|| "Missing 'stages' array".to_string())?
        .iter()
        .map(parse_lane_stage)
        .collect::<Result<Vec<LaneStage>, String>>()?;
    Ok(Lane {
        name: required_string(value, "name")?,
        kind: LaneKind::parse(&required_string(value, "kind")?)?,
        stages,
        optional: optional_bool(value, "optional")?,
    })
}

fn parse_schedule(args: &HashMap<String, Value>) -> Result<Option<Vec<Lane>>, String> {
    match args.get("schedule") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(schedule)) => {
            let lanes = schedule
                .get("lanes")
                .and_then(|value| value.as_array())
                .ok_or_else(|| "'schedule' requires a 'lanes' array".to_string())?;
            if lanes.is_empty() {
                return Err("'schedule.lanes' cannot be empty".to_string());
            }
            lanes
                .iter()
                .map(parse_lane)
                .collect::<Result<Vec<Lane>, String>>()
                .map(Some)
        }
        Some(_) => Err("'schedule' must be an object".to_string()),
    }
}

/// Result to record for a card before anything is spawned.
///
/// `None` means the card is left to the wave schedule to explain. A card that
/// never parsed is always reported, because a malformed entry cannot be
/// scheduled and must not disappear from the output.
fn pre_dispatch_result(
    fallback_name: &str,
    parsed: &Result<SpawnBatchItem, String>,
    validation_error: Option<&str>,
    is_selected: impl Fn(&str) -> bool,
) -> Option<SpawnBatchResult> {
    let card_id = match parsed {
        Ok(item) if !is_selected(&item.card_id) => return None,
        Ok(item) => item.card_id.clone(),
        Err(_) => fallback_name.to_string(),
    };
    validation_error.map(|error| SpawnBatchResult {
        card_id,
        agent_chat_id: None,
        success: false,
        error: Some(error.to_string()),
    })
}

fn wave_limits() -> WaveLimits {
    let settings = refact_work_core::coordination_settings();
    WaveLimits {
        max_wave_width: settings.max_wave_width,
        max_pipeline_depth: settings.max_pipeline_depth,
    }
}

/// Batch indices to submit, in the order the schedule wants them submitted.
///
/// One batch is spawned at a time, so two cards known to touch the same file
/// never enter the batch together. A schedule that ranked nothing yields a
/// single batch, which is exactly the unordered behaviour.
fn submission_batches(
    parsed: &[Result<SpawnBatchItem, String>],
    eligible: &[usize],
    schedule: Option<&WaveSchedule>,
) -> Vec<Vec<usize>> {
    let groups = match schedule.map(WaveSchedule::submission_groups) {
        Some(groups) if groups.len() > 1 => groups,
        _ => return vec![eligible.to_vec()],
    };
    groups
        .iter()
        .map(|group| {
            eligible
                .iter()
                .copied()
                .filter(|idx| {
                    parsed[*idx]
                        .as_ref()
                        .is_ok_and(|item| group.contains(&item.card_id))
                })
                .collect::<Vec<usize>>()
        })
        .filter(|group| !group.is_empty())
        .collect()
}

fn reconcile_schedule_with_cards(
    lanes: &[Lane],
    parsed: &[Result<SpawnBatchItem, String>],
) -> Result<(), String> {
    let requested: HashSet<&str> = parsed
        .iter()
        .filter_map(|item| item.as_ref().ok())
        .map(|item| item.card_id.as_str())
        .collect();
    let mut placed = HashSet::new();
    for lane in lanes {
        for stage in &lane.stages {
            if !requested.contains(stage.card_id.as_str()) {
                return Err(format!(
                    "schedule lane '{}' stages card {} which is not listed in 'cards'",
                    lane.name, stage.card_id
                ));
            }
            placed.insert(stage.card_id.as_str());
        }
    }
    for item in parsed.iter().filter_map(|item| item.as_ref().ok()) {
        if !placed.contains(item.card_id.as_str()) {
            return Err(format!(
                "card {} is not placed in any schedule lane",
                item.card_id
            ));
        }
    }
    Ok(())
}

fn parse_mark_item(value: &Value, text_key: &str) -> Result<(String, String), String> {
    if !value.is_object() {
        return Err("item must be an object".to_string());
    }
    Ok((
        required_string(value, "card_id")?,
        required_string(value, text_key)?,
    ))
}

fn dependency_ordered_done_agent_cards(board: &TaskBoard, max_merges: usize) -> Vec<String> {
    let candidates: HashSet<String> = board
        .cards
        .iter()
        .filter(|card| card.column == "done" && card.agent_branch.is_some())
        .map(|card| card.id.clone())
        .collect();
    let card_by_id: HashMap<String, &BoardCard> = board
        .cards
        .iter()
        .map(|card| (card.id.clone(), card))
        .collect();
    let mut ordered = Vec::new();
    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();

    fn visit(
        id: &str,
        candidates: &HashSet<String>,
        card_by_id: &HashMap<String, &BoardCard>,
        visiting: &mut HashSet<String>,
        visited: &mut HashSet<String>,
        ordered: &mut Vec<String>,
    ) {
        if visited.contains(id) || !candidates.contains(id) || visiting.contains(id) {
            return;
        }
        visiting.insert(id.to_string());
        if let Some(card) = card_by_id.get(id) {
            for dep in &card.depends_on {
                visit(dep, candidates, card_by_id, visiting, visited, ordered);
            }
        }
        visiting.remove(id);
        visited.insert(id.to_string());
        ordered.push(id.to_string());
    }

    for card in &board.cards {
        if candidates.contains(&card.id) {
            visit(
                &card.id,
                &candidates,
                &card_by_id,
                &mut visiting,
                &mut visited,
                &mut ordered,
            );
        }
    }
    ordered.truncate(max_merges);
    ordered
}

fn contexts_text(contexts: &[ContextEnum]) -> String {
    contexts
        .iter()
        .map(|ctx| match ctx {
            ContextEnum::ChatMessage(message) => message.content.content_text_only(),
            ContextEnum::ContextFile(file) => file.file_content.clone(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn merge_result_from_tool(
    card_id: String,
    result: Result<(bool, Vec<ContextEnum>), String>,
) -> MergeBatchResult {
    match result {
        Ok((_, contexts)) => {
            let text = contexts_text(&contexts);
            let conflict = text.contains("Merge Conflicts Detected")
                || text.contains("Merge Already In Progress");
            let auto_reverted = text.contains("**Auto-reverted:** true");
            let merged = text.contains("Agent Work Merged") && !auto_reverted;
            MergeBatchResult {
                card_id,
                merged,
                conflict,
                error: auto_reverted.then(|| {
                    "Merge was automatically reverted after post-merge verification failed"
                        .to_string()
                }),
            }
        }
        Err(error) => MergeBatchResult {
            card_id,
            merged: false,
            conflict: false,
            error: Some(error),
        },
    }
}

fn should_continue_merges(result: &MergeBatchResult, stop_on_conflict: bool) -> bool {
    result.merged && !(stop_on_conflict && result.conflict)
}

fn find_cycle_ids(items: &[BoardCreateBatchItem], active_ids: &HashSet<String>) -> HashSet<String> {
    let graph: HashMap<String, Vec<String>> = items
        .iter()
        .filter(|item| active_ids.contains(&item.card_id))
        .map(|item| {
            (
                item.card_id.clone(),
                item.depends_on
                    .iter()
                    .filter(|dep| active_ids.contains(*dep))
                    .cloned()
                    .collect(),
            )
        })
        .collect();
    let mut state: HashMap<String, u8> = HashMap::new();
    let mut stack: Vec<String> = Vec::new();
    let mut cycles = HashSet::new();

    fn dfs(
        id: &str,
        graph: &HashMap<String, Vec<String>>,
        state: &mut HashMap<String, u8>,
        stack: &mut Vec<String>,
        cycles: &mut HashSet<String>,
    ) {
        state.insert(id.to_string(), 1);
        stack.push(id.to_string());
        if let Some(deps) = graph.get(id) {
            for dep in deps {
                match state.get(dep).copied().unwrap_or(0) {
                    0 => dfs(dep, graph, state, stack, cycles),
                    1 => {
                        if let Some(pos) = stack.iter().position(|v| v == dep) {
                            for node in &stack[pos..] {
                                cycles.insert(node.clone());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        stack.pop();
        state.insert(id.to_string(), 2);
    }

    for id in graph.keys() {
        if state.get(id).copied().unwrap_or(0) == 0 {
            dfs(id, &graph, &mut state, &mut stack, &mut cycles);
        }
    }

    cycles
}

fn propagate_invalid_dependencies(
    items: &[Result<BoardCreateBatchItem, String>],
    errors: &mut [Option<String>],
) {
    loop {
        let invalid_ids: HashSet<String> = items
            .iter()
            .enumerate()
            .filter(|(idx, item)| errors[*idx].is_some() && item.is_ok())
            .filter_map(|(_, item)| item.as_ref().ok().map(|item| item.card_id.clone()))
            .collect();
        let mut changed = false;
        for (idx, item) in items.iter().enumerate() {
            if errors[idx].is_some() {
                continue;
            }
            let Ok(item) = item else {
                continue;
            };
            if let Some(dep) = item
                .depends_on
                .iter()
                .find(|dep| invalid_ids.contains(*dep))
            {
                errors[idx] = Some(format!("Dependency {} is invalid in this batch", dep));
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

fn validate_board_create_batch(
    board: &TaskBoard,
    items: &[Result<BoardCreateBatchItem, String>],
) -> Vec<Option<String>> {
    let mut errors: Vec<Option<String>> = items
        .iter()
        .map(|item| item.as_ref().err().cloned())
        .collect();
    let existing_ids: HashSet<String> = board.cards.iter().map(|card| card.id.clone()).collect();
    let mut batch_ids = HashSet::new();
    let mut seen: HashMap<String, usize> = HashMap::new();

    for (idx, item) in items.iter().enumerate() {
        let Ok(item) = item else {
            continue;
        };
        batch_ids.insert(item.card_id.clone());
        if existing_ids.contains(&item.card_id) {
            errors[idx] = Some(format!("Card {} already exists", item.card_id));
        }
        if let Some(prev) = seen.insert(item.card_id.clone(), idx) {
            let message = format!("Duplicate card_id {} in batch", item.card_id);
            if errors[prev].is_none() {
                errors[prev] = Some(message.clone());
            }
            errors[idx] = Some(message);
        }
    }

    for (idx, item) in items.iter().enumerate() {
        if errors[idx].is_some() {
            continue;
        }
        let Ok(item) = item else {
            continue;
        };
        for dep in &item.depends_on {
            if !existing_ids.contains(dep) && !batch_ids.contains(dep) {
                errors[idx] = Some(format!(
                    "Dependency {} does not exist and is not in this batch",
                    dep
                ));
                break;
            }
        }
    }

    propagate_invalid_dependencies(items, &mut errors);

    let mut combined_items: Vec<BoardCreateBatchItem> = board
        .cards
        .iter()
        .map(|card| BoardCreateBatchItem {
            card_id: card.id.clone(),
            title: card.title.clone(),
            priority: card.priority.clone(),
            instructions: card.instructions.clone(),
            depends_on: card.depends_on.clone(),
            target_files: card.target_files.clone(),
        })
        .collect();
    combined_items.extend(
        items
            .iter()
            .enumerate()
            .filter(|(idx, item)| errors[*idx].is_none() && item.is_ok())
            .filter_map(|(_, item)| item.as_ref().ok().cloned()),
    );
    let combined_ids: HashSet<String> = combined_items
        .iter()
        .map(|item| item.card_id.clone())
        .collect();
    let cycle_ids = find_cycle_ids(&combined_items, &combined_ids);
    if !cycle_ids.is_empty() {
        for (idx, item) in items.iter().enumerate() {
            if errors[idx].is_none() {
                if let Ok(item) = item {
                    if cycle_ids.contains(&item.card_id) {
                        errors[idx] = Some("Dependency cycle detected".to_string());
                    }
                }
            }
        }
    }

    propagate_invalid_dependencies(items, &mut errors);
    errors
}

fn board_create_order(
    items: &[Result<BoardCreateBatchItem, String>],
    errors: &[Option<String>],
) -> Vec<usize> {
    let mut remaining: HashSet<String> = items
        .iter()
        .enumerate()
        .filter(|(idx, item)| errors[*idx].is_none() && item.is_ok())
        .filter_map(|(_, item)| item.as_ref().ok().map(|item| item.card_id.clone()))
        .collect();
    let mut order = Vec::new();

    while !remaining.is_empty() {
        let next = items.iter().enumerate().find_map(|(idx, item)| {
            if errors[idx].is_some() {
                return None;
            }
            let item = item.as_ref().ok()?;
            if !remaining.contains(&item.card_id) {
                return None;
            }
            if item.depends_on.iter().all(|dep| !remaining.contains(dep)) {
                Some((idx, item.card_id.clone()))
            } else {
                None
            }
        });
        let Some((idx, id)) = next else {
            break;
        };
        remaining.remove(&id);
        order.push(idx);
    }

    order
}

fn board_create_args(task_id: &str, item: &BoardCreateBatchItem) -> HashMap<String, Value> {
    let mut args = HashMap::new();
    args.insert("task_id".to_string(), json!(task_id));
    args.insert("card_id".to_string(), json!(item.card_id));
    args.insert("title".to_string(), json!(item.title));
    args.insert("priority".to_string(), json!(item.priority));
    args.insert("instructions".to_string(), json!(item.instructions));
    args.insert("depends_on".to_string(), json!(item.depends_on));
    args.insert("target_files".to_string(), json!(item.target_files));
    args
}

fn spawn_args(task_id: &str, item: &SpawnBatchItem) -> HashMap<String, Value> {
    let mut args = HashMap::new();
    args.insert("task_id".to_string(), json!(task_id));
    args.insert("card_id".to_string(), json!(item.card_id));
    if let Some(suggested_steps) = item.suggested_steps {
        args.insert("suggested_steps".to_string(), json!(suggested_steps));
    }
    if !item.files_to_open.is_empty() {
        args.insert("files_to_open".to_string(), json!(item.files_to_open));
    }
    if let Some(base_branch) = item.base_branch.as_ref() {
        args.insert("base_branch".to_string(), json!(base_branch));
    }
    if let Some(base_commit) = item.base_commit.as_ref() {
        args.insert("base_commit".to_string(), json!(base_commit));
    }
    args
}

fn mark_args(task_id: &str, card_id: &str, key: &str, text: &str) -> HashMap<String, Value> {
    let mut args = HashMap::new();
    args.insert("task_id".to_string(), json!(task_id));
    args.insert("card_id".to_string(), json!(card_id));
    args.insert(key.to_string(), json!(text));
    args
}

fn merge_args(
    task_id: &str,
    card_id: &str,
    strategy: &str,
    auto_revert: bool,
    auto_revert_timeout_secs: u64,
) -> HashMap<String, Value> {
    HashMap::from([
        ("task_id".to_string(), json!(task_id)),
        ("card_id".to_string(), json!(card_id)),
        ("strategy".to_string(), json!(strategy)),
        ("auto_revert".to_string(), json!(auto_revert)),
        (
            "auto_revert_timeout_secs".to_string(),
            json!(auto_revert_timeout_secs),
        ),
    ])
}

pub struct ToolSpawnAgentsBatch;
pub struct ToolMergeReadyInOrder;
pub struct ToolMarkDoneBatch;
pub struct ToolMarkFailedBatch;
pub struct ToolBoardCreateBatch;

impl ToolSpawnAgentsBatch {
    pub fn new() -> Self {
        Self
    }
}

impl ToolMergeReadyInOrder {
    pub fn new() -> Self {
        Self
    }
}

impl ToolMarkDoneBatch {
    pub fn new() -> Self {
        Self
    }
}

impl ToolMarkFailedBatch {
    pub fn new() -> Self {
        Self
    }
}

impl ToolBoardCreateBatch {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ToolSpawnAgentsBatch {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "spawn_agents_batch".to_string(),
            display_name: "Spawn Agents Batch".to_string(),
            source: make_source(),
            experimental: false,
            allow_parallel: false,
            description: "Spawn task agents for up to 10 ready planned cards in parallel. Pass 'schedule' to order submission into dependency waves with explicit parallel, sequential and pipeline lanes."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "Task UUID (optional in planner context)" },
                    "cards": {
                        "type": "array",
                        "maxItems": MAX_SPAWN_BATCH,
                        "items": {
                            "type": "object",
                            "properties": {
                                "card_id": { "type": "string" },
                                "suggested_steps": { "type": "integer" },
                                "files_to_open": { "type": "array", "items": { "type": "string" } },
                                "base_branch": { "type": "string" },
                                "base_commit": { "type": "string" }
                            },
                            "required": ["card_id"]
                        }
                    },
                    "schedule": {
                        "type": "object",
                        "description": "Optional. Waves are derived from the dependency DAG, never declared: a wave only admits once every required card in the previous wave is terminal. Every card in 'cards' must be placed in exactly one lane stage.",
                        "properties": {
                            "lanes": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "name": { "type": "string" },
                                        "kind": { "type": "string", "enum": LaneKind::NAMES },
                                        "optional": { "type": "boolean", "default": false, "description": "Optional lanes never hold a wave open" },
                                        "stages": {
                                            "type": "array",
                                            "description": "Ordered positions. A pipeline advances by position, so the same agent_id may appear more than once.",
                                            "items": {
                                                "type": "object",
                                                "properties": {
                                                    "card_id": { "type": "string" },
                                                    "agent_id": { "type": "string" },
                                                    "optional": { "type": "boolean", "default": false }
                                                },
                                                "required": ["card_id"]
                                            }
                                        }
                                    },
                                    "required": ["name", "kind", "stages"]
                                }
                            }
                        },
                        "required": ["lanes"]
                    }
                },
                "required": ["cards"]
            }),
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
        let (gcx, task_id) = planner_context(&ccx, args, "spawn_agents_batch").await?;
        let raw_items = array_arg(args, "cards")?;
        if raw_items.len() > MAX_SPAWN_BATCH {
            return Err(format!(
                "spawn_agents_batch accepts at most {} cards",
                MAX_SPAWN_BATCH
            ));
        }

        let parsed: Vec<Result<SpawnBatchItem, String>> =
            raw_items.iter().map(parse_spawn_item).collect();
        let board = storage::load_board(gcx.clone(), &task_id).await?;
        let schedule = match parse_schedule(args)? {
            None => None,
            Some(lanes) => {
                reconcile_schedule_with_cards(&lanes, &parsed)?;
                let overlap = match crate::tasks::overlap::overlap_ordering_enabled() {
                    true => Some(crate::tasks::overlap::overlap_inputs(gcx.clone(), &board).await),
                    false => None,
                };
                Some(plan_schedule_with_overlap(
                    &board,
                    &lanes,
                    &wave_limits(),
                    overlap.as_ref(),
                )?)
            }
        };
        let selected: Option<HashSet<String>> = schedule
            .as_ref()
            .map(|schedule| schedule.dispatch.iter().cloned().collect());
        let is_selected = |card_id: &str| {
            selected
                .as_ref()
                .map(|selected| selected.contains(card_id))
                .unwrap_or(true)
        };
        let validation_errors = validate_spawn_agent_cards(&board, &parsed);
        let mut results: Vec<Option<SpawnBatchResult>> = raw_items
            .iter()
            .enumerate()
            .map(|(idx, value)| {
                pre_dispatch_result(
                    &item_name(value, idx),
                    &parsed[idx],
                    validation_errors[idx].as_deref(),
                    &is_selected,
                )
            })
            .collect();
        let eligible: Vec<usize> = (0..parsed.len())
            .filter(|idx| validation_errors[*idx].is_none())
            .filter(|idx| {
                parsed[*idx]
                    .as_ref()
                    .is_ok_and(|item| is_selected(&item.card_id))
            })
            .collect();
        let mut dispatched: Vec<usize> = Vec::new();

        for group in submission_batches(&parsed, &eligible, schedule.as_ref()) {
            let mut joins = JoinSet::new();
            for idx in group {
                let item = parsed[idx].as_ref().map_err(|e| e.clone())?.clone();
                dispatched.push(idx);
                let ccx_clone = ccx.clone();
                let gcx_clone = gcx.clone();
                let task_id_clone = task_id.clone();
                let call_id = format!("{}:{}", tool_call_id, item.card_id);
                joins.spawn(async move {
                    let mut tool = ToolTaskSpawnAgent::new();
                    let card_id = item.card_id.clone();
                    let single_args = spawn_args(&task_id_clone, &item);
                    let result = tool.tool_execute(ccx_clone, &call_id, &single_args).await;
                    match result {
                        Ok(_) => {
                            let agent_chat_id = storage::load_board(gcx_clone, &task_id_clone)
                                .await
                                .ok()
                                .and_then(|board| {
                                    board
                                        .get_card(&card_id)
                                        .and_then(|card| card.agent_chat_id.clone())
                                });
                            (
                                idx,
                                SpawnBatchResult {
                                    card_id,
                                    agent_chat_id,
                                    success: true,
                                    error: None,
                                },
                            )
                        }
                        Err(error) => (
                            idx,
                            SpawnBatchResult {
                                card_id,
                                agent_chat_id: None,
                                success: false,
                                error: Some(error),
                            },
                        ),
                    }
                });
            }

            while let Some(joined) = joins.join_next().await {
                match joined {
                    Ok((idx, result)) => results[idx] = Some(result),
                    Err(error) => {
                        let idx = dispatched
                            .iter()
                            .copied()
                            .find(|idx| results[*idx].is_none())
                            .unwrap_or(0);
                        results[idx] = Some(SpawnBatchResult {
                            card_id: item_name(&raw_items[idx], idx),
                            agent_chat_id: None,
                            success: false,
                            error: Some(format!("spawn task failed: {}", error)),
                        });
                    }
                }
            }
        }

        let results: Vec<SpawnBatchResult> = results.into_iter().flatten().collect();
        match schedule {
            None => batch_tool_message(tool_call_id, &results),
            Some(schedule) => batch_tool_message(
                tool_call_id,
                &ScheduledSpawnBatchResult {
                    waves: schedule.waves,
                    admitted_wave: schedule.admitted_wave,
                    lanes: schedule.lanes,
                    overlap_order: schedule.overlap_order,
                    results,
                },
            ),
        }
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolMergeReadyInOrder {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "merge_ready_in_order".to_string(),
            display_name: "Merge Ready In Order".to_string(),
            source: make_source(),
            experimental: false,
            allow_parallel: false,
            description:
                "Merge done task-agent cards with agent branches one at a time in dependency order."
                    .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "Task UUID (optional in planner context)" },
                    "strategy": { "type": "string", "enum": ["merge", "squash"], "default": "squash" },
                    "stop_on_conflict": { "type": "boolean", "default": true },
                    "max_merges": { "type": "integer", "default": MAX_MERGE_BATCH },
                    "auto_revert": { "type": "boolean", "default": false, "description": "Opt in to post-merge verification and automatic git revert on deterministic regression" },
                    "auto_revert_timeout_secs": { "type": "integer", "default": crate::chat::post_merge_check::DEFAULT_POST_MERGE_CHECK_TIMEOUT_SECS, "description": "Timeout for post-merge verification and revert commands" }
                }
            }),
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
        let (gcx, task_id) = planner_context(&ccx, args, "merge_ready_in_order").await?;
        let strategy = args
            .get("strategy")
            .and_then(|v| v.as_str())
            .unwrap_or("squash");
        if strategy != "merge" && strategy != "squash" {
            return Err(format!(
                "Invalid strategy '{}', must be 'merge' or 'squash'",
                strategy
            ));
        }
        let stop_on_conflict = args
            .get("stop_on_conflict")
            .and_then(refact_tool_api::coerce_bool)
            .unwrap_or(true);
        let max_merges = usize_arg(args, "max_merges", MAX_MERGE_BATCH)?.min(MAX_MERGE_BATCH);
        let auto_revert = auto_revert_arg(args)?;
        let auto_revert_timeout_secs = auto_revert_timeout_secs_arg(args)?;
        let board = storage::load_board(gcx, &task_id).await?;
        let candidates = dependency_ordered_done_agent_cards(&board, max_merges);
        let mut results = Vec::new();

        for card_id in candidates {
            let single_args = merge_args(
                &task_id,
                &card_id,
                strategy,
                auto_revert,
                auto_revert_timeout_secs,
            );
            let call_id = format!("{}:{}", tool_call_id, card_id);
            let mut tool = ToolTaskMergeAgent::new();
            let result = merge_result_from_tool(
                card_id.clone(),
                tool.tool_execute(ccx.clone(), &call_id, &single_args).await,
            );
            let keep_going = should_continue_merges(&result, stop_on_conflict);
            results.push(result);
            if !keep_going {
                break;
            }
        }

        batch_tool_message(tool_call_id, &results)
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolMarkDoneBatch {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "mark_done_batch".to_string(),
            display_name: "Mark Done Batch".to_string(),
            source: make_source(),
            experimental: false,
            allow_parallel: false,
            description: "Mark multiple task cards done with per-card reports.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "Task UUID (optional in planner context)" },
                    "items": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "card_id": { "type": "string" },
                                "report": { "type": "string" }
                            },
                            "required": ["card_id", "report"]
                        }
                    }
                },
                "required": ["items"]
            }),
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
        let (_, task_id) = planner_context(&ccx, args, "mark_done_batch").await?;
        let raw_items = array_arg(args, "items")?;
        let mut results = Vec::new();

        for (idx, value) in raw_items.iter().enumerate() {
            match parse_mark_item(value, "report") {
                Ok((card_id, report)) => {
                    let single_args = mark_args(&task_id, &card_id, "report", &report);
                    let call_id = format!("{}:{}", tool_call_id, card_id);
                    let mut tool = ToolTaskMarkCardDone::new();
                    match tool.tool_execute(ccx.clone(), &call_id, &single_args).await {
                        Ok(_) => results.push(SimpleBatchResult {
                            card_id,
                            success: true,
                            error: None,
                        }),
                        Err(error) => results.push(SimpleBatchResult {
                            card_id,
                            success: false,
                            error: Some(error),
                        }),
                    }
                }
                Err(error) => results.push(SimpleBatchResult {
                    card_id: item_name(value, idx),
                    success: false,
                    error: Some(error),
                }),
            }
        }

        batch_tool_message(tool_call_id, &results)
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolMarkFailedBatch {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "mark_failed_batch".to_string(),
            display_name: "Mark Failed Batch".to_string(),
            source: make_source(),
            experimental: false,
            allow_parallel: false,
            description: "Mark multiple task cards failed with per-card reasons.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "Task UUID (optional in planner context)" },
                    "items": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "card_id": { "type": "string" },
                                "reason": { "type": "string" }
                            },
                            "required": ["card_id", "reason"]
                        }
                    }
                },
                "required": ["items"]
            }),
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
        let (_, task_id) = planner_context(&ccx, args, "mark_failed_batch").await?;
        let raw_items = array_arg(args, "items")?;
        let mut results = Vec::new();

        for (idx, value) in raw_items.iter().enumerate() {
            match parse_mark_item(value, "reason") {
                Ok((card_id, reason)) => {
                    let single_args = mark_args(&task_id, &card_id, "reason", &reason);
                    let call_id = format!("{}:{}", tool_call_id, card_id);
                    let mut tool = ToolTaskMarkCardFailed::new();
                    match tool.tool_execute(ccx.clone(), &call_id, &single_args).await {
                        Ok(_) => results.push(SimpleBatchResult {
                            card_id,
                            success: true,
                            error: None,
                        }),
                        Err(error) => results.push(SimpleBatchResult {
                            card_id,
                            success: false,
                            error: Some(error),
                        }),
                    }
                }
                Err(error) => results.push(SimpleBatchResult {
                    card_id: item_name(value, idx),
                    success: false,
                    error: Some(error),
                }),
            }
        }

        batch_tool_message(tool_call_id, &results)
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolBoardCreateBatch {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "board_create_batch".to_string(),
            display_name: "Board Create Batch".to_string(),
            source: make_source(),
            experimental: false,
            allow_parallel: false,
            description:
                "Create up to 30 task cards after validating dependency references and cycles."
                    .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "Task UUID (optional in planner context)" },
                    "cards": {
                        "type": "array",
                        "maxItems": MAX_CREATE_BATCH,
                        "items": {
                            "type": "object",
                            "properties": {
                                "card_id": { "type": "string" },
                                "title": { "type": "string" },
                                "priority": { "type": "string" },
                                "instructions": { "type": "string" },
                                "depends_on": { "type": "array", "items": { "type": "string" } },
                                "target_files": { "type": "array", "items": { "type": "string" } }
                            },
                            "required": ["card_id", "title"]
                        }
                    }
                },
                "required": ["cards"]
            }),
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
        let (gcx, task_id) = planner_context(&ccx, args, "board_create_batch").await?;
        let raw_items = array_arg(args, "cards")?;
        if raw_items.len() > MAX_CREATE_BATCH {
            return Err(format!(
                "board_create_batch accepts at most {} cards",
                MAX_CREATE_BATCH
            ));
        }

        let parsed: Vec<Result<BoardCreateBatchItem, String>> =
            raw_items.iter().map(parse_board_create_item).collect();
        let board = storage::load_board(gcx, &task_id).await?;
        let validation_errors = validate_board_create_batch(&board, &parsed);
        let mut results: Vec<Option<SimpleBatchResult>> = raw_items
            .iter()
            .enumerate()
            .map(|(idx, value)| {
                validation_errors[idx]
                    .as_ref()
                    .map(|error| SimpleBatchResult {
                        card_id: parsed[idx]
                            .as_ref()
                            .map(|item| item.card_id.clone())
                            .unwrap_or_else(|_| item_name(value, idx)),
                        success: false,
                        error: Some(error.clone()),
                    })
            })
            .collect();
        let mut available_ids: HashSet<String> =
            board.cards.iter().map(|card| card.id.clone()).collect();

        for idx in board_create_order(&parsed, &validation_errors) {
            let item = parsed[idx].as_ref().map_err(|e| e.clone())?;
            if let Some(dep) = item
                .depends_on
                .iter()
                .find(|dep| !available_ids.contains(*dep))
            {
                results[idx] = Some(SimpleBatchResult {
                    card_id: item.card_id.clone(),
                    success: false,
                    error: Some(format!("Dependency {} was not created", dep)),
                });
                continue;
            }
            let single_args = board_create_args(&task_id, item);
            let call_id = format!("{}:{}", tool_call_id, item.card_id);
            let mut tool = ToolTaskBoardCreateCard::new();
            match tool.tool_execute(ccx.clone(), &call_id, &single_args).await {
                Ok(_) => {
                    available_ids.insert(item.card_id.clone());
                    results[idx] = Some(SimpleBatchResult {
                        card_id: item.card_id.clone(),
                        success: true,
                        error: None,
                    });
                }
                Err(error) => {
                    results[idx] = Some(SimpleBatchResult {
                        card_id: item.card_id.clone(),
                        success: false,
                        error: Some(error),
                    });
                }
            }
        }

        let results: Vec<SimpleBatchResult> = results.into_iter().flatten().collect();
        batch_tool_message(tool_call_id, &results)
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use crate::tasks::types::{TaskMeta as StoredTaskMeta, TaskStatus};
    use chrono::Utc;
    use refact_chat_api::TaskMeta as ThreadTaskMeta;

    fn card(id: &str, column: &str, deps: Vec<&str>) -> BoardCard {
        BoardCard {
            id: id.to_string(),
            title: format!("Card {}", id),
            column: column.to_string(),
            priority: "P1".to_string(),
            depends_on: deps.into_iter().map(str::to_string).collect(),
            instructions: String::new(),
            assignee: None,
            agent_chat_id: None,
            status_updates: vec![],
            comments: vec![],
            final_report: None,
            final_report_structured: None,
            verifier_report: None,
            created_at: Utc::now().to_rfc3339(),
            started_at: None,
            last_heartbeat_at: None,
            completed_at: None,
            agent_branch: None,
            agent_worktree: None,
            agent_worktree_name: None,
            base_branch: None,
            base_commit: None,
            ab_variants: None,
            team_members: vec![],
            target_files: vec![],
            scope_guard_mode: Default::default(),
            attention: None,
        }
    }

    fn task_meta(task_id: &str) -> StoredTaskMeta {
        let now = Utc::now().to_rfc3339();
        StoredTaskMeta {
            schema_version: 1,
            id: task_id.to_string(),
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

    async fn setup_task(board: TaskBoard) -> (Arc<GlobalContext>, tempfile::TempDir) {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
        let task_dir = temp.path().join(".refact").join("tasks").join("task-1");
        tokio::fs::create_dir_all(&task_dir).await.unwrap();
        storage::save_task_meta(gcx.clone(), "task-1", &task_meta("task-1"))
            .await
            .unwrap();
        storage::save_board(gcx.clone(), "task-1", &board)
            .await
            .unwrap();
        (gcx, temp)
    }

    async fn planner_ccx(gcx: Arc<GlobalContext>) -> Arc<AMutex<AtCommandsContext>> {
        Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                AppState::from_gcx(gcx).await,
                4096,
                20,
                false,
                Vec::new(),
                "planner-chat".to_string(),
                None,
                "model".to_string(),
                Some(ThreadTaskMeta {
                    task_id: "task-1".to_string(),
                    role: "planner".to_string(),
                    agent_id: None,
                    card_id: None,
                    planner_chat_id: Some("planner-chat".to_string()),
                }),
                None,
            )
            .await,
        ))
    }

    fn output_value(contexts: Vec<ContextEnum>) -> Value {
        let text = contexts_text(&contexts);
        serde_json::from_str(&text).unwrap()
    }

    fn rendered_merge_result(message: &str) -> Result<(bool, Vec<ContextEnum>), String> {
        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(message.to_string()),
                ..Default::default()
            })],
        ))
    }

    #[tokio::test]
    async fn planner_context_rejects_task_id_mismatching_bound_task() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let ccx = planner_ccx(gcx).await;
        let args = HashMap::from([("task_id".to_string(), json!("task-2"))]);

        let error = match planner_context(&ccx, &args, "batch_tool").await {
            Ok(_) => panic!("mismatching task_id should be rejected"),
            Err(error) => error,
        };

        assert!(error.contains("task-2"));
        assert!(error.contains("task-1"));
        assert!(error.contains("does not match"));
    }

    #[tokio::test]
    async fn planner_context_accepts_task_id_matching_bound_task() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let ccx = planner_ccx(gcx).await;
        let args = HashMap::from([("task_id".to_string(), json!("task-1"))]);

        let (_, task_id) = planner_context(&ccx, &args, "batch_tool").await.unwrap();

        assert_eq!(task_id, "task-1");
    }

    #[tokio::test]
    async fn planner_context_rejects_non_string_task_id() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let ccx = planner_ccx(gcx).await;
        let args = HashMap::from([("task_id".to_string(), json!(123))]);

        let error = match planner_context(&ccx, &args, "batch_tool").await {
            Ok(_) => panic!("non-string task_id should be rejected"),
            Err(error) => error,
        };

        assert!(error.contains("must be a string"));
    }

    #[tokio::test]
    async fn planner_context_rejects_blank_task_id() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let ccx = planner_ccx(gcx).await;
        let args = HashMap::from([("task_id".to_string(), json!("  \n  "))]);

        let error = match planner_context(&ccx, &args, "batch_tool").await {
            Ok(_) => panic!("blank task_id should be rejected"),
            Err(error) => error,
        };

        assert!(error.contains("non-empty string"));
    }

    #[test]
    fn spawn_agents_batch_with_3_valid_cards() {
        let board = TaskBoard {
            cards: vec![
                card("T-1", "planned", vec![]),
                card("T-2", "planned", vec![]),
                card("T-3", "planned", vec![]),
            ],
            ..Default::default()
        };
        let items = vec![
            Ok(SpawnBatchItem {
                card_id: "T-1".to_string(),
                suggested_steps: None,
                files_to_open: vec![],
                base_branch: None,
                base_commit: None,
            }),
            Ok(SpawnBatchItem {
                card_id: "T-2".to_string(),
                suggested_steps: None,
                files_to_open: vec![],
                base_branch: None,
                base_commit: None,
            }),
            Ok(SpawnBatchItem {
                card_id: "T-3".to_string(),
                suggested_steps: None,
                files_to_open: vec![],
                base_branch: None,
                base_commit: None,
            }),
        ];

        let errors = validate_spawn_agent_cards(&board, &items);

        assert_eq!(errors, vec![None, None, None]);
    }

    #[test]
    fn spawn_batch_forwards_card_base_refs() {
        let item = parse_spawn_item(&json!({
            "card_id": "T-1",
            "base_branch": "release",
            "base_commit": "abc123"
        }))
        .unwrap();
        let args = spawn_args("task-1", &item);
        assert_eq!(args.get("base_branch"), Some(&json!("release")));
        assert_eq!(args.get("base_commit"), Some(&json!("abc123")));
    }

    #[test]
    fn merge_ready_in_order_stops_on_conflict() {
        let mut first = card("T-1", "done", vec![]);
        first.agent_branch = Some("agent/T-1".to_string());
        let mut second = card("T-2", "done", vec!["T-1"]);
        second.agent_branch = Some("agent/T-2".to_string());
        let mut third = card("T-3", "done", vec!["T-2"]);
        third.agent_branch = Some("agent/T-3".to_string());
        let board = TaskBoard {
            cards: vec![third, second, first],
            ..Default::default()
        };
        let candidates = dependency_ordered_done_agent_cards(&board, 10);
        let outcomes = vec![
            MergeBatchResult {
                card_id: candidates[0].clone(),
                merged: true,
                conflict: false,
                error: None,
            },
            MergeBatchResult {
                card_id: candidates[1].clone(),
                merged: false,
                conflict: true,
                error: None,
            },
            MergeBatchResult {
                card_id: candidates[2].clone(),
                merged: true,
                conflict: false,
                error: None,
            },
        ];
        let mut processed = Vec::new();
        for outcome in outcomes {
            let keep_going = should_continue_merges(&outcome, true);
            processed.push(outcome.card_id);
            if !keep_going {
                break;
            }
        }

        assert_eq!(candidates, vec!["T-1", "T-2", "T-3"]);
        assert_eq!(processed, vec!["T-1", "T-2"]);
    }

    #[test]
    fn merge_result_reports_rendered_auto_revert_as_not_merged() {
        let rendered = r#"# Agent Work Merged

The agent's work has been successfully merged back to the target branch.

## Post-merge regression detected

**Verification:** `cargo test` failed with exit code 1
**Auto-reverted:** true
**Merge commit:** abc123
**Revert commit:** def456"#;

        let result = merge_result_from_tool("T-1".to_string(), rendered_merge_result(rendered));

        assert!(!result.merged);
        assert!(!result.conflict);
        assert!(result
            .error
            .as_deref()
            .is_some_and(|error| error.contains("automatically reverted")));
        assert!(!should_continue_merges(&result, false));
    }

    #[test]
    fn merge_result_reports_rendered_non_reverted_merge_as_merged() {
        let rendered = r#"# Agent Work Merged

The agent's work has been successfully merged back to the target branch.

**Post-merge check:** `cargo test` passed"#;

        let result = merge_result_from_tool("T-1".to_string(), rendered_merge_result(rendered));

        assert!(result.merged);
        assert!(!result.conflict);
        assert_eq!(result.error, None);
        assert!(should_continue_merges(&result, true));
    }

    #[test]
    fn merge_ready_in_order_stops_after_generic_non_merge() {
        let result = MergeBatchResult {
            card_id: "T-1".to_string(),
            merged: false,
            conflict: false,
            error: Some("merge failed".to_string()),
        };

        assert!(!should_continue_merges(&result, false));
    }

    #[test]
    fn merge_ready_in_order_conflict_setting_only_controls_conflicts() {
        let result = MergeBatchResult {
            card_id: "T-1".to_string(),
            merged: true,
            conflict: true,
            error: None,
        };

        assert!(!should_continue_merges(&result, true));
        assert!(should_continue_merges(&result, false));
    }

    #[test]
    fn merge_ready_in_order_exposes_post_merge_verification_schema() {
        let schema = ToolMergeReadyInOrder::new().tool_description().input_schema;
        let properties = schema["properties"].as_object().unwrap();

        assert_eq!(properties["auto_revert"]["type"], json!("boolean"));
        assert_eq!(properties["auto_revert"]["default"], json!(false));
        assert_eq!(
            properties["auto_revert_timeout_secs"]["type"],
            json!("integer")
        );
        assert_eq!(
            properties["auto_revert_timeout_secs"]["default"],
            json!(crate::chat::post_merge_check::DEFAULT_POST_MERGE_CHECK_TIMEOUT_SECS)
        );
        assert!(!properties.contains_key("force"));
    }

    #[test]
    fn merge_ready_in_order_forwards_post_merge_verification_args() {
        let requested = HashMap::from([
            ("auto_revert".to_string(), json!(true)),
            ("auto_revert_timeout_secs".to_string(), json!(120)),
        ]);
        let auto_revert = auto_revert_arg(&requested).unwrap();
        let timeout = auto_revert_timeout_secs_arg(&requested).unwrap();

        let forwarded = merge_args("task-1", "T-1", "squash", auto_revert, timeout);

        assert_eq!(forwarded["auto_revert"], json!(true));
        assert_eq!(forwarded["auto_revert_timeout_secs"], json!(120));
        assert!(!forwarded.contains_key("force"));
    }

    #[test]
    fn merge_ready_in_order_post_merge_verification_args_match_merge_agent_semantics() {
        let empty = HashMap::new();
        assert!(!auto_revert_arg(&empty).unwrap());
        assert_eq!(
            auto_revert_timeout_secs_arg(&empty).unwrap(),
            crate::chat::post_merge_check::DEFAULT_POST_MERGE_CHECK_TIMEOUT_SECS
        );

        let invalid_bool = HashMap::from([("auto_revert".to_string(), json!("sometimes"))]);
        assert!(auto_revert_arg(&invalid_bool)
            .unwrap_err()
            .contains("must be a boolean"));

        let invalid_timeout =
            HashMap::from([("auto_revert_timeout_secs".to_string(), json!("120"))]);
        assert!(auto_revert_timeout_secs_arg(&invalid_timeout)
            .unwrap_err()
            .contains("positive integer"));

        let zero_timeout = HashMap::from([("auto_revert_timeout_secs".to_string(), json!(0))]);
        assert!(auto_revert_timeout_secs_arg(&zero_timeout)
            .unwrap_err()
            .contains("greater than zero"));
    }

    #[tokio::test]
    async fn mark_done_batch_mixed_success_failure() {
        let board = TaskBoard {
            cards: vec![card("T-1", "planned", vec![])],
            ..Default::default()
        };
        let (gcx, _temp) = setup_task(board).await;
        let ccx = planner_ccx(gcx.clone()).await;
        let args = HashMap::from_iter([(
            "items".to_string(),
            json!([
                { "card_id": "T-1", "report": "done cleanly" },
                { "card_id": "missing", "report": "not found" }
            ]),
        )]);
        let mut tool = ToolMarkDoneBatch::new();

        let (_, contexts) = tool
            .tool_execute(ccx, &"tool-call".to_string(), &args)
            .await
            .unwrap();
        let output = output_value(contexts);
        let board = storage::load_board(gcx, "task-1").await.unwrap();

        assert_eq!(output[0]["success"], json!(true));
        assert_eq!(output[1]["success"], json!(false));
        assert!(output[1]["error"].as_str().unwrap().contains("not found"));
        let card = board.get_card("T-1").unwrap();
        assert_eq!(card.column, "done");
        assert_eq!(card.final_report.as_deref(), Some("done cleanly"));
    }

    #[test]
    fn spawn_agents_batch_without_a_schedule_stays_unscheduled() {
        let args = HashMap::from([("cards".to_string(), json!([{ "card_id": "T-1" }]))]);

        assert_eq!(parse_schedule(&args).unwrap(), None);
    }

    #[test]
    fn spawn_agents_batch_schedule_is_absent_from_the_legacy_output_shape() {
        let results = vec![SpawnBatchResult {
            card_id: "T-1".to_string(),
            agent_chat_id: None,
            success: true,
            error: None,
        }];

        let rendered = serde_json::to_value(&results).unwrap();

        assert!(rendered.is_array());
        assert_eq!(rendered[0]["card_id"], json!("T-1"));
    }

    #[test]
    fn spawn_agents_batch_parses_every_lane_kind() {
        let args = HashMap::from([(
            "schedule".to_string(),
            json!({
                "lanes": [
                    { "name": "roots", "kind": "parallel", "stages": [{ "card_id": "T-1" }] },
                    { "name": "chain", "kind": "sequential", "stages": [{ "card_id": "T-2" }] },
                    {
                        "name": "review",
                        "kind": "pipeline",
                        "optional": true,
                        "stages": [
                            { "card_id": "T-3", "agent_id": "reviewer" },
                            { "card_id": "T-4", "agent_id": "reviewer", "optional": true }
                        ]
                    }
                ]
            }),
        )]);

        let lanes = parse_schedule(&args).unwrap().unwrap();

        assert_eq!(lanes[0].kind, LaneKind::Parallel);
        assert_eq!(lanes[1].kind, LaneKind::Sequential);
        assert_eq!(lanes[2].kind, LaneKind::Pipeline);
        assert!(lanes[2].optional);
        assert_eq!(lanes[2].stages[0].agent_id.as_deref(), Some("reviewer"));
        assert_eq!(lanes[2].stages[1].agent_id.as_deref(), Some("reviewer"));
        assert!(lanes[2].stages[1].optional);
    }

    #[test]
    fn spawn_agents_batch_rejects_an_unknown_lane_kind() {
        let args = HashMap::from([(
            "schedule".to_string(),
            json!({ "lanes": [{ "name": "roots", "kind": "waves", "stages": [] }] }),
        )]);

        let error = parse_schedule(&args).unwrap_err();

        assert!(error.contains("unknown lane kind"), "{error}");
    }

    #[test]
    fn spawn_agents_batch_requires_every_requested_card_to_be_placed_in_a_lane() {
        let parsed = vec![
            parse_spawn_item(&json!({ "card_id": "T-1" })),
            parse_spawn_item(&json!({ "card_id": "T-2" })),
        ];
        let lanes = vec![Lane {
            name: "roots".to_string(),
            kind: LaneKind::Parallel,
            stages: vec![LaneStage {
                card_id: "T-1".to_string(),
                agent_id: None,
                optional: false,
            }],
            optional: false,
        }];

        let error = reconcile_schedule_with_cards(&lanes, &parsed).unwrap_err();

        assert!(
            error.contains("T-2 is not placed in any schedule lane"),
            "{error}"
        );
    }

    #[test]
    fn spawn_agents_batch_rejects_a_lane_staging_an_unrequested_card() {
        let parsed = vec![parse_spawn_item(&json!({ "card_id": "T-1" }))];
        let lanes = vec![Lane {
            name: "roots".to_string(),
            kind: LaneKind::Parallel,
            stages: vec![
                LaneStage {
                    card_id: "T-1".to_string(),
                    agent_id: None,
                    optional: false,
                },
                LaneStage {
                    card_id: "T-9".to_string(),
                    agent_id: None,
                    optional: false,
                },
            ],
            optional: false,
        }];

        let error = reconcile_schedule_with_cards(&lanes, &parsed).unwrap_err();

        assert!(error.contains("not listed in 'cards'"), "{error}");
    }

    #[test]
    fn spawn_agents_batch_schedule_dispatches_only_the_admitted_wave() {
        let board = TaskBoard {
            cards: vec![
                card("T-1", "planned", vec![]),
                card("T-2", "planned", vec!["T-1"]),
            ],
            ..Default::default()
        };
        let lanes = vec![Lane {
            name: "roots".to_string(),
            kind: LaneKind::Parallel,
            stages: vec![
                LaneStage {
                    card_id: "T-1".to_string(),
                    agent_id: None,
                    optional: false,
                },
                LaneStage {
                    card_id: "T-2".to_string(),
                    agent_id: None,
                    optional: false,
                },
            ],
            optional: false,
        }];

        let schedule = plan_schedule(&board, &lanes, &wave_limits()).unwrap();

        assert_eq!(schedule.dispatch, vec!["T-1".to_string()]);
        assert_eq!(schedule.admitted_wave, Some(0));
    }

    #[test]
    fn wave_limits_come_from_coordination_settings() {
        let settings = refact_work_core::CoordinationSettings::default();

        assert_eq!(
            wave_limits(),
            WaveLimits {
                max_wave_width: settings.max_wave_width,
                max_pipeline_depth: settings.max_pipeline_depth,
            }
        );
        assert!(settings.max_wave_width >= 1);
        assert!(settings.max_pipeline_depth >= 1);
    }

    #[test]
    fn a_malformed_card_is_reported_even_when_a_schedule_excludes_it() {
        let malformed = parse_spawn_item(&json!({ "suggested_steps": 3 }));
        assert!(malformed.is_err());

        let result =
            pre_dispatch_result("<item 1>", &malformed, Some("Missing 'card_id'"), |_| false)
                .expect("a card that never parsed must not vanish from the output");

        assert_eq!(result.card_id, "<item 1>");
        assert!(!result.success);
        assert_eq!(result.error.as_deref(), Some("Missing 'card_id'"));
    }

    #[test]
    fn a_card_the_schedule_did_not_admit_is_left_to_the_lane_report() {
        let parsed = parse_spawn_item(&json!({ "card_id": "T-2" }));

        assert_eq!(
            pre_dispatch_result("T-2", &parsed, Some("not ready"), |_| false),
            None
        );
        assert!(pre_dispatch_result("T-2", &parsed, Some("not ready"), |_| true).is_some());
        assert_eq!(
            pre_dispatch_result("T-2", &parsed, None, |_| true),
            None,
            "an admitted card with no validation error is spawned, not pre-reported"
        );
    }

    #[test]
    fn spawn_agents_batch_publishes_the_schedule_in_its_input_schema() {
        let schema = ToolSpawnAgentsBatch::new().tool_description().input_schema;
        let lane = &schema["properties"]["schedule"]["properties"]["lanes"]["items"];

        assert_eq!(lane["properties"]["kind"]["enum"], json!(LaneKind::NAMES));
        assert_eq!(
            schema["required"],
            json!(["cards"]),
            "schedule must stay optional for existing callers"
        );
        assert!(lane["properties"]["stages"]["items"]["properties"]["agent_id"].is_object());
    }

    fn overlap_schedule(
        board: &TaskBoard,
        card_ids: &[&str],
        files: &[(&str, &str)],
    ) -> WaveSchedule {
        let lanes = vec![Lane {
            name: "roots".to_string(),
            kind: LaneKind::Parallel,
            stages: card_ids
                .iter()
                .map(|card_id| LaneStage {
                    card_id: card_id.to_string(),
                    agent_id: None,
                    optional: false,
                })
                .collect(),
            optional: false,
        }];
        let mut inputs = crate::tasks::overlap_order::OverlapInputs::new();
        for (card_id, file) in files {
            inputs.declare(card_id, [file.to_string()]);
        }
        plan_schedule_with_overlap(board, &lanes, &wave_limits(), Some(&inputs)).unwrap()
    }

    fn spawn_items(card_ids: &[&str]) -> Vec<Result<SpawnBatchItem, String>> {
        card_ids
            .iter()
            .map(|card_id| parse_spawn_item(&json!({ "card_id": card_id })))
            .collect()
    }

    #[test]
    fn spawn_agents_batch_submits_colliding_cards_in_separate_batches() {
        let board = TaskBoard {
            cards: vec![
                card("T-1", "planned", vec![]),
                card("T-2", "planned", vec![]),
                card("T-3", "planned", vec![]),
            ],
            ..Default::default()
        };
        let schedule = overlap_schedule(
            &board,
            &["T-1", "T-2", "T-3"],
            &[
                ("T-1", "/repo/src/lib.rs"),
                ("T-2", "/repo/src/other.rs"),
                ("T-3", "/repo/src/lib.rs"),
            ],
        );
        let parsed = spawn_items(&["T-1", "T-2", "T-3"]);

        let batches = submission_batches(&parsed, &[0, 1, 2], Some(&schedule));

        assert_eq!(
            batches,
            vec![vec![0, 1], vec![2]],
            "T-3 collides with T-1 and must not be spawned alongside it"
        );
        assert_eq!(
            batches.iter().flatten().count(),
            3,
            "every eligible card is still submitted"
        );
    }

    #[test]
    fn spawn_agents_batch_submits_everything_at_once_without_overlap_ordering() {
        let parsed = spawn_items(&["T-1", "T-2", "T-3"]);

        let batches = submission_batches(&parsed, &[0, 1, 2], None);

        assert_eq!(batches, vec![vec![0, 1, 2]]);
    }

    #[test]
    fn spawn_agents_batch_overlap_ordering_never_drops_an_eligible_card() {
        let board = TaskBoard {
            cards: vec![
                card("T-1", "planned", vec![]),
                card("T-2", "planned", vec![]),
                card("T-3", "planned", vec![]),
            ],
            ..Default::default()
        };
        let clique = overlap_schedule(
            &board,
            &["T-1", "T-2", "T-3"],
            &[
                ("T-1", "/repo/src/shared.rs"),
                ("T-2", "/repo/src/shared.rs"),
                ("T-3", "/repo/src/shared.rs"),
            ],
        );
        let parsed = spawn_items(&["T-1", "T-2", "T-3"]);

        let batches = submission_batches(&parsed, &[0, 1, 2], Some(&clique));

        assert_eq!(batches, vec![vec![0], vec![1], vec![2]]);
        assert_eq!(
            clique.dispatch,
            vec!["T-1".to_string(), "T-2".to_string(), "T-3".to_string()],
            "a total collision still dispatches every card"
        );
    }

    #[test]
    fn overlap_ordering_is_enabled_by_default_and_readable_from_settings() {
        let settings = refact_work_core::CoordinationSettings::default();

        assert!(settings.overlap_ordering_enabled);
        assert_eq!(
            crate::tasks::overlap::overlap_ordering_enabled(),
            refact_work_core::coordination_settings().overlap_ordering_enabled
        );
    }

    #[test]
    fn board_create_batch_detects_cycles() {
        let board = TaskBoard::default();
        let items = vec![
            Ok(BoardCreateBatchItem {
                card_id: "T-1".to_string(),
                title: "First".to_string(),
                priority: "P1".to_string(),
                instructions: String::new(),
                depends_on: vec!["T-2".to_string()],
                target_files: vec![],
            }),
            Ok(BoardCreateBatchItem {
                card_id: "T-2".to_string(),
                title: "Second".to_string(),
                priority: "P1".to_string(),
                instructions: String::new(),
                depends_on: vec!["T-1".to_string()],
                target_files: vec![],
            }),
        ];

        let errors = validate_board_create_batch(&board, &items);

        assert!(errors.iter().all(|error| error
            .as_deref()
            .is_some_and(|error| error.contains("cycle"))));
    }
}
