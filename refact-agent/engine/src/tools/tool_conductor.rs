use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use refact_buddy_core::conductor::{
    ConductorGoal, ConductorMemo, GoalAutonomy, GoalLedger, GoalStatus, MemoKind,
};
use refact_buddy_core::conductor_store::{load_goal_ledger, save_goal_ledger};
use refact_chat_api::{ChatCommand, MessageOrigin};
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::buddy::actor::{buddy_enqueue_event, make_runtime_event};
use crate::buddy::events::BuddyEvent;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::global_context::GlobalContext;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

const MAX_CONTENT_CHARS: usize = 4000;

pub struct ToolConductorSteerChat;
pub struct ToolConductorSpawnPlanner;
pub struct ToolConductorMemo;
pub struct ToolConductorSetAutonomy;
pub struct ToolConductorEscalate;
pub struct ToolConductorGoalStatus;
pub struct ToolConductorSay;
pub struct ToolConductorAsk;

impl ToolConductorSteerChat {
    pub fn new() -> Self {
        Self
    }
}

impl ToolConductorSpawnPlanner {
    pub fn new() -> Self {
        Self
    }
}

impl ToolConductorMemo {
    pub fn new() -> Self {
        Self
    }
}

impl ToolConductorSetAutonomy {
    pub fn new() -> Self {
        Self
    }
}

impl ToolConductorEscalate {
    pub fn new() -> Self {
        Self
    }
}

impl ToolConductorGoalStatus {
    pub fn new() -> Self {
        Self
    }
}

impl ToolConductorSay {
    pub fn new() -> Self {
        Self
    }
}

impl ToolConductorAsk {
    pub fn new() -> Self {
        Self
    }
}

fn desc(name: &str, display_name: &str, description: &str, input_schema: Value) -> ToolDesc {
    ToolDesc {
        name: name.to_string(),
        display_name: display_name.to_string(),
        source: ToolSource {
            source_type: ToolSourceType::Builtin,
            config_path: String::new(),
        },
        experimental: false,
        allow_parallel: false,
        description: description.to_string(),
        input_schema,
        output_schema: None,
        annotations: None,
    }
}

fn required_string(args: &HashMap<String, Value>, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("Missing '{}'", key))
}

