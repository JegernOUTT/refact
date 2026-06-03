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
    let task_id = args
        .get("task_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| ccx_lock.task_meta.as_ref().map(|m| m.task_id.clone()))
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

#[derive(Clone, Debug)]
struct SpawnBatchItem {
    card_id: String,
    suggested_steps: Option<usize>,
    files_to_open: Vec<String>,
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
    })
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
            let merged = text.contains("Agent Work Merged");
            MergeBatchResult {
                card_id,
                merged,
                conflict,
                error: None,
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
    !(stop_on_conflict && result.conflict)
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
    args
}

fn mark_args(task_id: &str, card_id: &str, key: &str, text: &str) -> HashMap<String, Value> {
    let mut args = HashMap::new();
    args.insert("task_id".to_string(), json!(task_id));
    args.insert("card_id".to_string(), json!(card_id));
    args.insert(key.to_string(), json!(text));
    args
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
            description: "Spawn task agents for up to 10 ready planned cards in parallel."
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
                                "files_to_open": { "type": "array", "items": { "type": "string" } }
                            },
                            "required": ["card_id"]
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
        let validation_errors = validate_spawn_agent_cards(&board, &parsed);
        let mut results: Vec<Option<SpawnBatchResult>> = raw_items
            .iter()
            .enumerate()
            .map(|(idx, value)| {
                validation_errors[idx]
                    .as_ref()
                    .map(|error| SpawnBatchResult {
                        card_id: parsed[idx]
                            .as_ref()
                            .map(|item| item.card_id.clone())
                            .unwrap_or_else(|_| item_name(value, idx)),
                        agent_chat_id: None,
                        success: false,
                        error: Some(error.clone()),
                    })
            })
            .collect();
        let mut joins = JoinSet::new();

        for (idx, item) in parsed.iter().enumerate() {
            if validation_errors[idx].is_some() {
                continue;
            }
            let item = item.as_ref().map_err(|e| e.clone())?.clone();
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
                    let idx = results.iter().position(Option::is_none).unwrap_or(0);
                    results[idx] = Some(SpawnBatchResult {
                        card_id: item_name(&raw_items[idx], idx),
                        agent_chat_id: None,
                        success: false,
                        error: Some(format!("spawn task failed: {}", error)),
                    });
                }
            }
        }

        let results: Vec<SpawnBatchResult> = results.into_iter().flatten().collect();
        batch_tool_message(tool_call_id, &results)
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
                    "max_merges": { "type": "integer", "default": MAX_MERGE_BATCH }
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
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let max_merges = usize_arg(args, "max_merges", MAX_MERGE_BATCH)?.min(MAX_MERGE_BATCH);
        let board = storage::load_board(gcx, &task_id).await?;
        let candidates = dependency_ordered_done_agent_cards(&board, max_merges);
        let mut results = Vec::new();

        for card_id in candidates {
            let mut single_args = HashMap::new();
            single_args.insert("task_id".to_string(), json!(task_id));
            single_args.insert("card_id".to_string(), json!(card_id));
            single_args.insert("strategy".to_string(), json!(strategy));
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
            ab_variants: None,
            team_members: vec![],
            target_files: vec![],
            scope_guard_mode: Default::default(),
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
            conductor: None,
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
            }),
            Ok(SpawnBatchItem {
                card_id: "T-2".to_string(),
                suggested_steps: None,
                files_to_open: vec![],
            }),
            Ok(SpawnBatchItem {
                card_id: "T-3".to_string(),
                suggested_steps: None,
                files_to_open: vec![],
            }),
        ];

        let errors = validate_spawn_agent_cards(&board, &items);

        assert_eq!(errors, vec![None, None, None]);
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
