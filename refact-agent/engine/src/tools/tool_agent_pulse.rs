use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ChatToolCall, ContextEnum};
use crate::tasks::storage;
use crate::tasks::types::BoardCard;
use crate::tools::task_tool_helpers::{
    human_age, required_string, require_bound_planner_task, truncate_chars,
};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use refact_core::string_utils::redact_sensitive;
use refact_runtime_api::{ChatSessionFacade, ChatSessionSnapshot, SessionState};

const PREVIEW_CHARS: usize = 200;
const VALUE_PREVIEW_CHARS: usize = 80;
const EDITING_TOOL_NAMES: &[&str] = &[
    "patch",
    "apply",
    "write",
    "replace_textdoc",
    "create_textdoc",
    "update_textdoc",
    "update_textdoc_regex",
    "update_textdoc_by_lines",
    "update_textdoc_anchored",
    "apply_patch",
];

pub struct ToolAgentPulse;

impl ToolAgentPulse {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LastToolCall {
    preview: String,
    currently_editing: Option<String>,
}

#[derive(Debug, Clone)]
struct AgentPulse {
    card_id: String,
    card_title: String,
    state: Option<SessionState>,
    card_column: String,
    last_activity_at: Option<DateTime<Utc>>,
    tokens_used: Option<usize>,
    token_cap: Option<usize>,
    currently_editing: Option<String>,
    last_assistant_preview: Option<String>,
    last_tool_call: Option<String>,
    session_note: Option<String>,
}

async fn fetch_session_snapshot(
    chat_facade: Arc<dyn ChatSessionFacade>,
    chat_id: Option<&str>,
) -> (
    Option<ChatSessionSnapshot>,
    Option<SessionState>,
    Option<String>,
) {
    let Some(chat_id) = chat_id else {
        return (
            None,
            None,
            Some("Card has no agent chat session; showing board state only.".to_string()),
        );
    };

    let live_state = chat_facade.session_state(chat_id).await.unwrap_or(None);
    match chat_facade.session_snapshot(chat_id).await {
        Ok(snapshot) => {
            if live_state.is_none() && snapshot.messages.is_empty() {
                (
                    Some(snapshot),
                    None,
                    Some(
                        "Session snapshot unavailable; showing last known board state.".to_string(),
                    ),
                )
            } else {
                let state = live_state.or(Some(snapshot.session_state));
                (Some(snapshot), state, None)
            }
        }
        Err(_) => (
            None,
            live_state,
            Some("Session snapshot unavailable; showing last known board state.".to_string()),
        ),
    }
}

fn build_agent_pulse(
    card: &BoardCard,
    snapshot: Option<&ChatSessionSnapshot>,
    session_state: Option<SessionState>,
    session_note: Option<String>,
    fallback_token_cap: usize,
) -> AgentPulse {
    let messages = snapshot
        .map(|snapshot| snapshot.messages.as_slice())
        .unwrap_or(&[]);
    let last_tool = last_tool_call_from_messages(messages);
    let token_cap = snapshot
        .and_then(|snapshot| snapshot.thread.context_tokens_cap)
        .or_else(|| (fallback_token_cap > 0).then_some(fallback_token_cap));

    AgentPulse {
        card_id: card.id.clone(),
        card_title: card.title.clone(),
        state: session_state,
        card_column: card.column.clone(),
        last_activity_at: last_activity_timestamp(card),
        tokens_used: tokens_used(messages),
        token_cap,
        currently_editing: last_tool
            .as_ref()
            .and_then(|tool_call| tool_call.currently_editing.clone()),
        last_assistant_preview: last_assistant_preview(messages),
        last_tool_call: last_tool.map(|tool_call| tool_call.preview),
        session_note,
    }
}

fn render_agent_pulse(pulse: &AgentPulse) -> String {
    render_agent_pulse_at(pulse, Utc::now())
}

fn render_agent_pulse_at(pulse: &AgentPulse, now: DateTime<Utc>) -> String {
    let last_activity = pulse
        .last_activity_at
        .map(|timestamp| {
            let seconds = now.signed_duration_since(timestamp).num_seconds().max(0);
            human_age(Utc::now() - chrono::Duration::seconds(seconds))
        })
        .unwrap_or_else(|| "unknown".to_string());
    let currently_editing = pulse.currently_editing.as_deref().unwrap_or("unknown");
    let assistant = pulse.last_assistant_preview.as_deref().unwrap_or("(none)");
    let mut result = format!(
        "# Agent Pulse: {}\n\n**Card:** {}\n**State:** {}\n**Last activity:** {}\n**Tokens used:** {}\n**Currently editing:** {}\n",
        pulse.card_id,
        pulse.card_title,
        format_session_state(pulse.state, &pulse.card_column),
        last_activity,
        format_tokens(pulse.tokens_used, pulse.token_cap),
        currently_editing
    );

    if let Some(note) = &pulse.session_note {
        result.push_str("\n");
        result.push_str(note);
        result.push_str("\n");
    }

    result.push_str("\n## Last assistant message\n> ");
    result.push_str(&markdown_quote_text(assistant));
    result.push_str("\n\n## Last tool call\n");
    match pulse.last_tool_call.as_deref() {
        Some(tool_call) => {
            result.push('`');
            result.push_str(&inline_code_text(tool_call));
            result.push_str("`\n");
        }
        None => result.push_str("(none)\n"),
    }
    result
}

fn format_session_state(state: Option<SessionState>, card_column: &str) -> String {
    match state {
        Some(SessionState::Idle) => "💤 idle".to_string(),
        Some(SessionState::Generating) => "🔄 generating response".to_string(),
        Some(SessionState::ExecutingTools) => "⚙️ executing tools".to_string(),
        Some(SessionState::Paused) => "⏸️ paused".to_string(),
        Some(SessionState::WaitingIde) => "⏳ waiting for IDE".to_string(),
        Some(SessionState::WaitingUserInput) => "⏳ waiting for user input".to_string(),
        Some(SessionState::Completed) => "✅ completed".to_string(),
        Some(SessionState::Error) => "❌ error".to_string(),
        None => format!("❓ last known: {}", card_column),
    }
}

fn format_tokens(tokens_used: Option<usize>, token_cap: Option<usize>) -> String {
    match (tokens_used, token_cap) {
        (Some(used), Some(cap)) => format!(
            "~{} / {}",
            format_token_count(used),
            format_token_count(cap)
        ),
        (Some(used), None) => format!("~{}", format_token_count(used)),
        (None, Some(cap)) => format!("unknown / {}", format_token_count(cap)),
        (None, None) => "unknown".to_string(),
    }
}

fn format_token_count(value: usize) -> String {
    if value < 1_000 {
        return value.to_string();
    }
    if value % 1_000 == 0 {
        return format!("{}k", value / 1_000);
    }
    let mut text = format!("{:.1}k", value as f64 / 1_000.0);
    if text.ends_with(".0k") {
        text = format!("{}k", value / 1_000);
    }
    text
}

fn last_tool_call_from_messages(messages: &[ChatMessage]) -> Option<LastToolCall> {
    messages.iter().rev().find_map(|message| {
        message
            .tool_calls
            .as_ref()
            .and_then(|tool_calls| tool_calls.last())
            .map(|tool_call| LastToolCall {
                preview: format_tool_call(tool_call),
                currently_editing: currently_editing_from_tool_call(tool_call),
            })
    })
}

fn format_tool_call(tool_call: &ChatToolCall) -> String {
    let args_preview = format_tool_args(&tool_call.function.arguments);
    if args_preview.is_empty() {
        format!("{}()", tool_call.function.name)
    } else {
        format!("{}({})", tool_call.function.name, args_preview)
    }
}

fn format_tool_args(raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return String::new();
    }
    if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(raw) {
        let mut keys = map.keys().cloned().collect::<Vec<_>>();
        keys.sort_by(|a, b| arg_key_rank(a).cmp(&arg_key_rank(b)).then_with(|| a.cmp(b)));
        let parts = keys
            .into_iter()
            .filter_map(|key| {
                map.get(&key).map(|value| {
                    format!(
                        "{}={}",
                        key,
                        sanitize_arg_value(&key, value, VALUE_PREVIEW_CHARS)
                    )
                })
            })
            .collect::<Vec<_>>();
        return truncate_chars(&parts.join(", "), PREVIEW_CHARS);
    }
    sanitize_preview(raw, PREVIEW_CHARS)
}

