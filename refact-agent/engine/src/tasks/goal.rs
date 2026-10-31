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
    let existing = if session_epoch.is_some_and(|epoch| epoch.partial_write.is_some()) {
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
    let expected_revision = match session_epoch.and_then(|epoch| epoch.partial_write.as_ref()) {
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

pub async fn hydrate_session_task_goal(
    gcx: Arc<GlobalContext>,
    session_arc: &Arc<tokio::sync::Mutex<crate::chat::types::ChatSession>>,
) -> Result<(), String> {
    let reconcile = session_arc.lock().await.task_goal_partial_write.is_some();
    if reconcile {
        commit_session_task_goal(gcx.clone(), session_arc).await?;
    }
    let Some(epoch) = TaskGoalSessionEpoch::capture(&gcx, session_arc).await? else {
        return Ok(());
    };
    let task_id = &epoch.task_meta.task_id;
    let (_, guard) = super::storage::locked_task_dir(gcx.clone(), task_id).await?;
    super::storage::load_board_locked(gcx.clone(), task_id, &guard)
        .await?
        .require_owner(&epoch.chat_id)?;
    epoch.validate_task_domain(&gcx, &guard).await?;
    let record = resolve_task_goal_locked(gcx.clone(), task_id, &guard).await?;
    super::storage::load_board_locked(gcx.clone(), task_id, &guard)
        .await?
        .require_owner(&epoch.chat_id)?;
    epoch.validate_task_domain(&gcx, &guard).await?;
    let sessions = gcx.chat_sessions.read().await;
    epoch.validate_registry(&sessions)?;
    let mut session = epoch.lock_current()?;
    if let Some(record) = record {
        let snapshot = record.snapshot(task_id);
        if session.goal.as_ref().is_some_and(|goal| {
            !matches!(
                goal.status,
                refact_chat_api::GoalStatus::Active | refact_chat_api::GoalStatus::Verifying
            ) && snapshot.as_ref().is_some_and(|canonical| {
                canonical.status != goal.status || (!goal.active && canonical.active)
            })
        }) {
            return Err(
                "Task goal hydration would replace a held session goal; retry persistence".into(),
            );
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
            return Ok(());
        }
        session.set_goal_projection(snapshot);
        session.set_goal_ledger(record.ledger);
        session.task_goal_messages = Some(record.messages);
        session.emit_goal_status();
    } else if let Some(goal) = session.goal.as_mut() {
        goal.owner = Some(GoalOwner::Task {
            id: task_id.clone(),
        });
        goal.revision = 0;
    }
    Ok(())
}

pub(crate) struct TaskGoalPartialWrite {
    session: std::sync::Weak<tokio::sync::Mutex<crate::chat::types::ChatSession>>,
    incarnation: std::sync::Weak<std::sync::atomic::AtomicBool>,
    identity: serde_json::Value,
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
            if serde_json::to_value(Some(accepted)).map_err(|error| error.to_string())?
                != serde_json::to_value(&epoch.goal).map_err(|error| error.to_string())?
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
                    message.role != "event"
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
                        )
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
                refact_chat_api::GoalLedgerOp::StatusChanged { from, to, reason }
                    if *from == held.status
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
        prior.status = goal.status;
        prior.stop_reason = goal.stop_reason.clone();
        prior.snoozed_until_ms = goal.snoozed_until_ms;
        let prior = crate::chat::goal_snapshot_from_messages(&proposed.messages, Some(&prior))
            .ok_or("Task goal partial write accepted goal unavailable")?;
        if serde_json::to_value(prior).map_err(|error| error.to_string())?
            != serde_json::to_value(goal).map_err(|error| error.to_string())?
        {
            return Err(
                "Task goal partial write goal/budget superseded; inspect before retry".into(),
            );
        }
        Ok(self.record.revision)
    }
}

struct TaskGoalSessionEpoch {
    session: Arc<tokio::sync::Mutex<crate::chat::types::ChatSession>>,
    incarnation: std::sync::Weak<std::sync::atomic::AtomicBool>,
    registered: bool,
    chat_id: String,
    task_meta: refact_chat_api::TaskMeta,
    identity: serde_json::Value,
    goal: Option<GoalSnapshot>,
    source_root: Option<std::path::PathBuf>,
    verification_generation: u64,
    partial_write: Option<Arc<TaskGoalPartialWrite>>,
    operation: std::sync::OnceLock<Arc<TaskGoalPartialWrite>>,
    state: serde_json::Value,
}

impl TaskGoalSessionEpoch {
    fn identity(session: &crate::chat::types::ChatSession) -> Result<serde_json::Value, String> {
        serde_json::to_value((
            &session.chat_id,
            &session.thread,
            &session.work_project_root,
            &session.created_at,
            session.closed,
            session
                .closed_flag
                .load(std::sync::atomic::Ordering::Relaxed),
            session.board_restored_pending_tools,
        ))
        .map_err(|error| error.to_string())
    }

    fn state(session: &crate::chat::types::ChatSession) -> Result<serde_json::Value, String> {
        serde_json::to_value((
            session.trajectory_version,
            session.goal_verification_generation,
            session.planner_pause_revision,
            session
                .planner_pause
                .as_ref()
                .map(|pause| (&pause.since, &pause.by)),
            &session.thread,
            &session.work_project_root,
            &session.goal,
            &session.goal_ledger,
            &session.messages,
            &session.task_goal_messages,
            &session.post_tool_side_effects,
            &session.pending_deliveries,
        ))
        .map_err(|error| error.to_string())
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
        let session = self
            .session
            .try_lock()
            .map_err(|_| "Task goal session is busy; retry")?;
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
                Some(&epoch),
            )
            .await
        }
        Err(error) => Err(error.clone()),
    };
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
            .map_err(|error| match &result {
                Ok(record) => format!(
                    "Task goal revision {} persisted; session projection not applied: {error}",
                    record.revision
                ),
                Err(commit_error) => {
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
            if let Some(goal) = session.goal.as_mut() {
                goal.active = false;
                goal.status = refact_chat_api::GoalStatus::Paused;
                goal.stop_reason = Some(error.clone());
            }
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
        .await;
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
                    assert!(hydrate_session_task_goal(app.gcx.clone(), &restored)
                        .await
                        .unwrap_err()
                        .contains("held session goal"));
                    assert_eq!(
                        TaskGoalSessionEpoch::state(&*restored.lock().await).unwrap(),
                        held
                    );
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
        assert_eq!(reconciled.messages.len(), partial.messages.len() + 1);
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
            session.goal_ledger_append(refact_chat_api::GoalLedgerOp::BudgetSet { budget });
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
    async fn replay_recovery_refuses_nonowner_and_foreign_duplicate_without_mutation() {
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
        assert!(binding
            .scope(hydrate_session_task_goal(app.gcx.clone(), &session))
            .await
            .unwrap_err()
            .contains("owning planner"));
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