fn optional_string(args: &HashMap<String, Value>, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn cap(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn parse_autonomy(value: &str) -> Result<GoalAutonomy, String> {
    serde_json::from_value(json!(value)).map_err(|_| {
        format!("Invalid autonomy '{value}', must be one of: read_only, governed, full_auto")
    })
}

fn parse_status(value: &str) -> Result<GoalStatus, String> {
    serde_json::from_value(json!(value)).map_err(|_| {
        format!(
            "Invalid status '{value}', must be one of: planned, running, waiting_for_human, paused, done, failed, cancelled"
        )
    })
}

fn parse_memo_kind(value: Option<&Value>) -> Result<MemoKind, String> {
    let value = value.and_then(Value::as_str).unwrap_or("progress");
    serde_json::from_value(json!(value)).map_err(|_| {
        format!(
            "Invalid memo kind '{value}', must be one of: progress, decision, risk, handoff, human_steering, surgery, escalation"
        )
    })
}

fn is_done_status(args: &HashMap<String, Value>) -> bool {
    args.get("status")
        .and_then(Value::as_str)
        .is_some_and(|status| status == "done")
}

fn context_allowed_from_snapshot(thread: &refact_chat_api::ThreadParams) -> bool {
    thread
        .buddy_meta
        .as_ref()
        .is_some_and(|meta| meta.is_buddy_chat && meta.buddy_chat_kind == "conductor")
}

async fn context_goal_id(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    args: &HashMap<String, Value>,
) -> Result<String, String> {
    if let Some(goal_id) = optional_string(args, "goal_id") {
        return Ok(goal_id);
    }
    let (chat_id, chat_facade) = {
        let lock = ccx.lock().await;
        (lock.chat_id.clone(), lock.app.chat.facade.clone())
    };
    let snapshot = chat_facade.session_snapshot(&chat_id).await?;
    if !context_allowed_from_snapshot(&snapshot.thread) {
        return Err(
            "conductor tools require a conductor Buddy chat or an explicit goal_id".to_string(),
        );
    }
    snapshot
        .thread
        .buddy_meta
        .and_then(|meta| meta.goal_id)
        .map(|goal_id| goal_id.trim().to_string())
        .filter(|goal_id| !goal_id.is_empty())
        .ok_or_else(|| "conductor Buddy chat is missing goal_id".to_string())
}

async fn goal_context(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    args: &HashMap<String, Value>,
) -> Result<(Arc<GlobalContext>, String, GoalLedger), String> {
    let goal_id = context_goal_id(ccx, args).await?;
    let gcx = ccx.lock().await.app.gcx.clone();
    let project_root = conductor_project_root(gcx.clone()).await?;
    let ledger = load_goal_ledger(&project_root, &goal_id)
        .await
        .map_err(|error| error.to_string())?
        .unwrap_or_default();
    Ok((gcx, goal_id, ledger))
}

async fn conductor_project_root(gcx: Arc<GlobalContext>) -> Result<std::path::PathBuf, String> {
    crate::files_correction::get_project_dirs(gcx)
        .await
        .into_iter()
        .next()
        .ok_or_else(|| "No workspace folder found".to_string())
}

async fn persist_ledger(
    gcx: Arc<GlobalContext>,
    goal_id: &str,
    ledger: &GoalLedger,
) -> Result<(), String> {
    let project_root = conductor_project_root(gcx.clone()).await?;
    save_goal_ledger(&project_root, goal_id, ledger)
        .await
        .map_err(|error| error.to_string())?;
    crate::buddy::conductor::wake::refresh_conductor_wake_targets(gcx).await;
    Ok(())
}

fn ledger_autonomy(ledger: &GoalLedger) -> GoalAutonomy {
    ledger.autonomy.unwrap_or_default()
}

fn ledger_status(ledger: &GoalLedger) -> GoalStatus {
    ledger.status.unwrap_or_default()
}

fn ensure_can_mutate(ledger: &GoalLedger, action: &str) -> Result<(), String> {
    if ledger_autonomy(ledger) == GoalAutonomy::ReadOnly {
        return Err(format!(
            "read-only conductor goals cannot {action}; set a non-read-only autonomy first"
        ));
    }
    Ok(())
}

fn goal_from_ledger(goal_id: &str, ledger: GoalLedger) -> ConductorGoal {
    ConductorGoal {
        id: goal_id.to_string(),
        title: goal_id.to_string(),
        status: ledger_status(&ledger),
        autonomy: ledger_autonomy(&ledger),
        ledger,
        ..ConductorGoal::default()
    }
}

fn push_memo(
    ledger: &mut GoalLedger,
    kind: MemoKind,
    content: impl Into<String>,
    source_chat_id: Option<String>,
) {
    ledger.memos.push(ConductorMemo {
        id: Uuid::new_v4().to_string(),
        kind,
        content: cap(&content.into(), MAX_CONTENT_CHARS),
        created_at: Utc::now().to_rfc3339(),
        source_chat_id,
        related_task_id: ledger.planner_task_id.clone(),
    });
}

fn push_unique(values: &mut Vec<String>, value: String) {
    let value = value.trim().to_string();
    if !value.is_empty() && !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

async fn emit_goal_updated(gcx: Arc<GlobalContext>, goal_id: &str, ledger: &GoalLedger) {
    let goal = goal_from_ledger(goal_id, ledger.clone());
    if let Some(tx) = gcx.buddy_events_tx.as_ref() {
        let _ = tx.send(BuddyEvent::ConductorGoalUpdated { goal: goal.clone() });
    }
    let status = match goal.status {
        GoalStatus::WaitingForHuman | GoalStatus::Paused => "paused",
        GoalStatus::Done => "completed",
        GoalStatus::Failed | GoalStatus::Cancelled => "failed",
        _ => "running",
    };
    let event = make_runtime_event(
        "conductor_goal_updated",
        &format!("Conductor goal updated: {}", goal.id),
        "conductor",
        &format!("conductor_goal:{}", goal.id),
        status,
        Some("normal"),
    );
    buddy_enqueue_event(AppState::from_gcx(gcx).await, event).await;
}

fn tool_message(
    tool_call_id: &String,
    content: impl Into<String>,
) -> Result<(bool, Vec<ContextEnum>), String> {
    Ok((
        false,
        vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(content.into()),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        })],
    ))
}