fn arg_key_rank(key: &str) -> u8 {
    match key {
        "path" => 0,
        "file_path" => 1,
        "filename" => 2,
        _ => 3,
    }
}

fn sanitize_arg_value(key: &str, value: &Value, limit: usize) -> String {
    let raw = match value {
        Value::String(text) => text.clone(),
        _ => serde_json::to_string(value).unwrap_or_default(),
    };
    let collapsed = collapse_whitespace(&raw);
    let sanitized = if is_path_key(key) {
        collapsed
    } else {
        redact_sensitive(&collapsed)
    };
    truncate_chars(&sanitized, limit)
}

fn is_path_key(key: &str) -> bool {
    matches!(key, "path" | "file_path" | "filename")
}

fn currently_editing_from_tool_call(tool_call: &ChatToolCall) -> Option<String> {
    if !EDITING_TOOL_NAMES.contains(&tool_call.function.name.as_str()) {
        return None;
    }
    let Value::Object(map) = serde_json::from_str::<Value>(&tool_call.function.arguments).ok()?
    else {
        return None;
    };
    ["path", "file_path", "filename"]
        .iter()
        .find_map(|key| map.get(*key).and_then(|value| value.as_str()))
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_string)
}

fn last_assistant_preview(messages: &[ChatMessage]) -> Option<String> {
    messages
        .iter()
        .rev()
        .filter(|message| message.role == "assistant")
        .find_map(|message| {
            let text = message.content.content_text_only();
            let preview = sanitize_preview(&text, PREVIEW_CHARS);
            (!preview.is_empty()).then_some(preview)
        })
}

