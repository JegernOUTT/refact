use std::sync::Arc;
use serde::{Deserialize, Serialize};
use crate::{call_validation::ChatMessage, global_context::GlobalContext};
use refact_chat_api::{GoalLedgerEntry, GoalOwner, GoalSnapshot};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TaskGoalRecord {
    pub revision: u64,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub ledger: Vec<GoalLedgerEntry>,
}

impl TaskGoalRecord {
    pub fn snapshot(&self, task_id: &str) -> Option<GoalSnapshot> {
        let mut goal = crate::chat::goal_snapshot_from_messages(&self.messages, None)?;
        if let Some(state) = refact_chat_api::reduce_goal_ledger(&self.ledger) {
            state.apply_to_snapshot(&mut goal);
        }
        goal.owner = Some(GoalOwner::Task { id: task_id.into() });
        goal.revision = self.revision;
        Some(goal)
    }
}

fn control_message(message: &ChatMessage) -> bool {
    matches!(message.role.as_str(), "goal" | "plan")
        || (message.role == "event"
            && message
                .extra
                .get("event")
                .and_then(|event| event.get("subkind"))
                .and_then(serde_json::Value::as_str)
                .is_some_and(|kind| matches!(kind, "goal_delta" | "plan_delta" | "goal_pursuit")))
}

