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
        goal.owner = Some(GoalOwner::Task {
            id: task_id.into(),
        });
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
    let (_, guard) = super::storage::locked_task_dir(gcx.clone(), task_id).await?;
    resolve_task_goal_locked(gcx, task_id, &guard).await
}

async fn resolve_task_goal_locked(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    guard: &super::board_lock::BoardLockGuard,
) -> Result<Option<TaskGoalRecord>, String> {
    let board = super::storage::load_board_locked(gcx.clone(), task_id, guard).await?;
    let dir = guard.dir();
    let path = dir.join("goal.json");
    if let Some(record) = read_record(&path).await? {
        return Ok(Some(record));
    }
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
    let current = super::storage::load_board_locked(gcx, task_id, guard).await?;
    if current.owner_chat_id != board.owner_chat_id {
        return Err("Task goal migration owner changed; retry".into());
    }
    write_record(&path, &record).await?;
    Ok(Some(record))
}

async fn write_record(path: &std::path::Path, record: &TaskGoalRecord) -> Result<(), String> {
    let bytes = serde_json::to_vec(record).map_err(|e| e.to_string())?;
    let write = crate::file_activity::write_file_durably(
        path,
        &bytes,
        crate::file_activity::ReplaceMode::Replace,
    );
    #[cfg(test)]
    if let Ok((pending, resume)) = TASK_GOAL_WRITE_PAUSE.try_with(Clone::clone) {
        use std::future::Future;
        let mut write = std::pin::pin!(write);
        let ready = std::future::poll_fn(|cx| {
            std::task::Poll::Ready(match write.as_mut().poll(cx) {
                std::task::Poll::Ready(result) => Some(result),
                std::task::Poll::Pending => None,
            })
        })
        .await;
        if let Some(result) = ready {
            return result.map(|_| ());
        }
        pending.notify_one();
        resume.notified().await;
        return write.await.map(|_| ());
    }
    write.await.map(|_| ())
}

#[cfg(test)]
tokio::task_local! {
    static TASK_GOAL_WRITE_PAUSE: (Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>);
    static TASK_GOAL_BUSY_WAIT: Arc<tokio::sync::Notify>;
}

fn recover_replayed_controls(
    session: &mut crate::chat::types::ChatSession,
    controls: &[ChatMessage],
) -> Result<bool, String> {
    use refact_core::active_context::{
        active_context, is_reconstruction_report, make_reconstruction_report,
        parse_reconstruction_report, ActiveContextError, ReconstructionMetadata,
    };

    if !matches!(
        active_context(&session.messages),
        Err(ActiveContextError::InvalidPayload(ref reason))
            if reason == "duplicate active message ID"
    ) {
        return Ok(false);
    }
    let Some(index) = session.messages.iter().rposition(is_reconstruction_report) else {
        return Ok(false);
    };
    let report = match parse_reconstruction_report(&session.messages[index]) {
        Ok(report) => report,
        Err(_) => return Ok(false),
    };
    let normalize = |message: &ChatMessage| -> Result<serde_json::Value, String> {
        let mut message = message.clone();
        message.extra.remove("delivery");
        serde_json::to_value(message).map_err(|error| error.to_string())
    };
    let canonical = controls
        .iter()
        .filter(|message| control_message(message))
        .map(|message| Ok((message.message_id.clone(), normalize(message)?)))
        .collect::<Result<std::collections::HashMap<_, _>, String>>()?;
    let mut seen = std::collections::HashMap::<String, usize>::new();
    let mut recovered: Vec<ChatMessage> = Vec::new();
    for message in report
        .payload
        .messages
        .iter()
        .chain(session.messages[index + 1..].iter())
    {
        if let Some(previous) = seen.get(&message.message_id) {
            let Some(expected) = canonical.get(&message.message_id) else {
                return Ok(false);
            };
            if !control_message(message)
                || !control_message(&recovered[*previous])
                || normalize(message)? != *expected
                || normalize(&recovered[*previous])? != *expected
            {
                return Ok(false);
            }
        } else {
            seen.insert(message.message_id.clone(), recovered.len());
            recovered.push(message.clone());
        }
    }
    let report = match make_reconstruction_report(
        recovered,
        ReconstructionMetadata {
            source_version: Some(session.trajectory_version),
            trigger: Some("canonical_control_recovery".into()),
            ..Default::default()
        },
    ) {
        Ok(report) => report,
        Err(_) => return Ok(false),
    };
    session.add_message(report);
    Ok(true)
}

pub async fn commit_task_goal(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    controller_chat_id: &str,
    expected_revision: u64,
    messages: Vec<ChatMessage>,
    ledger: Vec<GoalLedgerEntry>,
) -> Result<TaskGoalRecord, String> {
    let (_, guard) = super::storage::locked_task_dir(gcx.clone(), task_id).await?;
    commit_task_goal_locked(
        gcx,
        task_id,
        controller_chat_id,
        expected_revision,
        messages,
        ledger,
        &guard,
        None,
    )
    .await
}

async fn commit_task_goal_locked(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    controller_chat_id: &str,
    expected_revision: u64,
    messages: Vec<ChatMessage>,
    ledger: Vec<GoalLedgerEntry>,
    guard: &super::board_lock::BoardLockGuard,
    session_epoch: Option<&TaskGoalSessionEpoch>,
) -> Result<TaskGoalRecord, String> {
    let board = super::storage::load_board_locked(gcx.clone(), task_id, guard).await?;
    board.require_owner(controller_chat_id)?;
    if let Some(epoch) = session_epoch {
        epoch.validate_task_domain(&gcx, guard).await?;
    }
    let path = guard.dir().join("goal.json");
    let existing = if session_epoch.is_some_and(|epoch| epoch.pending_write().is_some()) {
        read_record(&path).await?
    } else {
        resolve_task_goal_locked(gcx.clone(), task_id, guard).await?
    };
    let actual = existing.as_ref().map(|record| record.revision).unwrap_or(0);
    let record = TaskGoalRecord {
        revision: actual.checked_add(1).ok_or("task goal revision overflow")?,
        messages: messages.into_iter().filter(control_message).collect(),
        ledger,
    };
    if record.snapshot(task_id).is_none() {
        return Err("Task goal requires an installed goal".into());
    }
    let local_revision = expected_revision;
    let expected_revision = match session_epoch.and_then(TaskGoalSessionEpoch::pending_write) {
        Some(partial) => partial.reconcile_revision(
            session_epoch.ok_or("Task goal reconciliation requires its session epoch")?,
            guard,
            existing.as_ref(),
            expected_revision,
            &record,
        )?,
        None => expected_revision,
    };
    if let Some(existing) = existing {
        if existing.ledger == record.ledger
            && serde_json::to_value(&existing.messages).map_err(|e| e.to_string())?
                == serde_json::to_value(&record.messages).map_err(|e| e.to_string())?
        {
            if let Some(epoch) = session_epoch {
                epoch.validate(&gcx).await?;
            }
            return Ok(existing);
        }
    }
    if actual != expected_revision {
        return Err(format!(
            "Task goal revision conflict: expected {expected_revision}, actual {actual}"
        ));
    }
    super::storage::load_board_locked(gcx.clone(), task_id, guard)
        .await?
        .require_owner(controller_chat_id)?;
    if let Some(epoch) = session_epoch {
        epoch.validate(&gcx).await?;
    }
    let operation = match session_epoch {
        Some(epoch) => {
            epoch.validate_task_domain(&gcx, guard).await?;
            Some(
                epoch
                    .begin_write(&gcx, guard, local_revision, &record)
                    .await?,
            )
        }
        None => None,
    };
    let result = write_record(&path, &record).await;
    if let Some(operation) = operation {
        let _ = operation.outcome.set(result.clone());
    }
    result?;
    Ok(record)
}

const SESSION_BUSY: &str = "Task goal session is busy; retry";

async fn wait_for_session(session_arc: &Arc<tokio::sync::Mutex<crate::chat::types::ChatSession>>) {
    #[cfg(test)]
    if let Ok(notify) = TASK_GOAL_BUSY_WAIT.try_with(Clone::clone) {
        notify.notify_one();
    }
    drop(session_arc.lock().await);
}

pub async fn hydrate_session_task_goal(
    gcx: Arc<GlobalContext>,
    session_arc: &Arc<tokio::sync::Mutex<crate::chat::types::ChatSession>>,
) -> Result<(), String> {
    hydrate_session_task_goal_for_pursuit(gcx, session_arc)
        .await
        .map(|_| ())
}

pub async fn hydrate_session_task_goal_for_pursuit(
    gcx: Arc<GlobalContext>,
    session_arc: &Arc<tokio::sync::Mutex<crate::chat::types::ChatSession>>,
) -> Result<bool, String> {
    hydrate_session_task_goal_authorized(gcx, session_arc, false).await
}

pub async fn hydrate_session_task_goal_for_mutation(
    gcx: Arc<GlobalContext>,
    session_arc: &Arc<tokio::sync::Mutex<crate::chat::types::ChatSession>>,
) -> Result<(), String> {
    hydrate_session_task_goal_authorized(gcx, session_arc, true)
        .await
        .map(|_| ())
}

enum Hydration {
    Ready,
    ReadOnly(String),
    Persist,
}

async fn hydrate_session_task_goal_authorized(
    gcx: Arc<GlobalContext>,
    session_arc: &Arc<tokio::sync::Mutex<crate::chat::types::ChatSession>>,
    strict: bool,
) -> Result<bool, String> {
    let mut repairs = 0;
    let mut retries = 0u32;
    'capture: loop {
        let captured = TaskGoalSessionEpoch::capture(&gcx, session_arc).await;
        let captured = match captured {
            Err(error) if error.ends_with("; retry") && retries < 3 => {
                retries += 1;
                tokio::time::sleep(std::time::Duration::from_millis(10 * (1 << retries))).await;
                continue;
            }
            result => result?,
        };
        let Some(epoch) = captured else {
            return Ok(true);
        };
        let result = loop {
            match hydrate_session_task_goal_once(gcx.clone(), &epoch).await {
                Err(error) if error == SESSION_BUSY => wait_for_session(session_arc).await,
                Err(error) if error.ends_with("; retry") && retries < 3 => {
                    retries += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(10 * (1 << retries))).await;
                    continue 'capture;
                }
                result => break result?,
            }
        };
        match result {
            Hydration::Ready => return Ok(true),
            Hydration::ReadOnly(error) => return if strict { Err(error) } else { Ok(false) },
            Hydration::Persist => {
                if repairs == 2 {
                    return Err("Task goal hold changed repeatedly during hydration; hold retained. Inspect the current task goal and reconcile the pending hold before resuming.".into());
                }
                if let Err(error) = commit_session_task_goal(gcx.clone(), session_arc).await {
                    let board =
                        super::storage::load_board(gcx.clone(), &epoch.task_meta.task_id).await?;
                    if let Some(owner) = board
                        .owner_chat_id
                        .as_deref()
                        .filter(|owner| *owner != epoch.chat_id)
                    {
                        return if strict {
                            Err(planner_owner_error(owner))
                        } else {
                            Ok(false)
                        };
                    }
                    return Err(format!("Task goal hold/pending write could not be persisted: {error}. Local state retained; retry, or create a new task planner chat to adopt canonical state."));
                }
                repairs += 1;
            }
        }
    }
}

fn planner_owner_error(owner: &str) -> String {
    format!("This chat no longer owns the task board; the current planner chat is {owner}. Continue there or transfer ownership.")
}

