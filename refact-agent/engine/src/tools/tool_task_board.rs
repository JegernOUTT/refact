use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use async_trait::async_trait;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;
use chrono::{DateTime, Utc};

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::tasks::storage;
use crate::tasks::types::{AbVariants, BoardCard, ScopeGuardMode, TaskBoard};
use crate::tasks::events::{TaskEvent, emit_task_event};

fn make_source() -> ToolSource {
    ToolSource {
        source_type: ToolSourceType::Builtin,
        config_path: String::new(),
    }
}

fn parse_depends_on(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        Some(Value::String(s)) => s
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        _ => vec![],
    }
}

fn parse_target_files(value: Option<&Value>, instructions: &str) -> Vec<String> {
    let mut files: Vec<String> = match value {
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str().map(str::trim))
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        Some(Value::String(s)) => s
            .split([',', '\n'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        _ => vec![],
    };
    if files.is_empty() {
        for token in instructions.split_whitespace() {
            let t = token.trim_matches(|c: char| {
                matches!(
                    c,
                    '`' | ',' | '.' | ':' | ';' | '(' | ')' | '[' | ']' | '{' | '}'
                )
            });
            if t.contains('/') && t.contains('.') && !files.iter().any(|f| f == t) {
                files.push(t.to_string());
            }
        }
    }
    files.sort();
    files.dedup();
    files
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BoardMode {
    Summary,
    Tree,
    Mermaid,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BoardVerbosity {
    Minimal,
    Brief,
    Full,
    UpdatesOnly,
}

#[derive(Default)]
struct BoardFilter {
    columns: HashSet<String>,
    priorities: HashSet<String>,
    assignee: Option<String>,
}

#[derive(Serialize)]
struct CardBrief {
    id: String,
    title: String,
    column: String,
    priority: String,
    depends_on: Vec<String>,
    assignee: Option<String>,
    agent_chat_id: Option<String>,
    created_at: String,
    started_at: Option<String>,
    last_heartbeat_at: Option<String>,
    completed_at: Option<String>,
    agent_branch: Option<String>,
    agent_worktree_name: Option<String>,
    ab_variants: Option<AbVariants>,
    target_files: Vec<String>,
    scope_guard_mode: ScopeGuardMode,
}

#[derive(Serialize)]
struct CardUpdatesOnly {
    id: String,
    status_updates: Vec<crate::tasks::types::StatusUpdate>,
}

fn parse_mode(args: &HashMap<String, Value>) -> Result<BoardMode, String> {
    match args
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("summary")
    {
        "summary" => Ok(BoardMode::Summary),
        "tree" => Ok(BoardMode::Tree),
        "mermaid" => Ok(BoardMode::Mermaid),
        mode => Err(format!(
            "Invalid mode: {}. Must be one of: summary, tree, mermaid",
            mode
        )),
    }
}

fn parse_board_verbosity(
    args: &HashMap<String, Value>,
    default: BoardVerbosity,
) -> Result<BoardVerbosity, String> {
    let raw = args.get("verbosity").and_then(|v| v.as_str());
    match raw {
        None => Ok(default),
        Some("minimal") => Ok(BoardVerbosity::Minimal),
        Some("brief") => Ok(BoardVerbosity::Brief),
        Some("full") => Ok(BoardVerbosity::Full),
        Some("updates_only") => Ok(BoardVerbosity::UpdatesOnly),
        Some(verbosity) => Err(format!(
            "Invalid verbosity: {}. Must be one of: minimal, brief, full, updates_only",
            verbosity
        )),
    }
}

fn parse_filter(args: &HashMap<String, Value>) -> BoardFilter {
    let mut filter = BoardFilter::default();
    let Some(Value::Object(obj)) = args.get("filter") else {
        return filter;
    };
    filter.columns = parse_string_set(obj.get("column"));
    filter.priorities = parse_string_set(obj.get("priority"));
    filter.assignee = obj
        .get("assignee")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    filter
}

fn parse_string_set(value: Option<&Value>) -> HashSet<String> {
    match value {
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        Some(Value::String(s)) => s
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        _ => HashSet::new(),
    }
}

fn card_matches_filter(card: &BoardCard, filter: &BoardFilter) -> bool {
    (filter.columns.is_empty() || filter.columns.contains(&card.column))
        && (filter.priorities.is_empty() || filter.priorities.contains(&card.priority))
        && filter
            .assignee
            .as_ref()
            .map(|assignee| card.assignee.as_deref() == Some(assignee.as_str()))
            .unwrap_or(true)
}

fn render_board_summary(
    board: &TaskBoard,
    cards: &[&BoardCard],
    verbosity: BoardVerbosity,
) -> Result<String, String> {
    let cards = cards
        .iter()
        .map(|card| card_summary_value(card, verbosity))
        .collect::<Vec<_>>();
    serde_yaml::to_string(&json!({
        "rev": board.rev,
        "cards": cards,
    }))
    .map_err(|e| e.to_string())
}

fn card_summary_value(card: &BoardCard, verbosity: BoardVerbosity) -> Value {
    match verbosity {
        BoardVerbosity::Minimal => json!({
            "id": card.id,
            "column": card.column,
            "priority": card.priority,
        }),
        BoardVerbosity::Full => json!({
            "id": card.id,
            "title": card.title,
            "column": card.column,
            "priority": card.priority,
            "depends_on": card.depends_on,
            "instructions_excerpt": excerpt(&card.instructions, 200),
        }),
        BoardVerbosity::Brief | BoardVerbosity::UpdatesOnly => json!({
            "id": card.id,
            "title": card.title,
            "column": card.column,
            "priority": card.priority,
            "depends_on": card.depends_on,
        }),
    }
}

fn render_card_details(card: &BoardCard, verbosity: BoardVerbosity) -> Result<String, String> {
    match verbosity {
        BoardVerbosity::UpdatesOnly => serde_yaml::to_string(&CardUpdatesOnly {
            id: card.id.clone(),
            status_updates: card.status_updates.clone(),
        })
        .map_err(|e| e.to_string()),
        BoardVerbosity::Full => serde_yaml::to_string(card).map_err(|e| e.to_string()),
        BoardVerbosity::Minimal | BoardVerbosity::Brief => serde_yaml::to_string(&CardBrief {
            id: card.id.clone(),
            title: card.title.clone(),
            column: card.column.clone(),
            priority: card.priority.clone(),
            depends_on: card.depends_on.clone(),
            assignee: card.assignee.clone(),
            agent_chat_id: card.agent_chat_id.clone(),
            created_at: card.created_at.clone(),
            started_at: card.started_at.clone(),
            last_heartbeat_at: card.last_heartbeat_at.clone(),
            completed_at: card.completed_at.clone(),
            agent_branch: card.agent_branch.clone(),
            agent_worktree_name: card.agent_worktree_name.clone(),
            ab_variants: card.ab_variants.clone(),
            target_files: card.target_files.clone(),
            scope_guard_mode: card.scope_guard_mode,
        })
        .map_err(|e| e.to_string()),
    }
}

fn render_board_tree(cards: &[&BoardCard], verbosity: BoardVerbosity) -> String {
    let visible_ids = cards
        .iter()
        .map(|card| card.id.as_str())
        .collect::<HashSet<_>>();
    let mut children: HashMap<&str, Vec<&BoardCard>> = HashMap::new();
    for card in cards {
        for dep in &card.depends_on {
            if visible_ids.contains(dep.as_str()) {
                children.entry(dep.as_str()).or_default().push(*card);
            }
        }
    }
    for child_cards in children.values_mut() {
        child_cards.sort_by(|a, b| a.id.cmp(&b.id));
    }
    let mut roots = cards
        .iter()
        .copied()
        .filter(|card| {
            !card
                .depends_on
                .iter()
                .any(|dep| visible_ids.contains(dep.as_str()))
        })
        .collect::<Vec<_>>();
    roots.sort_by(|a, b| a.id.cmp(&b.id));

    let mut output = String::new();
    let mut rendered = HashSet::new();
    for root in roots {
        render_tree_node(
            root,
            &children,
            verbosity,
            "",
            true,
            false,
            &mut rendered,
            &mut output,
        );
    }
    for card in cards {
        if !rendered.contains(card.id.as_str()) {
            render_tree_node(
                card,
                &children,
                verbosity,
                "",
                true,
                false,
                &mut rendered,
                &mut output,
            );
        }
    }
    if output.is_empty() {
        output.push_str("No cards matched.\n");
    }
    output
}

fn render_tree_node<'a>(
    card: &'a BoardCard,
    children: &HashMap<&str, Vec<&'a BoardCard>>,
    verbosity: BoardVerbosity,
    prefix: &str,
    last: bool,
    show_connector: bool,
    rendered: &mut HashSet<&'a str>,
    output: &mut String,
) {
    output.push_str(prefix);
    if show_connector {
        output.push_str(if last { "└─ " } else { "├─ " });
    }
    output.push_str(&format_tree_card(card, verbosity));
    if !rendered.insert(card.id.as_str()) {
        output.push_str(" ↩\n");
        return;
    }
    output.push('\n');

    let Some(child_cards) = children.get(card.id.as_str()) else {
        return;
    };
    let child_prefix = if show_connector && last {
        format!("{}   ", prefix)
    } else if show_connector {
        format!("{}│  ", prefix)
    } else {
        format!("{}  ", prefix)
    };
    for (index, child) in child_cards.iter().enumerate() {
        render_tree_node(
            child,
            children,
            verbosity,
            &child_prefix,
            index + 1 == child_cards.len(),
            true,
            rendered,
            output,
        );
    }
}

fn format_tree_card(card: &BoardCard, verbosity: BoardVerbosity) -> String {
    match verbosity {
        BoardVerbosity::Minimal => format!("{} ({}, {})", card.id, card.column, card.priority),
        BoardVerbosity::Full => format!(
            "{} ({}, {}) {} — {}",
            card.id,
            card.column,
            card.priority,
            card.title,
            excerpt(&card.instructions, 200)
        ),
        BoardVerbosity::Brief | BoardVerbosity::UpdatesOnly => format!(
            "{} ({}, {}) {}",
            card.id, card.column, card.priority, card.title
        ),
    }
}

fn render_board_mermaid(cards: &[&BoardCard]) -> String {
    let visible_ids = cards
        .iter()
        .map(|card| card.id.as_str())
        .collect::<HashSet<_>>();
    let mut output = String::from("flowchart TD\n");
    for card in cards {
        output.push_str(&format!(
            "    {}[\"{}\"]:::{}\n",
            mermaid_node_id(&card.id),
            mermaid_label(card),
            mermaid_class(&card.column)
        ));
    }
    for card in cards {
        for dep in &card.depends_on {
            if visible_ids.contains(dep.as_str()) {
                output.push_str(&format!(
                    "    {} --> {}\n",
                    mermaid_node_id(dep),
                    mermaid_node_id(&card.id)
                ));
            }
        }
    }
    output.push_str("    classDef planned fill:#eef,stroke:#668;\n");
    output.push_str("    classDef doing fill:#ffe8a3,stroke:#a66;\n");
    output.push_str("    classDef done fill:#d6f5d6,stroke:#686;\n");
    output.push_str("    classDef failed fill:#ffd6d6,stroke:#a66;\n");
    output.push_str("    classDef other fill:#eee,stroke:#888;\n");
    output
}

fn mermaid_node_id(id: &str) -> String {
    let mut output = String::from("card_");
    for c in id.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            output.push(c);
        } else {
            output.push('_');
        }
    }
    output
}

