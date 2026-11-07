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
use crate::tools::task_tool_helpers::require_bound_planner_task;

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
    let ccx = ccx.lock().await;
    if ccx.chat_id.is_empty() {
        return Err("This board tool requires a bound task chat".to_string());
    }
    let meta = ccx
        .task_meta
        .as_ref()
        .filter(|meta| matches!(meta.role.as_str(), "planner" | "agent" | "agents"))
        .filter(|meta| !meta.task_id.is_empty())
        .ok_or_else(|| {
            "This board tool requires a bound task planner or agent context".to_string()
        })?;
    match args.get("task_id") {
        None | Some(Value::Null) => {}
        Some(Value::String(id)) if id.trim() == meta.task_id => {}
        Some(Value::String(_)) => {
            return Err("task_id override is not allowed from this task chat".to_string());
        }
        Some(_) => return Err("task_id must be a string".to_string()),
    }
    Ok(meta.task_id.clone())
}

struct PlannerBoardContext {
    gcx: Arc<crate::global_context::GlobalContext>,
    task_id: String,
    actor: String,
}

async fn planner_context(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    args: &HashMap<String, Value>,
) -> Result<PlannerBoardContext, String> {
    let task_id = require_bound_planner_task(ccx, args).await?;
    let ccx = ccx.lock().await;
    if task_id.is_empty()
        || !ccx
            .task_meta
            .as_ref()
            .is_some_and(|meta| meta.role == "planner" && meta.task_id == task_id)
        || ccx.chat_id.is_empty()
    {
        return Err("This board tool requires a bound task planner context".to_string());
    }
    Ok(PlannerBoardContext {
        gcx: ccx.app.gcx.clone(),
        task_id,
        actor: ccx.chat_id.clone(),
    })
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
        let PlannerBoardContext {
            gcx,
            task_id,
            ..
        } = planner_context(&ccx, args).await?;
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
        let PlannerBoardContext {
            gcx,
            task_id,
            actor,
        } = planner_context(&ccx, args).await?;
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

        storage::update_board_atomic(gcx.clone(), &task_id, move |board| {
            board.authorize_owner(&actor)?;
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
                comments: vec![],
                final_report: None,
                final_report_structured: None,
                verifier_report: None,
                created_at: Utc::now().to_rfc3339(),
                started_at: None,
                last_heartbeat_at: None,
                paused_at: None,
                paused_by: None,
                completed_at: None,
                agent_branch: None,
                agent_worktree: None,
                agent_worktree_name: None,
                base_branch: None,
                base_commit: None,
                ab_variants: None,
                target_files,
                scope_guard_mode: Default::default(),
                team_members: vec![],
                attention: None,
                deliverable: None,
                verification: Vec::new(),
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
        let PlannerBoardContext {
            gcx,
            task_id,
            actor,
        } = planner_context(&ccx, args).await?;
        let card_id = args
            .get("card_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'card_id'")?;
        let title = args.get("title").and_then(|v| v.as_str());
        let priority = args.get("priority").and_then(|v| v.as_str());
        let instructions = args.get("instructions").and_then(|v| v.as_str());
        let depends_on = args
            .contains_key("depends_on")
            .then(|| parse_depends_on(args.get("depends_on")));
        let target_files_arg = args
            .contains_key("target_files")
            .then(|| args.get("target_files"));

        storage::update_board_atomic(gcx, &task_id, move |board| {
            board.authorize_owner(&actor)?;
            let card = board
                .get_card_mut(card_id)
                .ok_or(format!("Card {} not found", card_id))?;

            if let Some(title) = title {
                card.title = title.to_string();
            }
            if let Some(priority) = priority {
                card.priority = priority.to_string();
            }
            if let Some(instructions) = instructions {
                card.instructions = instructions.to_string();
            }
            if let Some(depends_on) = depends_on {
                card.depends_on = depends_on;
            }
            if let Some(target_files) = target_files_arg {
                card.target_files = parse_target_files(target_files, &card.instructions);
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
        let PlannerBoardContext {
            gcx,
            task_id,
            actor,
        } = planner_context(&ccx, args).await?;
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

        let (board, old_column) =
            storage::update_board_atomic(gcx.clone(), &task_id, move |board| {
                board.authorize_owner(&actor)?;
                board
                    .ensure_column_move_allowed(card_id, column)
                    .map_err(storage::violation_error)?;
                let card = board
                    .get_card_mut(card_id)
                    .ok_or(format!("Card {} not found", card_id))?;
                let old_column = card.column.clone();

                if column == "doing" && card.started_at.is_none() {
                    card.started_at = Some(now.clone());
                }
                if (column == "done" || column == "failed" || column == "regressed")
                    && card.completed_at.is_none()
                {
                    card.completed_at = Some(now);
                }
                card.column = column.to_string();
                Ok(old_column)
            })
            .await?;
        let _ = board;
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
        let PlannerBoardContext {
            gcx,
            task_id,
            actor,
        } = planner_context(&ccx, args).await?;
        let card_id = args
            .get("card_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'card_id'")?;
        storage::update_board_atomic(gcx.clone(), &task_id, |board| {
            board.authorize_owner(&actor)?;
            let card = board
                .get_card(card_id)
                .ok_or_else(|| format!("Card {} not found", card_id))?;
            if card.column == "doing"
                || card.assignee.is_some()
                || card.agent_chat_id.is_some()
                || card.agent_worktree.is_some()
                || card.agent_worktree_name.is_some()
                || card.agent_branch.is_some()
                || card.ab_variants.is_some()
                || !card.team_members.is_empty()
            {
                return Err(format!(
                    "Cannot delete active or referenced card {}. Use the task agent lifecycle to stop or resolve its work, preserve its evidence, and merge or explicitly clean up retained worktrees and branches before deleting an inactive card. board_delete never cancels work or forces deletion.",
                    card_id
                ));
            }
            board
                .remove_card(card_id)
                .ok_or_else(|| format!("Card {} not found", card_id))?;
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
        let PlannerBoardContext {
            gcx,
            task_id,
            ..
        } = planner_context(&ccx, args).await?;
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

pub struct ToolTaskBoardClaimCard;
pub struct ToolTaskBoardDiscover;

impl ToolTaskBoardClaimCard {
    pub fn new() -> Self {
        Self
    }
}

impl ToolTaskBoardDiscover {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ToolTaskBoardClaimCard {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let task_id = get_task_id(&ccx, args).await?;
        let (gcx, claimant) = {
            let ccx_lock = ccx.lock().await;
            (ccx_lock.app.gcx.clone(), ccx_lock.chat_id.clone())
        };
        let card_id = args
            .get("card_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or("Missing 'card_id'")?;
        let release = args
            .get("release")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let result = if release {
            let board = storage::release_card_claim(gcx, &task_id, card_id, &claimant).await?;
            json!({
                "type": "task_board_claim",
                "action": "release",
                "task_id": task_id,
                "card_id": card_id,
                "claimant_chat_id": claimant,
                "rev": board.rev,
            })
        } else {
            let idempotency_key = args
                .get("idempotency_key")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(claimant.as_str())
                .to_string();
            let (board, outcome) =
                storage::claim_card(gcx, &task_id, card_id, &claimant, &idempotency_key).await?;
            json!({
                "type": "task_board_claim",
                "action": "claim",
                "task_id": task_id,
                "card_id": card_id,
                "claimant_chat_id": claimant,
                "idempotency_key": idempotency_key,
                "already_held": outcome == crate::tasks::types::ClaimOutcome::AlreadyHeld,
                "rev": board.rev,
            })
        };

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(result.to_string()),
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
            name: "board_claim".to_string(),
            display_name: "Task Board Claim Card".to_string(),
            source: make_source(),
            experimental: false,
            allow_parallel: false,
            description: "Atomically claim a board card for this chat, or release a claim this chat holds. Claiming is a compare-and-set: a card already claimed by someone else fails with already_claimed, and repeating the same idempotency key is a no-op.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "task_id": {"type": "string", "description": "Task UUID (optional if in task context)"},
                    "card_id": {"type": "string", "description": "Card ID to claim or release"},
                    "idempotency_key": {"type": "string", "description": "Retry key; repeating the same key for the same chat re-confirms the existing claim"},
                    "release": {"type": "boolean", "description": "Release the claim this chat holds instead of acquiring one"}
                },
                "required": ["card_id"]
            }),
            output_schema: None,
            annotations: None,
        }
    }
}

#[async_trait]
impl Tool for ToolTaskBoardDiscover {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (app, chat_id, model_id) = {
            let ccx = ccx.lock().await;
            (
                ccx.app.clone(),
                ccx.chat_id.clone(),
                ccx.current_model.clone(),
            )
        };
        let caller = crate::coordination::scope::model_caller(&app, &chat_id, &model_id)
            .await
            .map_err(|error| error.to_string())?;
        let model = caller.model.as_ref().ok_or_else(|| {
            crate::coordination::error::WorkError::policy_unavailable().to_string()
        })?;
        let policy = crate::coordination::privacy_guard::policy_key(&app.gcx, model)
            .map_err(|error| error.to_string())?;
        let task_id = args
            .get("task_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());

        let (result, records) = match task_id {
            Some(task_id) => {
                let (authority, records) =
                    storage::discover_model_authority(&app.gcx, &caller, task_id)
                        .await
                        .map_err(|error| error.to_string())?;
                (
                    serde_yaml::to_string(&authority)
                        .map_err(|_| "Board discovery unavailable".to_string())?,
                    records,
                )
            }
            None => {
                let (cards, records, projection_unavailable) =
                    storage::discover_model_cards(&app.gcx, &caller)
                        .await
                        .map_err(|error| error.to_string())?;
                (
                    serde_yaml::to_string(&json!({
                        "coverage": "partial",
                        "projection_unavailable": projection_unavailable,
                        "cards": cards,
                    }))
                    .map_err(|_| "Board discovery unavailable".to_string())?,
                    records,
                )
            }
        };

        let current = crate::coordination::scope::model_caller(&app, &chat_id, &model_id)
            .await
            .map_err(|error| error.to_string())?;
        if current.source_root != caller.source_root
            || current.principal != caller.principal
            || crate::coordination::privacy_guard::policy_key(&app.gcx, model)
                .map_err(|error| error.to_string())?
                != policy
        {
            return Err(crate::coordination::error::WorkError::policy_unavailable().to_string());
        }
        let mut message = ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(result),
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        };
        crate::privacy::records::merge_records(&mut message, records);
        Ok((false, vec![ContextEnum::ChatMessage(message)]))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "board_discover".to_string(),
            display_name: "Task Board Discover".to_string(),
            source: make_source(),
            experimental: false,
            allow_parallel: true,
            description: "Read-only, privacy-cleared discovery in this chat's trusted project. Without task_id it reads the published board projection with partial coverage; with task_id it reports visible owner, reviewers, review policy, claimant/times and dependency override state, never retry keys or free-text reasons. Guessed IDs grant no visibility or control.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "task_id": {"type": "string", "description": "Report claim and role state for one task instead of listing the project"}
                },
                "required": []
            }),
            output_schema: None,
            annotations: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use crate::global_context::GlobalContext;
    use crate::tasks::types::{TaskMeta as StoredTaskMeta, TaskStatus};
    use refact_chat_api::TaskMeta as ThreadTaskMeta;

    async fn setup_task(board: TaskBoard) -> (Arc<GlobalContext>, tempfile::TempDir) {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
        let task_dir = temp.path().join(".refact").join("tasks").join("task-1");
        tokio::fs::create_dir_all(&task_dir).await.unwrap();
        let now = Utc::now().to_rfc3339();
        storage::save_task_meta(
            gcx.clone(),
            "task-1",
            &StoredTaskMeta {
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
            },
        )
        .await
        .unwrap();
        storage::create_board(gcx.clone(), "task-1", &board)
            .await
            .unwrap();
        (gcx, temp)
    }

    async fn chat_ccx(gcx: Arc<GlobalContext>, chat_id: &str) -> Arc<AMutex<AtCommandsContext>> {
        Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                AppState::from_gcx(gcx).await,
                4096,
                20,
                false,
                Vec::new(),
                chat_id.to_string(),
                None,
                "model".to_string(),
                Some(ThreadTaskMeta {
                    task_id: "task-1".to_string(),
                    role: "planner".to_string(),
                    agent_id: None,
                    card_id: None,
                    planner_chat_id: Some(chat_id.to_string()),
                }),
                None,
            )
            .await,
        ))
    }

    fn tool_text(contexts: Vec<ContextEnum>) -> String {
        match contexts
            .into_iter()
            .next()
            .expect("tool produced no message")
        {
            ContextEnum::ChatMessage(message) => message.content.content_text_only(),
            _ => panic!("expected a tool chat message"),
        }
    }

    #[tokio::test]
    async fn delete_attested_card_prunes_metadata_before_reusing_id() {
        let report = crate::tasks::types::VerifierReport {
            passed: true,
            ..Default::default()
        };
        let mut done = card("T-1", "Reviewed", "done", "P1", vec![]);
        done.verifier_report = Some(report.clone());
        let mut board = TaskBoard {
            cards: vec![done],
            owner_chat_id: Some("planner-chat".into()),
            reviewer_chat_ids: vec!["reviewer-chat".into()],
            ..Default::default()
        };
        board
            .attest_review("T-1", "reviewer-chat", &report, "t0".into(), false)
            .unwrap();
        board
            .claim_card("T-1", "agent-chat", "key", "t0".into())
            .unwrap();
        board
            .record_dependency_override("T-1", "planner-chat", "verified", "t0".into())
            .unwrap();
        board
            .finalize_card("T-1", "planner-chat", false, "t0".into())
            .unwrap();
        let (gcx, _temp) = setup_task(board).await;
        let ccx = chat_ccx(gcx.clone(), "planner-chat").await;

        let inactive = storage::load_board(gcx.clone(), "task-1").await.unwrap();
        storage::update_board_atomic(gcx.clone(), "task-1", |board| {
            board.cards[0].column = "doing".into();
            Ok(())
        })
        .await
        .unwrap();
        let active = storage::load_board(gcx.clone(), "task-1").await.unwrap();
        let denied = ToolTaskBoardDeleteCard::new()
            .tool_execute(
                ccx.clone(),
                &"active-delete".into(),
                &HashMap::from([("card_id".to_string(), json!("T-1"))]),
            )
            .await
            .unwrap_err();
        assert!(denied.contains("Cannot delete active or referenced card T-1"));
        assert!(denied.contains("task agent lifecycle"));
        assert_eq!(
            serde_json::to_value(storage::load_board(gcx.clone(), "task-1").await.unwrap())
                .unwrap(),
            serde_json::to_value(&active).unwrap()
        );
        storage::update_board_atomic(gcx.clone(), "task-1", |board| {
            board.cards = inactive.cards;
            board.cards[0].agent_branch = Some("retained-agent-branch".into());
            Ok(())
        })
        .await
        .unwrap();
        let referenced = storage::load_board(gcx.clone(), "task-1").await.unwrap();
        let denied = ToolTaskBoardDeleteCard::new()
            .tool_execute(
                ccx.clone(),
                &"referenced-delete".into(),
                &HashMap::from([("card_id".to_string(), json!("T-1"))]),
            )
            .await
            .unwrap_err();
        assert!(denied.contains("Cannot delete active or referenced card T-1"));
        assert_eq!(
            serde_json::to_value(storage::load_board(gcx.clone(), "task-1").await.unwrap())
                .unwrap(),
            serde_json::to_value(&referenced).unwrap()
        );
        storage::update_board_atomic(gcx.clone(), "task-1", |board| {
            board.cards[0].agent_branch = None;
            Ok(())
        })
        .await
        .unwrap();

        ToolTaskBoardDeleteCard::new()
            .tool_execute(
                ccx.clone(),
                &"delete".to_string(),
                &HashMap::from([("card_id".to_string(), json!("T-1"))]),
            )
            .await
            .expect("deleting an attested and finalized card must persist");

        let deleted = storage::load_board(gcx.clone(), "task-1").await.unwrap();
        assert_eq!(deleted.rev, referenced.rev + 2);
        assert!(deleted.get_card("T-1").is_none());
        assert!(deleted.review_attestations.is_empty());
        assert!(deleted.card_claims.is_empty());
        assert!(deleted.dependency_overrides.is_empty());
        assert!(deleted.card_outcomes.is_empty());

        ToolTaskBoardCreateCard::new()
            .tool_execute(
                ccx,
                &"create".to_string(),
                &HashMap::from([
                    ("card_id".to_string(), json!("T-1")),
                    ("title".to_string(), json!("Replacement")),
                ]),
            )
            .await
            .unwrap();

        let recreated = storage::load_board(gcx, "task-1").await.unwrap();
        assert_eq!(recreated.rev, deleted.rev + 1);
        assert_eq!(recreated.get_card("T-1").unwrap().column, "planned");
        assert!(!recreated.card_is_finalized("T-1"));
        assert!(recreated.review_attestations.is_empty());
        assert!(recreated.card_claims.is_empty());
        assert!(recreated.dependency_overrides.is_empty());
        assert!(recreated.card_outcomes.is_empty());
    }

    #[tokio::test]
    async fn moving_to_doing_is_blocked_until_an_override_is_recorded() {
        let board = TaskBoard {
            cards: vec![
                card("T-1", "Dependency", "planned", "P1", vec![]),
                card("T-2", "Dependent", "planned", "P1", vec!["T-1"]),
            ],
            ..Default::default()
        };
        let (gcx, temp) = setup_task(board).await;
        let ccx = chat_ccx(gcx.clone(), "planner-chat").await;
        let args = HashMap::from([
            ("card_id".to_string(), json!("T-2")),
            ("column".to_string(), json!("doing")),
        ]);

        let foreign_dir = temp.path().join(".refact/tasks/task-2");
        tokio::fs::create_dir_all(&foreign_dir).await.unwrap();
        let bound_path = temp.path().join(".refact/tasks/task-1/board.yaml");
        let before_override = tokio::fs::read(&bound_path).await.unwrap();
        tokio::fs::write(foreign_dir.join("board.yaml"), &before_override)
            .await
            .unwrap();
        let mut foreign_args = HashMap::from([
            ("task_id".to_string(), json!("task-2")),
            ("card_id".to_string(), json!("T-3")),
            ("title".to_string(), json!("Changed")),
            ("column".to_string(), json!("failed")),
        ]);
        let mut mutations: Vec<Box<dyn Tool>> = vec![
            Box::new(ToolTaskBoardCreateCard::new()),
            Box::new(ToolTaskBoardUpdateCard::new()),
            Box::new(ToolTaskBoardMoveCard::new()),
            Box::new(ToolTaskBoardDeleteCard::new()),
        ];
        for tool in &mut mutations {
            foreign_args.insert(
                "card_id".into(),
                json!(if tool.tool_description().name == "board_create" {
                    "T-3"
                } else {
                    "T-2"
                }),
            );
            let denied = tool
                .tool_execute(ccx.clone(), &"foreign".into(), &foreign_args)
                .await
                .unwrap_err();
            assert_eq!(
                denied,
                "task_id override is not allowed from this planner chat"
            );
        }
        assert_eq!(
            tokio::fs::read(foreign_dir.join("board.yaml"))
                .await
                .unwrap(),
            before_override
        );
        assert_eq!(tokio::fs::read(bound_path).await.unwrap(), before_override);

        let unbound = chat_ccx(gcx.clone(), "planner-task-1-1").await;
        unbound.lock().await.task_meta = None;
        for tool in &mut mutations {
            assert_eq!(
                tool.tool_execute(unbound.clone(), &"unbound".into(), &args)
                    .await
                    .unwrap_err(),
                "This board tool requires a bound task planner context"
            );
        }

        let blocked = ToolTaskBoardMoveCard::new()
            .tool_execute(ccx.clone(), &"call".to_string(), &args)
            .await
            .unwrap_err();
        let decoded: Value = serde_json::from_str(&blocked).unwrap();
        assert_eq!(decoded["code"], "dependency_unsatisfied");
        assert!(decoded["message"].as_str().unwrap().contains("T-1"));
        assert_eq!(
            storage::load_board(gcx.clone(), "task-1")
                .await
                .unwrap()
                .get_card("T-2")
                .unwrap()
                .column,
            "planned"
        );

        let terminal = HashMap::from([
            ("card_id".to_string(), json!("T-2")),
            ("column".to_string(), json!("failed")),
        ]);
        ToolTaskBoardMoveCard::new()
            .tool_execute(ccx.clone(), &"call".to_string(), &terminal)
            .await
            .unwrap();

        storage::update_board_atomic(gcx.clone(), "task-1", |board| {
            board.owner_chat_id = Some("planner-chat".to_string());
            Ok(())
        })
        .await
        .unwrap();
        storage::override_card_dependencies(
            gcx.clone(),
            "task-1",
            "T-2",
            "planner-chat",
            "dependency verified out of band",
        )
        .await
        .unwrap();

        let before = storage::load_board(gcx.clone(), "task-1").await.unwrap();
        let stranger = chat_ccx(gcx.clone(), "other-planner").await;
        assert!(crate::file_activity::current_actor().is_none());
        for tool in &mut mutations {
            let card_id = if tool.tool_description().name == "board_create" {
                "T-3"
            } else {
                "T-2"
            };
            let args = HashMap::from([
                ("card_id".to_string(), json!(card_id)),
                ("title".to_string(), json!("Changed")),
                ("column".to_string(), json!("failed")),
            ]);
            let denied = tool
                .tool_execute(stranger.clone(), &"nonowner".into(), &args)
                .await
                .unwrap_err();
            assert_eq!(
                denied,
                "task board mutation requires the owning planner chat"
            );
            assert_eq!(
                serde_json::to_value(storage::load_board(gcx.clone(), "task-1").await.unwrap())
                    .unwrap(),
                serde_json::to_value(&before).unwrap()
            );
        }

        ToolTaskBoardMoveCard::new()
            .tool_execute(ccx, &"call".to_string(), &args)
            .await
            .unwrap();

        let board = storage::load_board(gcx, "task-1").await.unwrap();
        assert_eq!(board.get_card("T-2").unwrap().column, "doing");
        assert_eq!(
            board.dependency_overrides["T-2"].reason,
            "dependency verified out of band"
        );
    }

    #[tokio::test]
    async fn board_claim_tool_gives_one_holder_and_supports_idempotent_retry() {
        let board = TaskBoard {
            cards: vec![card("T-1", "Claimable", "planned", "P1", vec![])],
            ..Default::default()
        };
        let (gcx, temp) = setup_task(board).await;
        let first = chat_ccx(gcx.clone(), "agent-one").await;
        let second = chat_ccx(gcx.clone(), "agent-two").await;
        let planner = chat_ccx(gcx.clone(), "planner-chat").await;
        first.lock().await.task_meta.as_mut().unwrap().role = "agents".into();
        second.lock().await.task_meta.as_mut().unwrap().role = "agent".into();
        let args = HashMap::from([("card_id".to_string(), json!("T-1"))]);

        let foreign_dir = temp.path().join(".refact/tasks/task-2");
        tokio::fs::create_dir_all(&foreign_dir).await.unwrap();
        let foreign = TaskBoard {
            cards: vec![card("T-1", "Foreign card", "planned", "P1", vec![])],
            ..Default::default()
        };
        let foreign_yaml = serde_yaml::to_string(&foreign).unwrap();
        tokio::fs::write(foreign_dir.join("board.yaml"), &foreign_yaml)
            .await
            .unwrap();
        assert!(storage::load_board(gcx.clone(), "task-2")
            .await
            .unwrap()
            .get_card("T-1")
            .is_some());
        let foreign_args = HashMap::from([
            ("task_id".to_string(), json!("task-2")),
            ("card_id".to_string(), json!("T-1")),
        ]);
        let mut tools: Vec<Box<dyn Tool>> = vec![
            Box::new(ToolTaskBoardGet::new()),
            Box::new(ToolTaskReadyCards::new()),
            Box::new(ToolTaskBoardClaimCard::new()),
        ];
        for tool in &mut tools {
            let (caller, expected) = if tool.tool_description().name == "board_claim" {
                (
                    first.clone(),
                    "task_id override is not allowed from this task chat",
                )
            } else {
                (
                    planner.clone(),
                    "task_id override is not allowed from this planner chat",
                )
            };
            let denied = tool
                .tool_execute(caller, &"foreign".into(), &foreign_args)
                .await
                .unwrap_err();
            assert_eq!(denied, expected);
        }
        assert_eq!(
            tokio::fs::read(foreign_dir.join("board.yaml"))
                .await
                .unwrap(),
            foreign_yaml.as_bytes()
        );
        assert!(tool_text(
            ToolTaskBoardGet::new()
                .tool_execute(planner, &"bound".into(), &args)
                .await
                .unwrap()
                .1
        )
        .contains("Claimable"));
        let unbound = chat_ccx(gcx.clone(), "planner-task-2-1").await;
        unbound.lock().await.task_meta = None;
        for tool in &mut tools {
            assert!(tool
                .tool_execute(unbound.clone(), &"unbound".into(), &args)
                .await
                .unwrap_err()
                .contains("requires a bound task"));
        }

        let claimed: Value = serde_json::from_str(&tool_text(
            ToolTaskBoardClaimCard::new()
                .tool_execute(first.clone(), &"call".to_string(), &args)
                .await
                .unwrap()
                .1,
        ))
        .unwrap();
        assert_eq!(claimed["already_held"], json!(false));

        let retried: Value = serde_json::from_str(&tool_text(
            ToolTaskBoardClaimCard::new()
                .tool_execute(first.clone(), &"call".to_string(), &args)
                .await
                .unwrap()
                .1,
        ))
        .unwrap();
        assert_eq!(retried["already_held"], json!(true));

        let denied = ToolTaskBoardClaimCard::new()
            .tool_execute(second, &"call".to_string(), &args)
            .await
            .unwrap_err();
        let decoded: Value = serde_json::from_str(&denied).unwrap();
        assert_eq!(decoded["code"], "already_claimed");

        let released = HashMap::from([
            ("card_id".to_string(), json!("T-1")),
            ("release".to_string(), json!(true)),
        ]);
        ToolTaskBoardClaimCard::new()
            .tool_execute(first, &"call".to_string(), &released)
            .await
            .unwrap();
        assert!(storage::load_board(gcx, "task-1")
            .await
            .unwrap()
            .card_claims
            .is_empty());
    }

    #[tokio::test]
    async fn board_discover_reports_state_without_conferring_control() {
        let board = TaskBoard {
            cards: vec![card("T-1", "Claimable", "planned", "P1", vec![])],
            ..Default::default()
        };
        let (gcx, temp) = setup_task(board).await;
        let workspace_root = dunce::canonicalize(temp.path()).unwrap();
        let holder = chat_ccx(gcx.clone(), "agent-one").await;
        let observer = chat_ccx(gcx.clone(), "agent-two").await;
        ToolTaskBoardClaimCard::new()
            .tool_execute(
                holder,
                &"call".to_string(),
                &HashMap::from([("card_id".to_string(), json!("T-1"))]),
            )
            .await
            .unwrap();

        let source_path =
            crate::coordination::scope::canonical_candidate(&workspace_root.join("source.rs"))
                .unwrap();
        let mut board = storage::load_board(gcx.clone(), "task-1").await.unwrap();
        board.owner_chat_id = Some("owner-chat".into());
        board.reviewer_chat_ids = vec!["reviewer-chat".into()];
        board
            .record_dependency_override(
                "T-1",
                "owner-chat",
                "private free-text reason",
                "t0".into(),
            )
            .unwrap();
        let records =
            crate::privacy::records::declared_file_records(&gcx, [source_path.clone()]).unwrap();
        let provenance =
            crate::coordination::board_rows::write_provenance(&board, None, Some((records, true)));
        let mut value = serde_yaml::to_value(&board).unwrap();
        value.as_mapping_mut().unwrap().insert(
            "work_provenance".into(),
            serde_yaml::to_value(&provenance).unwrap(),
        );
        tokio::fs::write(
            workspace_root.join(".refact/tasks/task-1/board.yaml"),
            serde_yaml::to_string(&value).unwrap(),
        )
        .await
        .unwrap();
        gcx.work_boards.publish(
            &workspace_root.join(".refact/tasks/task-1"),
            Some((&board, &provenance)),
        );
        let (root, _) = crate::caps::project_model_defaults_state(gcx.clone()).await;
        let mut caps = crate::caps::CodeAssistantCaps::default();
        caps.chat_models.insert(
            "test/model".into(),
            Arc::new(crate::caps::ChatModelRecord {
                base: crate::caps::BaseModelRecord {
                    id: "test/model".into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
        );
        {
            let mut state = gcx.caps_state.write().await;
            state.caps = Some(Arc::new(caps));
            state.loaded_project_root = Some(root);
            state.last_attempted_ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
        }
        let mut session = crate::chat::types::ChatSession::new("agent-two".into());
        session.thread.model = "test/model".into();
        session.work_project_root = Some(workspace_root.clone());
        let session = Arc::new(AMutex::new(session));
        gcx.chat_sessions
            .write()
            .await
            .insert("agent-two".into(), session.clone());
        observer.lock().await.current_model = "test/model".into();
        let contexts = ToolTaskBoardDiscover::new()
            .tool_execute(
                observer.clone(),
                &"call".to_string(),
                &HashMap::from([("task_id".to_string(), json!("task-1"))]),
            )
            .await
            .unwrap()
            .1;
        let message = match &contexts[0] {
            ContextEnum::ChatMessage(message) => message,
            _ => panic!("expected message"),
        };
        assert!(message.extra.get("privacy").is_some());
        assert!(
            refact_privacy::records_from_messages(std::slice::from_ref(message))
                .unwrap()
                .iter()
                .any(|(_, record)| record.path
                    == refact_core::chat_types::normalize_file_name(
                        source_path.to_string_lossy().into_owned()
                    ))
        );
        let discovered = tool_text(contexts);

        assert!(discovered.contains("T-1"));
        assert!(discovered.contains("agent-one"));
        assert!(discovered.contains("owner-chat"));
        assert!(discovered.contains("reviewer-chat"));
        assert!(!discovered.contains("idempotency_key"));
        assert!(!discovered.contains("private free-text reason"));
        assert!(!discovered.contains("reason:"));
        let visible = tool_text(
            ToolTaskBoardDiscover::new()
                .tool_execute(observer.clone(), &"visible".into(), &HashMap::new())
                .await
                .unwrap()
                .1,
        );
        assert!(visible.contains("Claimable"));
        assert!(visible.contains("T-1"));
        let original_policy = gcx.privacy_policy_load.read().unwrap().policy.clone();
        gcx.privacy_policy_load.write().unwrap().policy = Arc::new(refact_privacy::PrivacyPolicy {
            zones: vec![refact_privacy::Zone {
                name: "guarded".into(),
                patterns: vec![source_path.to_string_lossy().into_owned()],
                send_to: vec![],
                on_shell_read: refact_privacy::ShellBehavior::Withhold,
            }],
            ..Default::default()
        });
        let hidden = tool_text(
            ToolTaskBoardDiscover::new()
                .tool_execute(observer.clone(), &"hidden".into(), &HashMap::new())
                .await
                .unwrap()
                .1,
        );
        assert!(!hidden.contains("Claimable"));
        assert!(!hidden.contains("T-1"));
        assert!(!hidden.contains("withheld"));
        assert!(!hidden.contains("guarded"));
        assert!(hidden.contains("projection_unavailable: false"));
        let authority_args = HashMap::from([("task_id".into(), json!("task-1"))]);
        let hidden_authority = ToolTaskBoardDiscover::new()
            .tool_execute(observer.clone(), &"hidden".into(), &authority_args)
            .await
            .unwrap_err();
        assert!(hidden_authority.contains("not_found_or_not_visible"));
        assert!(!hidden_authority.contains("source.rs"));
        assert_eq!(
            storage::discover_board_authority(gcx.clone(), "task-1")
                .await
                .unwrap()
                .dependency_overrides["T-1"]
                .reason,
            "private free-text reason"
        );
        gcx.privacy_policy_load.write().unwrap().policy = original_policy;
        observer.lock().await.current_model = "unknown/model".into();
        assert!(ToolTaskBoardDiscover::new()
            .tool_execute(observer.clone(), &"unknown".into(), &HashMap::new())
            .await
            .unwrap_err()
            .contains("policy_unavailable"));
        observer.lock().await.current_model = "test/model".into();
        session.lock().await.work_project_root = None;
        assert!(ToolTaskBoardDiscover::new()
            .tool_execute(observer.clone(), &"scope".into(), &authority_args)
            .await
            .unwrap_err()
            .contains("scope_denied"));
        let foreign = tempfile::tempdir().unwrap();
        session.lock().await.work_project_root = Some(foreign.path().into());
        assert!(ToolTaskBoardDiscover::new()
            .tool_execute(observer.clone(), &"foreign".into(), &authority_args)
            .await
            .is_err());
        session.lock().await.work_project_root = Some(workspace_root.clone());
        gcx.privacy_policy_load.write().unwrap().error = Some("unavailable".into());
        assert!(ToolTaskBoardDiscover::new()
            .tool_execute(observer.clone(), &"policy".into(), &HashMap::new())
            .await
            .unwrap_err()
            .contains("policy_unavailable"));
        gcx.privacy_policy_load.write().unwrap().error = None;
        let denied = ToolTaskBoardClaimCard::new()
            .tool_execute(
                observer,
                &"call".to_string(),
                &HashMap::from([("card_id".to_string(), json!("T-1"))]),
            )
            .await
            .unwrap_err();
        assert!(denied.contains("already_claimed"));
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
            paused_at: None,
            paused_by: None,
            completed_at: None,
            agent_branch: None,
            agent_worktree: None,
            agent_worktree_name: None,
            base_branch: None,
            base_commit: None,
            ab_variants: None,
            target_files: vec![],
            scope_guard_mode: Default::default(),
            team_members: vec![],
            attention: None,
            deliverable: None,
            verification: Vec::new(),
        }
    }
}