async fn hydrate_session_task_goal_once(
    gcx: Arc<GlobalContext>,
    epoch: &TaskGoalSessionEpoch,
) -> Result<Hydration, String> {
    let task_id = &epoch.task_meta.task_id;
    let (_, guard) = super::storage::locked_task_dir(gcx.clone(), task_id).await?;
    let board = super::storage::load_board_locked(gcx.clone(), task_id, &guard).await?;
    if board
        .owner_chat_id
        .as_deref()
        .is_some_and(|owner| owner != epoch.chat_id)
    {
        return Ok(Hydration::ReadOnly(planner_owner_error(
            board.owner_chat_id.as_deref().unwrap(),
        )));
    }
    board.require_owner(&epoch.chat_id)?;
    epoch.validate_task_domain(&gcx, &guard).await?;
    if epoch.partial_write.is_some() {
        return Ok(Hydration::Persist);
    }
    let record = resolve_task_goal_locked(gcx.clone(), task_id, &guard).await?;
    super::storage::load_board_locked(gcx.clone(), task_id, &guard)
        .await?
        .require_owner(&epoch.chat_id)?;
    epoch.validate_task_domain(&gcx, &guard).await?;
    let sessions = gcx.chat_sessions.read().await;
    epoch.validate_registry(&sessions)?;
    let mut session = epoch.lock_current()?;
    if let Some(canonical) = record.as_ref() {
        if let Some(ledger) = recover_contention_hold(&session, canonical, task_id)? {
            let mut held = canonical
                .snapshot(task_id)
                .ok_or("Task goal recovery lost its goal")?;
            held.status = refact_chat_api::GoalStatus::Paused;
            session.set_goal_projection(Some(held));
            session.set_goal_ledger(ledger);
            session.mark_persisted_runtime_changed();
            session.emit_goal_status();
            return Ok(Hydration::Persist);
        }
    }
    if let Some(record) = record {
        let snapshot = record.snapshot(task_id);
        if session.goal.as_ref().is_some_and(|goal| {
            !matches!(
                goal.status,
                refact_chat_api::GoalStatus::Active | refact_chat_api::GoalStatus::Verifying
            ) && snapshot.as_ref().is_some_and(|canonical| {
                canonical.status != goal.status
                    || (!goal.active && canonical.active)
                    || (goal.status == refact_chat_api::GoalStatus::Stopped
                        && canonical.stop_reason != goal.stop_reason)
                    || canonical.snoozed_until_ms != goal.snoozed_until_ms
            })
        }) {
            let held = session.goal.as_ref().unwrap().clone();
            let projection = session.try_accepted_control_projection()?;
            let mut projected =
                crate::chat::goal_snapshot_from_messages(&projection.messages, Some(&held))
                    .ok_or("Task goal hold lost its installed goal")?;
            if let Some(state) = refact_chat_api::reduce_goal_ledger(&projection.goal_ledger) {
                state.apply_to_snapshot(&mut projected);
            }
            let baseline = session
                .task_goal_messages
                .as_ref()
                .unwrap_or(&projection.messages);
            let mut local_content = held.clone();
            local_content.owner = projected.owner.clone();
            local_content.revision = projected.revision;
            local_content.active = projected.active;
            local_content.status = projected.status;
            local_content.stop_reason = projected.stop_reason.clone();
            local_content.snoozed_until_ms = projected.snoozed_until_ms;
            local_content.events = projected.events.clone();
            let local_content_matches = serde_json::to_value(&local_content)
                .map_err(|error| error.to_string())?
                == serde_json::to_value(&projected).map_err(|error| error.to_string())?;
            // No rebase is needed when these accepted controls still own the
            // canonical revision. Persist them with the hold instead of losing
            // an accepted content/ledger update while adopting canonical state.
            if held.revision == record.revision
                && held.owner
                    == Some(GoalOwner::Task {
                        id: task_id.clone(),
                    })
                && local_content_matches
            {
                return Ok(Hydration::Persist);
            }
            // task_goal_messages includes newly accepted local controls, not
            // just the last persisted baseline. An advanced canonical revision
            // may extend that baseline, but must not replace a divergent local
            // outcome or plan. Pursuit events and delivery stamps are not content.
            let content_controls =
                |messages: &[ChatMessage]| -> Result<Vec<serde_json::Value>, String> {
                    messages
                        .iter()
                        .filter(|message| {
                            control_message(message)
                                && !(message.role == "event"
                                    && message.extra["event"]["subkind"].as_str()
                                        == Some("goal_pursuit"))
                        })
                        .map(|message| {
                            let mut message = message.clone();
                            message.extra.remove("delivery");
                            serde_json::to_value(message).map_err(|error| error.to_string())
                        })
                        .collect()
                };
            let baseline_content = content_controls(baseline)?;
            let canonical_content = content_controls(&record.messages)?;
            let common = projection
                .goal_ledger
                .iter()
                .zip(&record.ledger)
                .take_while(|(local, canonical)| local == canonical)
                .count();
            let hold_only = projection.goal_ledger[common..].iter().all(|entry| {
                matches!(
                    &entry.op,
                    refact_chat_api::GoalLedgerOp::StatusChanged {
                        to: refact_chat_api::GoalStatus::Paused
                            | refact_chat_api::GoalStatus::Stopped
                            | refact_chat_api::GoalStatus::BudgetExhausted
                            | refact_chat_api::GoalStatus::NoProgress,
                        ..
                    } | refact_chat_api::GoalLedgerOp::SnoozeSet { .. }
                )
            });
            let messages_hold_only = projection.messages.len() >= baseline.len()
                && serde_json::to_value(baseline).map_err(|error| error.to_string())?
                    == serde_json::to_value(&projection.messages[..baseline.len()])
                        .map_err(|error| error.to_string())?
                && projection.messages[baseline.len()..].iter().all(|message| {
                    let event = message
                        .extra
                        .get("event")
                        .unwrap_or(&serde_json::Value::Null);
                    informational_pursuit_message(message)
                        || (message.role == "event"
                            && event["subkind"].as_str() == Some("goal_pursuit")
                            && matches!(
                                (event["source"].as_str(), event["payload"]["kind"].as_str()),
                                (Some("tool.goal_pursuit"), Some("paused" | "snoozed"))
                                    | (
                                        Some("chat.command.goal_control" | "chat.session"),
                                        Some("stopped")
                                    )
                            ))
                });
            if !messages_hold_only
                || !hold_only
                || !canonical_content.starts_with(&baseline_content)
                || !local_content_matches
            {
                return Err("Task goal hold conflicts with local goal content. Local hold retained; create a new task planner chat to transfer ownership and adopt canonical state.".into());
            }
            projected = snapshot
                .clone()
                .ok_or("Task goal hold lost canonical goal")?;
            session.set_goal_projection(Some(projected.clone()));
            session.set_goal_ledger(record.ledger);
            session.task_goal_messages = Some(record.messages);
            if projected.status != held.status || projected.stop_reason != held.stop_reason {
                session.goal_set_status_reason(
                    held.status,
                    held.stop_reason.as_deref().unwrap_or("session hold"),
                );
            }
            if projected.snoozed_until_ms != held.snoozed_until_ms {
                session.goal_set_snooze(held.snoozed_until_ms);
            }
            return Ok(Hydration::Persist);
        }
        recover_replayed_controls(&mut session, &record.messages)?;
        if session.task_goal_messages.is_some()
            && session.goal.as_ref().is_some_and(|goal| {
                goal.revision == record.revision
                    && goal.owner
                        == Some(GoalOwner::Task {
                            id: task_id.clone(),
                        })
            })
        {
            return Ok(Hydration::Ready);
        }
        session.set_goal_projection(snapshot);
        session.set_goal_ledger(record.ledger);
        session.task_goal_messages = Some(record.messages);
        session.task_goal_partial_write = None;
        session.emit_goal_status();
    } else if let Some(goal) = session.goal.as_mut() {
        goal.owner = Some(GoalOwner::Task {
            id: task_id.clone(),
        });
        goal.revision = 0;
    }
    Ok(Hydration::Ready)
}

fn recover_contention_hold(
    session: &crate::chat::types::ChatSession,
    record: &TaskGoalRecord,
    task_id: &str,
) -> Result<Option<Vec<GoalLedgerEntry>>, String> {
    let Some(goal) = session.goal.as_ref() else {
        return Ok(None);
    };
    let Some(canonical) = record.snapshot(task_id) else {
        return Ok(None);
    };
    if goal.status != refact_chat_api::GoalStatus::Paused
        || !canonical.active
        || !matches!(
            canonical.status,
            refact_chat_api::GoalStatus::Active | refact_chat_api::GoalStatus::Verifying
        )
    {
        return Ok(None);
    }
    let mut held = goal.clone();
    held.active = canonical.active;
    held.status = canonical.status;
    held.stop_reason = canonical.stop_reason.clone();
    if serde_json::to_value(held).map_err(|error| error.to_string())?
        != serde_json::to_value(&canonical).map_err(|error| error.to_string())?
    {
        return Ok(None);
    }
    let projection = session.try_accepted_control_projection()?;
    if projection.goal_ledger != record.ledger
        || serde_json::to_value(&projection.messages).map_err(|error| error.to_string())?
            != serde_json::to_value(&record.messages).map_err(|error| error.to_string())?
    {
        return Ok(None);
    }
    let mut ledger = record.ledger.clone();
    let seq = ledger
        .last()
        .map(|entry| entry.seq)
        .unwrap_or(0)
        .checked_add(1)
        .ok_or("Task goal ledger sequence overflow")?;
    ledger.push(GoalLedgerEntry {
        seq,
        at_ms: chrono::Utc::now().timestamp_millis().max(0) as u64,
        op: refact_chat_api::GoalLedgerOp::StatusChanged {
            from: canonical.status,
            to: refact_chat_api::GoalStatus::Paused,
            reason: goal
                .stop_reason
                .clone()
                .unwrap_or_else(|| "session hold".into()),
        },
    });
    Ok(Some(ledger))
}

fn informational_pursuit_message(message: &ChatMessage) -> bool {
    let event = message
        .extra
        .get("event")
        .unwrap_or(&serde_json::Value::Null);
    message.role == "event"
        && event["subkind"].as_str() == Some("goal_pursuit")
        && event["source"].as_str() == Some("chat.goal_monitor")
        && matches!(
            event["payload"]["kind"].as_str(),
            Some("pursuit_quiescent" | "budget_exhausted" | "no_progress")
        )
}

pub(crate) struct TaskGoalPartialWrite {
    session: std::sync::Weak<tokio::sync::Mutex<crate::chat::types::ChatSession>>,
    incarnation: std::sync::Weak<std::sync::atomic::AtomicBool>,
    identity: AuthorityIdentity,
    registered: bool,
    previous_revision: u64,
    previous_goal: Option<GoalSnapshot>,
    verification_generation: u64,
    record: TaskGoalRecord,
    task_dir: std::path::PathBuf,
    task_identity: std::fs::Metadata,
    source: Option<(std::path::PathBuf, std::fs::Metadata)>,
    outcome: std::sync::OnceLock<Result<(), String>>,
}