fn mermaid_class(column: &str) -> &'static str {
    match column {
        "planned" => "planned",
        "doing" => "doing",
        "done" => "done",
        "failed" => "failed",
        _ => "other",
    }
}

fn mermaid_label(card: &BoardCard) -> String {
    let label = format!("{} ({}) {}", card.id, card.column, card.title);
    label.replace('"', "'")
}

fn render_ready_cards(board: &TaskBoard) -> String {
    let ready = board.get_ready_cards();
    let cards_by_id = board
        .cards
        .iter()
        .map(|card| (card.id.as_str(), card))
        .collect::<HashMap<_, _>>();
    let mut output = String::new();

    output.push_str(&format!("# Ready Cards ({})\n\n", ready.ready.len()));
    if ready.ready.is_empty() {
        output.push_str("None\n\n");
    } else {
        output.push_str("| Card | Title | Priority | Depends On | Brief |\n");
        output.push_str("|------|-------|----------|------------|-------|\n");
        for card_id in &ready.ready {
            if let Some(card) = cards_by_id.get(card_id.as_str()) {
                output.push_str(&format!(
                    "| {} | {} | {} | {} | {} |\n",
                    markdown_cell(&card.id),
                    markdown_cell(&card.title),
                    markdown_cell(&card.priority),
                    markdown_cell(&depends_on_label(&card.depends_on)),
                    markdown_cell(&excerpt(&card.instructions, 80))
                ));
            }
        }
        output.push('\n');
    }

    output.push_str(&format!("# Blocked ({})\n", ready.blocked.len()));
    if ready.blocked.is_empty() {
        output.push_str("None\n\n");
    } else {
        output.push_str("| Card | Title | Waiting On |\n");
        output.push_str("|------|-------|------------|\n");
        for card_id in &ready.blocked {
            if let Some(card) = cards_by_id.get(card_id.as_str()) {
                output.push_str(&format!(
                    "| {} | {} | {} |\n",
                    markdown_cell(&card.id),
                    markdown_cell(&card.title),
                    markdown_cell(&waiting_on(card, &cards_by_id))
                ));
            }
        }
        output.push('\n');
    }

    output.push_str(&format!("# In Progress ({})\n", ready.in_progress.len()));
    if ready.in_progress.is_empty() {
        output.push_str("None\n\n");
    } else {
        let items = ready
            .in_progress
            .iter()
            .filter_map(|card_id| cards_by_id.get(card_id.as_str()))
            .map(|card| {
                format!(
                    "{} ({})",
                    card.id,
                    elapsed_label(card.started_at.as_deref())
                )
            })
            .collect::<Vec<_>>();
        output.push_str(&items.join(", "));
        output.push_str("\n\n");
    }

    output.push_str("# Completed\n");
    output.push_str(&format!(
        "{} cards (use board_get to list)\n\n",
        ready.completed.len()
    ));

    output.push_str("# Failed\n");
    if ready.failed.is_empty() {
        output.push_str("None\n");
    } else {
        output.push_str(&ready.failed.join(", "));
        output.push('\n');
    }
    output
}

