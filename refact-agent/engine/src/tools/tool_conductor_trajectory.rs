use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use refact_buddy_core::conductor::{ConductorMemo, GoalLedger, MemoKind};
use refact_buddy_core::conductor_store::{load_goal_ledger, mutate_goal_ledger, MissingGoalBehavior};
use refact_runtime_api::{ChatSessionSnapshot, ChatSessionUpdate, SessionState};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;
use uuid::Uuid;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::buddy::conductor::trajectory::{
    build_trajectory_digest, edit_message, insert_message, remove_messages, validate_pairing,
    InsertPosition, TrajectoryDigestInput, TrajectoryPauseReasonInput,
};
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::global_context::GlobalContext;
use crate::tasks::storage;
use crate::tasks::types::TrajectoryInfo;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

const MAX_AUDIT_CHARS: usize = 4000;

pub struct ToolConductorTrajectoryList;
pub struct ToolConductorTrajectoryDigest;
pub struct ToolConductorTrajectoryEdit;
pub struct ToolConductorTrajectoryRemove;
pub struct ToolConductorTrajectoryInsert;

impl ToolConductorTrajectoryList {
    pub fn new() -> Self {
        Self
    }
}

impl ToolConductorTrajectoryDigest {
    pub fn new() -> Self {
        Self
    }
}

impl ToolConductorTrajectoryEdit {
    pub fn new() -> Self {
        Self
    }
}

impl ToolConductorTrajectoryRemove {
    pub fn new() -> Self {
        Self
    }
}

impl ToolConductorTrajectoryInsert {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Serialize)]
struct ListedConductorTrajectory {
    task_id: String,
    role: String,
    agent_id: Option<String>,
    id: String,
    title: String,
    created_at: String,
    updated_at: String,
    session_state: Option<String>,
    waiting_for_card_ids: Vec<String>,
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

fn optional_bool(args: &HashMap<String, Value>, key: &str) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn cap(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
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

async fn conductor_project_root(gcx: Arc<GlobalContext>) -> Result<std::path::PathBuf, String> {
    crate::files_correction::get_project_dirs(gcx)
        .await
        .into_iter()
        .next()
        .ok_or_else(|| "No workspace folder found".to_string())
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
            "conductor trajectory tools require a conductor Buddy chat or an explicit goal_id"
                .to_string(),
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
        .ok_or_else(|| format!("conductor goal not found: {goal_id}"))?;
    Ok((gcx, goal_id, ledger))
}

async fn mutate_existing_ledger<R>(
    gcx: Arc<GlobalContext>,
    goal_id: &str,
    updater: impl FnOnce(&mut GoalLedger) -> Result<R, String>,
) -> Result<(GoalLedger, R), String> {
    let project_root = conductor_project_root(gcx.clone()).await?;
    let updated = mutate_goal_ledger(
        &project_root,
        goal_id,
        MissingGoalBehavior::RequireExisting,
        updater,
    )
    .await
    .map_err(|error| error.to_string())?;
    crate::buddy::conductor::wake::refresh_conductor_wake_targets(gcx).await;
    Ok(updated)
}

fn push_surgery_memo(
    ledger: &mut GoalLedger,
    chat_id: &str,
    action: &str,
    detail: impl Into<String>,
    source_chat_id: Option<String>,
) {
    ledger.memos.push(ConductorMemo {
        id: Uuid::new_v4().to_string(),
        kind: MemoKind::Surgery,
        content: cap(
            &format!(
                "Trajectory surgery {action} on chat {chat_id}: {}",
                detail.into()
            ),
            MAX_AUDIT_CHARS,
        ),
        created_at: Utc::now().to_rfc3339(),
        source_chat_id,
        related_task_id: ledger.planner_task_id.clone(),
    });
}

fn ensure_owned_snapshot(
    ledger: &GoalLedger,
    chat_id: &str,
    snapshot: &ChatSessionSnapshot,
) -> Result<(), String> {
    if ledger.chat_ids.iter().any(|id| id == chat_id) {
        return Ok(());
    }
    if let Some(task_meta) = snapshot.thread.task_meta.as_ref() {
        if ledger
            .task_ids
            .iter()
            .any(|task_id| task_id == &task_meta.task_id)
        {
            return Ok(());
        }
    }
    Err(format!(
        "chat {chat_id} is not owned by this conductor goal"
    ))
}

fn ensure_surgery_state(state: SessionState) -> Result<(), String> {
    if matches!(state, SessionState::Idle | SessionState::Paused) {
        Ok(())
    } else {
        Err(format!(
            "trajectory surgery requires an idle or paused session; current state is {state}"
        ))
    }
}

fn trajectory_digest_input<'a>(
    chat_id: &'a str,
    snapshot: &'a ChatSessionSnapshot,
) -> TrajectoryDigestInput<'a> {
    let task_meta = snapshot.thread.task_meta.as_ref();
    TrajectoryDigestInput {
        chat_id,
        title: Some(snapshot.thread.title.as_str()),
        model: Some(snapshot.thread.model.as_str()),
        mode: Some(snapshot.thread.mode.as_str()),
        tool_use: Some(snapshot.thread.tool_use.as_str()),
        created_at: None,
        updated_at: None,
        version: None,
        parent_id: snapshot.thread.parent_id.as_deref(),
        link_type: snapshot.thread.link_type.as_deref(),
        root_chat_id: snapshot.thread.root_chat_id.as_deref(),
        task_id: task_meta.map(|meta| meta.task_id.as_str()),
        task_role: task_meta.map(|meta| meta.role.as_str()),
        agent_id: task_meta.and_then(|meta| meta.agent_id.as_deref()),
        card_id: task_meta.and_then(|meta| meta.card_id.as_deref()),
        session_state: Some(match snapshot.session_state {
            SessionState::Idle => "idle",
            SessionState::Generating => "generating",
            SessionState::ExecutingTools => "executing_tools",
            SessionState::Paused => "paused",
            SessionState::WaitingIde => "waiting_ide",
            SessionState::WaitingUserInput => "waiting_user_input",
            SessionState::Completed => "completed",
            SessionState::Error => "error",
        }),
        runtime_error: None,
        pause_reasons: snapshot
            .pause_reasons
            .iter()
            .map(|reason| TrajectoryPauseReasonInput {
                reason_type: reason.reason_type.as_str(),
                tool_name: reason.tool_name.as_str(),
                command: reason.command.as_str(),
                rule: reason.rule.as_str(),
                tool_call_id: reason.tool_call_id.as_str(),
            })
            .collect(),
        messages: &snapshot.messages,
    }
}