impl TaskGoalPartialWrite {
    fn reconcile_revision(
        &self,
        epoch: &TaskGoalSessionEpoch,
        guard: &super::board_lock::BoardLockGuard,
        existing: Option<&TaskGoalRecord>,
        expected_revision: u64,
        proposed: &TaskGoalRecord,
    ) -> Result<u64, String> {
        if !matches!(self.outcome.get(), Some(Ok(()))) {
            return Err(format!(
                "Task goal partial write is not acknowledged; outcome unknown or failed: {:?}",
                self.outcome.get()
            ));
        }
        if !self.registered
            || self.source.is_none()
            || self.identity != epoch.identity
            || self.registered != epoch.registered
            || !self.session.ptr_eq(&Arc::downgrade(&epoch.session))
            || !self.incarnation.ptr_eq(&epoch.incarnation)
            || self.previous_revision != expected_revision
            || epoch.verification_generation < self.verification_generation
            || guard.dir() != self.task_dir
            || dunce::canonicalize(&self.task_dir).ok().as_ref() != Some(&self.task_dir)
            || !super::storage::same_directory_identity(&self.task_identity, &self.task_dir)?
        {
            return Err("Task goal partial write identity superseded; inspect before retry".into());
        }
        if let Some((path, identity)) = &self.source {
            if epoch
                .source_root
                .as_ref()
                .and_then(|source| dunce::canonicalize(source).ok())
                .as_ref()
                != Some(path)
                || dunce::canonicalize(path).ok().as_ref() != Some(path)
                || !super::storage::same_directory_identity(identity, path)?
            {
                return Err(
                    "Task goal partial write source superseded; inspect before retry".into(),
                );
            }
        }
        if serde_json::to_value(existing).map_err(|error| error.to_string())?
            != serde_json::to_value(Some(&self.record)).map_err(|error| error.to_string())?
        {
            return Err("Task goal partial write canonical revision/content superseded; inspect before retry".into());
        }
        if proposed.ledger == self.record.ledger
            && serde_json::to_value(&proposed.messages).map_err(|error| error.to_string())?
                == serde_json::to_value(&self.record.messages).map_err(|error| error.to_string())?
            && epoch
                .operation
                .get()
                .is_some_and(|operation| std::ptr::eq(operation.as_ref(), self))
            && epoch.verification_generation == self.verification_generation
            && serde_json::to_value(&epoch.goal).map_err(|error| error.to_string())?
                == serde_json::to_value(&self.previous_goal).map_err(|error| error.to_string())?
        {
            return Ok(self.record.revision);
        }
        let informational_suffix = proposed.messages.len() >= self.record.messages.len()
            && serde_json::to_value(&proposed.messages[..self.record.messages.len()])
                .map_err(|error| error.to_string())?
                == serde_json::to_value(&self.record.messages)
                    .map_err(|error| error.to_string())?
            && proposed.messages[self.record.messages.len()..]
                .iter()
                .all(informational_pursuit_message);
        if proposed.ledger == self.record.ledger
            && informational_suffix
            && epoch.goal.as_ref().is_some_and(|goal| {
                matches!(
                    goal.status,
                    refact_chat_api::GoalStatus::Active | refact_chat_api::GoalStatus::Verifying
                )
            })
        {
            if epoch.verification_generation != self.verification_generation {
                return Err("Task goal partial write verification generation superseded; inspect before retry".into());
            }
            let prior = self
                .previous_goal
                .as_ref()
                .ok_or("Task goal partial write prior goal unavailable")?;
            let accepted =
                crate::chat::goal_snapshot_from_messages(&proposed.messages, Some(prior))
                    .ok_or("Task goal partial write accepted goal unavailable")?;
            let current =
                crate::chat::goal_snapshot_from_messages(&proposed.messages, epoch.goal.as_ref());
            // Rebuilding prefers the base message's budget, so it cannot
            // detect a changed live budget by comparing rebuilt snapshots alone.
            if serde_json::to_value(epoch.goal.as_ref().map(|goal| &goal.budget))
                .map_err(|error| error.to_string())?
                != serde_json::to_value(Some(&prior.budget)).map_err(|error| error.to_string())?
                || serde_json::to_value(Some(accepted)).map_err(|error| error.to_string())?
                    != serde_json::to_value(current).map_err(|error| error.to_string())?
            {
                return Err(
                    "Task goal partial write goal/budget superseded; inspect before retry".into(),
                );
            }
            return Ok(self.record.revision);
        }
        if proposed.messages.len() < self.record.messages.len()
            || serde_json::to_value(&proposed.messages[..self.record.messages.len()])
                .map_err(|error| error.to_string())?
                != serde_json::to_value(&self.record.messages).map_err(|error| error.to_string())?
            || proposed.messages[self.record.messages.len()..]
                .iter()
                .any(|message| {
                    let event = message
                        .extra
                        .get("event")
                        .unwrap_or(&serde_json::Value::Null);
                    !informational_pursuit_message(message)
                        && (message.role != "event"
                            || event["subkind"].as_str() != Some("goal_pursuit")
                            || !matches!(
                                (
                                    event["source"].as_str(),
                                    event["payload"]["kind"].as_str(),
                                    event["payload"]["trigger"].as_str()
                                ),
                                (
                                    Some("chat.command.goal_control"),
                                    Some("stopped"),
                                    Some("goal_control")
                                ) | (Some("chat.session"), Some("stopped"), Some("manual_abort"))
                                    | (
                                        Some("tool.goal_pursuit"),
                                        Some("paused" | "snoozed"),
                                        Some("pause")
                                    )
                            ))
                })
            || !proposed.ledger.starts_with(&self.record.ledger)
        {
            return Err(
                "Task goal partial write control content/ledger superseded; inspect before retry"
                    .into(),
            );
        }
        let mut seq = self
            .record
            .ledger
            .last()
            .map(|entry| entry.seq)
            .unwrap_or(0);
        let mut held = self
            .record
            .snapshot(&epoch.task_meta.task_id)
            .ok_or("Task goal partial write lost its goal")?;
        for entry in &proposed.ledger[self.record.ledger.len()..] {
            if Some(entry.seq) != seq.checked_add(1) {
                return Err("Task goal partial write ledger sequence superseded".into());
            }
            match &entry.op {
                refact_chat_api::GoalLedgerOp::StatusChanged {
                    from,
                    to,
                    reason,
                } if *from == held.status
                    && matches!(
                        to,
                        refact_chat_api::GoalStatus::Paused
                            | refact_chat_api::GoalStatus::Stopped
                            | refact_chat_api::GoalStatus::BudgetExhausted
                            | refact_chat_api::GoalStatus::NoProgress
                    ) =>
                {
                    held.status = *to;
                    if *to == refact_chat_api::GoalStatus::Stopped {
                        held.stop_reason = (!reason.is_empty()).then(|| reason.clone());
                    }
                }
                refact_chat_api::GoalLedgerOp::SnoozeSet {
                    until_ms: Some(until),
                } => {
                    held.snoozed_until_ms = Some(*until);
                }
                _ => {
                    return Err(
                        "Task goal partial write has non-hold changes; inspect before retry".into(),
                    )
                }
            }
            seq = entry.seq;
        }
        let goal = epoch
            .goal
            .as_ref()
            .ok_or("Task goal partial write goal superseded")?;
        if !matches!(
            goal.status,
            refact_chat_api::GoalStatus::Paused
                | refact_chat_api::GoalStatus::Stopped
                | refact_chat_api::GoalStatus::BudgetExhausted
                | refact_chat_api::GoalStatus::NoProgress
        ) || goal.status != held.status
            || goal.stop_reason != held.stop_reason
            || goal.snoozed_until_ms != held.snoozed_until_ms
        {
            return Err(
                "Task goal partial write current hold superseded; inspect before retry".into(),
            );
        }
        let mut prior = self
            .previous_goal
            .clone()
            .ok_or("Task goal partial write prior goal unavailable")?;
        if serde_json::to_value(&prior.budget).map_err(|error| error.to_string())?
            != serde_json::to_value(&goal.budget).map_err(|error| error.to_string())?
        {
            return Err(
                "Task goal partial write goal/budget superseded; inspect before retry".into(),
            );
        }
        prior.status = goal.status;
        prior.stop_reason = goal.stop_reason.clone();
        prior.snoozed_until_ms = goal.snoozed_until_ms;
        let prior = crate::chat::goal_snapshot_from_messages(&proposed.messages, Some(&prior))
            .ok_or("Task goal partial write accepted goal unavailable")?;
        let current = crate::chat::goal_snapshot_from_messages(&proposed.messages, Some(goal))
            .ok_or("Task goal partial write current goal unavailable")?;
        if serde_json::to_value(prior).map_err(|error| error.to_string())?
            != serde_json::to_value(current).map_err(|error| error.to_string())?
        {
            return Err(
                "Task goal partial write goal/budget superseded; inspect before retry".into(),
            );
        }
        Ok(self.record.revision)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct AuthorityIdentity {
    chat_id: String,
    task_meta: Option<refact_chat_api::TaskMeta>,
    source_root: Option<std::path::PathBuf>,
    closed: bool,
    closed_flag: bool,
    restore_pending: bool,
}

#[derive(Debug, PartialEq, Serialize)]
struct GoalControlFence {
    goal: serde_json::Value,
    ledger: Vec<GoalLedgerEntry>,
    verification_generation: u64,
    pause_revision: u64,
    pause: serde_json::Value,
    controls: Vec<serde_json::Value>,
}

struct TaskGoalSessionEpoch {
    session: Arc<tokio::sync::Mutex<crate::chat::types::ChatSession>>,
    incarnation: std::sync::Weak<std::sync::atomic::AtomicBool>,
    registered: bool,
    chat_id: String,
    task_meta: refact_chat_api::TaskMeta,
    identity: AuthorityIdentity,
    goal: Option<GoalSnapshot>,
    source_root: Option<std::path::PathBuf>,
    verification_generation: u64,
    partial_write: Option<Arc<TaskGoalPartialWrite>>,
    operation: std::sync::OnceLock<Arc<TaskGoalPartialWrite>>,
    state: GoalControlFence,
}

impl TaskGoalSessionEpoch {
    fn pending_write(&self) -> Option<&Arc<TaskGoalPartialWrite>> {
        self.operation.get().or(self.partial_write.as_ref())
    }

    fn identity(session: &crate::chat::types::ChatSession) -> Result<AuthorityIdentity, String> {
        Ok(AuthorityIdentity {
            chat_id: session.chat_id.clone(),
            task_meta: session.thread.task_meta.clone(),
            source_root: crate::coordination::scope::session_source_root(session),
            closed: session.closed,
            closed_flag: session
                .closed_flag
                .load(std::sync::atomic::Ordering::Relaxed),
            restore_pending: session.board_restored_pending_tools,
        })
    }

    fn state(session: &crate::chat::types::ChatSession) -> Result<GoalControlFence, String> {
        let projection = match session.try_accepted_control_projection() {
            Ok(projection) => projection,
            Err(error) => {
                let mut recovered = crate::chat::types::ChatSession::new(session.chat_id.clone());
                recovered.messages = session.messages.clone();
                let controls = session
                    .task_goal_messages
                    .as_deref()
                    .unwrap_or(&session.messages);
                if !recover_replayed_controls(&mut recovered, controls)? {
                    return Err(error);
                }
                recovered.task_goal_messages = session.task_goal_messages.clone();
                recovered.post_tool_side_effects = session.post_tool_side_effects.clone();
                recovered.pending_deliveries = session.pending_deliveries.clone();
                recovered.try_accepted_control_projection()?
            }
        };
        let controls = projection
            .messages
            .iter()
            .filter(|m| control_message(m))
            .map(|message| {
                let mut message = message.clone();
                message.extra.remove("delivery");
                serde_json::to_value(message).map_err(|error| error.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(GoalControlFence {
            goal: serde_json::to_value(&session.goal).map_err(|error| error.to_string())?,
            ledger: session.goal_ledger.clone(),
            verification_generation: session.goal_verification_generation,
            pause_revision: session.planner_pause_revision,
            pause: serde_json::to_value(
                session
                    .planner_pause
                    .as_ref()
                    .map(|pause| (&pause.since, &pause.by)),
            )
            .map_err(|error| error.to_string())?,
            controls,
        })
    }

    async fn capture(
        gcx: &Arc<GlobalContext>,
        session_arc: &Arc<tokio::sync::Mutex<crate::chat::types::ChatSession>>,
    ) -> Result<Option<Self>, String> {
        let (
            chat_id,
            task_meta,
            identity,
            goal,
            source_root,
            verification_generation,
            incarnation,
            partial_write,
            state,
        ) = {
            let session = session_arc.lock().await;
            let Some(meta) = session
                .thread
                .task_meta
                .as_ref()
                .filter(|meta| meta.role == "planner")
            else {
                if session.task_goal_partial_write.is_some() {
                    return Err("Task goal partial write task identity superseded".into());
                }
                return Ok(None);
            };
            (
                session.chat_id.clone(),
                meta.clone(),
                Self::identity(&session)?,
                session.goal.clone(),
                crate::coordination::scope::session_source_root(&session),
                session.goal_verification_generation,
                Arc::downgrade(&session.closed_flag),
                session.task_goal_partial_write.clone(),
                Self::state(&session)?,
            )
        };
        if let Some(root) = &source_root {
            let source = crate::coordination::roots::resolve_source_workspace_root(
                gcx.clone(),
                Some(root.to_str().ok_or("Task goal source is not UTF-8")?),
            )
            .await?;
            if dunce::canonicalize(root).ok().as_ref() != Some(&source) {
                return Err("Task goal source identity changed; retry".into());
            }
        }
        let sessions = gcx.chat_sessions.read().await;
        let registered = sessions.get(&chat_id);
        if registered.is_some_and(|current| !Arc::ptr_eq(current, session_arc)) {
            return Err("Task goal session identity changed; retry".into());
        }
        Ok(Some(Self {
            session: session_arc.clone(),
            incarnation,
            registered: registered.is_some(),
            chat_id,
            task_meta,
            identity,
            goal,
            source_root,
            verification_generation,
            partial_write,
            operation: std::sync::OnceLock::new(),
            state,
        }))
    }

    fn validate_registry(
        &self,
        sessions: &std::collections::HashMap<
            String,
            Arc<tokio::sync::Mutex<crate::chat::types::ChatSession>>,
        >,
    ) -> Result<(), String> {
        match (self.registered, sessions.get(&self.chat_id)) {
            (true, Some(current)) if Arc::ptr_eq(&self.session, current) => Ok(()),
            (false, None) => Ok(()),
            _ => Err("Task goal session identity changed; retry".into()),
        }
    }

    fn lock_current(
        &self,
    ) -> Result<tokio::sync::MutexGuard<'_, crate::chat::types::ChatSession>, String> {
        let session = self.session.try_lock().map_err(|_| SESSION_BUSY)?;
        if session.chat_id != self.chat_id
            || session.thread.task_meta.as_ref() != Some(&self.task_meta)
            || Self::identity(&session)? != self.identity
            || !self
                .incarnation
                .ptr_eq(&Arc::downgrade(&session.closed_flag))
            || match (
                self.operation.get().or(self.partial_write.as_ref()),
                session.task_goal_partial_write.as_ref(),
            ) {
                (Some(expected), Some(current)) => !Arc::ptr_eq(expected, current),
                (None, None) => false,
                _ => true,
            }
            || Self::state(&session)? != self.state
        {
            return Err("Task goal session changed while awaiting persistence; retry".into());
        }
        Ok(session)
    }

    async fn validate(&self, gcx: &GlobalContext) -> Result<(), String> {
        let sessions = gcx.chat_sessions.read().await;
        self.validate_registry(&sessions)?;
        self.lock_current().map(|_| ())
    }

    async fn validate_task_domain(
        &self,
        gcx: &Arc<GlobalContext>,
        guard: &super::board_lock::BoardLockGuard,
    ) -> Result<(), String> {
        let root = self
            .source_root
            .as_ref()
            .ok_or("Task goal operation requires its authenticated source")?;
        let source = crate::coordination::roots::resolve_source_workspace_root(
            gcx.clone(),
            Some(root.to_str().ok_or("Task goal source is not UTF-8")?),
        )
        .await?;
        let task_id = &self.task_meta.task_id;
        let local = source.join(".refact/tasks").join(task_id);
        let global = gcx.config_dir.join("tasks").join(task_id);
        let accepted = [local, global]
            .into_iter()
            .find(|path| dunce::canonicalize(path).ok().as_deref() == Some(guard.dir()))
            .ok_or("Task goal lock does not belong to its authenticated source/task domain")?;
        for path in accepted.ancestors().take(3) {
            let metadata = tokio::fs::symlink_metadata(path)
                .await
                .map_err(|error| error.to_string())?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err("Task goal domain has an unsafe directory alias".into());
            }
        }
        let identity = std::fs::metadata(guard.dir()).map_err(|error| error.to_string())?;
        let path = guard.dir().join("meta.yaml");
        super::storage::require_regular_task_file(&path).await?;
        let body = crate::coordination::board_bootstrap::read_bounded(
            &path,
            guard.dir(),
            crate::file_activity::settings().max_work_history_bytes,
        )
        .await
        .map_err(|error| format!("Task goal metadata unavailable: {error:?}"))?;
        let meta: super::types::TaskMeta =
            serde_yaml::from_str(&body).map_err(|error| error.to_string())?;
        if meta.id != *task_id
            || dunce::canonicalize(&accepted).ok().as_deref() != Some(guard.dir())
            || !super::storage::same_directory_identity(&identity, guard.dir())?
        {
            return Err("Task goal metadata/directory identity changed; retry".into());
        }
        self.validate(gcx).await
    }

    async fn begin_write(
        &self,
        gcx: &GlobalContext,
        guard: &super::board_lock::BoardLockGuard,
        previous_revision: u64,
        record: &TaskGoalRecord,
    ) -> Result<Arc<TaskGoalPartialWrite>, String> {
        let source = self
            .source_root
            .as_ref()
            .map(|path| {
                let canonical = dunce::canonicalize(path).map_err(|error| error.to_string())?;
                let identity = std::fs::metadata(&canonical).map_err(|error| error.to_string())?;
                Ok::<_, String>((canonical, identity))
            })
            .transpose()?;
        let operation = Arc::new(TaskGoalPartialWrite {
            session: Arc::downgrade(&self.session),
            incarnation: self.incarnation.clone(),
            identity: self.identity.clone(),
            registered: self.registered,
            previous_revision,
            previous_goal: self.goal.clone(),
            verification_generation: self.verification_generation,
            record: record.clone(),
            task_dir: guard.dir().into(),
            task_identity: std::fs::metadata(guard.dir()).map_err(|error| error.to_string())?,
            source,
            outcome: std::sync::OnceLock::new(),
        });
        let sessions = gcx.chat_sessions.read().await;
        self.validate_registry(&sessions)?;
        let mut session = self.lock_current()?;
        self.operation
            .set(operation.clone())
            .map_err(|_| "Task goal write already started")?;
        session.task_goal_partial_write = Some(operation.clone());
        Ok(operation)
    }
}

pub async fn commit_session_task_goal(
    gcx: Arc<GlobalContext>,
    session_arc: &Arc<tokio::sync::Mutex<crate::chat::types::ChatSession>>,
) -> Result<(), String> {
    let Some(epoch) = TaskGoalSessionEpoch::capture(&gcx, session_arc).await? else {
        return Ok(());
    };
    loop {
        match commit_session_task_goal_once(gcx.clone(), &epoch).await {
            Err(error) if error == SESSION_BUSY => wait_for_session(session_arc).await,
            result => return result,
        }
    }
}

async fn commit_session_task_goal_once(
    gcx: Arc<GlobalContext>,
    epoch: &TaskGoalSessionEpoch,
) -> Result<(), String> {
    let task_id = &epoch.task_meta.task_id;
    let (projection, expected_revision) = {
        let session = epoch.lock_current()?;
        let projection = session.try_accepted_control_projection()?;
        if crate::chat::goal_snapshot_from_messages(&projection.messages, projection.goal.as_ref())
            .is_none()
        {
            if epoch.partial_write.is_some() {
                return Err("Task goal partial write installed goal superseded".into());
            }
            return Ok(());
        }
        let revision = session
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
        (projection, revision)
    };
    let locked = super::storage::locked_task_dir(gcx.clone(), task_id).await;
    let result = match &locked {
        Ok((_, guard)) => {
            commit_task_goal_locked(
                gcx.clone(),
                task_id,
                &epoch.chat_id,
                expected_revision,
                projection.messages,
                projection.goal_ledger,
                guard,
                Some(epoch),
            )
            .await
        }
        Err(error) => Err(error.clone()),
    };
    if result.as_ref().is_err_and(|error| error == SESSION_BUSY) {
        return Err(SESSION_BUSY.into());
    }
    let authoritative =
        if result.is_err() && epoch.partial_write.is_none() && epoch.operation.get().is_none() {
            match &locked {
                Ok((_, guard)) => read_record(&guard.dir().join("goal.json")).await,
                Err(error) => Err(error.clone()),
            }
        } else {
            Ok(None)
        };
    if let Ok((_, guard)) = &locked {
        if let Err(error) = super::storage::load_board_locked(gcx.clone(), task_id, guard)
            .await
            .and_then(|board| board.require_owner(&epoch.chat_id))
        {
            return Err(format!(
                "Task goal session projection not applied: {error}; persistence result: {}",
                match &result {
                    Ok(record) => format!("revision {} persisted", record.revision),
                    Err(error) => error.clone(),
                }
            ));
        }
    }
    if let Ok((_, guard)) = &locked {
        epoch
            .validate_task_domain(&gcx, guard)
            .await
            .map_err(|error| match (&result, error.as_str()) {
                (_, SESSION_BUSY) => error,
                (Ok(record), _) => format!(
                    "Task goal revision {} persisted; session projection not applied: {error}",
                    record.revision
                ),
                (Err(commit_error), _) => {
                    format!("{commit_error}; session rollback not applied: {error}")
                }
            })?;
    }
    let sessions = gcx.chat_sessions.read().await;
    let application = epoch
        .validate_registry(&sessions)
        .and_then(|_| epoch.lock_current());
    let mut session = match application {
        Ok(session) => session,
        Err(error) if error == SESSION_BUSY => return Err(error),
        Err(error) => {
            return Err(match &result {
                Ok(record) => format!(
                    "Task goal revision {} persisted; session projection not applied: {error}",
                    record.revision
                ),
                Err(commit_error) => {
                    format!("{commit_error}; session rollback not applied: {error}")
                }
            })
        }
    };
    match result {
        Ok(record) => {
            session.set_goal_projection(record.snapshot(task_id));
            session.set_goal_ledger(record.ledger);
            session.task_goal_messages = Some(record.messages);
            session.task_goal_partial_write = None;
            session.emit_goal_status();
            Ok(())
        }
        Err(error) => {
            if session.task_goal_partial_write.is_some() {
                return Err(format!(
                    "{error}; partial write retained for inspection without rollback"
                ));
            }
            if session.goal.as_ref().is_some_and(|goal| {
                !matches!(
                    goal.status,
                    refact_chat_api::GoalStatus::Active | refact_chat_api::GoalStatus::Verifying
                )
            }) {
                return Err(format!(
                    "{error}; held session goal retained without rollback"
                ));
            }
            if let Err(reload_error) = &authoritative {
                return Err(format!("{error}; canonical goal reload failed: {reload_error}; local controls retained"));
            }
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
                    session.set_goal_projection(record.snapshot(task_id));
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
            session.command_queue.retain(|request| {
                !matches!(&request.command, refact_chat_api::ChatCommand::Regenerate {})
                    && !matches!(&request.command, refact_chat_api::ChatCommand::GoalControl { action }
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
    destination_chat_id: &str,
    source_goal: Option<&GoalSnapshot>,
) -> Result<Option<TaskGoalRecord>, String> {
    let board = super::storage::load_board(gcx.clone(), task_id).await?;
    if board
        .owner_chat_id
        .as_deref()
        .is_some_and(|owner| owner != source_chat_id && owner != destination_chat_id)
    {
        return board.require_owner(source_chat_id).map(|_| None);
    }
    let canonical = resolve_task_goal(gcx, task_id).await?;
    let same_goal = source_goal.is_some_and(|goal| {
        goal.owner
            == Some(GoalOwner::Task {
                id: task_id.into(),
            })
    });
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
    if expected_owner.as_deref() == Some(chat_id) {
        if let Err(error) = super::custody::rebind_task_agents(gcx, task_id).await {
            tracing::warn!(%error, "committed planner custody reconciliation deferred");
        }
        return Ok(());
    }
    if expected_owner
        .as_deref()
        .is_some_and(|owner| owner != source_chat_id)
    {
        return Err(format!("This chat no longer owns task board {task_id}; the current planner chat is {}. Continue there or transfer ownership.", expected_owner.as_deref().unwrap()));
    }
    super::storage::update_board_atomic(gcx.clone(), task_id, |board| {
        if board.owner_chat_id != expected_owner {
            return Err("Task planner owner changed during transfer".into());
        }
        board
            .try_transfer_owner(chat_id, chrono::Utc::now().timestamp_millis().max(0) as u64)
            .map_err(|error| error.to_string())?;
        Ok(())
    })
    .await?;
    if let Err(error) = super::custody::rebind_task_agents(gcx, task_id).await {
        tracing::warn!(%error, "committed planner custody reconciliation deferred");
    }
    Ok(())
}

pub async fn initialize_transition_goal(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    chat_id: &str,
    messages: Vec<ChatMessage>,
    ledger: Vec<GoalLedgerEntry>,
) -> Result<(), String> {
    let (_, guard) = super::storage::locked_task_dir(gcx.clone(), task_id).await?;
    super::storage::load_board_locked(gcx.clone(), task_id, &guard)
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
        super::storage::load_board_locked(gcx, task_id, &guard)
            .await?
            .require_owner(chat_id)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use crate::chat::types::ChatSession;
    use refact_chat_api::{GoalStatus, TaskMeta};
    use refact_core::active_context::{
        active_context, make_reconstruction_report, ReconstructionMetadata,
    };
    use tokio::sync::Mutex;

    #[tokio::test]
    async fn committed_handoff_survives_rebind_failure_and_preflight_retry() {
        let (_temp, app, task, _, _, _) =
            crate::chat::task_agent_monitor::tests::setup_monitor_case(
                "doing",
                crate::chat::types::SessionState::Idle,
                std::time::Duration::ZERO,
                vec![],
            )
            .await;
        super::super::storage::update_board_atomic(app.gcx.clone(), &task, |board| {
            board.transfer_owner("planner-test", 1);
            Ok(())
        })
        .await
        .unwrap();
        super::super::custody::REBIND_FAILURE
            .scope(true, async {
                move_planner_controller(app.gcx.clone(), &task, "planner-test", "destination")
                    .await
                    .unwrap();
                assert_eq!(
                    super::super::storage::load_board(app.gcx.clone(), &task)
                        .await
                        .unwrap()
                        .owner_chat_id
                        .as_deref(),
                    Some("destination")
                );
                preflight_planner_transition(
                    app.gcx.clone(),
                    &task,
                    "planner-test",
                    "destination",
                    None,
                )
                .await
                .unwrap();
                move_planner_controller(app.gcx.clone(), &task, "planner-test", "destination")
                    .await
                    .unwrap();
                assert!(preflight_planner_transition(
                    app.gcx.clone(),
                    &task,
                    "planner-test",
                    "third-party",
                    None
                )
                .await
                .is_err());
            })
            .await;
    }

    fn replayed_session(chat_id: &str) -> (ChatSession, Vec<ChatMessage>) {
        let mut goal = crate::chat::internal_roles::goal(
            "task_planner",
            1,
            "keep approved outcome",
            None,
            true,
            Default::default(),
        );
        goal.message_id = "legacy-plan-goal-existing".into();
        goal.extra["goal"]["status"] = serde_json::json!("paused");
        goal.extra["goal"]["budget"] = serde_json::json!({
            "explicit": true, "max_turns": 7, "max_tokens": 12000, "max_minutes": 20,
            "max_cost_cents": 40, "no_progress_turns": 3,
            "cooldown_ms": 30000, "no_progress_token_threshold": 25
        });
        let mut user = ChatMessage::new("user".into(), "retain conversation".into());
        user.message_id = "conversation-user".into();
        let report =
            make_reconstruction_report(vec![user, goal.clone()], ReconstructionMetadata::default())
                .unwrap();
        let mut replay = goal.clone();
        replay.extra.insert(
            "delivery".into(),
            serde_json::json!({"source":"tool.update_goal"}),
        );
        let mut session = ChatSession::new(chat_id.into());
        session.messages = vec![report, replay];
        (session, vec![goal])
    }

    async fn task_fixture() -> (tempfile::TempDir, AppState, String, String) {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            temp.path().join("cache"),
            temp.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
        let task = super::super::storage::create_task(gcx.clone(), "recovery task")
            .await
            .unwrap();
        let chat_id = format!("planner-{}-2", task.id);
        super::super::storage::update_board_atomic(gcx.clone(), &task.id, |board| {
            board.authorize_owner(&chat_id)
        })
        .await
        .unwrap();
        (temp, AppState::from_gcx(gcx).await, task.id, chat_id)
    }

    #[tokio::test]
    async fn partial_write_reconciles_quiescent_event_without_a_hold() {
        let (temp, app, task_id, chat_id) = task_fixture().await;
        let mut session = ChatSession::new(chat_id.clone());
        session.thread.task_meta = Some(TaskMeta {
            task_id: task_id.clone(),
            role: "planner".into(),
            planner_chat_id: Some(chat_id.clone()),
            ..Default::default()
        });
        session.work_project_root = Some(dunce::canonicalize(temp.path()).unwrap());
        session.install_goal(
            "task_planner",
            "retain approved work",
            true,
            Default::default(),
        );
        session.ensure_goal_ledger_installed();
        let session = Arc::new(Mutex::new(session));
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.clone(), session.clone());
        commit_session_task_goal(app.gcx.clone(), &session)
            .await
            .unwrap();
        let prior = resolve_task_goal(app.gcx.clone(), &task_id)
            .await
            .unwrap()
            .unwrap();
        let mut own = prior.clone();
        own.revision += 1;
        let epoch = TaskGoalSessionEpoch::capture(&app.gcx, &session)
            .await
            .unwrap()
            .unwrap();
        let (_, guard) = super::super::storage::locked_task_dir(app.gcx.clone(), &task_id)
            .await
            .unwrap();
        let partial = epoch
            .begin_write(&app.gcx, &guard, prior.revision, &own)
            .await
            .unwrap();
        write_record(&guard.dir().join("goal.json"), &own)
            .await
            .unwrap();
        partial.outcome.set(Ok(())).unwrap();
        drop(guard);
        let mut pending = session.lock().await;
        pending.draft_message = Some(ChatMessage::new("assistant".into(), "draft".into()));
        let events_before = pending.goal.as_ref().unwrap().events.len();
        pending.enqueue_delivery(crate::chat::types::PendingDelivery::new(
            vec![crate::chat::internal_roles::event(
                crate::chat::internal_roles::EventSubkind::GoalPursuit,
                "chat.goal_monitor",
                serde_json::json!({"kind":"pursuit_quiescent", "trigger":"monitor", "at_ms":44}),
                "Goal pursuit paused: agent idle with no progress; will resume on your next message.",
            )],
            crate::chat::types::PushMode::Append,
            "chat.goal_monitor",
            false,
        )).unwrap();
        assert_eq!(pending.pending_deliveries.len(), 1);
        assert_eq!(pending.goal.as_ref().unwrap().events.len(), events_before);
        drop(pending);
        hydrate_session_task_goal(app.gcx.clone(), &session)
            .await
            .unwrap();
        let reconciled = resolve_task_goal(app.gcx.clone(), &task_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reconciled.revision, own.revision + 1);
        assert_eq!(reconciled.ledger, own.ledger);
        assert_eq!(reconciled.messages.len(), own.messages.len() + 1);
        assert_eq!(
            reconciled.messages.last().unwrap().extra["event"]["payload"]["kind"],
            "pursuit_quiescent"
        );
        assert_eq!(
            reconciled.snapshot(&task_id).unwrap().status,
            GoalStatus::Active
        );
        assert!(session.lock().await.task_goal_partial_write.is_none());
        assert!(
            crate::chat::linearize::apply_summarization_linearize(reconciled.messages)
                .unwrap()
                .iter()
                .all(|message| message.extra.get("event").is_none())
        );
    }

    #[tokio::test]
    async fn nonowner_hydration_retains_partial_write_and_skips_pursuit() {
        let (temp, app, task_id, chat_id) = task_fixture().await;
        let mut session = ChatSession::new(chat_id.clone());
        session.thread.task_meta = Some(TaskMeta {
            task_id: task_id.clone(),
            role: "planner".into(),
            planner_chat_id: Some(chat_id.clone()),
            ..Default::default()
        });
        session.work_project_root = Some(dunce::canonicalize(temp.path()).unwrap());
        session.install_goal(
            "task_planner",
            "retain approved work",
            true,
            Default::default(),
        );
        session.ensure_goal_ledger_installed();
        let session = Arc::new(Mutex::new(session));
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.clone(), session.clone());
        commit_session_task_goal(app.gcx.clone(), &session)
            .await
            .unwrap();
        let canonical = resolve_task_goal(app.gcx.clone(), &task_id)
            .await
            .unwrap()
            .unwrap();
        let epoch = TaskGoalSessionEpoch::capture(&app.gcx, &session)
            .await
            .unwrap()
            .unwrap();
        let (_, guard) = super::super::storage::locked_task_dir(app.gcx.clone(), &task_id)
            .await
            .unwrap();
        let partial = epoch
            .begin_write(&app.gcx, &guard, canonical.revision, &canonical)
            .await
            .unwrap();
        assert!(partial.outcome.get().is_none());
        drop(guard);
        super::super::storage::update_board_atomic(app.gcx.clone(), &task_id, |board| {
            board.owner_chat_id = Some("new-planner".into());
            Ok(())
        })
        .await
        .unwrap();
        let before = TaskGoalSessionEpoch::state(&*session.lock().await).unwrap();
        hydrate_session_task_goal(app.gcx.clone(), &session)
            .await
            .unwrap();
        assert!(
            !hydrate_session_task_goal_for_pursuit(app.gcx.clone(), &session)
                .await
                .unwrap()
        );
        assert!(
            !crate::chat::goal_monitor::handle_goal_turn_end(app.clone(), session.clone()).await
        );
        assert!(matches!(
            crate::chat::goal_verifier::verify_goal_before_completion(
                app.clone(),
                session.clone(),
                "turn_end"
            )
            .await,
            crate::chat::goal_verifier::GoalCompletionGateOutcome::Passthrough
        ));
        let error = hydrate_session_task_goal_for_mutation(app.gcx.clone(), &session)
            .await
            .unwrap_err();
        assert_eq!(error, "This chat no longer owns the task board; the current planner chat is new-planner. Continue there or transfer ownership.");
        let held = session.lock().await;
        assert_eq!(TaskGoalSessionEpoch::state(&held).unwrap(), before);
        assert!(Arc::ptr_eq(
            held.task_goal_partial_write.as_ref().unwrap(),
            &partial
        ));
        drop(held);
        assert_eq!(
            serde_json::to_value(
                resolve_task_goal(app.gcx.clone(), &task_id)
                    .await
                    .unwrap()
                    .unwrap()
            )
            .unwrap(),
            serde_json::to_value(canonical).unwrap()
        );
    }

    #[tokio::test]
    async fn held_goal_hydration_rebases_after_owner_returns() {
        let (temp, app, task_id, chat_id) = task_fixture().await;
        let mut session = ChatSession::new(chat_id.clone());
        session.thread.task_meta = Some(TaskMeta {
            task_id: task_id.clone(),
            role: "planner".into(),
            planner_chat_id: Some(chat_id.clone()),
            ..Default::default()
        });
        session.work_project_root = Some(dunce::canonicalize(temp.path()).unwrap());
        session.install_goal(
            "task_planner",
            "retain approved work",
            true,
            Default::default(),
        );
        session.ensure_goal_ledger_installed();
        let session = Arc::new(Mutex::new(session));
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.clone(), session.clone());
        commit_session_task_goal(app.gcx.clone(), &session)
            .await
            .unwrap();
        let canonical = resolve_task_goal(app.gcx.clone(), &task_id)
            .await
            .unwrap()
            .unwrap();
        assert!(session
            .lock()
            .await
            .goal_set_status_reason(GoalStatus::Paused, "user pause"));
        super::super::storage::update_board_atomic(app.gcx.clone(), &task_id, |board| {
            board.owner_chat_id = Some("other-planner".into());
            Ok(())
        })
        .await
        .unwrap();
        assert!(
            !hydrate_session_task_goal_for_pursuit(app.gcx.clone(), &session)
                .await
                .unwrap()
        );
        let mut messages = canonical.messages.clone();
        messages.push(crate::chat::internal_roles::goal_delta(
            "task_planner",
            serde_json::json!({"seq":1}),
            "new canonical outcome",
        ));
        let newer = commit_task_goal(
            app.gcx.clone(),
            &task_id,
            "other-planner",
            canonical.revision,
            messages,
            canonical.ledger,
        )
        .await
        .unwrap();
        assert!(newer.revision > canonical.revision);
        super::super::storage::update_board_atomic(app.gcx.clone(), &task_id, |board| {
            board.owner_chat_id = Some(chat_id.clone());
            Ok(())
        })
        .await
        .unwrap();
        assert!(
            hydrate_session_task_goal_for_pursuit(app.gcx.clone(), &session)
                .await
                .unwrap()
        );
        let rebased = resolve_task_goal(app.gcx.clone(), &task_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rebased.revision, newer.revision + 1);
        assert_eq!(
            serde_json::to_value(&rebased.messages).unwrap(),
            serde_json::to_value(&newer.messages).unwrap()
        );
        assert!(rebased.ledger.starts_with(&newer.ledger));
        assert_eq!(
            rebased.snapshot(&task_id).unwrap().status,
            GoalStatus::Paused
        );
        assert_eq!(
            session.lock().await.goal.as_ref().unwrap().revision,
            rebased.revision
        );
        {
            let mut local = session.lock().await;
            let goal = local.goal.as_mut().unwrap();
            goal.owner = Some(GoalOwner::Chat {
                id: chat_id.clone(),
            });
            goal.revision = rebased.revision + 100;
            local.goal_set_status_reason(GoalStatus::Stopped, "user stop");
        }
        assert!(
            hydrate_session_task_goal_for_pursuit(app.gcx.clone(), &session)
                .await
                .unwrap()
        );
        let stopped = resolve_task_goal(app.gcx.clone(), &task_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stopped.revision, rebased.revision + 1);
        assert_eq!(
            stopped.snapshot(&task_id).unwrap().status,
            GoalStatus::Stopped
        );
        assert_eq!(
            stopped.snapshot(&task_id).unwrap().stop_reason.as_deref(),
            Some("user stop")
        );
    }

    #[tokio::test]
    async fn held_goal_hydration_refuses_local_content_conflict() {
        let (temp, app, task_id, chat_id) = task_fixture().await;
        let mut session = ChatSession::new(chat_id.clone());
        session.thread.task_meta = Some(TaskMeta {
            task_id: task_id.clone(),
            role: "planner".into(),
            planner_chat_id: Some(chat_id.clone()),
            ..Default::default()
        });
        session.work_project_root = Some(dunce::canonicalize(temp.path()).unwrap());
        session.install_goal(
            "task_planner",
            "retain approved work",
            true,
            Default::default(),
        );
        session.ensure_goal_ledger_installed();
        let session = Arc::new(Mutex::new(session));
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.clone(), session.clone());
        commit_session_task_goal(app.gcx.clone(), &session)
            .await
            .unwrap();
        let canonical = resolve_task_goal(app.gcx.clone(), &task_id)
            .await
            .unwrap()
            .unwrap();
        assert!(session
            .lock()
            .await
            .goal_set_status_reason(GoalStatus::Paused, "user pause"));
        session
            .lock()
            .await
            .add_message(crate::chat::internal_roles::goal_delta(
                "task_planner",
                serde_json::json!({"seq":1}),
                "unpersisted local outcome",
            ));
        let mut messages = canonical.messages.clone();
        messages.push(crate::chat::internal_roles::goal_delta(
            "task_planner",
            serde_json::json!({"seq":1}),
            "new canonical outcome",
        ));
        let newer = commit_task_goal(
            app.gcx.clone(),
            &task_id,
            &chat_id,
            canonical.revision,
            messages,
            canonical.ledger,
        )
        .await
        .unwrap();
        assert!(newer.revision > canonical.revision);
        let before = TaskGoalSessionEpoch::state(&*session.lock().await).unwrap();
        let error = hydrate_session_task_goal(app.gcx.clone(), &session)
            .await
            .unwrap_err();
        assert_eq!(error, "Task goal hold conflicts with local goal content. Local hold retained; create a new task planner chat to transfer ownership and adopt canonical state.");
        assert_eq!(
            TaskGoalSessionEpoch::state(&*session.lock().await).unwrap(),
            before
        );
        assert_eq!(
            serde_json::to_value(
                resolve_task_goal(app.gcx.clone(), &task_id)
                    .await
                    .unwrap()
                    .unwrap()
            )
            .unwrap(),
            serde_json::to_value(newer).unwrap()
        );
    }

    #[tokio::test]
    async fn task_goal_epoch_waits_for_session_and_revalidates() {
        tokio::time::timeout(std::time::Duration::from_secs(60), async {
        let (temp, app, task_id, chat_id) = task_fixture().await;
        let mut session = ChatSession::new(chat_id.clone());
        session.thread.task_meta = Some(TaskMeta {
            task_id: task_id.clone(),
            role: "planner".into(),
            planner_chat_id: Some(chat_id.clone()),
            ..Default::default()
        });
        session.work_project_root = Some(dunce::canonicalize(temp.path()).unwrap());
        session.install_goal("task_planner", "retain approved work", true, Default::default());
        session.ensure_goal_ledger_installed();
        let session = Arc::new(Mutex::new(session));
        app.chat.sessions.write().await.insert(chat_id.clone(), session.clone());
        commit_session_task_goal(app.gcx.clone(), &session).await.unwrap();
        let before = resolve_task_goal(app.gcx.clone(), &task_id).await.unwrap().unwrap();
        let (_, board) = super::super::storage::locked_task_dir(app.gcx.clone(), &task_id).await.unwrap();
        let waiting = Arc::new(tokio::sync::Notify::new());
        let busy = Arc::new(tokio::sync::Notify::new());
        let mut hydration = Box::pin(super::super::storage::TASK_BOARD_LOCK_WAIT.scope(
            waiting.clone(), TASK_GOAL_BUSY_WAIT.scope(busy.clone(), hydrate_session_task_goal(app.gcx.clone(), &session))));
        tokio::select! {
            result = &mut hydration => panic!("hydration returned before board contention: {result:?}"),
            _ = waiting.notified() => {}
        }
        let held = session.lock().await;
        drop(board);
        tokio::select! {
            result = &mut hydration => panic!("session contention must wait without failing: {result:?}"),
            _ = busy.notified() => {}
        }
        let registry = tokio::time::timeout(std::time::Duration::from_secs(60), app.chat.sessions.write()).await.unwrap();
        drop(registry);
        let (_, board) = super::super::storage::locked_task_dir(app.gcx.clone(), &task_id).await.unwrap();
        drop(board);
        drop(held);
        hydration.await.unwrap();
        assert_eq!(resolve_task_goal(app.gcx.clone(), &task_id).await.unwrap().unwrap().revision, before.revision);
        assert_eq!(session.lock().await.goal.as_ref().unwrap().status, GoalStatus::Active);

        let waiting = Arc::new(tokio::sync::Notify::new());
        let busy = Arc::new(tokio::sync::Notify::new());
        let (_, board) = super::super::storage::locked_task_dir(app.gcx.clone(), &task_id).await.unwrap();
        let mut hydration = Box::pin(super::super::storage::TASK_BOARD_LOCK_WAIT.scope(
            waiting.clone(), TASK_GOAL_BUSY_WAIT.scope(busy.clone(), hydrate_session_task_goal(app.gcx.clone(), &session))));
        tokio::select! {
            result = &mut hydration => panic!("hydration returned before stale-epoch setup: {result:?}"),
            _ = waiting.notified() => {}
        }
        let mut held = session.lock().await;
        drop(board);
        tokio::select! {
            result = &mut hydration => panic!("session contention must wait before stale-epoch mutation: {result:?}"),
            _ = busy.notified() => {}
        }
        held.add_message(ChatMessage::new("user".into(), "new accepted input".into()));
        let state = TaskGoalSessionEpoch::state(&held).unwrap();
        drop(held);
        hydration.await.unwrap();
        assert_eq!(TaskGoalSessionEpoch::state(&*session.lock().await).unwrap(), state);
        assert_eq!(session.lock().await.messages.last().unwrap().content.content_text_only(), "new accepted input");
        assert_eq!(resolve_task_goal(app.gcx.clone(), &task_id).await.unwrap().unwrap().revision, before.revision);

        let (_, board) = super::super::storage::locked_task_dir(app.gcx.clone(), &task_id).await.unwrap();
        let waiting = Arc::new(tokio::sync::Notify::new());
        let mut hydration = Box::pin(super::super::storage::TASK_BOARD_LOCK_WAIT.scope(
            waiting.clone(), hydrate_session_task_goal(app.gcx.clone(), &session)));
        tokio::select! {
            result = &mut hydration => panic!("hydration returned before retry setup: {result:?}"),
            _ = waiting.notified() => {}
        }
        session.lock().await.goal_verification_generation += 1;
        drop(board);
        hydration.await.unwrap();
        assert_eq!(session.lock().await.goal.as_ref().unwrap().revision, before.revision);
        }).await.expect("task goal recovery regression timed out");
    }

    #[tokio::test]
    async fn post_write_contention_rechecks_acknowledged_canonical_record() {
        tokio::time::timeout(std::time::Duration::from_secs(60), async {
        for competing in [false, true] {
            let (temp, app, task_id, chat_id) = task_fixture().await;
            let mut session = ChatSession::new(chat_id.clone());
            session.thread.task_meta = Some(TaskMeta {
                task_id: task_id.clone(), role: "planner".into(),
                planner_chat_id: Some(chat_id.clone()), ..Default::default()
            });
            session.work_project_root = Some(dunce::canonicalize(temp.path()).unwrap());
            session.install_goal("task_planner", "retain approved work", true, Default::default());
            session.ensure_goal_ledger_installed();
            let session = Arc::new(Mutex::new(session));
            app.chat.sessions.write().await.insert(chat_id.clone(), session.clone());
            commit_session_task_goal(app.gcx.clone(), &session).await.unwrap();
            let before = resolve_task_goal(app.gcx.clone(), &task_id).await.unwrap().unwrap();
            session.lock().await.add_message(crate::chat::internal_roles::goal_delta(
                "task_planner", serde_json::json!({"seq":1}), "accepted update"));
            let pending = Arc::new(tokio::sync::Notify::new());
            let resume = Arc::new(tokio::sync::Notify::new());
            let busy = Arc::new(tokio::sync::Notify::new());
            let mut writer = Box::pin(TASK_GOAL_WRITE_PAUSE.scope(
                (pending.clone(), resume.clone()), TASK_GOAL_BUSY_WAIT.scope(
                    busy.clone(), commit_session_task_goal(app.gcx.clone(), &session))));
            tokio::select! {
                result = &mut writer => panic!("write returned before durable pending barrier: {result:?}"),
                _ = pending.notified() => {}
            }
            let mut held = session.lock().await;
            resume.notify_one();
            tokio::select! {
                result = &mut writer => panic!("post-write contention must wait: {result:?}"),
                _ = busy.notified() => {}
            }
            assert!(matches!(held.task_goal_partial_write.as_ref().unwrap().outcome.get(), Some(Ok(()))));
            let own = resolve_task_goal(app.gcx.clone(), &task_id).await.unwrap().unwrap();
            assert_eq!(own.revision, before.revision + 1);
            assert_eq!(held.goal.as_ref().unwrap().revision, before.revision);
            held.thread.model = "concurrent/model-selection".into();
            held.trajectory_version += 1;
            held.pending_deliveries.push_back(crate::chat::types::PendingDelivery::new(
                vec![ChatMessage::new("user".into(), "unrelated delivery".into())],
                crate::chat::types::PushMode::Append, "test", false));
            let canonical = if competing {
                let mut messages = own.messages.clone();
                messages.push(crate::chat::internal_roles::goal_delta(
                    "task_planner", serde_json::json!({"seq":2}), "competing canonical update"));
                commit_task_goal(app.gcx.clone(), &task_id, &chat_id, own.revision, messages, own.ledger.clone()).await.unwrap()
            } else { own.clone() };
            let state = TaskGoalSessionEpoch::state(&held).unwrap();
            drop(held);
            let result = writer.await;
            if competing {
                assert!(result.unwrap_err().contains("canonical revision/content superseded"));
                let held = session.lock().await;
                assert_eq!(TaskGoalSessionEpoch::state(&held).unwrap(), state);
                assert!(held.task_goal_partial_write.is_some());
            } else {
                result.unwrap();
                let held = session.lock().await;
                assert_eq!(held.goal.as_ref().unwrap().revision, own.revision);
                assert!(held.task_goal_partial_write.is_none());
                assert_eq!(held.thread.model, "concurrent/model-selection");
                assert_eq!(held.pending_deliveries.back().unwrap().messages[0].content.content_text_only(), "unrelated delivery");
            }
            let after = resolve_task_goal(app.gcx.clone(), &task_id).await.unwrap().unwrap();
            assert_eq!(serde_json::to_value(after).unwrap(), serde_json::to_value(canonical).unwrap());
        }
        }).await.expect("task goal recovery regression timed out");
    }

    #[tokio::test]
    async fn goal_fence_tracks_control_payload_not_delivery_transport() {
        let (temp, app, task_id, chat_id) = task_fixture().await;
        let mut local = ChatSession::new(chat_id.clone());
        local.thread.task_meta = Some(TaskMeta {
            task_id,
            role: "planner".into(),
            planner_chat_id: Some(chat_id.clone()),
            ..Default::default()
        });
        local.work_project_root = Some(dunce::canonicalize(temp.path()).unwrap());
        local.install_goal("task_planner", "approved", true, Default::default());
        let delta = crate::chat::internal_roles::goal_delta(
            "test",
            serde_json::json!({"seq":1}),
            "accepted correction",
        );
        local
            .pending_deliveries
            .push_back(crate::chat::types::PendingDelivery::new(
                vec![delta],
                crate::chat::types::PushMode::Append,
                "test",
                false,
            ));
        let session = Arc::new(Mutex::new(local));
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id, session.clone());
        let epoch = TaskGoalSessionEpoch::capture(&app.gcx, &session)
            .await
            .unwrap()
            .unwrap();
        {
            let mut local = session.lock().await;
            let mut delta = local
                .pending_deliveries
                .pop_front()
                .unwrap()
                .messages
                .remove(0);
            delta
                .extra
                .insert("delivery".into(), serde_json::json!({"source":"test"}));
            local.messages.push(delta);
        }
        epoch.validate(&app.gcx).await.unwrap();
        session.lock().await.messages.last_mut().unwrap().content =
            crate::call_validation::ChatContent::SimpleText("changed same-ID correction".into());
        assert_eq!(
            epoch.validate(&app.gcx).await.unwrap_err(),
            "Task goal session changed while awaiting persistence; retry"
        );
    }

    #[tokio::test]
    async fn rejected_commit_restores_canonical_without_creating_pause() {
        let (temp, app, task_id, chat_id) = task_fixture().await;
        let mut local = ChatSession::new(chat_id.clone());
        local.thread.task_meta = Some(TaskMeta {
            task_id: task_id.clone(),
            role: "planner".into(),
            planner_chat_id: Some(chat_id.clone()),
            ..Default::default()
        });
        local.work_project_root = Some(dunce::canonicalize(temp.path()).unwrap());
        local.install_goal("task_planner", "approved", true, Default::default());
        let session = Arc::new(Mutex::new(local));
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.clone(), session.clone());
        commit_session_task_goal(app.gcx.clone(), &session)
            .await
            .unwrap();
        let prior = resolve_task_goal(app.gcx.clone(), &task_id)
            .await
            .unwrap()
            .unwrap();
        let mut controls = prior.messages.clone();
        controls.push(crate::chat::internal_roles::goal_delta(
            "test",
            serde_json::json!({"seq":1}),
            "canonical correction",
        ));
        let canonical = commit_task_goal(
            app.gcx.clone(),
            &task_id,
            &chat_id,
            prior.revision,
            controls,
            prior.ledger,
        )
        .await
        .unwrap();
        session
            .lock()
            .await
            .add_message(crate::chat::internal_roles::goal_delta(
                "test",
                serde_json::json!({"seq":1}),
                "stale local correction",
            ));
        assert!(commit_session_task_goal(app.gcx.clone(), &session)
            .await
            .unwrap_err()
            .contains("revision conflict"));
        let local = session.lock().await;
        assert_eq!(
            serde_json::to_value(&local.goal).unwrap(),
            serde_json::to_value(canonical.snapshot(&task_id)).unwrap()
        );
        assert!(local.goal.as_ref().unwrap().active);
        assert_eq!(local.goal.as_ref().unwrap().status, GoalStatus::Active);
        assert_eq!(local.goal.as_ref().unwrap().stop_reason, None);
    }

    #[tokio::test]
    async fn contention_rollback_hold_is_persisted_without_resuming() {
        tokio::time::timeout(std::time::Duration::from_secs(60), async {
        let (temp, app, task_id, chat_id) = task_fixture().await;
        let mut session = ChatSession::new(chat_id.clone());
        session.thread.task_meta = Some(TaskMeta {
            task_id: task_id.clone(), role: "planner".into(),
            planner_chat_id: Some(chat_id.clone()), ..Default::default()
        });
        session.work_project_root = Some(dunce::canonicalize(temp.path()).unwrap());
        session.install_goal("task_planner", "retain approved work", true, Default::default());
        session.ensure_goal_ledger_installed();
        let session = Arc::new(Mutex::new(session));
        app.chat.sessions.write().await.insert(chat_id.clone(), session.clone());
        commit_session_task_goal(app.gcx.clone(), &session).await.unwrap();
        let before = resolve_task_goal(app.gcx.clone(), &task_id).await.unwrap().unwrap();
        {
            let mut session = session.lock().await;
            let goal = session.goal.as_mut().unwrap();
            assert!(goal.active);
            assert_eq!(goal.status, GoalStatus::Active);
            goal.active = false;
            goal.status = GoalStatus::Paused;
            goal.stop_reason = Some(SESSION_BUSY.into());
            session.refresh_goal_runtime_mirror();
        }
        hydrate_session_task_goal(app.gcx.clone(), &session).await.unwrap();
        let held = resolve_task_goal(app.gcx.clone(), &task_id).await.unwrap().unwrap();
        assert_eq!(held.revision, before.revision + 1);
        assert!(held.ledger.starts_with(&before.ledger));
        let canonical = held.snapshot(&task_id).unwrap();
        assert!(canonical.active);
        assert_eq!(canonical.status, GoalStatus::Paused);
        assert_eq!(serde_json::to_value(&held.messages).unwrap(), serde_json::to_value(&before.messages).unwrap());
        assert_eq!(serde_json::to_value(&session.lock().await.goal).unwrap(), serde_json::to_value(Some(canonical)).unwrap());
        let mut events = session.lock().await.subscribe();
        crate::chat::handle_v1_chat_command(
            axum::extract::State(app.clone()), axum::extract::Path(chat_id.clone()),
            serde_json::to_vec(&serde_json::json!({"type":"goal_control", "action":"resume", "client_request_id":"resume-contention-hold"})).unwrap().into(),
        ).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(60), async {
            loop {
                let wire = events.recv().await.unwrap();
                let event: crate::chat::types::ChatEvent = serde_json::from_str(&wire).unwrap();
                if matches!(event, crate::chat::types::ChatEvent::RuntimeUpdated { goal_status: Some(GoalStatus::Active), .. }) {
                    break;
                }
            }
        }).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(60), async {
            loop {
                let session = session.lock().await;
                if session.native_commands_in_flight.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                    break;
                }
                drop(session);
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        let resumed = resolve_task_goal(app.gcx.clone(), &task_id).await.unwrap().unwrap().snapshot(&task_id).unwrap();
        assert!(resumed.active);
        assert_eq!(resumed.status, GoalStatus::Active);
        }).await.expect("task goal recovery regression timed out");
    }

    #[tokio::test]
    async fn opening_replayed_planner_recovers_without_losing_archive_or_resuming_goal() {
        let (temp, app, task_id, chat_id) = task_fixture().await;
        let binding = super::super::storage::TaskDirectoryBinding::capture(
            app.gcx.clone(),
            temp.path(),
            &task_id,
        )
        .await
        .unwrap();
        let (mut session, controls) = replayed_session(&chat_id);
        session.thread.task_meta = Some(TaskMeta {
            task_id: task_id.clone(),
            role: "planner".into(),
            planner_chat_id: Some(chat_id.clone()),
            ..Default::default()
        });
        let record = binding
            .scope(commit_task_goal(
                app.gcx.clone(),
                &task_id,
                &chat_id,
                0,
                controls,
                vec![],
            ))
            .await
            .unwrap();
        session.set_goal_projection(record.snapshot(&task_id));
        let archive = serde_json::to_value(&session.messages).unwrap();
        assert!(active_context(&session.messages).is_err());
        let snapshot = crate::chat::trajectories::trajectory_snapshot_from_session(&session);
        crate::chat::trajectories::save_trajectory_snapshot(app.gcx.clone(), snapshot)
            .await
            .unwrap();

        let restored = crate::chat::get_or_create_session_with_trajectory(
            app.clone(),
            &app.chat.sessions,
            &chat_id,
        )
        .await
        .unwrap();
        let restored_guard = restored.lock().await;
        let context = active_context(&restored_guard.snapshot_messages())
            .expect("opening must recover the invalid planner before publishing its snapshot");
        assert_eq!(
            serde_json::to_value(&restored_guard.messages[..2]).unwrap(),
            archive
        );
        assert_eq!(restored_guard.messages.len(), 3);
        assert_eq!(context.messages.len(), 2);
        assert_eq!(
            context.messages[0].content.content_text_only(),
            "retain conversation"
        );
        assert_eq!(
            context.messages[1].content.content_text_only(),
            "keep approved outcome"
        );
        let goal = restored_guard.goal.as_ref().unwrap();
        assert!(goal.active);
        assert_eq!(goal.status, GoalStatus::Paused);
        assert_eq!(goal.revision, record.revision);
        drop(restored_guard);
        hydrate_session_task_goal(app.gcx.clone(), &restored)
            .await
            .unwrap();
        assert_eq!(restored.lock().await.messages.len(), 3);
        assert_eq!(
            resolve_task_goal(app.gcx.clone(), &task_id)
                .await
                .unwrap()
                .unwrap()
                .revision,
            record.revision
        );
        {
            let mut session = restored.lock().await;
            session.thread.model = "test/model".into();
            session.work_project_root = Some(temp.path().canonicalize().unwrap());
        }
        let binding = crate::file_activity::TOOL_ACTOR
            .scope(
                chat_id.clone(),
                super::super::storage::TaskDirectoryBinding::capture(
                    app.gcx.clone(),
                    temp.path(),
                    &task_id,
                ),
            )
            .await
            .unwrap();
        let (_, guard) = super::super::storage::locked_task_dir(app.gcx.clone(), &task_id)
            .await
            .unwrap();
        let waiting = Arc::new(tokio::sync::Notify::new());
        let waiter = tokio::spawn({
            let waiting = waiting.clone();
            let binding = binding.clone();
            let gcx = app.gcx.clone();
            let restored = restored.clone();
            let chat_id = chat_id.clone();
            async move {
                super::super::storage::TASK_BOARD_LOCK_WAIT
                    .scope(
                        waiting,
                        crate::file_activity::TOOL_ACTOR.scope(
                            chat_id,
                            binding.scope(hydrate_session_task_goal(gcx, &restored)),
                        ),
                    )
                    .await
            }
        });
        tokio::time::timeout(std::time::Duration::from_secs(60), waiting.notified())
            .await
            .unwrap();
        assert!(!waiter.is_finished());
        assert!(
            restored.try_lock().is_ok(),
            "board waiter must release the session mutex"
        );
        drop(guard);
        tokio::time::timeout(std::time::Duration::from_secs(60), waiter)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        crate::file_activity::TOOL_ACTOR
            .scope(
                chat_id.clone(),
                binding.scope(async {
                    let mut controls = record.messages.clone();
                    controls.push(crate::chat::internal_roles::goal_delta(
                        "task_planner",
                        serde_json::json!({"seq": 1}),
                        "retain guarded update",
                    ));
                    let updated = commit_task_goal(
                        app.gcx.clone(),
                        &task_id,
                        &chat_id,
                        record.revision,
                        controls,
                        record.ledger.clone(),
                    )
                    .await
                    .unwrap();
                    assert_eq!(updated.revision, record.revision + 1);
                    hydrate_session_task_goal(app.gcx.clone(), &restored)
                        .await
                        .unwrap();
                    commit_session_task_goal(app.gcx.clone(), &restored)
                        .await
                        .unwrap();
                    let session = restored.lock().await;
                    let goal = session.goal.as_ref().unwrap();
                    assert!(goal.revision >= updated.revision);
                    let session_revision = goal.revision;
                    assert_eq!(goal.status, GoalStatus::Paused);
                    assert_eq!(goal.budget.max_turns, Some(7));
                    assert_eq!(goal.budget.max_tokens, Some(12000));
                    assert_eq!(goal.budget.max_minutes, Some(20));
                    assert_eq!(goal.budget.max_cost_cents, Some(40));
                    assert_eq!(goal.budget.no_progress_turns, Some(3));
                    assert_eq!(
                        serde_json::to_value(&session.messages[..2]).unwrap(),
                        archive
                    );
                    drop(session);
                    assert_eq!(
                        resolve_task_goal(app.gcx.clone(), &task_id)
                            .await
                            .unwrap()
                            .unwrap()
                            .revision,
                        session_revision
                    );
                }),
            )
            .await;
        let goal_path = temp
            .path()
            .join(".refact/tasks")
            .join(&task_id)
            .join("goal.json");
        for status in [GoalStatus::Active, GoalStatus::Verifying] {
            {
                let mut session = restored.lock().await;
                session.ensure_goal_ledger_installed();
                assert!(session.goal_set_status_reason(status, "accepted pursuit transition"));
            }
            crate::file_activity::TOOL_ACTOR
                .scope(
                    chat_id.clone(),
                    binding.scope(commit_session_task_goal(app.gcx.clone(), &restored)),
                )
                .await
                .unwrap();
            let prior = read_record(&goal_path).await.unwrap().unwrap();
            let accepted = {
                let mut session = restored.lock().await;
                session.add_message(crate::chat::internal_roles::goal_delta(
                    "task_planner",
                    serde_json::json!({"seq": 2}),
                    "accepted non-hold write must heal without a later user pause",
                ));
                session.try_accepted_control_projection().unwrap()
            };
            assert_ne!(
                serde_json::to_value(&accepted.messages).unwrap(),
                serde_json::to_value(&prior.messages).unwrap()
            );
            let pending = Arc::new(tokio::sync::Notify::new());
            let resume = Arc::new(tokio::sync::Notify::new());
            let writer = tokio::spawn({
                let binding = binding.clone();
                let gcx = app.gcx.clone();
                let restored = restored.clone();
                let chat_id = chat_id.clone();
                let pause = (pending.clone(), resume.clone());
                async move {
                    TASK_GOAL_WRITE_PAUSE
                        .scope(
                            pause,
                            crate::file_activity::TOOL_ACTOR.scope(
                                chat_id,
                                binding.scope(commit_session_task_goal(gcx, &restored)),
                            ),
                        )
                        .await
                }
            });
            tokio::time::timeout(std::time::Duration::from_secs(60), pending.notified())
                .await
                .unwrap();
            assert!(!writer.is_finished());
            let (messages, queue, version) = {
                let mut session = restored
                    .try_lock()
                    .expect("pending non-hold writer must release session");
                let version = session.trajectory_version;
                session.add_message(ChatMessage::new(
                    "user".into(),
                    "new conversation while own write is pending".into(),
                ));
                session.goal_verification_generation += 1;
                assert!(session.trajectory_version > version);
                session
                    .command_queue
                    .push_back(refact_chat_api::CommandRequest {
                        native_approval_guard: None,
                        supervision_guard: None,
                        client_request_id: "pending-user-input".into(),
                        priority: false,
                        command: refact_chat_api::ChatCommand::UserMessage {
                            content: serde_json::json!("retain queued user input"),
                            attachments: vec![],
                            context_files: vec![],
                            suppress_auto_enrichment: false,
                            client_message_id: None,
                            app_context_ids: Vec::new(),
                        },
                    });
                assert!(session
                    .task_goal_partial_write
                    .as_ref()
                    .unwrap()
                    .outcome
                    .get()
                    .is_none());
                (
                    serde_json::to_value(&session.messages).unwrap(),
                    serde_json::to_value(&session.command_queue).unwrap(),
                    session.trajectory_version,
                )
            };
            resume.notify_one();
            assert_eq!(
                tokio::time::timeout(std::time::Duration::from_secs(60), writer)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap_err(),
                format!("Task goal revision {} persisted; session projection not applied: Task goal session changed while awaiting persistence; retry", prior.revision + 1)
            );
            restored.lock().await.goal_verification_generation -= 1;
            let bytes = tokio::fs::read(&goal_path).await.unwrap();
            let own: TaskGoalRecord = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(own.revision, prior.revision + 1);
            assert_eq!(own.snapshot(&task_id).unwrap().status, status);
            assert_eq!(
                serde_json::to_value(&own.messages).unwrap(),
                serde_json::to_value(&accepted.messages).unwrap()
            );
            assert_eq!(own.ledger, accepted.goal_ledger);
            assert_eq!(
                restored.lock().await.goal.as_ref().unwrap().revision,
                prior.revision
            );
            assert!(matches!(
                restored
                    .lock()
                    .await
                    .task_goal_partial_write
                    .as_ref()
                    .unwrap()
                    .outcome
                    .get(),
                Some(Ok(()))
            ));
            if status == GoalStatus::Active {
                let acknowledged = {
                    let mut session = restored.lock().await;
                    let mut partial = session.task_goal_partial_write.take().unwrap();
                    let acknowledged =
                        std::mem::take(&mut Arc::get_mut(&mut partial).unwrap().outcome);
                    session.task_goal_partial_write = Some(partial);
                    acknowledged
                };
                crate::file_activity::TOOL_ACTOR
                    .scope(
                        chat_id.clone(),
                        binding.scope(async {
                            let unchanged =
                                TaskGoalSessionEpoch::state(&*restored.lock().await).unwrap();
                            assert!(hydrate_session_task_goal(app.gcx.clone(), &restored)
                                .await
                                .unwrap_err()
                                .contains("not acknowledged"));
                            assert_eq!(
                                TaskGoalSessionEpoch::state(&*restored.lock().await).unwrap(),
                                unchanged
                            );
                            assert_eq!(tokio::fs::read(&goal_path).await.unwrap(), bytes);
                        }),
                    )
                    .await;
                let incarnation = {
                    let mut session = restored.lock().await;
                    let mut partial = session.task_goal_partial_write.take().unwrap();
                    Arc::get_mut(&mut partial).unwrap().outcome = acknowledged;
                    session.task_goal_partial_write = Some(partial);
                    std::mem::replace(
                        &mut session.closed_flag,
                        Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    )
                };
                crate::file_activity::TOOL_ACTOR
                    .scope(
                        chat_id.clone(),
                        binding.scope(async {
                            let unchanged =
                                TaskGoalSessionEpoch::state(&*restored.lock().await).unwrap();
                            assert!(commit_session_task_goal(app.gcx.clone(), &restored)
                                .await
                                .unwrap_err()
                                .contains("identity superseded"));
                            assert_eq!(
                                TaskGoalSessionEpoch::state(&*restored.lock().await).unwrap(),
                                unchanged
                            );
                            assert_eq!(tokio::fs::read(&goal_path).await.unwrap(), bytes);
                        }),
                    )
                    .await;
                let goal = {
                    let mut session = restored.lock().await;
                    session.closed_flag = incarnation;
                    let goal = session.goal.clone();
                    session.goal.as_mut().unwrap().budget.max_turns = Some(1);
                    goal
                };
                crate::file_activity::TOOL_ACTOR
                    .scope(
                        chat_id.clone(),
                        binding.scope(async {
                            let unchanged =
                                TaskGoalSessionEpoch::state(&*restored.lock().await).unwrap();
                            assert!(hydrate_session_task_goal(app.gcx.clone(), &restored)
                                .await
                                .unwrap_err()
                                .contains("goal/budget superseded"));
                            assert_eq!(
                                TaskGoalSessionEpoch::state(&*restored.lock().await).unwrap(),
                                unchanged
                            );
                            assert_eq!(tokio::fs::read(&goal_path).await.unwrap(), bytes);
                        }),
                    )
                    .await;
                restored.lock().await.set_goal_projection(goal);
                let generation = {
                    let mut session = restored.lock().await;
                    let generation = session.goal_verification_generation;
                    assert!(session.supersede_goal_verification() > generation);
                    generation
                };
                crate::file_activity::TOOL_ACTOR
                    .scope(
                        chat_id.clone(),
                        binding.scope(async {
                            let unchanged =
                                TaskGoalSessionEpoch::state(&*restored.lock().await).unwrap();
                            assert!(hydrate_session_task_goal(app.gcx.clone(), &restored)
                                .await
                                .unwrap_err()
                                .contains("verification generation superseded"));
                            assert_eq!(
                                TaskGoalSessionEpoch::state(&*restored.lock().await).unwrap(),
                                unchanged
                            );
                            assert_eq!(tokio::fs::read(&goal_path).await.unwrap(), bytes);
                        }),
                    )
                    .await;
                restored.lock().await.goal_verification_generation = generation;
            }
            crate::file_activity::TOOL_ACTOR
                .scope(
                    chat_id.clone(),
                    binding.scope(async {
                        hydrate_session_task_goal(app.gcx.clone(), &restored)
                            .await
                            .unwrap();
                        commit_session_task_goal(app.gcx.clone(), &restored)
                            .await
                            .unwrap();
                    }),
                )
                .await;
            assert_eq!(tokio::fs::read(&goal_path).await.unwrap(), bytes);
            let session = restored.lock().await;
            assert!(session.task_goal_partial_write.is_none());
            assert_eq!(
                serde_json::to_value(&session.goal).unwrap(),
                serde_json::to_value(own.snapshot(&task_id)).unwrap()
            );
            assert_eq!(session.goal_ledger, own.ledger);
            assert_eq!(serde_json::to_value(&session.messages).unwrap(), messages);
            assert_eq!(serde_json::to_value(&session.command_queue).unwrap(), queue);
            assert_eq!(session.trajectory_version, version);
        }
        {
            let mut session = restored.lock().await;
            assert!(
                session.goal_set_status_reason(GoalStatus::Paused, "restore existing hold stage")
            );
        }
        crate::file_activity::TOOL_ACTOR
            .scope(
                chat_id.clone(),
                binding.scope(commit_session_task_goal(app.gcx.clone(), &restored)),
            )
            .await
            .unwrap();
        let before = tokio::fs::read(&goal_path).await.unwrap();
        let canonical: TaskGoalRecord = serde_json::from_slice(&before).unwrap();
        {
            let mut session = restored.lock().await;
            session.add_message(crate::chat::internal_roles::goal_delta(
                "task_planner",
                serde_json::json!({"seq": 2}),
                "accepted update before pending writer capture",
            ));
            session.goal_push_event(refact_chat_api::GoalEvent {
                kind: "accepted_update".into(),
                at_ms: 42,
                text: "retain distinct accepted ledger".into(),
            });
            let projection = session.try_accepted_control_projection().unwrap();
            assert_eq!(session.goal.as_ref().unwrap().revision, canonical.revision);
            assert_ne!(
                serde_json::to_value(&projection.messages).unwrap(),
                serde_json::to_value(&canonical.messages).unwrap()
            );
            assert_ne!(projection.goal_ledger, canonical.ledger);
        }
        let (_, guard) = super::super::storage::locked_task_dir(app.gcx.clone(), &task_id)
            .await
            .unwrap();
        let waiting = Arc::new(tokio::sync::Notify::new());
        let writer = tokio::spawn({
            let waiting = waiting.clone();
            let binding = binding.clone();
            let gcx = app.gcx.clone();
            let restored = restored.clone();
            let chat_id = chat_id.clone();
            async move {
                super::super::storage::TASK_BOARD_LOCK_WAIT
                    .scope(
                        waiting,
                        crate::file_activity::TOOL_ACTOR.scope(
                            chat_id,
                            binding.scope(commit_session_task_goal(gcx, &restored)),
                        ),
                    )
                    .await
            }
        });
        tokio::time::timeout(std::time::Duration::from_secs(60), waiting.notified())
            .await
            .unwrap();
        assert!(!writer.is_finished());
        let held = {
            let mut session = restored
                .try_lock()
                .expect("pending writer must not hold session");
            session.ensure_goal_ledger_installed();
            assert!(session.goal_set_status_reason(GoalStatus::Stopped, "new user hold"));
            TaskGoalSessionEpoch::state(&session).unwrap()
        };
        drop(guard);
        let error = tokio::time::timeout(std::time::Duration::from_secs(60), writer)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(error, "Task goal session changed while awaiting persistence; retry; session rollback not applied: Task goal session changed while awaiting persistence; retry");
        assert!(!error.contains("persisted;"));
        assert!(restored.lock().await.task_goal_partial_write.is_none());
        assert_eq!(tokio::fs::read(&goal_path).await.unwrap(), before);
        assert_eq!(
            TaskGoalSessionEpoch::state(&*restored.lock().await).unwrap(),
            held
        );
        crate::file_activity::TOOL_ACTOR
            .scope(
                chat_id.clone(),
                binding.scope(async {
                    hydrate_session_task_goal(app.gcx.clone(), &restored)
                        .await
                        .unwrap();
                    commit_session_task_goal(app.gcx.clone(), &restored)
                        .await
                        .unwrap();
                    hydrate_session_task_goal(app.gcx.clone(), &restored)
                        .await
                        .unwrap();
                    assert_eq!(
                        restored.lock().await.goal.as_ref().unwrap().status,
                        GoalStatus::Stopped
                    );
                    assert_eq!(
                        resolve_task_goal(app.gcx.clone(), &task_id)
                            .await
                            .unwrap()
                            .unwrap()
                            .snapshot(&task_id)
                            .unwrap()
                            .status,
                        GoalStatus::Stopped
                    );
                }),
            )
            .await;
        let pending = Arc::new(tokio::sync::Notify::new());
        let resume = Arc::new(tokio::sync::Notify::new());
        let prior = read_record(&goal_path).await.unwrap().unwrap();
        let accepted = {
            let mut session = restored.lock().await;
            session.add_message(crate::chat::internal_roles::goal_delta(
                "task_planner",
                serde_json::json!({"seq": 3}),
                "accepted content must survive own partial write",
            ));
            session.try_accepted_control_projection().unwrap()
        };
        assert_ne!(
            serde_json::to_value(&accepted.messages).unwrap(),
            serde_json::to_value(&prior.messages).unwrap()
        );
        let writer = tokio::spawn({
            let binding = binding.clone();
            let gcx = app.gcx.clone();
            let restored = restored.clone();
            let chat_id = chat_id.clone();
            let pause = (pending.clone(), resume.clone());
            async move {
                TASK_GOAL_WRITE_PAUSE
                    .scope(
                        pause,
                        crate::file_activity::TOOL_ACTOR.scope(
                            chat_id,
                            binding.scope(commit_session_task_goal(gcx, &restored)),
                        ),
                    )
                    .await
            }
        });
        tokio::time::timeout(std::time::Duration::from_secs(60), pending.notified())
            .await
            .unwrap();
        assert!(!writer.is_finished());
        let (held, current_queue) = {
            let mut session = restored
                .try_lock()
                .expect("actual pending durable write must release session");
            session
                .command_queue
                .push_back(refact_chat_api::CommandRequest {
                    native_approval_guard: None,
                    supervision_guard: None,
                    client_request_id: "latest-user-regenerate".into(),
                    priority: true,
                    command: refact_chat_api::ChatCommand::Regenerate {},
                });
            session.purge_goal_generated_wakes();
            assert!(session.goal_set_status_reason(
                GoalStatus::Paused,
                "latest user pause during durable write"
            ));
            session.add_message(crate::chat::internal_roles::event(
                crate::chat::internal_roles::EventSubkind::GoalPursuit,
                "tool.goal_pursuit",
                serde_json::json!({"kind":"paused", "trigger":"pause", "at_ms":43}),
                "Goal pursuit paused. Latest user hold.",
            ));
            session.add_message(crate::chat::internal_roles::event(
                crate::chat::internal_roles::EventSubkind::GoalPursuit,
                "chat.goal_monitor",
                serde_json::json!({"kind":"pursuit_quiescent", "trigger":"monitor", "at_ms":44}),
                "Goal pursuit paused: agent idle with no progress; will resume on your next message.",
            ));
            assert!(session
                .task_goal_partial_write
                .as_ref()
                .unwrap()
                .outcome
                .get()
                .is_none());
            (
                TaskGoalSessionEpoch::state(&session).unwrap(),
                serde_json::to_value(&session.command_queue).unwrap(),
            )
        };
        resume.notify_one();
        let error = tokio::time::timeout(std::time::Duration::from_secs(60), writer)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(error, format!("Task goal revision {} persisted; session projection not applied: Task goal session changed while awaiting persistence; retry", prior.revision + 1));
        assert_eq!(
            TaskGoalSessionEpoch::state(&*restored.lock().await).unwrap(),
            held
        );
        let partial = read_record(&goal_path).await.unwrap().unwrap();
        assert_eq!(partial.revision, prior.revision + 1);
        assert_eq!(
            partial.snapshot(&task_id).unwrap().status,
            GoalStatus::Stopped
        );
        assert_eq!(
            serde_json::to_value(&partial.messages).unwrap(),
            serde_json::to_value(&accepted.messages).unwrap()
        );
        assert_eq!(partial.ledger, accepted.goal_ledger);
        {
            let session = restored.lock().await;
            assert_eq!(session.goal.as_ref().unwrap().revision, prior.revision);
            assert!(matches!(
                session
                    .task_goal_partial_write
                    .as_ref()
                    .unwrap()
                    .outcome
                    .get(),
                Some(Ok(()))
            ));
        }
        crate::file_activity::TOOL_ACTOR
            .scope(
                chat_id.clone(),
                binding.scope(async {
                    hydrate_session_task_goal(app.gcx.clone(), &restored)
                        .await
                        .unwrap();
                    commit_session_task_goal(app.gcx.clone(), &restored)
                        .await
                        .unwrap();
                    hydrate_session_task_goal(app.gcx.clone(), &restored)
                        .await
                        .unwrap();
                }),
            )
            .await;
        let reconciled = read_record(&goal_path).await.unwrap().unwrap();
        assert_eq!(reconciled.revision, partial.revision + 1);
        assert_eq!(
            reconciled.snapshot(&task_id).unwrap().status,
            GoalStatus::Paused
        );
        assert_eq!(
            serde_json::to_value(&reconciled.messages[..partial.messages.len()]).unwrap(),
            serde_json::to_value(&partial.messages).unwrap()
        );
        assert_eq!(reconciled.messages.len(), partial.messages.len() + 2);
        assert!(reconciled.ledger.starts_with(&partial.ledger));
        {
            let session = restored.lock().await;
            assert!(session.task_goal_partial_write.is_none());
            assert_eq!(
                serde_json::to_value(&session.command_queue).unwrap(),
                current_queue
            );
            assert_eq!(
                serde_json::to_value(&session.goal).unwrap(),
                serde_json::to_value(reconciled.snapshot(&task_id)).unwrap()
            );
            assert_eq!(session.goal_ledger, reconciled.ledger);
            assert_eq!(session.goal.as_ref().unwrap().budget.max_turns, Some(7));
            assert_eq!(
                session.goal.as_ref().unwrap().budget.max_tokens,
                Some(12000)
            );
            assert_eq!(
                serde_json::to_value(&session.messages[..2]).unwrap(),
                archive
            );
        }
        {
            let mut session = restored.lock().await;
            session.add_message(crate::chat::internal_roles::goal_delta(
                "task_planner",
                serde_json::json!({"seq": 4}),
                "next own accepted update",
            ));
        }
        let pending = Arc::new(tokio::sync::Notify::new());
        let resume = Arc::new(tokio::sync::Notify::new());
        let writer = tokio::spawn({
            let binding = binding.clone();
            let gcx = app.gcx.clone();
            let restored = restored.clone();
            let chat_id = chat_id.clone();
            let pause = (pending.clone(), resume.clone());
            async move {
                TASK_GOAL_WRITE_PAUSE
                    .scope(
                        pause,
                        crate::file_activity::TOOL_ACTOR.scope(
                            chat_id,
                            binding.scope(commit_session_task_goal(gcx, &restored)),
                        ),
                    )
                    .await
            }
        });
        tokio::time::timeout(std::time::Duration::from_secs(60), pending.notified())
            .await
            .unwrap();
        assert!(!writer.is_finished());
        {
            let mut session = restored
                .try_lock()
                .expect("competing write stage must release session");
            session.purge_goal_generated_wakes();
            assert!(session.goal_set_status_reason(GoalStatus::Stopped, "goal_control"));
            session.add_message(crate::chat::internal_roles::event(
                crate::chat::internal_roles::EventSubkind::GoalPursuit,
                "chat.command.goal_control",
                serde_json::json!({"kind":"stopped", "trigger":"goal_control", "at_ms":43}),
                "Goal pursuit stopped by user.",
            ));
        }
        resume.notify_one();
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(60), writer)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err()
                .contains("persisted; session projection not applied")
        );
        let own = read_record(&goal_path).await.unwrap().unwrap();
        assert_eq!(own.revision, reconciled.revision + 1);
        let held = {
            let mut session = restored.lock().await;
            let mut budget = session.goal.as_ref().unwrap().budget.clone();
            budget.max_turns = Some(2);
            let controls = session.task_goal_messages.as_mut().unwrap();
            let base = controls
                .iter_mut()
                .find(|message| message.role == "goal")
                .unwrap();
            base.extra["goal"]["budget"] = serde_json::to_value(&budget).unwrap();
            session.rebuild_goal_projection_from_messages();
            session.goal_ledger_append(refact_chat_api::GoalLedgerOp::BudgetSet {
                budget,
            });
            TaskGoalSessionEpoch::state(&session).unwrap()
        };
        let own_bytes = tokio::fs::read(&goal_path).await.unwrap();
        crate::file_activity::TOOL_ACTOR
            .scope(
                chat_id.clone(),
                binding.scope(async {
                    assert!(hydrate_session_task_goal(app.gcx.clone(), &restored)
                        .await
                        .unwrap_err()
                        .contains("control content/ledger superseded"));
                    assert_eq!(tokio::fs::read(&goal_path).await.unwrap(), own_bytes);
                    assert_eq!(
                        TaskGoalSessionEpoch::state(&*restored.lock().await).unwrap(),
                        held
                    );
                    assert_eq!(
                        restored
                            .lock()
                            .await
                            .goal
                            .as_ref()
                            .unwrap()
                            .budget
                            .max_turns,
                        Some(2)
                    );
                }),
            )
            .await;
        crate::file_activity::TOOL_ACTOR
            .scope(
                chat_id.clone(),
                binding.scope(async {
                    let mut competing = own.messages.clone();
                    competing.push(crate::chat::internal_roles::goal_delta(
                        "task_planner",
                        serde_json::json!({"seq": 5}),
                        "new canonical content must not be overwritten",
                    ));
                    let competing = commit_task_goal(
                        app.gcx.clone(),
                        &task_id,
                        &chat_id,
                        own.revision,
                        competing,
                        own.ledger.clone(),
                    )
                    .await
                    .unwrap();
                    assert_eq!(competing.revision, own.revision + 1);
                    let bytes = tokio::fs::read(&goal_path).await.unwrap();
                    assert!(hydrate_session_task_goal(app.gcx.clone(), &restored)
                        .await
                        .unwrap_err()
                        .contains("canonical revision/content superseded"));
                    assert!(commit_session_task_goal(app.gcx.clone(), &restored)
                        .await
                        .unwrap_err()
                        .contains("canonical revision/content superseded"));
                    assert_eq!(tokio::fs::read(&goal_path).await.unwrap(), bytes);
                    assert_eq!(
                        TaskGoalSessionEpoch::state(&*restored.lock().await).unwrap(),
                        held
                    );
                    assert!(restored.lock().await.task_goal_partial_write.is_some());
                }),
            )
            .await;
        let foreign = temp.path().join("foreign-source");
        tokio::fs::create_dir(&foreign).await.unwrap();
        *app.gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![foreign.clone(), temp.path().into()];
        restored.lock().await.work_project_root = Some(foreign.clone());
        let foreign_state = TaskGoalSessionEpoch::state(&*restored.lock().await).unwrap();
        let bytes = tokio::fs::read(&goal_path).await.unwrap();
        assert!(hydrate_session_task_goal(app.gcx.clone(), &restored)
            .await
            .unwrap_err()
            .contains("authenticated source/task domain"));
        assert_eq!(tokio::fs::read(&goal_path).await.unwrap(), bytes);
        assert_eq!(
            TaskGoalSessionEpoch::state(&*restored.lock().await).unwrap(),
            foreign_state
        );
        assert!(!foreign.join(".refact/tasks").join(&task_id).exists());
    }