fn tokens_used(messages: &[ChatMessage]) -> Option<usize> {
    let mut total = 0usize;
    let mut found = false;
    for message in messages
        .iter()
        .filter(|message| message.role == "assistant")
    {
        if let Some(usage) = &message.usage {
            found = true;
            let fallback_total = usage.prompt_tokens.saturating_add(usage.completion_tokens);
            total = total.saturating_add(usage.total_tokens.max(fallback_total));
        }
    }
    found.then_some(total)
}

fn sanitize_preview(text: &str, limit: usize) -> String {
    let redacted = redact_sensitive(text);
    let collapsed = collapse_whitespace(&redacted);
    truncate_chars(&collapsed, limit).trim().to_string()
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn markdown_quote_text(text: &str) -> String {
    text.replace('\n', "\n> ")
}

fn inline_code_text(text: &str) -> String {
    text.replace('`', "'")
}

fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.with_timezone(&Utc))
}

fn last_activity_timestamp(card: &BoardCard) -> Option<DateTime<Utc>> {
    let last_status_update_at = card
        .status_updates
        .last()
        .and_then(|update| parse_timestamp(&update.timestamp));
    latest_timestamp([
        card.last_heartbeat_at.as_deref().and_then(parse_timestamp),
        last_status_update_at,
        card.completed_at.as_deref().and_then(parse_timestamp),
        card.started_at.as_deref().and_then(parse_timestamp),
        parse_timestamp(&card.created_at),
    ])
}

fn latest_timestamp(
    times: impl IntoIterator<Item = Option<DateTime<Utc>>>,
) -> Option<DateTime<Utc>> {
    times.into_iter().flatten().max()
}