fn waiting_on(card: &BoardCard, cards_by_id: &HashMap<&str, &BoardCard>) -> String {
    let missing = card
        .depends_on
        .iter()
        .filter(|dep| {
            cards_by_id
                .get(dep.as_str())
                .map(|dep_card| dep_card.column != "done")
                .unwrap_or(true)
        })
        .cloned()
        .collect::<Vec<_>>();
    depends_on_label(&missing)
}

fn depends_on_label(depends_on: &[String]) -> String {
    if depends_on.is_empty() {
        "(none)".to_string()
    } else {
        depends_on.join(", ")
    }
}

fn elapsed_label(started_at: Option<&str>) -> String {
    let Some(started_at) = started_at else {
        return "unknown".to_string();
    };
    let Ok(started_at) = DateTime::parse_from_rfc3339(started_at) else {
        return "unknown".to_string();
    };
    let elapsed = Utc::now().signed_duration_since(started_at.with_timezone(&Utc));
    if elapsed.num_hours() >= 1 {
        format!("{}h", elapsed.num_hours())
    } else if elapsed.num_minutes() >= 1 {
        format!("{}m", elapsed.num_minutes())
    } else {
        format!("{}s", elapsed.num_seconds().max(0))
    }
}

fn excerpt(text: &str, max_chars: usize) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_chars {
        return compact;
    }
    compact.chars().take(max_chars).collect::<String>()
}

