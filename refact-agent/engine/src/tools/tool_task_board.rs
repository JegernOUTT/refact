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
use crate::tools::task_tool_helpers::{optional_id_string, resolve_readonly_task_id};

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
        "regressed" => "failed",
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

async fn get_readonly_task_id(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    args: &HashMap<String, Value>,
    tool_name: &str,
) -> Result<String, String> {
    match optional_id_string(args, "task_id")? {
        Some(_) => resolve_readonly_task_id(ccx, args, tool_name).await,
        None if args.contains_key("task_id") => {
            let mut fallback_args = args.clone();
            fallback_args.remove("task_id");
            get_task_id(ccx, &fallback_args).await
        }
        None => get_task_id(ccx, args).await,
    }
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
        let task_id = get_readonly_task_id(&ccx, args, "board_get").await?;
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
            name: "board_get".to_string(),
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
            return Err("board_create can only be called by the task planner. \
                 Switch to the planner chat to create cards."
                .to_string());
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
        let card_id_owned = card_id.to_string();
        let title_owned = title.to_string();
        let priority_owned = priority.to_string();
        let instructions_owned = instructions.to_string();

        storage::update_board_atomic(gcx.clone(), &task_id, move |board| {
            if board.cards.iter().any(|c| c.id == card_id_owned) {
                return Err(format!("Card {} already exists", card_id_owned));
            }

            board.cards.push(BoardCard {
                id: card_id_owned,
                title: title_owned,
                column: "planned".to_string(),
                priority: priority_owned,
                depends_on,
                instructions: instructions_owned,
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
                target_files,
                scope_guard_mode: Default::default(),
                team_members: vec![],
            });
            Ok(())
        })
        .await?;
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
            name: "board_create".to_string(),
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
            return Err("board_update can only be called by the task planner. \
                 Switch to the planner chat to update cards."
                .to_string());
        }

        let task_id = get_task_id(&ccx, args).await?;
        let card_id = args
            .get("card_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'card_id'")?;
        let card_id_owned = card_id.to_string();
        let title = args
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let priority = args
            .get("priority")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let instructions = args
            .get("instructions")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let depends_on = args
            .contains_key("depends_on")
            .then(|| parse_depends_on(args.get("depends_on")));
        let target_files_arg = args.get("target_files").cloned();

        storage::update_board_atomic(gcx, &task_id, move |board| {
            let card = board
                .get_card_mut(&card_id_owned)
                .ok_or(format!("Card {} not found", card_id_owned))?;

            if let Some(title) = title {
                card.title = title;
            }
            if let Some(priority) = priority {
                card.priority = priority;
            }
            if let Some(instructions) = instructions {
                card.instructions = instructions;
            }
            if let Some(depends_on) = depends_on {
                card.depends_on = depends_on;
            }
            if let Some(target_files_arg) = target_files_arg.as_ref() {
                card.target_files = parse_target_files(Some(target_files_arg), &card.instructions);
            }
            Ok(())
        })
        .await?;

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
            name: "board_update".to_string(),
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
            return Err("board_move can only be called by the task planner. \
                 Switch to the planner chat to move cards."
                .to_string());
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

        let valid_columns = ["planned", "doing", "done", "failed", "regressed"];
        if !valid_columns.contains(&column) {
            return Err(format!(
                "Invalid column: {}. Must be one of: {:?}",
                column, valid_columns
            ));
        }
        let now = Utc::now().to_rfc3339();
        let card_id_owned = card_id.to_string();
        let column_owned = column.to_string();

        let (_, old_column) = storage::update_board_atomic(gcx.clone(), &task_id, move |board| {
            let card = board
                .get_card_mut(&card_id_owned)
                .ok_or(format!("Card {} not found", card_id_owned))?;
            let old_column = card.column.clone();

            if column_owned == "doing" && card.started_at.is_none() {
                card.started_at = Some(now.clone());
            }
            if (column_owned == "done" || column_owned == "failed" || column_owned == "regressed")
                && card.completed_at.is_none()
            {
                card.completed_at = Some(now);
            }
            card.column = column_owned;
            Ok(old_column)
        })
        .await?;
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
            name: "board_move".to_string(),
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
                        "Target column: planned, doing, done, failed, or regressed",
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
            return Err("board_delete can only be called by the task planner. \
                 Switch to the planner chat to delete cards."
                .to_string());
        }

        let task_id = get_task_id(&ccx, args).await?;
        let card_id = args
            .get("card_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'card_id'")?;
        let card_id_owned = card_id.to_string();
        storage::update_board_atomic(gcx.clone(), &task_id, move |board| {
            let existed = board.cards.iter().any(|c| c.id == card_id_owned);
            if !existed {
                return Err(format!("Card {} not found", card_id_owned));
            }

            board.cards.retain(|c| c.id != card_id_owned);
            Ok(())
        })
        .await?;
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
            name: "board_delete".to_string(),
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
        let task_id = get_readonly_task_id(&ccx, args, "ready_cards").await?;

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
            name: "ready_cards".to_string(),
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
    use crate::app_state::AppState;
    use crate::tasks::types::{TaskMeta, TaskStatus};
    use refact_buddy_core::user_action::UserAction;

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
            comments: vec![],
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
            team_members: vec![],
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

    async fn unbound_ccx(
        gcx: Arc<crate::global_context::GlobalContext>,
    ) -> Arc<AMutex<AtCommandsContext>> {
        Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                AppState::from_gcx(gcx).await,
                4096,
                20,
                false,
                vec![],
                "unbound-chat".to_string(),
                None,
                "model".to_string(),
                None,
                None,
            )
            .await,
        ))
    }

    async fn planner_ccx(
        gcx: Arc<crate::global_context::GlobalContext>,
        task_id: &str,
    ) -> Arc<AMutex<AtCommandsContext>> {
        Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                AppState::from_gcx(gcx).await,
                4096,
                20,
                false,
                vec![],
                format!("planner-{}-1", task_id),
                None,
                "model".to_string(),
                Some(crate::chat::types::TaskMeta {
                    task_id: task_id.to_string(),
                    role: "planner".to_string(),
                    agent_id: None,
                    card_id: None,
                    planner_chat_id: Some(format!("planner-{}-1", task_id)),
                }),
                None,
            )
            .await,
        ))
    }

    async fn write_task(
        task_id: &str,
        board: TaskBoard,
    ) -> (tempfile::TempDir, Arc<crate::global_context::GlobalContext>) {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
        let task_dir = temp.path().join(".refact/tasks").join(task_id);
        tokio::fs::create_dir_all(&task_dir).await.unwrap();
        let now = Utc::now().to_rfc3339();
        storage::save_task_meta(
            gcx.clone(),
            task_id,
            &TaskMeta {
                schema_version: 1,
                id: task_id.to_string(),
                name: "Task".to_string(),
                status: TaskStatus::Active,
                created_at: now.clone(),
                updated_at: now,
                cards_total: board.cards.len(),
                cards_done: board
                    .cards
                    .iter()
                    .filter(|card| card.column == "done")
                    .count(),
                cards_failed: board
                    .cards
                    .iter()
                    .filter(|card| card.column == "failed" || card.column == "regressed")
                    .count(),
                agents_active: 0,
                base_branch: None,
                base_commit: None,
                default_agent_model: None,
                is_name_generated: false,
                last_agents_summary_at: None,
                planner_session_state: None,
                conductor: None,
            },
        )
        .await
        .unwrap();
        storage::save_board(gcx.clone(), task_id, &board)
            .await
            .unwrap();
        (temp, gcx)
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

    async fn run_board_tool(
        tool: &mut dyn Tool,
        ccx: Arc<AMutex<AtCommandsContext>>,
        args: HashMap<String, Value>,
    ) -> Result<String, String> {
        tool.tool_execute(ccx, &"call".to_string(), &args)
            .await
            .map(output_text)
    }

    #[tokio::test]
    async fn board_mutation_tools_use_atomic_update() {
        let task_id = "atomic-board-tools";
        let (_temp, gcx) = write_task(
            task_id,
            TaskBoard {
                rev: 7,
                cards: vec![card("T-1", "One", "planned", "P1", vec![])],
                ..Default::default()
            },
        )
        .await;
        let ccx = planner_ccx(gcx.clone(), task_id).await;

        let created = run_board_tool(
            &mut ToolTaskBoardCreateCard::new(),
            ccx.clone(),
            HashMap::from([
                ("card_id".to_string(), json!("T-2")),
                ("title".to_string(), json!("Two")),
                ("instructions".to_string(), json!("Touch src/lib.rs")),
            ]),
        )
        .await
        .unwrap();
        assert_eq!(created, "Created card T-2 in Planned column");
        let board = storage::load_board(gcx.clone(), task_id).await.unwrap();
        assert_eq!(board.rev, 8);
        assert!(board.get_card("T-2").is_some());

        let updated = run_board_tool(
            &mut ToolTaskBoardUpdateCard::new(),
            ccx.clone(),
            HashMap::from([
                ("card_id".to_string(), json!("T-2")),
                ("priority".to_string(), json!("P0")),
            ]),
        )
        .await
        .unwrap();
        assert_eq!(updated, "Updated card T-2");
        let board = storage::load_board(gcx.clone(), task_id).await.unwrap();
        assert_eq!(board.rev, 9);
        assert_eq!(board.get_card("T-2").unwrap().priority, "P0");

        let moved = run_board_tool(
            &mut ToolTaskBoardMoveCard::new(),
            ccx.clone(),
            HashMap::from([
                ("card_id".to_string(), json!("T-2")),
                ("column".to_string(), json!("doing")),
            ]),
        )
        .await
        .unwrap();
        assert_eq!(moved, "Moved card T-2 from planned to doing");
        let board = storage::load_board(gcx.clone(), task_id).await.unwrap();
        assert_eq!(board.rev, 10);
        assert_eq!(board.get_card("T-2").unwrap().column, "doing");
        assert!(board.get_card("T-2").unwrap().started_at.is_some());

        let deleted = run_board_tool(
            &mut ToolTaskBoardDeleteCard::new(),
            ccx,
            HashMap::from([("card_id".to_string(), json!("T-2"))]),
        )
        .await
        .unwrap();
        assert_eq!(deleted, "Deleted card T-2");
        let board = storage::load_board(gcx, task_id).await.unwrap();
        assert_eq!(board.rev, 11);
        assert!(board.get_card("T-2").is_none());
    }

    #[tokio::test]
    async fn board_mutation_errors_do_not_change_rev() {
        let task_id = "atomic-board-errors";
        let (_temp, gcx) = write_task(
            task_id,
            TaskBoard {
                rev: 3,
                cards: vec![card("T-1", "One", "planned", "P1", vec![])],
                ..Default::default()
            },
        )
        .await;
        let ccx = planner_ccx(gcx.clone(), task_id).await;

        let duplicate = run_board_tool(
            &mut ToolTaskBoardCreateCard::new(),
            ccx.clone(),
            HashMap::from([
                ("card_id".to_string(), json!("T-1")),
                ("title".to_string(), json!("Duplicate")),
            ]),
        )
        .await
        .unwrap_err();
        assert_eq!(duplicate, "Card T-1 already exists");
        assert_eq!(
            storage::load_board(gcx.clone(), task_id).await.unwrap().rev,
            3
        );

        let missing_update = run_board_tool(
            &mut ToolTaskBoardUpdateCard::new(),
            ccx.clone(),
            HashMap::from([
                ("card_id".to_string(), json!("missing")),
                ("title".to_string(), json!("Missing")),
            ]),
        )
        .await
        .unwrap_err();
        assert_eq!(missing_update, "Card missing not found");
        assert_eq!(
            storage::load_board(gcx.clone(), task_id).await.unwrap().rev,
            3
        );

        let missing_move = run_board_tool(
            &mut ToolTaskBoardMoveCard::new(),
            ccx.clone(),
            HashMap::from([
                ("card_id".to_string(), json!("missing")),
                ("column".to_string(), json!("done")),
            ]),
        )
        .await
        .unwrap_err();
        assert_eq!(missing_move, "Card missing not found");
        assert_eq!(
            storage::load_board(gcx.clone(), task_id).await.unwrap().rev,
            3
        );

        let missing_delete = run_board_tool(
            &mut ToolTaskBoardDeleteCard::new(),
            ccx,
            HashMap::from([("card_id".to_string(), json!("missing"))]),
        )
        .await
        .unwrap_err();
        assert_eq!(missing_delete, "Card missing not found");
        let board = storage::load_board(gcx, task_id).await.unwrap();
        assert_eq!(board.rev, 3);
        assert_eq!(board.cards.len(), 1);
    }

    #[tokio::test]
    async fn board_move_failed_transition_records_task_failed() {
        let task_id = "atomic-board-failure-action";
        let mut failing_card = card("T-1", "One", "doing", "P1", vec![]);
        failing_card
            .status_updates
            .push(crate::tasks::types::StatusUpdate {
                timestamp: Utc::now().to_rfc3339(),
                message: "the gremlin tripped".to_string(),
            });
        let (_temp, gcx) = write_task(
            task_id,
            TaskBoard {
                rev: 1,
                cards: vec![failing_card],
                ..Default::default()
            },
        )
        .await;
        let ccx = planner_ccx(gcx.clone(), task_id).await;

        run_board_tool(
            &mut ToolTaskBoardMoveCard::new(),
            ccx,
            HashMap::from([
                ("card_id".to_string(), json!("T-1")),
                ("column".to_string(), json!("failed")),
            ]),
        )
        .await
        .unwrap();

        let ring = gcx.user_activity.lock().await;
        assert!(ring.snapshot().iter().any(|action| matches!(
            action,
            UserAction::TaskFailed { task_id, reason_short, .. }
                if task_id == "atomic-board-failure-action" && reason_short == "the gremlin tripped"
        )));
    }

    #[tokio::test]
    async fn concurrent_board_mutations_preserve_changes() {
        let task_id = "atomic-board-concurrent";
        let (_temp, gcx) = write_task(
            task_id,
            TaskBoard {
                rev: 0,
                cards: vec![],
                ..Default::default()
            },
        )
        .await;
        let mut handles = vec![];
        for i in 0..8 {
            let gcx = gcx.clone();
            handles.push(tokio::spawn(async move {
                storage::update_board_atomic(gcx, task_id, move |board| {
                    board.cards.push(card(
                        &format!("T-{}", i),
                        &format!("Card {}", i),
                        "planned",
                        "P1",
                        vec![],
                    ));
                    Ok(())
                })
                .await
                .unwrap();
            }));
        }
        for handle in handles {
            handle.await.unwrap();
        }

        let board = storage::load_board(gcx, task_id).await.unwrap();
        assert_eq!(board.rev, 8);
        let ids = board
            .cards
            .iter()
            .map(|card| card.id.as_str())
            .collect::<HashSet<_>>();
        for i in 0..8 {
            assert!(ids.contains(format!("T-{}", i).as_str()));
        }
    }

    #[tokio::test]
    async fn task_introspection_explicit_task_id_board_get_reads_unbound_task() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
        let task_id = "explicit-board-task";
        tokio::fs::create_dir_all(temp.path().join(".refact/tasks").join(task_id))
            .await
            .unwrap();
        storage::save_board(gcx.clone(), task_id, &sample_board())
            .await
            .unwrap();
        let ccx = unbound_ccx(gcx).await;

        let output = output_text(
            ToolTaskBoardGet::new()
                .tool_execute(
                    ccx,
                    &"call".to_string(),
                    &HashMap::from([
                        ("task_id".to_string(), serde_json::json!(task_id)),
                        ("verbosity".to_string(), serde_json::json!("minimal")),
                    ]),
                )
                .await
                .unwrap(),
        );

        assert!(output.contains("id: T-23"));
        assert!(output.contains("id: T-24"));
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