fn parse_message(args: &HashMap<String, Value>, key: &str) -> Result<ChatMessage, String> {
    let value = args
        .get(key)
        .cloned()
        .ok_or_else(|| format!("Missing '{key}'"))?;
    serde_json::from_value(value).map_err(|error| format!("Invalid '{key}': {error}"))
}

fn parse_message_ids(args: &HashMap<String, Value>) -> Result<Vec<String>, String> {
    let value = args
        .get("message_ids")
        .ok_or_else(|| "Missing 'message_ids'".to_string())?;
    let ids = value
        .as_array()
        .ok_or_else(|| "'message_ids' must be an array".to_string())?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .ok_or_else(|| "message_ids must contain non-empty strings".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ids)
}

fn parse_insert_position(args: &HashMap<String, Value>) -> Result<InsertPosition, String> {
    if let Some(index) = args.get("index").and_then(Value::as_u64) {
        return Ok(InsertPosition::Index(index as usize));
    }
    if let Some(message_id) = optional_string(args, "before_message_id") {
        return Ok(InsertPosition::BeforeMessage(message_id));
    }
    if let Some(message_id) = optional_string(args, "after_message_id") {
        return Ok(InsertPosition::AfterMessage(message_id));
    }
    Err(
        "Missing insert position: provide index, before_message_id, or after_message_id"
            .to_string(),
    )
}