fn markdown_cell(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

fn board_get_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "task_id": {
                "type": "string",
                "description": "Task UUID (optional if in task context)"
            },
            "card_id": {
                "type": "string",
                "description": "Card ID to get details for (optional)"
            },
            "filter": {
                "type": "object",
                "description": "Optional board filters used when card_id is omitted",
                "properties": {
                    "column": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Columns to include"
                    },
                    "priority": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Priorities to include"
                    },
                    "assignee": {
                        "type": "string",
                        "description": "Assignee to include"
                    }
                }
            },
            "mode": {
                "type": "string",
                "description": "Output mode when card_id is omitted: summary, tree, or mermaid"
            },
            "verbosity": {
                "type": "string",
                "description": "Output verbosity: minimal, brief, full, or updates_only for card_id"
            }
        },
        "required": []
    })
}

async fn get_task_id(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    args: &HashMap<String, Value>,
) -> Result<String, String> {
    if let Some(id) = args
        .get("task_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        return Ok(id.to_string());
    }
    let ccx_lock = ccx.lock().await;
    if let Some(ref meta) = ccx_lock.task_meta {
        return Ok(meta.task_id.clone());
    }
    storage::infer_task_id_from_chat_id(&ccx_lock.chat_id)
        .ok_or_else(|| "Missing 'task_id' (and chat is not bound to a task)".to_string())
}

pub struct ToolTaskBoardGet;
pub struct ToolTaskBoardCreateCard;
pub struct ToolTaskBoardUpdateCard;
pub struct ToolTaskBoardMoveCard;
pub struct ToolTaskBoardDeleteCard;
pub struct ToolTaskReadyCards;