async fn read_record(path: &std::path::Path) -> Result<Option<TaskGoalRecord>, String> {
    match tokio::fs::read(path).await {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| format!("Invalid task goal: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub async fn resolve_task_goal(
    gcx: Arc<GlobalContext>,
    task_id: &str,
) -> Result<Option<TaskGoalRecord>, String> {
    let dir = super::storage::find_task_dir(gcx.clone(), task_id).await?;
    let guard = super::board_lock::acquire_board(&dir).await?;
    resolve_task_goal_locked(gcx, task_id, guard.dir()).await
}

async fn resolve_task_goal_locked(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    dir: &std::path::Path,
) -> Result<Option<TaskGoalRecord>, String> {
    let path = dir.join("goal.json");
    if let Some(record) = read_record(&path).await? {
        return Ok(Some(record));
    }
    let board = super::storage::load_board(gcx.clone(), task_id).await?;
    let planner_dir = dir.join("trajectories/planner");
    let mut entries = match tokio::fs::read_dir(&planner_dir).await {
        Ok(entries) => Some(entries),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.to_string()),
    };
    let mut candidates = Vec::new();
    while let Some(entry) = match entries.as_mut() {
        Some(entries) => entries.next_entry().await.map_err(|e| e.to_string())?,
        None => None,
    } {
        if entry.file_name() == "index.json"
            || entry.path().extension().is_none_or(|ext| ext != "json")
        {
            continue;
        }
        let bytes = tokio::fs::read(entry.path())
            .await
            .map_err(|e| e.to_string())?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        let stored: Vec<ChatMessage> = serde_json::from_value(
            value
                .get("messages")
                .cloned()
                .unwrap_or(serde_json::json!([])),
        )
        .map_err(|error| {
            format!("Task goal migration conflict: invalid planner messages: {error}")
        })?;
        let mut messages = refact_core::active_context::legacy_rebuild_input(&stored)
            .map_err(|error| error.to_string())?;
        if let Some(deliveries) = value
            .get("pending_deliveries")
            .and_then(|value| value.as_array())
        {
            for delivery in deliveries {
                let pending: Vec<ChatMessage> = serde_json::from_value(
                    delivery
                        .get("messages")
                        .cloned()
                        .unwrap_or(serde_json::json!([])),
                )
                .map_err(|error| error.to_string())?;
                messages.extend(pending);
            }
        }
        let mut seen = std::collections::HashSet::new();
        messages.retain(|message| {
            message.message_id.is_empty() || seen.insert(message.message_id.clone())
        });
        if !messages.iter().any(control_message)
            && value.get("initial_plan_document").is_none()
            && value.get("authoritative_plan_document").is_none()
        {
            continue;
        }
        let id = value
            .get("chat_id")
            .and_then(|id| id.as_str())
            .unwrap_or("")
            .to_string();
        candidates.push((id, value, messages));
    }
    let owner = board.owner_chat_id.as_deref();
    if candidates
        .iter()
        .any(|(id, _, _)| Some(id.as_str()) != owner)
    {
        return Err(format!(
            "Task goal migration conflict: unlinked or alternate planner control histories: {}",
            candidates
                .iter()
                .map(|(id, _, _)| id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let (value, messages) = match candidates.pop() {
        Some((_, value, messages)) => (value, messages),
        None => (serde_json::Value::Null, Vec::new()),
    };
    let mut record = TaskGoalRecord {
        revision: 1,
        messages: messages.into_iter().filter(control_message).collect(),
        ledger: serde_json::from_value(
            value
                .get("goal_ledger")
                .cloned()
                .unwrap_or(serde_json::json!([])),
        )
        .map_err(|e| e.to_string())?,
    };
    if !record.messages.iter().any(|message| {
        message.role == "plan"
            || message
                .extra
                .get("goal")
                .and_then(|goal| goal.get("plan"))
                .is_some()
    }) {
        let mut plan_documents = Vec::new();
        if let Ok(mut entries) = tokio::fs::read_dir(dir.join("documents")).await {
            while let Some(entry) = entries
                .next_entry()
                .await
                .map_err(|error| error.to_string())?
            {
                if entry.path().extension().is_none_or(|ext| ext != "md") {
                    continue;
                }
                let text = tokio::fs::read_to_string(entry.path())
                    .await
                    .map_err(|error| error.to_string())?;
                if !text.starts_with("---") {
                    continue;
                }
                let mut sections = text.splitn(3, "---");
                sections.next();
                let front: serde_yaml::Value = serde_yaml::from_str(sections.next().unwrap_or(""))
                    .map_err(|error| error.to_string())?;
                if front.get("kind").and_then(|kind| kind.as_str()) == Some("plan") {
                    plan_documents.push((
                        entry.file_name().to_string_lossy().into_owned(),
                        sections.next().unwrap_or("").trim().to_string(),
                    ));
                }
            }
        }
        let explicit = value
            .get("authoritative_plan_document")
            .or_else(|| value.get("initial_plan_document"))
            .and_then(|slug| slug.as_str());
        let linked = explicit.and_then(|slug| {
            plan_documents
                .iter()
                .find(|(name, _)| name == &format!("{slug}.md"))
        });
        match linked {
            Some((_, content)) => record.messages.push(crate::chat::internal_roles::plan("task_planner", 1, content, None)),
            None if !plan_documents.is_empty() => return Err(format!("Task goal migration conflict: plan documents have no explicit authoritative owner link: {}", plan_documents.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>().join(", "))),
            None => {},
        }
    }
    if record.messages.is_empty() {
        return Ok(None);
    }
    if !record.messages.iter().any(|message| message.role == "goal") {
        record.messages.insert(
            0,
            crate::chat::internal_roles::goal(
                "task_planner",
                1,
                "Imported task plan",
                None,
                false,
                Default::default(),
            ),
        );
    }
    if record.ledger.is_empty() {
        if let Some(snapshot) = value.get("goal").filter(|value| !value.is_null()) {
            let snapshot: GoalSnapshot =
                serde_json::from_value(snapshot.clone()).map_err(|e| e.to_string())?;
            if let Some(base) = record
                .messages
                .iter_mut()
                .find(|message| message.role == "goal")
            {
                if let Some(meta) = base
                    .extra
                    .get_mut("goal")
                    .and_then(|value| value.as_object_mut())
                {
                    let fields = serde_json::to_value(snapshot).map_err(|e| e.to_string())?;
                    meta.extend(fields.as_object().ok_or("invalid legacy goal")?.clone());
                }
            }
        }
    }
    write_record(&path, &record).await?;
    Ok(Some(record))
}

async fn write_record(path: &std::path::Path, record: &TaskGoalRecord) -> Result<(), String> {
    crate::file_activity::write_file_durably(
        path,
        &serde_json::to_vec(record).map_err(|e| e.to_string())?,
        crate::file_activity::ReplaceMode::Replace,
    )
    .await
    .map(|_| ())
}

pub async fn commit_task_goal(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    controller_chat_id: &str,
    expected_revision: u64,
    messages: Vec<ChatMessage>,
    ledger: Vec<GoalLedgerEntry>,
) -> Result<TaskGoalRecord, String> {
    let dir = super::storage::find_task_dir(gcx.clone(), task_id).await?;
    let guard = super::board_lock::acquire_board(&dir).await?;
    let board = super::storage::load_board(gcx.clone(), task_id).await?;
    board.require_owner(controller_chat_id)?;
    let path = guard.dir().join("goal.json");
    let existing = resolve_task_goal_locked(gcx, task_id, guard.dir()).await?;
    let actual = existing.as_ref().map(|record| record.revision).unwrap_or(0);
    let record = TaskGoalRecord {
        revision: actual.checked_add(1).ok_or("task goal revision overflow")?,
        messages: messages.into_iter().filter(control_message).collect(),
        ledger,
    };
    if record.snapshot(task_id).is_none() {
        return Err("Task goal requires an installed goal".into());
    }
    if let Some(existing) = existing {
        if existing.ledger == record.ledger
            && serde_json::to_value(&existing.messages).map_err(|e| e.to_string())?
                == serde_json::to_value(&record.messages).map_err(|e| e.to_string())?
        {
            return Ok(existing);
        }
    }
    if actual != expected_revision {
        return Err(format!(
            "Task goal revision conflict: expected {expected_revision}, actual {actual}"
        ));
    }
    write_record(&path, &record).await?;
    Ok(record)
}

pub async fn hydrate_session_task_goal(
    gcx: Arc<GlobalContext>,
    session_arc: &Arc<tokio::sync::Mutex<crate::chat::types::ChatSession>>,
) -> Result<(), String> {
    let mut session = session_arc.lock().await;
    let Some(task_id) = session
        .thread
        .task_meta
        .as_ref()
        .filter(|meta| meta.role == "planner")
        .map(|meta| meta.task_id.clone())
    else {
        return Ok(());
    };
    let dir = super::storage::find_task_dir(gcx.clone(), &task_id).await?;
    let guard = super::board_lock::acquire_board(&dir).await?;
    super::storage::load_board(gcx.clone(), &task_id)
        .await?
        .require_owner(&session.chat_id)?;
    let record = resolve_task_goal_locked(gcx, &task_id, guard.dir()).await?;
    if let Some(record) = record {
        if session.task_goal_messages.is_some()
            && session.goal.as_ref().is_some_and(|goal| {
                goal.revision == record.revision
                    && goal.owner
                        == Some(GoalOwner::Task {
                            id: task_id.clone(),
                        })
            })
        {
            return Ok(());
        }
        session.set_goal_projection(record.snapshot(&task_id));
        session.set_goal_ledger(record.ledger);
        session.task_goal_messages = Some(record.messages);
        session.emit_goal_status();
    } else if let Some(goal) = session.goal.as_mut() {
        goal.owner = Some(GoalOwner::Task { id: task_id });
        goal.revision = 0;
    }
    Ok(())
}

pub async fn commit_session_task_goal(
    gcx: Arc<GlobalContext>,
    session_arc: &Arc<tokio::sync::Mutex<crate::chat::types::ChatSession>>,
) -> Result<(), String> {
    let mut session = session_arc.lock().await;
    let Some(task_id) = session
        .thread
        .task_meta
        .as_ref()
        .filter(|meta| meta.role == "planner")
        .map(|meta| meta.task_id.clone())
    else {
        return Ok(());
    };
    let projection = session.try_accepted_control_projection()?;
    if crate::chat::goal_snapshot_from_messages(&projection.messages, projection.goal.as_ref())
        .is_none()
    {
        return Ok(());
    }
    let expected_revision = session
        .goal
        .as_ref()
        .filter(|goal| {
            goal.owner
                == Some(GoalOwner::Task {
                    id: task_id.clone(),
                })
        })
        .map(|goal| goal.revision)
        .unwrap_or(0);
    let result = commit_task_goal(
        gcx.clone(),
        &task_id,
        &session.chat_id,
        expected_revision,
        projection.messages,
        projection.goal_ledger,
    )
    .await;
    match result {
        Ok(record) => {
            session.set_goal_projection(record.snapshot(&task_id));
            session.set_goal_ledger(record.ledger);
            session.task_goal_messages = Some(record.messages);
            session.emit_goal_status();
            Ok(())
        }
        Err(error) => {
            let authoritative = resolve_task_goal(gcx, &task_id).await;
            session
                .post_tool_side_effects
                .retain(|message| !control_message(message));
            for delivery in &mut session.pending_deliveries {
                delivery
                    .messages
                    .retain(|message| !control_message(message));
            }
            let error = match authoritative {
                Ok(Some(record)) => {
                    session.set_goal_projection(record.snapshot(&task_id));
                    session.set_goal_ledger(record.ledger);
                    session.task_goal_messages = Some(record.messages);
                    error
                }
                Ok(None) => {
                    session.task_goal_messages = Some(Vec::new());
                    session.set_goal_ledger(Vec::new());
                    session.set_goal_projection(None);
                    error
                }
                Err(reload_error) => {
                    tracing::warn!(%task_id, %reload_error, "Task goal rollback reload failed");
                    session.task_goal_messages = Some(Vec::new());
                    session.set_goal_ledger(Vec::new());
                    session.set_goal_projection(None);
                    format!("{error}; canonical goal reload failed: {reload_error}")
                }
            };
            if let Some(goal) = session.goal.as_mut() {
                goal.active = false;
                goal.status = refact_chat_api::GoalStatus::Paused;
                goal.stop_reason = Some(error.clone());
            }
            session.command_queue.retain(|request| {
                !matches!(
                    &request.command,
                    refact_chat_api::ChatCommand::Regenerate {}
                ) && !matches!(&request.command,
                        refact_chat_api::ChatCommand::GoalControl { action }
                        if action.trim().eq_ignore_ascii_case("resume"))
            });
            session.refresh_goal_runtime_mirror();
            session.emit_goal_status();
            Err(error)
        }
    }
}

pub async fn preflight_planner_transition(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    source_chat_id: &str,
    source_goal: Option<&GoalSnapshot>,
) -> Result<Option<TaskGoalRecord>, String> {
    let board = super::storage::load_board(gcx.clone(), task_id).await?;
    if board
        .owner_chat_id
        .as_deref()
        .is_some_and(|owner| owner != source_chat_id)
    {
        return Err("Task planner is controlled by another chat".into());
    }
    let canonical = resolve_task_goal(gcx, task_id).await?;
    let same_goal =
        source_goal.is_some_and(|goal| goal.owner == Some(GoalOwner::Task { id: task_id.into() }));
    if canonical.is_some() && !same_goal && source_goal.is_some_and(|goal| goal.active) {
        return Err("Cannot replace an existing task goal with an independent source goal".into());
    }
    if source_goal
        .is_some_and(|goal| matches!(&goal.owner, Some(GoalOwner::Task { id }) if id != task_id))
    {
        return Err("Cannot transfer another task's canonical goal".into());
    }
    Ok(canonical)
}

pub async fn move_planner_controller(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    source_chat_id: &str,
    chat_id: &str,
) -> Result<(), String> {
    let expected_owner = super::storage::load_board(gcx.clone(), task_id)
        .await?
        .owner_chat_id;
    if expected_owner
        .as_deref()
        .is_some_and(|owner| owner != source_chat_id)
    {
        return Err("Task planner is controlled by another chat".into());
    }
    super::storage::update_board_atomic(gcx, task_id, |board| {
        if board.owner_chat_id != expected_owner {
            return Err("Task planner owner changed during transfer".into());
        }
        board.owner_chat_id = Some(chat_id.into());
        Ok(())
    })
    .await?;
    Ok(())
}

pub async fn initialize_transition_goal(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    chat_id: &str,
    messages: Vec<ChatMessage>,
    ledger: Vec<GoalLedgerEntry>,
) -> Result<(), String> {
    let dir = super::storage::find_task_dir(gcx.clone(), task_id).await?;
    let guard = super::board_lock::acquire_board(&dir).await?;
    super::storage::load_board(gcx, task_id)
        .await?
        .require_owner(chat_id)?;
    let path = guard.dir().join("goal.json");
    if read_record(&path).await?.is_some() {
        return Err("Task goal appeared during planner transition".into());
    }
    let record = TaskGoalRecord {
        revision: 1,
        messages: messages.into_iter().filter(control_message).collect(),
        ledger,
    };
    if record.snapshot(task_id).is_some() {
        write_record(&path, &record).await?;
    }
    Ok(())
}

pub async fn attach_planner_goal(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    chat_id: &str,
    messages: Vec<ChatMessage>,
    ledger: Vec<GoalLedgerEntry>,
) -> Result<Option<TaskGoalRecord>, String> {
    let canonical = resolve_task_goal(gcx.clone(), task_id).await?;
    super::storage::update_board_atomic(gcx.clone(), task_id, |board| {
        board.authorize_owner(chat_id)
    })
    .await?;
    if canonical.is_some() {
        return Ok(canonical);
    }
    let messages = refact_chat_api::goal_role::import_legacy_plan(&messages);
    if crate::chat::goal_snapshot_from_messages(&messages, None).is_none() {
        return Ok(None);
    }
    commit_task_goal(gcx, task_id, chat_id, 0, messages, ledger)
        .await
        .map(Some)
}