async fn apply_surgery(
    ccx: Arc<AMutex<AtCommandsContext>>,
    args: &HashMap<String, Value>,
    action: &str,
    detail: String,
    build_messages: impl FnOnce(&[ChatMessage]) -> Result<Vec<ChatMessage>, String>,
) -> Result<String, String> {
    let (gcx, goal_id, ledger) = goal_context(&ccx, args).await?;
    let chat_id = required_string(args, "chat_id")?;
    let source_chat_id = ccx.lock().await.chat_id.clone();
    let chat_facade = ccx.lock().await.app.chat.facade.clone();
    let snapshot = chat_facade.session_snapshot(&chat_id).await?;
    ensure_owned_snapshot(&ledger, &chat_id, &snapshot)?;
    ensure_surgery_state(snapshot.session_state)?;
    validate_pairing(&snapshot.messages).map_err(|error| error.to_string())?;
    let messages = build_messages(&snapshot.messages)?;
    validate_pairing(&messages).map_err(|error| error.to_string())?;
    mutate_existing_ledger(gcx.clone(), &goal_id, |ledger| {
        ensure_owned_snapshot(ledger, &chat_id, &snapshot)?;
        push_surgery_memo(ledger, &chat_id, action, detail, Some(source_chat_id));
        Ok(())
    })
    .await?;
    chat_facade
        .update_session(
            &chat_id,
            ChatSessionUpdate {
                messages,
                previous_response_id: None,
            },
        )
        .await?;
    if optional_bool(args, "save") {
        chat_facade.maybe_save_session(&chat_id).await?;
    }
    Ok(chat_id)
}

fn listed(
    task_id: &str,
    role: &str,
    agent_id: Option<String>,
    info: TrajectoryInfo,
) -> ListedConductorTrajectory {
    ListedConductorTrajectory {
        task_id: task_id.to_string(),
        role: role.to_string(),
        agent_id,
        id: info.id,
        title: info.title,
        created_at: info.created_at,
        updated_at: info.updated_at,
        session_state: info.session_state,
        waiting_for_card_ids: info.waiting_for_card_ids,
    }
}