impl ToolTaskBoardGet {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ToolTaskBoardGet {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let task_id = get_task_id(&ccx, args).await?;
        let gcx = ccx.lock().await.app.gcx.clone();
        let board = storage::load_board(gcx, &task_id).await?;
        let card_id = args
            .get("card_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());

        let result = if let Some(cid) = card_id {
            let card = board
                .get_card(cid)
                .ok_or(format!("Card {} not found", cid))?;
            let verbosity = parse_board_verbosity(args, BoardVerbosity::Brief)?;
            render_card_details(card, verbosity)?
        } else {
            let mode = parse_mode(args)?;
            let verbosity = parse_board_verbosity(args, BoardVerbosity::Brief)?;
            let filter = parse_filter(args);
            let cards = board
                .cards
                .iter()
                .filter(|card| card_matches_filter(card, &filter))
                .collect::<Vec<_>>();
            match mode {
                BoardMode::Summary => render_board_summary(&board, &cards, verbosity)?,
                BoardMode::Tree => render_board_tree(&cards, verbosity),
                BoardMode::Mermaid => render_board_mermaid(&cards),
            }
        };

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

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_board_get".to_string(),
            display_name: "Task Board Get".to_string(),
            source: make_source(),
            experimental: false,
            allow_parallel: true,
            description: "Get task board state. Without card_id returns filtered board summary, dependency tree, or mermaid graph. With card_id returns compact card metadata by default; use verbosity=full for full details or updates_only for status updates.".to_string(),
            input_schema: board_get_input_schema(),
            output_schema: None,
            annotations: None,
        }
    }
}

impl ToolTaskBoardCreateCard {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ToolTaskBoardCreateCard {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (is_planner, gcx) = {
            let ccx_lock = ccx.lock().await;
            let is_planner = ccx_lock
                .task_meta
                .as_ref()
                .map(|m| m.role == "planner")
                .unwrap_or(false);
            let gcx = ccx_lock.app.gcx.clone();
            (is_planner, gcx)
        };

        if !is_planner {
            return Err(
                "task_board_create_card can only be called by the task planner. \
                 Switch to the planner chat to create cards."
                    .to_string(),
            );
        }

        let task_id = get_task_id(&ccx, args).await?;
        let card_id = args
            .get("card_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'card_id'")?;
        let title = args
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'title'")?;
        let priority = args
            .get("priority")
            .and_then(|v| v.as_str())
            .unwrap_or("P1");
        let instructions = args
            .get("instructions")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let depends_on: Vec<String> = parse_depends_on(args.get("depends_on"));
        let target_files = parse_target_files(args.get("target_files"), instructions);
        let mut board = storage::load_board(gcx.clone(), &task_id).await?;

        if board.cards.iter().any(|c| c.id == card_id) {
            return Err(format!("Card {} already exists", card_id));
        }

        board.cards.push(BoardCard {
            id: card_id.to_string(),
            title: title.to_string(),
            column: "planned".to_string(),
            priority: priority.to_string(),
            depends_on,
            instructions: instructions.to_string(),
            assignee: None,
            agent_chat_id: None,
            status_updates: vec![],
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
            target_files,
            scope_guard_mode: Default::default(),
        });
        board.rev += 1;

        storage::save_board(gcx.clone(), &task_id, &board).await?;
        emit_task_event(
            gcx.clone(),
            TaskEvent::BoardChanged {
                task_id: task_id.to_string(),
                rev: board.rev,
                board: board.clone(),
            },
        )
        .await;
        storage::update_task_stats(gcx, &task_id).await?;

        let result = format!("Created card {} in Planned column", card_id);
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

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_board_create_card".to_string(),
            display_name: "Task Board Create Card".to_string(),
            source: make_source(),
            experimental: false,
            allow_parallel: false,
            description: "Create a new card on the task board.".to_string(),
            input_schema: json_schema_from_params(&[("card_id", "string", "Card ID (e.g., T-1, T-2)"), ("title", "string", "Card title"), ("priority", "string", "Priority: P0, P1, or P2"), ("instructions", "string", "Detailed instructions for the agent"), ("depends_on", "string", "Comma-separated list of card IDs this card depends on (e.g., \"T-1, T-2\")"), ("target_files", "string", "Comma-separated target file paths this card is expected to touch")], &["card_id", "title"]),
            output_schema: None,
            annotations: None,
        }
    }
}

impl ToolTaskBoardUpdateCard {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ToolTaskBoardUpdateCard {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (is_planner, gcx) = {
            let ccx_lock = ccx.lock().await;
            let is_planner = ccx_lock
                .task_meta
                .as_ref()
                .map(|m| m.role == "planner")
                .unwrap_or(false);
            let gcx = ccx_lock.app.gcx.clone();
            (is_planner, gcx)
        };