#[async_trait]
impl Tool for ToolConductorSteerChat {
    fn tool_description(&self) -> ToolDesc {
        desc(
            "conductor_steer_chat",
            "Conductor Steer Chat",
            "Conductor-only tool that injects a conductor-origin user message into an owned chat.",
            json!({"type":"object","properties":{"goal_id":{"type":"string"},"chat_id":{"type":"string"},"message":{"type":"string"}},"required":["chat_id","message"]}),
        )
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (gcx, goal_id, mut ledger) = goal_context(&ccx, args).await?;
        ensure_can_mutate(&ledger, "steer chats")?;
        let chat_id = required_string(args, "chat_id")?;
        let message = cap(&required_string(args, "message")?, MAX_CONTENT_CHARS);
        let chat_facade = ccx.lock().await.app.chat.facade.clone();
        chat_facade
            .push_priority_command(
                &chat_id,
                ChatCommand::UserMessage {
                    content: Value::String(message.clone()),
                    origin: Some(MessageOrigin::Conductor),
                    attachments: vec![],
                    context_files: vec![],
                    suppress_auto_enrichment: true,
                },
            )
            .await?;
        push_unique(&mut ledger.chat_ids, chat_id.clone());
        push_memo(
            &mut ledger,
            MemoKind::HumanSteering,
            format!("Conductor steered chat {chat_id}: {message}"),
            Some(chat_id.clone()),
        );
        persist_ledger(gcx.clone(), &goal_id, &ledger).await?;
        emit_goal_updated(gcx, &goal_id, &ledger).await;
        tool_message(tool_call_id, format!("✅ Conductor steered chat {chat_id}"))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolConductorSpawnPlanner {
    fn tool_description(&self) -> ToolDesc {
        desc(
            "conductor_spawn_planner",
            "Conductor Spawn Planner",
            "Create a task-planner chat owned by a conductor goal.",
            json!({"type":"object","properties":{"goal_id":{"type":"string"},"instructions":{"type":"string"}},"required":["instructions"]}),
        )
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (gcx, goal_id, ledger) = goal_context(&ccx, args).await?;
        ensure_can_mutate(&ledger, "spawn planner tasks")?;
        let instructions = required_string(args, "instructions")?;
        let goal = goal_from_ledger(&goal_id, ledger);
        let spawn = crate::buddy::conductor::planner_spawn::conductor_spawn_planner_for_goal(
            gcx.clone(),
            &goal,
            &instructions,
        )
        .await?;
        emit_goal_updated(gcx, &goal_id, &spawn.ledger).await;
        tool_message(
            tool_call_id,
            format!(
                "✅ Conductor spawned planner task {} with chat {}",
                spawn.task_id, spawn.planner_chat_id
            ),
        )
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolConductorMemo {
    fn tool_description(&self) -> ToolDesc {
        desc(
            "conductor_memo",
            "Conductor Memo",
            "Persist a concise memo in the conductor goal ledger.",
            json!({"type":"object","properties":{"goal_id":{"type":"string"},"kind":{"type":"string","enum":["progress","decision","risk","handoff","human_steering","surgery","escalation"]},"content":{"type":"string"},"source_chat_id":{"type":"string"}},"required":["content"]}),
        )
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (gcx, goal_id, mut ledger) = goal_context(&ccx, args).await?;
        ensure_can_mutate(&ledger, "write memos")?;
        let kind = parse_memo_kind(args.get("kind"))?;
        let content = required_string(args, "content")?;
        let source_chat_id = optional_string(args, "source_chat_id");
        push_memo(&mut ledger, kind, content, source_chat_id);
        persist_ledger(gcx.clone(), &goal_id, &ledger).await?;
        emit_goal_updated(gcx, &goal_id, &ledger).await;
        tool_message(
            tool_call_id,
            format!("✅ Conductor memo saved for {goal_id}"),
        )
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolConductorSetAutonomy {
    fn tool_description(&self) -> ToolDesc {
        desc(
            "conductor_set_autonomy",
            "Conductor Set Autonomy",
            "Persist the conductor goal autonomy level.",
            json!({"type":"object","properties":{"goal_id":{"type":"string"},"autonomy":{"type":"string","enum":["read_only","governed","full_auto"]},"reason":{"type":"string"}},"required":["autonomy"]}),
        )
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (gcx, goal_id, mut ledger) = goal_context(&ccx, args).await?;
        let autonomy = parse_autonomy(&required_string(args, "autonomy")?)?;
        if ledger_autonomy(&ledger) == GoalAutonomy::ReadOnly {
            return Err("read-only conductor goals cannot change autonomy".to_string());
        }
        ledger.autonomy = Some(autonomy);
        push_memo(
            &mut ledger,
            MemoKind::Decision,
            optional_string(args, "reason")
                .unwrap_or_else(|| format!("Conductor autonomy set to {autonomy:?}")),
            None,
        );
        persist_ledger(gcx.clone(), &goal_id, &ledger).await?;
        emit_goal_updated(gcx, &goal_id, &ledger).await;
        tool_message(
            tool_call_id,
            format!("✅ Conductor autonomy for {goal_id} set to {autonomy:?}"),
        )
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolConductorEscalate {
    fn tool_description(&self) -> ToolDesc {
        desc(
            "conductor_escalate",
            "Conductor Escalate",
            "Mark the conductor goal as waiting for human input and record an escalation memo.",
            json!({"type":"object","properties":{"goal_id":{"type":"string"},"reason":{"type":"string"}},"required":["reason"]}),
        )
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (gcx, goal_id, mut ledger) = goal_context(&ccx, args).await?;
        ensure_can_mutate(&ledger, "escalate")?;
        let reason = required_string(args, "reason")?;
        ledger.status = Some(GoalStatus::WaitingForHuman);
        push_memo(&mut ledger, MemoKind::Escalation, reason, None);
        persist_ledger(gcx.clone(), &goal_id, &ledger).await?;
        emit_goal_updated(gcx, &goal_id, &ledger).await;
        tool_message(tool_call_id, format!("✅ Conductor escalated {goal_id}"))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolConductorGoalStatus {
    fn tool_description(&self) -> ToolDesc {
        desc(
            "conductor_goal_status",
            "Conductor Goal Status",
            "Set conductor goal status. Setting done is always allowed and does not run creation validation.",
            json!({"type":"object","properties":{"goal_id":{"type":"string"},"status":{"type":"string","enum":["planned","running","waiting_for_human","paused","done","failed","cancelled"]},"reason":{"type":"string"}},"required":["status"]}),
        )
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (gcx, goal_id, mut ledger) = goal_context(&ccx, args).await?;
        if !is_done_status(args) {
            ensure_can_mutate(&ledger, "change status")?;
        }
        let status = parse_status(&required_string(args, "status")?)?;
        ledger.status = Some(status);
        if status == GoalStatus::Done {
            ledger.last_progress_at = Some(Utc::now().to_rfc3339());
        }
        push_memo(
            &mut ledger,
            if status == GoalStatus::Done {
                MemoKind::Progress
            } else {
                MemoKind::Decision
            },
            optional_string(args, "reason")
                .unwrap_or_else(|| format!("Goal status set to {status:?}")),
            None,
        );
        persist_ledger(gcx.clone(), &goal_id, &ledger).await?;
        emit_goal_updated(gcx, &goal_id, &ledger).await;
        tool_message(
            tool_call_id,
            format!("✅ Conductor goal {goal_id} status set to {status:?}"),
        )
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolConductorSay {
    fn tool_description(&self) -> ToolDesc {
        desc(
            "conductor_say",
            "Conductor Say",
            "Emit a conductor ghost status message for the user.",
            json!({"type":"object","properties":{"goal_id":{"type":"string"},"message":{"type":"string"}},"required":["message"]}),
        )
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (gcx, goal_id, _ledger) = goal_context(&ccx, args).await?;
        let message = cap(&required_string(args, "message")?, MAX_CONTENT_CHARS);
        let source_chat_id = ccx.lock().await.chat_id.clone();
        let project_root = conductor_project_root(gcx.clone()).await?;
        crate::buddy::conductor::ghost::conductor_ghost_say(
            gcx,
            &project_root,
            &goal_id,
            &message,
            Some(source_chat_id),
        )
        .await
        .map_err(|error| error.message)?;
        tool_message(tool_call_id, format!("✅ Conductor said: {message}"))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolConductorAsk {
    fn tool_description(&self) -> ToolDesc {
        desc(
            "conductor_ask",
            "Conductor Ask",
            "Ask the user a conductor ghost question and persist it as pending in the goal ledger.",
            json!({"type":"object","properties":{"goal_id":{"type":"string"},"question":{"type":"string"},"blocking":{"type":"boolean"}},"required":["question"]}),
        )
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (gcx, goal_id, ledger) = goal_context(&ccx, args).await?;
        ensure_can_mutate(&ledger, "ask questions")?;
        let question = cap(&required_string(args, "question")?, MAX_CONTENT_CHARS);
        let blocking = args
            .get("blocking")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let source_chat_id = ccx.lock().await.chat_id.clone();
        let project_root = conductor_project_root(gcx.clone()).await?;
        let result = crate::buddy::conductor::ghost::conductor_ghost_ask(
            gcx.clone(),
            &project_root,
            &goal_id,
            &question,
            blocking,
            Some(source_chat_id),
        )
        .await
        .map_err(|error| error.message)?;
        let ledger = load_goal_ledger(&project_root, &goal_id)
            .await
            .map_err(|error| error.to_string())?
            .unwrap_or_default();
        emit_goal_updated(gcx, &goal_id, &ledger).await;
        tool_message(
            tool_call_id,
            format!("✅ Conductor asked [{}]: {question}", result.question.id),
        )
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::tools_description::Tool;
    use refact_buddy_core::conductor_store::load_goal_ledger;
    use refact_runtime_api::{
        ChatSessionFacade, ChatSessionSnapshot, ChatSessionUpdate, CreateSessionRequest,
        RuntimeTrajectorySnapshot, SessionState,
    };
    use std::sync::Mutex as StdMutex;

    struct MockChatFacade {
        pushed: StdMutex<Vec<(String, ChatCommand)>>,
        updates: StdMutex<usize>,
        thread: StdMutex<refact_chat_api::ThreadParams>,
    }

    impl MockChatFacade {
        fn new(thread: refact_chat_api::ThreadParams) -> Self {
            Self {
                pushed: StdMutex::new(Vec::new()),
                updates: StdMutex::new(0),
                thread: StdMutex::new(thread),
            }
        }

        fn pushed_commands(&self) -> Vec<(String, ChatCommand)> {
            self.pushed.lock().unwrap().clone()
        }

        fn update_count(&self) -> usize {
            *self.updates.lock().unwrap()
        }
    }

    #[async_trait]
    impl ChatSessionFacade for MockChatFacade {
        async fn session_snapshot(&self, _chat_id: &str) -> Result<ChatSessionSnapshot, String> {
            Ok(ChatSessionSnapshot {
                messages: vec![],
                thread: self.thread.lock().unwrap().clone(),
                session_state: SessionState::Idle,
                pause_reasons: vec![],
            })
        }

        async fn update_session(
            &self,
            _chat_id: &str,
            _update: ChatSessionUpdate,
        ) -> Result<(), String> {
            *self.updates.lock().unwrap() += 1;
            Ok(())
        }

        async fn create_session(&self, _request: CreateSessionRequest) -> Result<(), String> {
            Ok(())
        }

        async fn push_command(&self, chat_id: &str, command: ChatCommand) -> Result<(), String> {
            self.pushed
                .lock()
                .unwrap()
                .push((chat_id.to_string(), command));
            Ok(())
        }

        async fn session_state(&self, _chat_id: &str) -> Result<Option<SessionState>, String> {
            Ok(Some(SessionState::Idle))
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

    async fn gcx(root: &std::path::Path) -> Arc<GlobalContext> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.to_path_buf()];
        gcx
    }

    async fn ccx(
        gcx: Arc<GlobalContext>,
        facade: Arc<dyn ChatSessionFacade>,
    ) -> Arc<AMutex<AtCommandsContext>> {
        let mut app = AppState::from_gcx(gcx).await;
        app.chat.facade = facade;
        Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                app,
                4096,
                20,
                false,
                vec![],
                "conductor-chat".to_string(),
                None,
                "model".to_string(),
                None,
                None,
            )
            .await,
        ))
    }

    fn conductor_thread(goal_id: &str) -> refact_chat_api::ThreadParams {
        refact_chat_api::ThreadParams {
            buddy_meta: Some(refact_buddy_core::types::BuddyThreadMeta {
                is_buddy_chat: true,
                buddy_chat_kind: "conductor".to_string(),
                workflow_id: Some("buddy_conductor".to_string()),
                goal_id: Some(goal_id.to_string()),
            }),
            ..Default::default()
        }
    }

    fn normal_thread() -> refact_chat_api::ThreadParams {
        refact_chat_api::ThreadParams::default()
    }

    fn args(items: &[(&str, Value)]) -> HashMap<String, Value> {
        items
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect()
    }

    #[tokio::test]
    async fn tool_conductor_registration_includes_core_tools() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all(
            &gcx.config_dir,
        )
        .await
        .unwrap();

        let names = crate::tools::tools_list::get_available_tools(gcx)
            .await
            .into_iter()
            .map(|tool| tool.tool_description().name)
            .collect::<Vec<_>>();

        for expected in [
            "conductor_steer_chat",
            "conductor_spawn_planner",
            "conductor_memo",
            "conductor_set_autonomy",
            "conductor_escalate",
            "conductor_goal_status",
            "conductor_say",
            "conductor_ask",
        ] {
            assert!(names.contains(&expected.to_string()), "{names:?}");
        }
    }

    #[tokio::test]
    async fn tool_conductor_steer_chat_pushes_conductor_origin_command() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = gcx(dir.path()).await;
        let mock = Arc::new(MockChatFacade::new(conductor_thread("goal-steer")));
        let mut tool = ToolConductorSteerChat::new();

        tool.tool_execute(
            ccx(gcx.clone(), mock.clone()).await,
            &"call".to_string(),
            &args(&[
                ("goal_id", json!("goal-steer")),
                ("chat_id", json!("agent-chat-1")),
                ("message", json!("Please re-check the failing test")),
            ]),
        )
        .await
        .unwrap();

        let pushed = mock.pushed_commands();
        assert_eq!(pushed.len(), 1);
        assert_eq!(pushed[0].0, "agent-chat-1");
        match &pushed[0].1 {
            ChatCommand::UserMessage {
                content,
                origin,
                suppress_auto_enrichment,
                ..
            } => {
                assert_eq!(content.as_str(), Some("Please re-check the failing test"));
                assert_eq!(*origin, Some(MessageOrigin::Conductor));
                assert!(*suppress_auto_enrichment);
            }
            other => panic!("expected user message, got {other:?}"),
        }
        let ledger = load_goal_ledger(dir.path(), "goal-steer")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ledger.chat_ids, vec!["agent-chat-1".to_string()]);
    }

    #[tokio::test]
    async fn tool_conductor_done_status_is_ungated_for_read_only_goal() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = gcx(dir.path()).await;
        save_goal_ledger(
            dir.path(),
            "goal-done",
            &GoalLedger {
                autonomy: Some(GoalAutonomy::ReadOnly),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let mock = Arc::new(MockChatFacade::new(normal_thread()));
        let mut tool = ToolConductorGoalStatus::new();

        tool.tool_execute(
            ccx(gcx, mock).await,
            &"call".to_string(),
            &args(&[("goal_id", json!("goal-done")), ("status", json!("done"))]),
        )
        .await
        .unwrap();

        let ledger = load_goal_ledger(dir.path(), "goal-done")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ledger.status, Some(GoalStatus::Done));
        assert!(ledger.last_progress_at.is_some());
    }

    #[tokio::test]
    async fn tool_conductor_read_only_rejects_mutating_steer() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = gcx(dir.path()).await;
        save_goal_ledger(
            dir.path(),
            "goal-read-only",
            &GoalLedger {
                autonomy: Some(GoalAutonomy::ReadOnly),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let mock = Arc::new(MockChatFacade::new(normal_thread()));
        let mut tool = ToolConductorSteerChat::new();

        let error = tool
            .tool_execute(
                ccx(gcx, mock.clone()).await,
                &"call".to_string(),
                &args(&[
                    ("goal_id", json!("goal-read-only")),
                    ("chat_id", json!("agent-chat-1")),
                    ("message", json!("do things")),
                ]),
            )
            .await
            .unwrap_err();

        assert!(error.contains("read-only"), "{error}");
        assert!(mock.pushed_commands().is_empty());
    }

    #[tokio::test]
    async fn tool_conductor_full_auto_allows_spawn_and_steer() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = gcx(dir.path()).await;
        save_goal_ledger(
            dir.path(),
            "goal-full-auto",
            &GoalLedger {
                autonomy: Some(GoalAutonomy::FullAuto),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let mock = Arc::new(MockChatFacade::new(normal_thread()));
        let mut steer = ToolConductorSteerChat::new();
        steer
            .tool_execute(
                ccx(gcx.clone(), mock.clone()).await,
                &"steer".to_string(),
                &args(&[
                    ("goal_id", json!("goal-full-auto")),
                    ("chat_id", json!("agent-chat-1")),
                    ("message", json!("continue")),
                ]),
            )
            .await
            .unwrap();
        assert_eq!(mock.pushed_commands().len(), 1);

        let mut spawn = ToolConductorSpawnPlanner::new();
        spawn
            .tool_execute(
                ccx(gcx.clone(), Arc::new(MockChatFacade::new(normal_thread()))).await,
                &"spawn".to_string(),
                &args(&[
                    ("goal_id", json!("goal-full-auto")),
                    ("instructions", json!("Plan the next work slice.")),
                ]),
            )
            .await
            .unwrap();
        let ledger = load_goal_ledger(dir.path(), "goal-full-auto")
            .await
            .unwrap()
            .unwrap();
        assert!(ledger.planner_task_id.is_some());
        assert_eq!(ledger.autonomy, Some(GoalAutonomy::FullAuto));
    }

    #[tokio::test]
    async fn conductor_ghost_tools_do_not_mutate_target_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = gcx(dir.path()).await;
        save_goal_ledger(dir.path(), "goal-ghost-tools", &GoalLedger::default())
            .await
            .unwrap();
        let mock = Arc::new(MockChatFacade::new(conductor_thread("goal-ghost-tools")));
        let mut say = ToolConductorSay::new();
        let mut ask = ToolConductorAsk::new();

        say.tool_execute(
            ccx(gcx.clone(), mock.clone()).await,
            &"say".to_string(),
            &args(&[("message", json!("Status without transcript poking"))]),
        )
        .await
        .unwrap();
        ask.tool_execute(
            ccx(gcx.clone(), mock.clone()).await,
            &"ask".to_string(),
            &args(&[
                ("question", json!("Answer outside provider transcript?")),
                ("blocking", json!(false)),
            ]),
        )
        .await
        .unwrap();

        assert!(mock.pushed_commands().is_empty());
        assert_eq!(mock.update_count(), 0);
        let ledger = load_goal_ledger(dir.path(), "goal-ghost-tools")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ledger.ghost_messages.len(), 2);
        assert_eq!(ledger.pending_questions.len(), 1);
        assert!(!ledger.pending_questions[0].blocking);
    }

    #[tokio::test]
    async fn tool_conductor_memo_persists_content() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = gcx(dir.path()).await;
        let mock = Arc::new(MockChatFacade::new(conductor_thread("goal-memo")));
        let mut tool = ToolConductorMemo::new();

        tool.tool_execute(
            ccx(gcx, mock).await,
            &"call".to_string(),
            &args(&[
                ("goal_id", json!("goal-memo")),
                ("kind", json!("decision")),
                (
                    "content",
                    json!("Use the existing planner task before spawning more."),
                ),
            ]),
        )
        .await
        .unwrap();

        let ledger = load_goal_ledger(dir.path(), "goal-memo")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ledger.memos.len(), 1);
        assert_eq!(ledger.memos[0].kind, MemoKind::Decision);
        assert_eq!(
            ledger.memos[0].content,
            "Use the existing planner task before spawning more."
        );
    }
}