async fn list_agent_ids(gcx: Arc<GlobalContext>, task_id: &str) -> Vec<String> {
    storage::load_board(gcx, task_id)
        .await
        .map(|board| {
            board
                .cards
                .into_iter()
                .filter_map(|card| card.assignee)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

#[async_trait]
impl Tool for ToolConductorTrajectoryList {
    fn tool_description(&self) -> ToolDesc {
        desc(
            "conductor_trajectory_list",
            "Conductor Trajectory List",
            "List task trajectories owned by a conductor goal.",
            json!({
                "type":"object",
                "properties":{
                    "goal_id":{"type":"string"},
                    "task_id":{"type":"string"},
                    "role":{"type":"string","enum":["all","planner","agents"]},
                    "agent_id":{"type":"string"}
                },
                "required":[]
            }),
        )
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (gcx, goal_id, ledger) = goal_context(&ccx, args).await?;
        let role = optional_string(args, "role").unwrap_or_else(|| "all".to_string());
        if !matches!(role.as_str(), "all" | "planner" | "agents") {
            return Err("role must be one of: all, planner, agents".to_string());
        }
        let task_ids = if let Some(task_id) = optional_string(args, "task_id") {
            if !ledger.task_ids.iter().any(|owned| owned == &task_id) {
                return Err(format!(
                    "task {task_id} is not owned by this conductor goal"
                ));
            }
            vec![task_id]
        } else {
            ledger.task_ids.clone()
        };
        let mut trajectories = Vec::new();
        for task_id in task_ids {
            if role == "all" || role == "planner" {
                for info in
                    storage::list_task_trajectories(gcx.clone(), &task_id, "planner", None).await?
                {
                    trajectories.push(listed(&task_id, "planner", None, info));
                }
            }
            if role == "all" || role == "agents" {
                let agent_ids = if let Some(agent_id) = optional_string(args, "agent_id") {
                    vec![agent_id]
                } else {
                    list_agent_ids(gcx.clone(), &task_id).await
                };
                for agent_id in agent_ids {
                    for info in storage::list_task_trajectories(
                        gcx.clone(),
                        &task_id,
                        "agents",
                        Some(agent_id.as_str()),
                    )
                    .await?
                    {
                        trajectories.push(listed(&task_id, "agents", Some(agent_id.clone()), info));
                    }
                }
            }
        }
        trajectories.sort_by(|left, right| {
            left.task_id
                .cmp(&right.task_id)
                .then_with(|| left.role.cmp(&right.role))
                .then_with(|| left.agent_id.cmp(&right.agent_id))
                .then_with(|| left.id.cmp(&right.id))
        });
        let output = json!({
            "goal_id": goal_id,
            "count": trajectories.len(),
            "trajectories": trajectories,
        });
        tool_message(
            tool_call_id,
            serde_json::to_string_pretty(&output).unwrap_or_default(),
        )
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolConductorTrajectoryDigest {
    fn tool_description(&self) -> ToolDesc {
        desc(
            "conductor_trajectory_digest",
            "Conductor Trajectory Digest",
            "Build a bounded redacted digest for an owned trajectory chat.",
            json!({"type":"object","properties":{"goal_id":{"type":"string"},"chat_id":{"type":"string"}},"required":["chat_id"]}),
        )
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (_gcx, _goal_id, ledger) = goal_context(&ccx, args).await?;
        let chat_id = required_string(args, "chat_id")?;
        let chat_facade = ccx.lock().await.app.chat.facade.clone();
        let snapshot = chat_facade.session_snapshot(&chat_id).await?;
        ensure_owned_snapshot(&ledger, &chat_id, &snapshot)?;
        let built = build_trajectory_digest(trajectory_digest_input(&chat_id, &snapshot));
        let output = json!({
            "chat_id": chat_id,
            "digest": built.digest,
            "json": built.json,
            "text": built.text,
        });
        tool_message(
            tool_call_id,
            serde_json::to_string_pretty(&output).unwrap_or_default(),
        )
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolConductorTrajectoryEdit {
    fn tool_description(&self) -> ToolDesc {
        desc(
            "conductor_trajectory_edit",
            "Conductor Trajectory Edit",
            "Replace one message in an idle or paused owned trajectory after pairing validation.",
            json!({
                "type":"object",
                "properties":{
                    "goal_id":{"type":"string"},
                    "chat_id":{"type":"string"},
                    "message_id":{"type":"string"},
                    "message":{"type":"object"},
                    "reason":{"type":"string"},
                    "save":{"type":"boolean"}
                },
                "required":["chat_id","message_id","message","reason"]
            }),
        )
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let message_id = required_string(args, "message_id")?;
        let replacement = parse_message(args, "message")?;
        let reason = required_string(args, "reason")?;
        let detail = format!("message_id={message_id}; reason={reason}");
        let updated_chat_id = apply_surgery(ccx, args, "edit", detail, |messages| {
            edit_message(messages, &message_id, replacement).map_err(|error| error.to_string())
        })
        .await?;
        tool_message(
            tool_call_id,
            format!("✅ Edited trajectory message {message_id} in {updated_chat_id}"),
        )
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolConductorTrajectoryRemove {
    fn tool_description(&self) -> ToolDesc {
        desc(
            "conductor_trajectory_remove",
            "Conductor Trajectory Remove",
            "Remove messages from an idle or paused owned trajectory after pairing validation.",
            json!({
                "type":"object",
                "properties":{
                    "goal_id":{"type":"string"},
                    "chat_id":{"type":"string"},
                    "message_ids":{"type":"array","items":{"type":"string"}},
                    "reason":{"type":"string"},
                    "save":{"type":"boolean"}
                },
                "required":["chat_id","message_ids","reason"]
            }),
        )
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let message_ids = parse_message_ids(args)?;
        let reason = required_string(args, "reason")?;
        let detail = format!("message_ids={}; reason={reason}", message_ids.join(","));
        let output_ids = message_ids.join(", ");
        let updated_chat_id = apply_surgery(ccx, args, "remove", detail, |messages| {
            remove_messages(messages, message_ids.iter().map(String::as_str))
                .map_err(|error| error.to_string())
        })
        .await?;
        tool_message(
            tool_call_id,
            format!("✅ Removed trajectory messages {output_ids} from {updated_chat_id}"),
        )
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolConductorTrajectoryInsert {
    fn tool_description(&self) -> ToolDesc {
        desc(
            "conductor_trajectory_insert",
            "Conductor Trajectory Insert",
            "Insert one message into an idle or paused owned trajectory after pairing validation.",
            json!({
                "type":"object",
                "properties":{
                    "goal_id":{"type":"string"},
                    "chat_id":{"type":"string"},
                    "index":{"type":"integer","minimum":0},
                    "before_message_id":{"type":"string"},
                    "after_message_id":{"type":"string"},
                    "message":{"type":"object"},
                    "reason":{"type":"string"},
                    "save":{"type":"boolean"}
                },
                "required":["chat_id","message","reason"]
            }),
        )
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let position = parse_insert_position(args)?;
        let message = parse_message(args, "message")?;
        let inserted_id = message.message_id.clone();
        let reason = required_string(args, "reason")?;
        let detail = format!("message_id={inserted_id}; position={position:?}; reason={reason}");
        let updated_chat_id = apply_surgery(ccx, args, "insert", detail, |messages| {
            insert_message(messages, position, message).map_err(|error| error.to_string())
        })
        .await?;
        tool_message(
            tool_call_id,
            format!("✅ Inserted trajectory message {inserted_id} into {updated_chat_id}"),
        )
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use crate::call_validation::{ChatToolCall, ChatToolFunction};
    use crate::tools::tools_description::Tool;
    use refact_buddy_core::conductor_store::{load_goal_ledger, save_goal_ledger};
    use refact_chat_history::trajectory_snapshot::TrajectorySnapshot;
    use refact_runtime_api::{ChatSessionFacade, CreateSessionRequest, RuntimeTrajectorySnapshot};
    use std::sync::Mutex as StdMutex;

    struct MockChatFacade {
        snapshots: StdMutex<HashMap<String, ChatSessionSnapshot>>,
        updates: StdMutex<Vec<(String, ChatSessionUpdate)>>,
        saves: StdMutex<Vec<String>>,
    }

    impl MockChatFacade {
        fn new() -> Self {
            Self {
                snapshots: StdMutex::new(HashMap::new()),
                updates: StdMutex::new(Vec::new()),
                saves: StdMutex::new(Vec::new()),
            }
        }

        fn insert_snapshot(&self, chat_id: &str, snapshot: ChatSessionSnapshot) {
            self.snapshots
                .lock()
                .unwrap()
                .insert(chat_id.to_string(), snapshot);
        }

        fn updates(&self) -> Vec<(String, ChatSessionUpdate)> {
            self.updates.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ChatSessionFacade for MockChatFacade {
        async fn session_snapshot(&self, chat_id: &str) -> Result<ChatSessionSnapshot, String> {
            self.snapshots
                .lock()
                .unwrap()
                .get(chat_id)
                .cloned()
                .ok_or_else(|| format!("missing snapshot {chat_id}"))
        }

        async fn update_session(
            &self,
            chat_id: &str,
            update: ChatSessionUpdate,
        ) -> Result<(), String> {
            if let Some(snapshot) = self.snapshots.lock().unwrap().get_mut(chat_id) {
                snapshot.messages = update.messages.clone();
            }
            self.updates
                .lock()
                .unwrap()
                .push((chat_id.to_string(), update));
            Ok(())
        }

        async fn create_session(&self, _request: CreateSessionRequest) -> Result<(), String> {
            Ok(())
        }

        async fn push_command(
            &self,
            _chat_id: &str,
            _command: refact_chat_api::ChatCommand,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn session_state(&self, chat_id: &str) -> Result<Option<SessionState>, String> {
            Ok(self
                .snapshots
                .lock()
                .unwrap()
                .get(chat_id)
                .map(|snapshot| snapshot.session_state))
        }

        async fn maybe_save_session(&self, chat_id: &str) -> Result<(), String> {
            self.saves.lock().unwrap().push(chat_id.to_string());
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

    fn args(items: &[(&str, Value)]) -> HashMap<String, Value> {
        items
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect()
    }

    fn user(id: &str, content: &str) -> ChatMessage {
        ChatMessage {
            message_id: id.to_string(),
            role: "user".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            ..Default::default()
        }
    }

    fn assistant(id: &str, content: &str) -> ChatMessage {
        ChatMessage {
            message_id: id.to_string(),
            role: "assistant".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            ..Default::default()
        }
    }

    fn assistant_tool(id: &str, tool_call_id: &str) -> ChatMessage {
        ChatMessage {
            message_id: id.to_string(),
            role: "assistant".to_string(),
            tool_calls: Some(vec![ChatToolCall {
                id: tool_call_id.to_string(),
                index: Some(0),
                function: ChatToolFunction {
                    arguments: "{}".to_string(),
                    name: "shell".to_string(),
                },
                tool_type: "function".to_string(),
                extra_content: None,
            }]),
            ..Default::default()
        }
    }

    fn tool_result(id: &str, tool_call_id: &str) -> ChatMessage {
        ChatMessage {
            message_id: id.to_string(),
            role: "tool".to_string(),
            content: ChatContent::SimpleText("ok".to_string()),
            tool_call_id: tool_call_id.to_string(),
            ..Default::default()
        }
    }

    fn task_thread(task_id: &str) -> refact_chat_api::ThreadParams {
        refact_chat_api::ThreadParams {
            id: "agent-chat".to_string(),
            title: "Agent Chat".to_string(),
            model: "test-model".to_string(),
            mode: "task_agent".to_string(),
            tool_use: "agent".to_string(),
            task_meta: Some(refact_chat_api::TaskMeta {
                task_id: task_id.to_string(),
                role: "agents".to_string(),
                agent_id: Some("agent-1".to_string()),
                card_id: Some("T-1".to_string()),
                planner_chat_id: Some("planner-chat".to_string()),
            }),
            ..Default::default()
        }
    }

    fn snapshot(state: SessionState, messages: Vec<ChatMessage>) -> ChatSessionSnapshot {
        ChatSessionSnapshot {
            messages,
            thread: task_thread("task-1"),
            session_state: state,
            pause_reasons: vec![],
        }
    }

    async fn setup_task(root: &std::path::Path, task_id: &str) -> Arc<GlobalContext> {
        let gcx = gcx(root).await;
        let task_dir = root.join(".refact/tasks").join(task_id);
        tokio::fs::create_dir_all(task_dir.join("trajectories/planner"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(task_dir.join("trajectories/agents/agent-1"))
            .await
            .unwrap();
        let now = Utc::now().to_rfc3339();
        let meta = crate::tasks::types::TaskMeta {
            schema_version: 1,
            id: task_id.to_string(),
            name: task_id.to_string(),
            status: crate::tasks::types::TaskStatus::Active,
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
        };
        storage::save_task_meta(gcx.clone(), task_id, &meta)
            .await
            .unwrap();
        storage::save_board(
            gcx.clone(),
            task_id,
            &crate::tasks::types::TaskBoard::default(),
        )
        .await
        .unwrap();
        gcx
    }

    fn trajectory_snapshot(chat_id: &str, task_id: &str) -> TrajectorySnapshot {
        TrajectorySnapshot {
            chat_id: chat_id.to_string(),
            title: "Planner".to_string(),
            model: "test-model".to_string(),
            mode: "task_planner".to_string(),
            tool_use: "agent".to_string(),
            messages: vec![user("user-1", "hello")],
            created_at: Utc::now().to_rfc3339(),
            boost_reasoning: false,
            checkpoints_enabled: false,
            context_tokens_cap: None,
            include_project_info: false,
            is_title_generated: false,
            auto_approve_editing_tools: false,
            auto_approve_dangerous_commands: false,
            autonomous_no_confirm: false,
            version: 1,
            task_meta: Some(refact_chat_api::TaskMeta {
                task_id: task_id.to_string(),
                role: "planner".to_string(),
                agent_id: None,
                card_id: None,
                planner_chat_id: Some(chat_id.to_string()),
            }),
            worktree: None,
            parent_id: None,
            link_type: None,
            root_chat_id: None,
            reasoning_effort: None,
            thinking_budget: None,
            temperature: None,
            frequency_penalty: None,
            max_tokens: None,
            parallel_tool_calls: None,
            previous_response_id: None,
            active_skill: None,
            auto_enrichment_enabled: None,
            buddy_meta: None,
            auto_compact_enabled: None,
            frozen_request_prefix: None,
            claude_code_identity: None,
            reactive_compact_attempts: None,
            wake_up_at: None,
            waiting_for_card_ids: Vec::new(),
        }
    }

    fn output_text(result: (bool, Vec<ContextEnum>)) -> String {
        match result.1.into_iter().next().unwrap() {
            ContextEnum::ChatMessage(message) => message.content.content_text_only(),
            ContextEnum::ContextFile(_) => panic!("expected chat message"),
        }
    }

    #[tokio::test]
    async fn tool_conductor_trajectory_owned_list_uses_task_trajectories() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = setup_task(dir.path(), "task-1").await;
        crate::chat::trajectories::save_trajectory_snapshot(
            gcx.clone(),
            trajectory_snapshot("planner-chat", "task-1"),
        )
        .await
        .unwrap();
        save_goal_ledger(
            dir.path(),
            "goal-list",
            &GoalLedger {
                task_ids: vec!["task-1".to_string()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let mock = Arc::new(MockChatFacade::new());
        let mut tool = ToolConductorTrajectoryList::new();

        let output = output_text(
            tool.tool_execute(
                ccx(gcx, mock).await,
                &"call".to_string(),
                &args(&[("goal_id", json!("goal-list")), ("role", json!("planner"))]),
            )
            .await
            .unwrap(),
        );

        assert!(output.contains("planner-chat"), "{output}");
        assert!(output.contains("task-1"), "{output}");
        assert!(output.contains("planner"), "{output}");
    }

    #[tokio::test]
    async fn tool_conductor_trajectory_digest_by_chat_id() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = gcx(dir.path()).await;
        save_goal_ledger(
            dir.path(),
            "goal-digest",
            &GoalLedger {
                task_ids: vec!["task-1".to_string()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let mock = Arc::new(MockChatFacade::new());
        mock.insert_snapshot(
            "agent-chat",
            snapshot(SessionState::Paused, vec![user("user-1", "inspect")]),
        );
        let mut tool = ToolConductorTrajectoryDigest::new();

        let output = output_text(
            tool.tool_execute(
                ccx(gcx, mock).await,
                &"call".to_string(),
                &args(&[
                    ("goal_id", json!("goal-digest")),
                    ("chat_id", json!("agent-chat")),
                ]),
            )
            .await
            .unwrap(),
        );

        assert!(output.contains("Trajectory digest: agent-chat"), "{output}");
        assert!(output.contains("paused"), "{output}");
    }

    #[tokio::test]
    async fn tool_conductor_trajectory_edit_rejected_while_generating() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = gcx(dir.path()).await;
        save_goal_ledger(
            dir.path(),
            "goal-busy",
            &GoalLedger {
                task_ids: vec!["task-1".to_string()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let mock = Arc::new(MockChatFacade::new());
        mock.insert_snapshot(
            "agent-chat",
            snapshot(SessionState::Generating, vec![user("user-1", "old")]),
        );
        let mut tool = ToolConductorTrajectoryEdit::new();

        let error = tool
            .tool_execute(
                ccx(gcx, mock.clone()).await,
                &"call".to_string(),
                &args(&[
                    ("goal_id", json!("goal-busy")),
                    ("chat_id", json!("agent-chat")),
                    ("message_id", json!("user-1")),
                    (
                        "message",
                        json!({"message_id":"user-1","role":"user","content":"new"}),
                    ),
                    ("reason", json!("test busy rejection")),
                ]),
            )
            .await
            .unwrap_err();

        assert!(error.contains("idle or paused"), "{error}");
        assert!(mock.updates().is_empty());
        let ledger = load_goal_ledger(dir.path(), "goal-busy")
            .await
            .unwrap()
            .unwrap();
        assert!(ledger.memos.is_empty());
    }

    #[tokio::test]
    async fn tool_conductor_trajectory_edit_succeeds_while_paused_or_idle_and_records_memo() {
        for state in [SessionState::Paused, SessionState::Idle] {
            let dir = tempfile::tempdir().unwrap();
            let gcx = gcx(dir.path()).await;
            save_goal_ledger(
                dir.path(),
                "goal-edit",
                &GoalLedger {
                    task_ids: vec!["task-1".to_string()],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            let mock = Arc::new(MockChatFacade::new());
            mock.insert_snapshot("agent-chat", snapshot(state, vec![user("user-1", "old")]));
            let mut tool = ToolConductorTrajectoryEdit::new();

            tool.tool_execute(
                ccx(gcx, mock.clone()).await,
                &"call".to_string(),
                &args(&[
                    ("goal_id", json!("goal-edit")),
                    ("chat_id", json!("agent-chat")),
                    ("message_id", json!("user-1")),
                    (
                        "message",
                        json!({"message_id":"ignored","role":"user","content":"new"}),
                    ),
                    ("reason", json!("test edit")),
                ]),
            )
            .await
            .unwrap();

            let updates = mock.updates();
            assert_eq!(updates.len(), 1);
            assert_eq!(updates[0].0, "agent-chat");
            assert_eq!(updates[0].1.messages[0].message_id, "user-1");
            assert_eq!(updates[0].1.messages[0].content.content_text_only(), "new");
            let ledger = load_goal_ledger(dir.path(), "goal-edit")
                .await
                .unwrap()
                .unwrap();
            assert_eq!(ledger.memos.len(), 1);
            assert_eq!(ledger.memos[0].kind, MemoKind::Surgery);
            assert!(ledger.memos[0].content.contains("edit"));
        }
    }

    #[tokio::test]
    async fn tool_conductor_trajectory_pairing_violation_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = gcx(dir.path()).await;
        save_goal_ledger(
            dir.path(),
            "goal-pairing",
            &GoalLedger {
                task_ids: vec!["task-1".to_string()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let mock = Arc::new(MockChatFacade::new());
        mock.insert_snapshot(
            "agent-chat",
            snapshot(
                SessionState::Paused,
                vec![
                    user("user-1", "old"),
                    assistant_tool("assistant-tool", "call-1"),
                    tool_result("tool-1", "call-1"),
                ],
            ),
        );
        let mut tool = ToolConductorTrajectoryEdit::new();

        let error = tool
            .tool_execute(
                ccx(gcx, mock.clone()).await,
                &"call".to_string(),
                &args(&[
                    ("goal_id", json!("goal-pairing")),
                    ("chat_id", json!("agent-chat")),
                    ("message_id", json!("assistant-tool")),
                    (
                        "message",
                        json!({
                            "message_id":"assistant-tool",
                            "role":"assistant",
                            "content":"",
                            "tool_calls":[{"id":"call-2","type":"function","function":{"name":"shell","arguments":"{}"}}]
                        }),
                    ),
                    ("reason", json!("test violation")),
                ]),
            )
            .await
            .unwrap_err();

        assert!(error.contains("has no tool result"), "{error}");
        assert!(mock.updates().is_empty());
        let ledger = load_goal_ledger(dir.path(), "goal-pairing")
            .await
            .unwrap()
            .unwrap();
        assert!(ledger.memos.is_empty());
    }

    #[tokio::test]
    async fn tool_conductor_trajectory_remove_and_insert_call_update_session() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = gcx(dir.path()).await;
        save_goal_ledger(
            dir.path(),
            "goal-surgery",
            &GoalLedger {
                task_ids: vec!["task-1".to_string()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let mock = Arc::new(MockChatFacade::new());
        mock.insert_snapshot(
            "agent-chat",
            snapshot(
                SessionState::Paused,
                vec![user("user-1", "one"), assistant("assistant-1", "two")],
            ),
        );

        let ccx = ccx(gcx.clone(), mock.clone()).await;
        let mut insert = ToolConductorTrajectoryInsert::new();
        insert
            .tool_execute(
                ccx.clone(),
                &"insert".to_string(),
                &args(&[
                    ("goal_id", json!("goal-surgery")),
                    ("chat_id", json!("agent-chat")),
                    ("after_message_id", json!("user-1")),
                    (
                        "message",
                        json!({"message_id":"inserted","role":"user","content":"between"}),
                    ),
                    ("reason", json!("test insert")),
                ]),
            )
            .await
            .unwrap();

        let mut remove = ToolConductorTrajectoryRemove::new();
        remove
            .tool_execute(
                ccx,
                &"remove".to_string(),
                &args(&[
                    ("goal_id", json!("goal-surgery")),
                    ("chat_id", json!("agent-chat")),
                    ("message_ids", json!(["inserted"])),
                    ("reason", json!("test remove")),
                ]),
            )
            .await
            .unwrap();

        let updates = mock.updates();
        assert_eq!(updates.len(), 2);
        assert_eq!(
            updates[0]
                .1
                .messages
                .iter()
                .map(|message| message.message_id.as_str())
                .collect::<Vec<_>>(),
            vec!["user-1", "inserted", "assistant-1"]
        );
        assert_eq!(
            updates[1]
                .1
                .messages
                .iter()
                .map(|message| message.message_id.as_str())
                .collect::<Vec<_>>(),
            vec!["user-1", "assistant-1"]
        );
        let ledger = load_goal_ledger(dir.path(), "goal-surgery")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ledger.memos.len(), 2);
        assert!(ledger
            .memos
            .iter()
            .all(|memo| memo.kind == MemoKind::Surgery));
    }
}