        if !is_planner {
            return Err(
                "task_board_update_card can only be called by the task planner. \
                 Switch to the planner chat to update cards."
                    .to_string(),
            );
        }

        let task_id = get_task_id(&ccx, args).await?;
        let card_id = args
            .get("card_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'card_id'")?;
        let mut board = storage::load_board(gcx.clone(), &task_id).await?;

        let card = board
            .get_card_mut(card_id)
            .ok_or(format!("Card {} not found", card_id))?;

        if let Some(title) = args.get("title").and_then(|v| v.as_str()) {
            card.title = title.to_string();
        }
        if let Some(priority) = args.get("priority").and_then(|v| v.as_str()) {
            card.priority = priority.to_string();
        }
        if let Some(instructions) = args.get("instructions").and_then(|v| v.as_str()) {
            card.instructions = instructions.to_string();
        }
        if args.contains_key("depends_on") {
            card.depends_on = parse_depends_on(args.get("depends_on"));
        }
        if args.contains_key("target_files") {
            card.target_files = parse_target_files(args.get("target_files"), &card.instructions);
        }

        board.rev += 1;
        storage::save_board(gcx.clone(), &task_id, &board).await?;
        emit_task_event(
            gcx,
            TaskEvent::BoardChanged {
                task_id: task_id.to_string(),
                rev: board.rev,
                board: board.clone(),
            },
        )
        .await;

        let result = format!("Updated card {}", card_id);
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

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_board_update_card".to_string(),
            display_name: "Task Board Update Card".to_string(),
            source: make_source(),
            experimental: false,
            allow_parallel: false,
            description: "Update an existing card's fields.".to_string(),
            input_schema: json_schema_from_params(
                &[
                    ("card_id", "string", "Card ID to update"),
                    ("title", "string", "New title"),
                    ("priority", "string", "New priority"),
                    ("instructions", "string", "New instructions"),
                    (
                        "depends_on",
                        "string",
                        "Comma-separated list of new dependencies (e.g., \"T-1, T-2\")",
                    ),
                    (
                        "target_files",
                        "string",
                        "Comma-separated target file paths this card is expected to touch",
                    ),
                ],
                &["card_id"],
            ),
            output_schema: None,
            annotations: None,
        }
    }
}

impl ToolTaskBoardMoveCard {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ToolTaskBoardMoveCard {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (is_planner, gcx) = {
            let ccx_lock = ccx.lock().await;
            let is_planner = ccx_lock
                .task_meta
                .as_ref()
                .map(|m| m.role == "planner")
                .unwrap_or(false);
            let gcx = ccx_lock.app.gcx.clone();
            (is_planner, gcx)
        };

        if !is_planner {
            return Err(
                "task_board_move_card can only be called by the task planner. \
                 Switch to the planner chat to move cards."
                    .to_string(),
            );
        }

        let task_id = get_task_id(&ccx, args).await?;
        let card_id = args
            .get("card_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'card_id'")?;
        let column = args
            .get("column")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'column'")?;

        let valid_columns = ["planned", "doing", "done", "failed"];
        if !valid_columns.contains(&column) {
            return Err(format!(
                "Invalid column: {}. Must be one of: {:?}",
                column, valid_columns
            ));
        }
        let mut board = storage::load_board(gcx.clone(), &task_id).await?;
        let now = Utc::now().to_rfc3339();

        let card = board
            .get_card_mut(card_id)
            .ok_or(format!("Card {} not found", card_id))?;
        let old_column = card.column.clone();

        if column == "doing" && card.started_at.is_none() {
            card.started_at = Some(now.clone());
        }
        if (column == "done" || column == "failed") && card.completed_at.is_none() {
            card.completed_at = Some(now);
        }
        card.column = column.to_string();
        board.rev += 1;

        storage::save_board(gcx.clone(), &task_id, &board).await?;
        emit_task_event(
            gcx.clone(),
            TaskEvent::BoardChanged {
                task_id: task_id.to_string(),
                rev: board.rev,
                board: board.clone(),
            },
        )
        .await;
        storage::update_task_stats(gcx, &task_id).await?;