#[async_trait]
impl Tool for ToolAgentPulse {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "agent_pulse".to_string(),
            display_name: "Agent Pulse".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Show a live planner-only pulse for one task agent: state, activity age, token usage, current edit target, last assistant preview, and last tool call.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "card_id": {"type": "string", "description": "Card ID whose agent pulse to inspect"},
                    "task_id": {"type": "string", "description": "Task ID (optional if chat is bound to a task)"}
                },
                "required": ["card_id"]
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
        let is_planner = {
            let ccx_lock = ccx.lock().await;
            ccx_lock
                .task_meta
                .as_ref()
                .map(|meta| meta.role == "planner")
                .unwrap_or(false)
        };
        if !is_planner {
            return Err(
                "agent_pulse can only be called by the task planner. Switch to the planner chat to inspect agent pulse."
                    .to_string(),
            );
        }

        let card_id = required_string(args, "card_id")?;
        let task_id = require_bound_planner_task(&ccx, args).await?;
        let (gcx, chat_facade, fallback_token_cap) = {
            let ccx_lock = ccx.lock().await;
            (
                ccx_lock.app.gcx.clone(),
                ccx_lock.app.chat.facade.clone(),
                ccx_lock.n_ctx,
            )
        };
        let board = storage::load_board(gcx, &task_id).await?;
        let card = board
            .get_card(&card_id)
            .ok_or_else(|| format!("Card {} not found", card_id))?;
        let (snapshot, session_state, session_note) =
            fetch_session_snapshot(chat_facade, card.agent_chat_id.as_deref()).await;
        let pulse = build_agent_pulse(
            card,
            snapshot.as_ref(),
            session_state,
            session_note,
            fallback_token_cap,
        );
        let result = render_agent_pulse(&pulse);

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
    use crate::tasks::types::{StatusUpdate, TaskBoard, TaskMeta, TaskStatus};
    use crate::tools::tools_description::Tool;
    use refact_chat_api::{ChatCommand, ThreadParams};
    use refact_runtime_api::{ChatSessionUpdate, CreateSessionRequest, RuntimeTrajectorySnapshot};
    use std::sync::Mutex as StdMutex;

    #[derive(Default)]
    struct MockChatSessionFacade {
        snapshots: StdMutex<HashMap<String, Result<ChatSessionSnapshot, String>>>,
        states: StdMutex<HashMap<String, Result<Option<SessionState>, String>>>,
    }

    #[async_trait]
    impl ChatSessionFacade for MockChatSessionFacade {
        async fn session_snapshot(&self, chat_id: &str) -> Result<ChatSessionSnapshot, String> {
            self.snapshots
                .lock()
                .unwrap()
                .get(chat_id)
                .cloned()
                .unwrap_or_else(|| Err("missing session".to_string()))
        }

        async fn update_session(
            &self,
            _chat_id: &str,
            _update: ChatSessionUpdate,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn create_session(&self, _request: CreateSessionRequest) -> Result<(), String> {
            Ok(())
        }

        async fn push_command(&self, _chat_id: &str, _command: ChatCommand) -> Result<(), String> {
            Ok(())
        }

        async fn session_state(&self, chat_id: &str) -> Result<Option<SessionState>, String> {
            self.states
                .lock()
                .unwrap()
                .get(chat_id)
                .cloned()
                .unwrap_or(Ok(None))
        }

        async fn maybe_save_session(&self, _chat_id: &str) -> Result<(), String> {
            Ok(())
        }

        async fn save_trajectory_snapshot(
            &self,
            _snapshot: RuntimeTrajectorySnapshot,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    fn test_card(agent_chat_id: Option<String>) -> BoardCard {
        let now = Utc::now();
        let created_at = (now - chrono::Duration::minutes(1)).to_rfc3339();
        let heartbeat_at = (now - chrono::Duration::seconds(12)).to_rfc3339();
        BoardCard {
            id: "T-29".to_string(),
            title: "check_agents redesign".to_string(),
            column: "doing".to_string(),
            priority: "P1".to_string(),
            depends_on: vec![],
            instructions: String::new(),
            assignee: Some("agent-1".to_string()),
            agent_chat_id,
            status_updates: vec![StatusUpdate {
                timestamp: created_at.clone(),
                message: "started".to_string(),
            }],
            comments: vec![],
            final_report: None,
            final_report_structured: None,
            verifier_report: None,
            created_at: created_at.clone(),
            started_at: Some(created_at),
            last_heartbeat_at: Some(heartbeat_at),
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

    fn task_meta() -> TaskMeta {
        let now = "2026-05-22T00:00:00Z".to_string();
        TaskMeta {
            schema_version: 1,
            id: "task-1".to_string(),
            name: "Task".to_string(),
            status: TaskStatus::Active,
            created_at: now.clone(),
            updated_at: now,
            cards_total: 1,
            cards_done: 0,
            cards_failed: 0,
            agents_active: 1,
            base_branch: None,
            base_commit: None,
            default_agent_model: None,
            is_name_generated: false,
            last_agents_summary_at: None,
            planner_session_state: None,
        }
    }

    async fn write_task(
        root: &std::path::Path,
        card: BoardCard,
    ) -> Arc<crate::global_context::GlobalContext> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let task_dir = root.join(".refact").join("tasks").join("task-1");
        tokio::fs::create_dir_all(&task_dir).await.unwrap();
        let mut board = TaskBoard::default();
        board.cards.push(card);
        tokio::fs::write(
            task_dir.join("meta.yaml"),
            serde_yaml::to_string(&task_meta()).unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::write(
            task_dir.join("board.yaml"),
            serde_yaml::to_string(&board).unwrap(),
        )
        .await
        .unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.to_path_buf()];
        gcx
    }

    async fn planner_ccx(
        gcx: Arc<crate::global_context::GlobalContext>,
        role: &str,
        facade: Arc<dyn ChatSessionFacade>,
    ) -> Arc<AMutex<AtCommandsContext>> {
        let mut app = AppState::from_gcx(gcx).await;
        app.chat.facade = facade;
        Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                app,
                200_000,
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

    fn tool_call(name: &str, arguments: Value) -> ChatToolCall {
        ChatToolCall {
            id: "call-1".to_string(),
            index: Some(0),
            function: crate::call_validation::ChatToolFunction {
                name: name.to_string(),
                arguments: arguments.to_string(),
            },
            tool_type: "function".to_string(),
            extra_content: None,
        }
    }

    #[test]
    fn tool_agent_pulse_description_is_correct() {
        let desc = ToolAgentPulse::new().tool_description();

        assert_eq!(desc.name, "agent_pulse");
        assert_eq!(desc.input_schema["required"], json!(["card_id"]));
        assert!(desc.input_schema["properties"].get("task_id").is_some());
        assert!(desc.description.contains("planner-only"));
    }

    #[tokio::test]
    async fn tool_agent_pulse_rejects_non_planner_role() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = write_task(temp.path(), test_card(Some("agent-chat-1".to_string()))).await;
        let ccx = planner_ccx(gcx, "agents", Arc::new(MockChatSessionFacade::default())).await;
        let mut tool = ToolAgentPulse::new();
        let args = HashMap::from([("card_id".to_string(), json!("T-29"))]);

        let err = tool
            .tool_execute(ccx, &"call".to_string(), &args)
            .await
            .unwrap_err();

        assert!(err.contains("can only be called by the task planner"));
    }

    #[tokio::test]
    async fn tool_agent_pulse_session_not_found_returns_graceful_message() {
        let temp = tempfile::tempdir().unwrap();
        let facade = Arc::new(MockChatSessionFacade::default());
        facade.snapshots.lock().unwrap().insert(
            "agent-chat-1".to_string(),
            Err("session missing".to_string()),
        );
        let gcx = write_task(temp.path(), test_card(Some("agent-chat-1".to_string()))).await;
        let ccx = planner_ccx(gcx, "planner", facade).await;
        let mut tool = ToolAgentPulse::new();
        let args = HashMap::from([
            ("card_id".to_string(), json!("T-29")),
            ("task_id".to_string(), json!("task-1")),
        ]);

        let output = tool_output_text(
            tool.tool_execute(ccx, &"call".to_string(), &args)
                .await
                .unwrap(),
        );

        assert!(output.contains("# Agent Pulse: T-29"));
        assert!(output.contains("**State:** ❓ last known: doing"));
        assert!(output.contains("Session snapshot unavailable"));
        assert!(output.contains("## Last assistant message\n> (none)"));
        assert!(output.contains("## Last tool call\n(none)"));
    }

    #[tokio::test]
    async fn tool_agent_pulse_rejects_mismatched_task_id() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let ccx = planner_ccx(gcx, "planner", Arc::new(MockChatSessionFacade::default())).await;
        let mut tool = ToolAgentPulse::new();
        let args = HashMap::from([
            ("card_id".to_string(), json!("T-29")),
            ("task_id".to_string(), json!("task-2")),
        ]);

        let err = tool
            .tool_execute(ccx, &"call".to_string(), &args)
            .await
            .unwrap_err();

        assert_eq!(err, "task_id override is not allowed from this planner chat");
    }

    #[tokio::test]
    async fn tool_agent_pulse_extracts_tool_and_message_previews() {
        let temp = tempfile::tempdir().unwrap();
        let facade = Arc::new(MockChatSessionFacade::default());
        let assistant_text = format!(
            "Adding sticky alerts logic to format_agent_status. The plan is to {} token=supersecret",
            "continue carefully. ".repeat(20)
        );
        let snapshot = ChatSessionSnapshot {
            messages: vec![
                ChatMessage {
                    role: "assistant".to_string(),
                    content: ChatContent::SimpleText("Earlier".to_string()),
                    usage: Some(crate::call_validation::ChatUsage {
                        prompt_tokens: 10_000,
                        completion_tokens: 5_000,
                        total_tokens: 15_000,
                        cache_creation_tokens: None,
                        cache_read_tokens: None,
                        metering_usd: None,
                    }),
                    ..Default::default()
                },
                ChatMessage {
                    role: "assistant".to_string(),
                    content: ChatContent::SimpleText(assistant_text),
                    tool_calls: Some(vec![tool_call(
                        "patch",
                        json!({
                            "path": "src/tools/tool_task_check_agents.rs",
                            "old_str": "old code",
                            "replacement": "new code"
                        }),
                    )]),
                    usage: Some(crate::call_validation::ChatUsage {
                        prompt_tokens: 20_000,
                        completion_tokens: 3_000,
                        total_tokens: 23_000,
                        cache_creation_tokens: None,
                        cache_read_tokens: None,
                        metering_usd: None,
                    }),
                    thinking_blocks: Some(vec![json!({"type": "thinking", "thinking": "hidden"})]),
                    ..Default::default()
                },
            ],
            thread: ThreadParams {
                context_tokens_cap: Some(200_000),
                ..Default::default()
            },
            session_state: SessionState::ExecutingTools,
        };
        facade
            .snapshots
            .lock()
            .unwrap()
            .insert("agent-chat-1".to_string(), Ok(snapshot));
        let gcx = write_task(temp.path(), test_card(Some("agent-chat-1".to_string()))).await;
        let ccx = planner_ccx(gcx, "planner", facade).await;
        let mut tool = ToolAgentPulse::new();
        let args = HashMap::from([("card_id".to_string(), json!("T-29"))]);

        let output = tool_output_text(
            tool.tool_execute(ccx, &"call".to_string(), &args)
                .await
                .unwrap(),
        );

        assert!(output.contains("**State:** ⚙️ executing tools"));
        assert!(output.contains("**Last activity:** 12s ago"));
        assert!(output.contains("**Tokens used:** ~38k / 200k"));
        assert!(output.contains("**Currently editing:** src/tools/tool_task_check_agents.rs"));
        assert!(output.contains("`patch(path=src/tools/tool_task_check_agents.rs"));
        assert!(output.contains("Adding sticky alerts logic to format_agent_status"));
        assert!(output.contains('…'));
        assert!(!output.contains("hidden"));
        assert!(!output.contains("supersecret"));
    }
}