    #[tokio::test]
    async fn replay_recovery_keeps_nonowner_readable_and_refuses_foreign_duplicate() {
        let (temp, app, task_id, owner) = task_fixture().await;
        let binding = super::super::storage::TaskDirectoryBinding::capture(
            app.gcx.clone(),
            temp.path(),
            &task_id,
        )
        .await
        .unwrap();
        let dir = super::super::storage::find_task_dir(app.gcx.clone(), &task_id)
            .await
            .unwrap();
        let planner_dir = dir.join("trajectories/planner");
        tokio::fs::create_dir_all(&planner_dir).await.unwrap();
        let (_, legacy_controls) = replayed_session(&owner);
        let legacy = serde_json::to_vec(&serde_json::json!({
            "chat_id": owner,
            "messages": legacy_controls,
            "goal_ledger": []
        }))
        .unwrap();
        let legacy_path = planner_dir.join(format!("{owner}.json"));
        tokio::fs::write(&legacy_path, &legacy).await.unwrap();
        let meta_bytes = tokio::fs::read(dir.join("meta.yaml")).await.unwrap();
        let board_before = tokio::fs::read(dir.join("board.yaml")).await.unwrap();
        let foreign = temp.path().join("foreign-source");
        tokio::fs::create_dir(&foreign).await.unwrap();
        *app.gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![foreign.clone(), temp.path().into()];
        let mut refused = ChatSession::new(owner.clone());
        refused.thread.task_meta = Some(TaskMeta {
            task_id: task_id.clone(),
            role: "planner".into(),
            planner_chat_id: Some(owner.clone()),
            ..Default::default()
        });
        refused.thread.model = "test/model".into();
        refused.work_project_root = Some(foreign.clone());
        for message in &legacy_controls {
            refused.add_message(message.clone());
        }
        let refused = Arc::new(Mutex::new(refused));
        app.chat
            .sessions
            .write()
            .await
            .insert(owner.clone(), refused.clone());
        assert_eq!(
            super::super::storage::find_task_dir(app.gcx.clone(), &task_id)
                .await
                .unwrap(),
            dir
        );
        assert_eq!(
            super::super::storage::load_board(app.gcx.clone(), &task_id)
                .await
                .unwrap()
                .owner_chat_id
                .as_deref(),
            Some(owner.as_str())
        );
        assert!(!dir.join("goal.json").exists());
        assert!(refused.lock().await.task_goal_partial_write.is_none());
        let state = TaskGoalSessionEpoch::state(&*refused.lock().await).unwrap();
        let queue = serde_json::to_value(&refused.lock().await.command_queue).unwrap();
        let error = commit_session_task_goal(app.gcx.clone(), &refused)
            .await
            .unwrap_err();
        assert_eq!(error, "Task goal lock does not belong to its authenticated source/task domain; session rollback not applied: Task goal lock does not belong to its authenticated source/task domain");
        assert!(
            !dir.join("goal.json").exists(),
            "refused commit rollback must not migrate legacy data"
        );
        assert!(!foreign.join(".refact/tasks").join(&task_id).exists());
        assert_eq!(tokio::fs::read(&legacy_path).await.unwrap(), legacy);
        assert_eq!(
            tokio::fs::read(dir.join("meta.yaml")).await.unwrap(),
            meta_bytes
        );
        assert_eq!(
            tokio::fs::read(dir.join("board.yaml")).await.unwrap(),
            board_before
        );
        assert_eq!(
            TaskGoalSessionEpoch::state(&*refused.lock().await).unwrap(),
            state
        );
        assert_eq!(
            serde_json::to_value(&refused.lock().await.command_queue).unwrap(),
            queue
        );
        assert!(refused.lock().await.task_goal_partial_write.is_none());
        refused.lock().await.work_project_root = Some(temp.path().canonicalize().unwrap());
        hydrate_session_task_goal(app.gcx.clone(), &refused)
            .await
            .unwrap();
        let migrated = read_record(&dir.join("goal.json")).await.unwrap().unwrap();
        assert_eq!(migrated.revision, 1);
        assert_eq!(
            serde_json::to_value(&migrated.messages).unwrap(),
            serde_json::to_value(&legacy_controls).unwrap()
        );
        assert_eq!(
            migrated.snapshot(&task_id).unwrap().status,
            GoalStatus::Paused
        );
        assert_eq!(
            migrated.snapshot(&task_id).unwrap().budget.max_turns,
            Some(7)
        );
        assert_eq!(
            refused.lock().await.goal.as_ref().unwrap().revision,
            migrated.revision
        );
        assert_eq!(tokio::fs::read(&legacy_path).await.unwrap(), legacy);
        assert_eq!(
            tokio::fs::read(dir.join("meta.yaml")).await.unwrap(),
            meta_bytes
        );
        assert_eq!(
            tokio::fs::read(dir.join("board.yaml")).await.unwrap(),
            board_before
        );
        app.chat.sessions.write().await.remove(&owner);
        let (mut session, controls) = replayed_session("old-planner");
        session.thread.task_meta = Some(TaskMeta {
            task_id: task_id.clone(),
            role: "planner".into(),
            planner_chat_id: Some("old-planner".into()),
            ..Default::default()
        });
        commit_task_goal(
            app.gcx.clone(),
            &task_id,
            &owner,
            migrated.revision,
            controls.clone(),
            vec![],
        )
        .await
        .unwrap();
        let original = serde_json::to_value(&session.messages).unwrap();
        let session = Arc::new(Mutex::new(session));
        let dir = super::super::storage::find_task_dir(app.gcx.clone(), &task_id)
            .await
            .unwrap();
        let goal_bytes = tokio::fs::read(dir.join("goal.json")).await.unwrap();
        let board_bytes = tokio::fs::read(dir.join("board.yaml")).await.unwrap();
        binding
            .scope(hydrate_session_task_goal(app.gcx.clone(), &session))
            .await
            .unwrap();
        assert_eq!(
            tokio::fs::read(dir.join("goal.json")).await.unwrap(),
            goal_bytes
        );
        assert_eq!(
            tokio::fs::read(dir.join("board.yaml")).await.unwrap(),
            board_bytes
        );
        let mut changed = controls.clone();
        changed[0].content =
            crate::call_validation::ChatContent::SimpleText("stale replacement".into());
        assert!(binding
            .scope(commit_task_goal(
                app.gcx.clone(),
                &task_id,
                &owner,
                0,
                changed,
                vec![],
            ))
            .await
            .unwrap_err()
            .contains("revision conflict"));
        assert_eq!(
            tokio::fs::read(dir.join("goal.json")).await.unwrap(),
            goal_bytes
        );
        assert_eq!(
            serde_json::to_value(&session.lock().await.messages).unwrap(),
            original
        );
        let (mut stale, _) = replayed_session(&owner);
        stale.thread.task_meta = Some(TaskMeta {
            task_id: task_id.clone(),
            role: "planner".into(),
            planner_chat_id: Some(owner.clone()),
            ..Default::default()
        });
        let stale = Arc::new(Mutex::new(stale));
        let stale_state = TaskGoalSessionEpoch::state(&*stale.lock().await).unwrap();
        super::super::storage::update_board_atomic(app.gcx.clone(), &task_id, |board| {
            board.owner_chat_id = Some("replacement-planner".into());
            Ok(())
        })
        .await
        .unwrap();
        let replaced_board = tokio::fs::read(dir.join("board.yaml")).await.unwrap();
        assert_ne!(replaced_board, board_bytes);
        assert!(binding
            .scope(hydrate_session_task_goal(app.gcx.clone(), &stale))
            .await
            .unwrap_err()
            .contains("Accepted task owner changed"));
        assert_eq!(
            tokio::fs::read(dir.join("board.yaml")).await.unwrap(),
            replaced_board
        );
        assert_eq!(
            tokio::fs::read(dir.join("goal.json")).await.unwrap(),
            goal_bytes
        );
        assert_eq!(
            TaskGoalSessionEpoch::state(&*stale.lock().await).unwrap(),
            stale_state
        );
        let (mut corrupted, corrupted_controls) = replayed_session(&owner);
        corrupted.messages[1].role = "user".into();
        let original = serde_json::to_value(&corrupted.messages).unwrap();
        assert!(!recover_replayed_controls(&mut corrupted, &corrupted_controls).unwrap());
        assert!(active_context(&corrupted.messages).is_err());
        assert_eq!(serde_json::to_value(&corrupted.messages).unwrap(), original);
        let (mut conflicting, conflicting_controls) = replayed_session(&owner);
        conflicting.messages[1].content =
            crate::call_validation::ChatContent::SimpleText("conflicting outcome".into());
        let original = serde_json::to_value(&conflicting.messages).unwrap();
        assert!(!recover_replayed_controls(&mut conflicting, &conflicting_controls).unwrap());
        assert!(active_context(&conflicting.messages).is_err());
        assert_eq!(
            serde_json::to_value(&conflicting.messages).unwrap(),
            original
        );
    }
}