        let result = format!("Moved card {} from {} to {}", card_id, old_column, column);
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

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_board_move_card".to_string(),
            display_name: "Task Board Move Card".to_string(),
            source: make_source(),
            experimental: false,
            allow_parallel: false,
            description: "Move a card to a different column.".to_string(),
            input_schema: json_schema_from_params(
                &[
                    ("card_id", "string", "Card ID to move"),
                    (
                        "column",
                        "string",
                        "Target column: planned, doing, done, or failed",
                    ),
                ],
                &["card_id", "column"],
            ),
            output_schema: None,
            annotations: None,
        }
    }
}

impl ToolTaskBoardDeleteCard {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ToolTaskBoardDeleteCard {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (is_planner, gcx) = {
            let ccx_lock = ccx.lock().await;
            let is_planner = ccx_lock
                .task_meta
                .as_ref()
                .map(|m| m.role == "planner")
                .unwrap_or(false);
            let gcx = ccx_lock.app.gcx.clone();
            (is_planner, gcx)
        };

        if !is_planner {
            return Err(
                "task_board_delete_card can only be called by the task planner. \
                 Switch to the planner chat to delete cards."
                    .to_string(),
            );
        }

        let task_id = get_task_id(&ccx, args).await?;
        let card_id = args
            .get("card_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'card_id'")?;
        let mut board = storage::load_board(gcx.clone(), &task_id).await?;

        let existed = board.cards.iter().any(|c| c.id == card_id);
        if !existed {
            return Err(format!("Card {} not found", card_id));
        }

        board.cards.retain(|c| c.id != card_id);
        board.rev += 1;

        storage::save_board(gcx.clone(), &task_id, &board).await?;
        emit_task_event(
            gcx.clone(),
            TaskEvent::BoardChanged {
                task_id: task_id.to_string(),
                rev: board.rev,
                board: board.clone(),
            },
        )
        .await;
        storage::update_task_stats(gcx, &task_id).await?;

        let result = format!("Deleted card {}", card_id);
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

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_board_delete_card".to_string(),
            display_name: "Task Board Delete Card".to_string(),
            source: make_source(),
            experimental: false,
            allow_parallel: false,
            description: "Delete a card from the board.".to_string(),
            input_schema: json_schema_from_params(
                &[("card_id", "string", "Card ID to delete")],
                &["card_id"],
            ),
            output_schema: None,
            annotations: None,
        }
    }
}

impl ToolTaskReadyCards {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ToolTaskReadyCards {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let task_id = get_task_id(&ccx, args).await?;

        let gcx = ccx.lock().await.app.gcx.clone();
        let board = storage::load_board(gcx, &task_id).await?;
        let result = render_ready_cards(&board);

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

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_ready_cards".to_string(),
            display_name: "Task Ready Cards".to_string(),
            source: make_source(),
            experimental: false,
            allow_parallel: true,
            description: "Get cards that are ready to be worked on (all dependencies satisfied)."
                .to_string(),
            input_schema: json_schema_from_params(
                &[(
                    "task_id",
                    "string",
                    "Task UUID (optional if in task context)",
                )],
                &[],
            ),
            output_schema: None,
            annotations: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_card_schema_includes_target_files() {
        let schema = ToolTaskBoardUpdateCard::new()
            .tool_description()
            .input_schema;
        assert!(schema["properties"].get("target_files").is_some());
        assert_eq!(
            schema["properties"]["target_files"]["type"],
            serde_json::json!("string")
        );
    }

    fn card(
        id: &str,
        title: &str,
        column: &str,
        priority: &str,
        depends_on: Vec<&str>,
    ) -> BoardCard {
        BoardCard {
            id: id.to_string(),
            title: title.to_string(),
            column: column.to_string(),
            priority: priority.to_string(),
            depends_on: depends_on.into_iter().map(String::from).collect(),
            instructions: format!("Implement {} with enough details for a short brief.", title),
            assignee: None,
            agent_chat_id: None,
            status_updates: vec![],
            final_report: None,
            final_report_structured: None,
            verifier_report: None,
            created_at: "2026-05-16T00:00:00Z".to_string(),
            started_at: None,
            last_heartbeat_at: None,
            completed_at: None,
            agent_branch: None,
            agent_worktree: None,
            agent_worktree_name: None,
            ab_variants: None,
            target_files: vec![],
            scope_guard_mode: Default::default(),
        }
    }

    fn sample_board() -> TaskBoard {
        let mut doing = card("T-22", "auto-nudge", "doing", "P0", vec![]);
        doing.started_at = Some(Utc::now().to_rfc3339());
        let mut done = card("T-21", "done prerequisite", "done", "P1", vec![]);
        done.completed_at = Some(Utc::now().to_rfc3339());
        TaskBoard {
            rev: 7,
            cards: vec![
                doing,
                done,
                card("T-23", "Task Documents", "planned", "P0", vec![]),
                card("T-24", "Memory search", "planned", "P1", vec!["T-23"]),
                card("T-28", "Scoped injection", "planned", "P2", vec!["T-24"]),
                card("T-29", "Filtered card", "planned", "P2", vec!["T-21"]),
                card("T-30", "Failed card", "failed", "P1", vec![]),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn ready_cards_renders_enriched_table() {
        let rendered = render_ready_cards(&sample_board());

        assert!(rendered.contains("# Ready Cards (2)"));
        assert!(rendered.contains("| Card | Title | Priority | Depends On | Brief |"));
        assert!(
            rendered.contains("| T-23 | Task Documents | P0 | (none) | Implement Task Documents")
        );
        assert!(rendered.contains("| T-29 | Filtered card | P2 | T-21 | Implement Filtered card"));
        assert!(rendered.contains("# Blocked (2)"));
        assert!(rendered.contains("| T-24 | Memory search | T-23 |"));
        assert!(rendered.contains("# In Progress (1)"));
        assert!(rendered.contains("T-22"));
        assert!(rendered.contains("# Completed\n1 cards (use board_get to list)"));
        assert!(rendered.contains("# Failed\nT-30"));
    }

    #[test]
    fn tree_mode_shows_dependency_structure() {
        let board = sample_board();
        let cards = board.cards.iter().collect::<Vec<_>>();
        let rendered = render_board_tree(&cards, BoardVerbosity::Brief);

        assert!(rendered.contains("T-23 (planned, P0) Task Documents"));
        assert!(rendered.contains("└─ T-24 (planned, P1) Memory search"));
        assert!(rendered.contains("└─ T-28 (planned, P2) Scoped injection"));
    }

    #[test]
    fn mermaid_mode_produces_valid_syntax() {
        let board = sample_board();
        let cards = board.cards.iter().collect::<Vec<_>>();
        let rendered = render_board_mermaid(&cards);

        assert!(rendered.starts_with("flowchart TD\n"));
        assert!(rendered.contains("card_T_23[\"T-23 (planned) Task Documents\"]:::planned"));
        assert!(rendered.contains("card_T_23 --> card_T_24"));
        assert!(rendered.contains("classDef planned"));
        assert!(rendered.contains("classDef doing"));
    }

    #[test]
    fn board_get_card_verbosity_filters_work() {
        let mut card = card("T-1", "Card", "done", "P0", vec![]);
        card.status_updates.push(crate::tasks::types::StatusUpdate {
            timestamp: "2026-05-16T00:00:00Z".to_string(),
            message: "updated".to_string(),
        });
        card.final_report = Some("final report".to_string());

        let brief = render_card_details(&card, BoardVerbosity::Brief).unwrap();
        assert!(brief.contains("id: T-1"));
        assert!(!brief.contains("status_updates"));
        assert!(!brief.contains("final_report"));

        let full = render_card_details(&card, BoardVerbosity::Full).unwrap();
        assert!(full.contains("status_updates"));
        assert!(full.contains("final_report"));

        let updates = render_card_details(&card, BoardVerbosity::UpdatesOnly).unwrap();
        assert!(updates.contains("status_updates"));
        assert!(updates.contains("updated"));
        assert!(!updates.contains("final_report"));
        assert!(!updates.contains("instructions"));
    }

    #[test]
    fn column_and_priority_filters_work() {
        let board = sample_board();
        let args = HashMap::from([(
            "filter".to_string(),
            serde_json::json!({"column": ["planned"], "priority": ["P2"]}),
        )]);
        let filter = parse_filter(&args);
        let cards = board
            .cards
            .iter()
            .filter(|card| card_matches_filter(card, &filter))
            .collect::<Vec<_>>();
        let rendered = render_board_summary(&board, &cards, BoardVerbosity::Minimal).unwrap();

        assert!(rendered.contains("id: T-28"));
        assert!(rendered.contains("id: T-29"));
        assert!(!rendered.contains("id: T-23"));
        assert!(!rendered.contains("id: T-22"));
        assert!(!rendered.contains("title:"));
    }

    #[test]
    fn board_get_schema_includes_new_params() {
        let schema = ToolTaskBoardGet::new().tool_description().input_schema;
        assert!(schema["properties"].get("filter").is_some());
        assert!(schema["properties"].get("mode").is_some());
        assert!(schema["properties"].get("verbosity").is_some());
    }
}
