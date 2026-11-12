//! Unified message delivery into a chat.
//!
//! Every producer (planner, cron, hooks, background runner, tools) reaches a
//! chat through [`deliver_to_chat`]. The `push` mode on the delivery decides
//! where it lands relative to an in-flight turn:
//!
//! * `preempt` — abort the draft, record a cancellation note, land now.
//! * `append` (default) — at the next safe message boundary, before the next
//!   generation, never inside an open assistant/tool-result window.
//! * `when_idle` — only after the whole turn (including the multi-step tool
//!   loop) has finished.
//!
//! Session deliveries are persisted in the durable outbox before they are
//! accepted, so transport is at-least-once across a restart. The recipient
//! deduplicates by delivery id, which makes insertion logically exactly-once;
//! it is not a claim about external side effects. When no usable outbox exists
//! the delivery still happens, in memory only, and the lost durability is
//! reported rather than assumed. See [`crate::coordination::outbox`].

use std::sync::Arc;
use std::sync::atomic::Ordering;

use tokio::sync::Mutex as AMutex;

use crate::app_state::AppState;
use crate::chat::queue::process_command_queue;
use crate::chat::trajectories::try_save_trajectory_with_intent;
use crate::chat::types::{
    ChatSession, DeliveryOutcome, DeliveryStatus, PendingDelivery, PushMode, SessionState,
    TrajectoryCommitIntent,
};
use crate::coordination::outbox;

#[cfg(test)]
tokio::task_local! {
    pub(crate) static BEFORE_REPLY_ACCEPT: (Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>);
}

/// Deliver a batch of messages to `chat_id`.
///
/// Restores the target session from its trajectory when needed and rejects
/// closed or unavailable chats honestly instead of silently dropping messages.
/// A background-agent runner that owns this chat gets the delivery first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrajectoryPersistence {
    Committed,
    Pending,
    Unknown,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DeliveryReceipt {
    pub outcome: DeliveryOutcome,
    pub delivery_id: String,
    pub trajectory_persistence: TrajectoryPersistence,
    pub outbox_durable: bool,
    pub note: Option<String>,
}

pub async fn deliver_to_chat(
    app: AppState,
    chat_id: &str,
    delivery: PendingDelivery,
) -> Result<DeliveryOutcome, String> {
    deliver_to_chat_with_receipt(app, chat_id, delivery)
        .await
        .map(|receipt| receipt.outcome)
}

pub(crate) async fn deliver_to_open_chat_with_receipt(
    app: AppState,
    chat_id: &str,
    delivery: PendingDelivery,
) -> Result<DeliveryReceipt, String> {
    let session = app
        .chat
        .sessions
        .read()
        .await
        .get(chat_id)
        .cloned()
        .ok_or("chat unavailable")?;
    if session.lock().await.closed {
        return Err("chat closed".into());
    }
    deliver_to_chat_internal(app, chat_id, delivery, Some(session)).await
}

pub async fn deliver_to_chat_with_receipt(
    app: AppState,
    chat_id: &str,
    delivery: PendingDelivery,
) -> Result<DeliveryReceipt, String> {
    deliver_to_chat_internal(app, chat_id, delivery, None).await
}

async fn deliver_to_chat_internal(
    app: AppState,
    chat_id: &str,
    delivery: PendingDelivery,
    open_session: Option<Arc<AMutex<ChatSession>>>,
) -> Result<DeliveryReceipt, String> {
    if delivery.deferred_job.is_some() && open_session.is_none() {
        return Err("deferred tool job delivery requires an already open chat".into());
    }
    if delivery.messages.is_empty() && delivery.deferred_job.is_none() {
        return Err("delivery must contain at least one message".to_string());
    }

    if let Some(patch) = &delivery.thread_patch {
        let fields = patch
            .as_object()
            .ok_or("delivery thread_patch must be an object")?;
        if fields
            .iter()
            .any(|(key, value)| !matches!(key.as_str(), "mode" | "model") || !value.is_string())
        {
            return Err("delivery thread_patch supports only string mode and model fields".into());
        }
    }

    let planner_reply = crate::tools::tool_agent_planner_qna::is_planner_reply_delivery(&delivery);
    if open_session.is_none()
        && !planner_reply
        && (!delivery.wake || !crate::agents::seal::is_sealed(&app, chat_id).await)
    {
        if let Some(result) =
            crate::agents::delivery::route_to_runner(&app, chat_id, delivery.clone()).await
        {
            return result.map(|outcome| DeliveryReceipt {
                outcome,
                delivery_id: delivery.id.clone(),
                trajectory_persistence: TrajectoryPersistence::Unknown,
                outbox_durable: false,
                note: None,
            });
        }
    }

    let sessions = app.chat.sessions.clone();
    let existing_session = { sessions.read().await.get(chat_id).cloned() };
    if let Some(session) = existing_session {
        if session.lock().await.closed {
            return Err(format!("chat '{chat_id}' is closed"));
        }
    }
    if open_session.is_none() {
        match crate::chat::session_runtime::restore_session_if_trajectory_exists(
            app.clone(),
            &sessions,
            chat_id,
        )
        .await
        {
            crate::chat::session_runtime::SessionRestore::Open => {}
            crate::chat::session_runtime::SessionRestore::NoTrajectory => {
                return Err(format!("chat '{chat_id}' does not exist"));
            }
            crate::chat::session_runtime::SessionRestore::Refused(error) => {
                return Err(format!("chat '{chat_id}' cannot be opened now: {error}"));
            }
        }
    }
    let session_arc = match open_session {
        Some(session) => {
            let current = sessions
                .read()
                .await
                .get(chat_id)
                .cloned()
                .ok_or("chat unavailable")?;
            if !Arc::ptr_eq(&session, &current) || session.lock().await.closed {
                return Err("chat lifecycle changed".into());
            }
            session
        }
        None => sessions
            .read()
            .await
            .get(chat_id)
            .cloned()
            .ok_or_else(|| format!("chat '{chat_id}' is unavailable"))?,
    };

    // Durability first, and with no session lock held across store I/O. A store
    // this engine cannot use costs the message its durability, not its
    // delivery: the outbox drops out, the loss is reported, and the rest of the
    // path is the in-memory one that ran before the outbox existed.
    let source_root = crate::coordination::scope::session_source_root(&*session_arc.lock().await);
    let project = outbox::project_key_for_root(&app, source_root.clone()).await;
    let delivery_id = delivery.id.clone();
    let causal_event = crate::coordination::causal::delivery_event(chat_id, &delivery);
    let outbox = match outbox::outbox_for(&app, project).await {
        Some(outbox) if outbox.accept_for_dispatch(chat_id, &delivery).await => Some(outbox),
        _ => None,
    };

    let sealed = delivery.wake && crate::agents::seal::is_sealed(&app, chat_id).await;
    let enqueue = |session: &mut ChatSession, delivery: PendingDelivery| {
        let wake = delivery.wake
            && !sealed
            && session.runtime.interruption.is_none()
            && session.runtime.state != SessionState::Interrupted;
        let accepted = if session.closed {
            Err(format!("chat '{chat_id}' is closed"))
        } else if sealed {
            Ok(session.drop_sealed_delivery(delivery.id))
        } else {
            session.enqueue_delivery(delivery)
        };
        let status = session.delivery_status(&delivery_id);
        // Only a landed delivery can need a restart; queued ones will be drained
        // at their boundary by the turn that is already running.
        let should_start_processor =
            wake && accepted.as_ref().is_ok_and(|outcome| {
                *outcome == DeliveryOutcome::Delivered
                    || (*outcome == DeliveryOutcome::Duplicate
                        && !session.delivery_wake_sources.is_empty())
            }) && matches!(
                session.runtime.state,
                SessionState::Idle | SessionState::Completed | SessionState::WaitingUserInput
            );
        (
            accepted,
            status,
            wake,
            should_start_processor,
            session.queue_processor_running.clone(),
            session.queue_notify.clone(),
        )
    };

    let accepted_reply = if planner_reply {
        #[cfg(test)]
        if let Ok(barrier) = BEFORE_REPLY_ACCEPT.try_with(Clone::clone) {
            barrier.0.notify_one();
            barrier.1.notified().await;
        }
        let guarded_delivery = delivery.clone();
        crate::tools::tool_agent_planner_qna::accept_reply_delivery(
            app.gcx.clone(),
            chat_id,
            &guarded_delivery,
            |attempt, planner_chat_id| {
                let mut session = session_arc
                    .try_lock()
                    .map_err(|_| "Reply recipient is busy; retry this known miss")?;
                crate::tools::tool_agent_planner_qna::validate_reply_session(
                    &app.gcx,
                    &session,
                    attempt,
                    planner_chat_id,
                    &delivery,
                )?;
                let result = enqueue(&mut session, delivery);
                if let Err(error) = &result.0 {
                    return Err(error.clone());
                }
                Ok(result)
            },
        )
        .await
    } else {
        let mut session = session_arc.lock().await;
        Ok(enqueue(&mut session, delivery))
    };
    let (accepted, status, wake, should_start_processor, processor_running, queue_notify) =
        match accepted_reply {
            Ok(result) => result,
            Err(error) => {
                record_outbox_status(outbox.as_ref(), &delivery_id, DeliveryStatus::Failed).await;
                return Err(error);
            }
        };

    // A refused delivery has a known outcome: it failed. Leaving it dispatched
    // would report it forever as a message whose effect nobody knows.
    let outcome = match accepted {
        Ok(outcome) => outcome,
        Err(error) => {
            record_outbox_status(outbox.as_ref(), &delivery_id, DeliveryStatus::Failed).await;
            return Err(error);
        }
    };
    // Causality is recorded after the delivery is already accepted, so a
    // recorder that cannot write costs the trace, never the message.
    crate::coordination::causal::record(&app, causal_event).await;

    let save = schedule_delivery_save(
        app.clone(),
        session_arc.clone(),
        source_root,
        vec![(delivery_id.clone(), status)],
        if planner_reply {
            vec![delivery_id.clone()]
        } else {
            vec![]
        },
    )
    .await;
    let persistence = match tokio::time::timeout(std::time::Duration::from_secs(2), save).await {
        Ok(Ok(true)) => TrajectoryPersistence::Committed,
        _ => {
            tracing::warn!(%delivery_id, "accepted delivery persist pending; do not resend");
            TrajectoryPersistence::Pending
        }
    };

    if should_start_processor {
        // Coalesce wakes: at most one non-interrupting processor restart.
        if !processor_running.swap(true, Ordering::SeqCst) {
            tokio::spawn(process_command_queue(
                app,
                session_arc.clone(),
                processor_running,
            ));
        } else {
            queue_notify.notify_one();
        }
    } else if wake {
        queue_notify.notify_one();
    }

    Ok(DeliveryReceipt {
        outcome,
        delivery_id,
        trajectory_persistence: persistence,
        outbox_durable: outbox.is_some(),
        note: (persistence == TrajectoryPersistence::Pending)
            .then(|| "Persistence pending; do not resend".into()),
    })
}

/// Update a still-pending delivery. Runner-owned deliveries are routed to the
/// runner first; otherwise the session's own pending queue is edited, which
/// works while a generation is in flight.
pub async fn update_pending_delivery_in_chat(
    app: AppState,
    chat_id: &str,
    delivery_id: &str,
    push: Option<PushMode>,
    cancel: bool,
) -> Result<(), String> {
    if let Some(result) =
        crate::agents::delivery::update_runner_delivery(&app, chat_id, delivery_id, push, cancel)
            .await
    {
        return result;
    }

    let session_arc = {
        let sessions = app.chat.sessions.read().await;
        sessions.get(chat_id).cloned()
    };
    let Some(session_arc) = session_arc else {
        return Err(format!("chat '{chat_id}' is not active"));
    };

    let ready = {
        let mut session = session_arc.lock().await;
        session.update_pending_delivery(delivery_id, push, cancel)?
    };

    if ready {
        drain_deliveries_at_boundary(app.clone(), session_arc.clone()).await;
    }
    try_save_trajectory_with_intent(app, session_arc, TrajectoryCommitIntent::Required).await?;
    Ok(())
}

struct SaveFinalizer {
    source_root: Option<std::path::PathBuf>,
    statuses: Vec<(String, DeliveryStatus)>,
    replies: Vec<String>,
    reply_permit: Option<crate::tools::tool_agent_planner_qna::ReplyDispatchPermit>,
    receipt: tokio::sync::oneshot::Sender<bool>,
}

type PendingSaves = std::collections::HashMap<uuid::Uuid, Vec<SaveFinalizer>>;

fn pending_saves() -> &'static std::sync::Mutex<PendingSaves> {
    static SAVES: std::sync::OnceLock<std::sync::Mutex<PendingSaves>> = std::sync::OnceLock::new();
    SAVES.get_or_init(Default::default)
}

pub(crate) async fn schedule_delivery_save(
    app: AppState,
    session: Arc<AMutex<ChatSession>>,
    source_root: Option<std::path::PathBuf>,
    statuses: Vec<(String, DeliveryStatus)>,
    replies: Vec<String>,
) -> tokio::sync::oneshot::Receiver<bool> {
    let (receipt, receiver) = tokio::sync::oneshot::channel();
    let key = session.lock().await.delivery_save_id;
    let session = Arc::downgrade(&session);
    let start = {
        let mut saves = pending_saves().lock().unwrap_or_else(|e| e.into_inner());
        let start = !saves.contains_key(&key);
        saves.entry(key).or_default().push(SaveFinalizer {
            source_root,
            statuses,
            replies,
            reply_permit: crate::tools::tool_agent_planner_qna::reply_dispatch_permit(),
            receipt,
        });
        start
    };
    if start {
        let worker = async move {
            let mut batch = Vec::new();
            let mut failures = 0;
            loop {
                {
                    let mut saves = pending_saves().lock().unwrap_or_else(|e| e.into_inner());
                    batch.extend(saves.get_mut(&key).unwrap().drain(..));
                }
                if app.runtime.shutdown_flag.load(Ordering::Acquire) {
                    finish_failed_delivery_saves(key, &mut batch, "shutdown");
                    break;
                }
                let Some(strong_session) = session.upgrade() else {
                    finish_failed_delivery_saves(key, &mut batch, "session dropped");
                    break;
                };
                let committed = matches!(
                    try_save_trajectory_with_intent(
                        app.clone(),
                        strong_session.clone(),
                        TrajectoryCommitIntent::Required,
                    )
                    .await,
                    Ok(true)
                );
                if committed {
                    let chat_id = strong_session.lock().await.chat_id.clone();
                    for finalizer in batch.drain(..) {
                        record_drained_statuses(&app, finalizer.source_root, finalizer.statuses)
                            .await;
                        for id in finalizer.replies {
                            if let Err(error) =
                                crate::tools::tool_agent_planner_qna::acknowledge_reply_with_permit(
                                    finalizer.reply_permit.clone(),
                                    app.gcx.clone(),
                                    &chat_id,
                                    &id,
                                )
                                .await
                            {
                                tracing::warn!(delivery_id = %id, %error, "accepted reply acknowledgement persist pending");
                            }
                        }
                        let _ = finalizer.receipt.send(true);
                    }
                    failures = 0;
                } else {
                    failures += 1;
                }
                drop(strong_session);
                if failures >= 4 {
                    finish_failed_delivery_saves(key, &mut batch, "retry exhausted");
                    break;
                }
                {
                    let mut saves = pending_saves().lock().unwrap_or_else(|e| e.into_inner());
                    if batch.is_empty() && saves.get(&key).unwrap().is_empty() {
                        saves.remove(&key);
                        break;
                    }
                }
                if !committed {
                    tokio::select! {
                        _ = tokio::time::sleep(std::time::Duration::from_secs(1 << (failures - 1))) => {},
                        _ = crate::background_tasks::wait_for_shutdown(app.gcx.clone()) => {
                            finish_failed_delivery_saves(key, &mut batch, "shutdown");
                            break;
                        }
                    }
                }
            }
        };
        if let Err(error) = crate::agents::spawn::track_spawn_owner(worker, true) {
            finish_failed_delivery_saves(key, &mut Vec::new(), &error);
        }
    }
    receiver
}

fn finish_failed_delivery_saves(key: uuid::Uuid, batch: &mut Vec<SaveFinalizer>, reason: &str) {
    if let Some(pending) = pending_saves()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&key)
    {
        batch.extend(pending);
    }
    let ids = batch
        .iter()
        .flat_map(|entry| {
            entry
                .statuses
                .iter()
                .map(|(id, _)| id.as_str())
                .chain(entry.replies.iter().map(String::as_str))
        })
        .collect::<Vec<_>>();
    tracing::warn!(session_id = %key, delivery_ids = ?ids, %reason, "delivery persistence outcome unknown; do not resend");
    for finalizer in batch.drain(..) {
        let _ = finalizer.receipt.send(false);
    }
}

async fn record_outbox_status(
    outbox: Option<&outbox::DeliveryOutbox>,
    delivery_id: &str,
    status: DeliveryStatus,
) {
    let Some(outbox) = outbox else {
        return;
    };
    if let Err(error) = outbox.record_status(delivery_id, status).await {
        tracing::warn!(%delivery_id, %error, "outbox could not record the delivery outcome");
    }
}

/// Mirror what the recipient now reports into the durable outbox, so recovery
/// after a restart never resurrects a message that already settled.
async fn record_drained_statuses(
    app: &AppState,
    source_root: Option<std::path::PathBuf>,
    statuses: Vec<(String, DeliveryStatus)>,
) {
    if statuses.is_empty() {
        return;
    }
    let project = outbox::project_key_for_root(app, source_root).await;
    let Some(outbox) = outbox::outbox_for(app, project).await else {
        return;
    };
    for (delivery_id, status) in statuses {
        record_outbox_status(Some(&outbox), &delivery_id, status).await;
    }
}

/// Land every pending delivery whose boundary is now open. Called from the
/// generation/tool boundaries; safe to call when nothing is pending.
pub async fn drain_deliveries_at_boundary(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
) -> bool {
    super::tool_job_delivery::land_deferred_jobs(&app, &session_arc).await;
    let batch = collect_boundary_deliveries(&mut *session_arc.lock().await);
    finalize_boundary_deliveries(app, session_arc, batch).await
}

pub(crate) struct DeliveryDrainBatch {
    delivered: bool,
    wake: bool,
    processor_running: Arc<std::sync::atomic::AtomicBool>,
    queue_notify: Arc<tokio::sync::Notify>,
    source_root: Option<std::path::PathBuf>,
    statuses: Vec<(String, DeliveryStatus)>,
}

pub(crate) fn collect_boundary_deliveries(session: &mut ChatSession) -> DeliveryDrainBatch {
    session.drain_post_tool_side_effects();
    let tracked: Vec<String> = session
        .pending_deliveries
        .iter()
        .map(|pending| pending.id.clone())
        .collect();
    let (delivered_ids, wake) = session.drain_pending_deliveries();
    let statuses = tracked
        .into_iter()
        .map(|id| {
            let status = session.delivery_status(&id);
            (id, status)
        })
        .collect();
    DeliveryDrainBatch {
        delivered: !delivered_ids.is_empty(),
        wake: wake
            && session.runtime.interruption.is_none()
            && session.runtime.state != SessionState::Interrupted,
        processor_running: session.queue_processor_running.clone(),
        queue_notify: session.queue_notify.clone(),
        source_root: crate::coordination::scope::session_source_root(session),
        statuses,
    }
}

pub(crate) async fn finalize_boundary_deliveries(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
    batch: DeliveryDrainBatch,
) -> bool {
    let DeliveryDrainBatch {
        delivered,
        wake,
        processor_running,
        queue_notify,
        source_root,
        statuses,
    } = batch;
    if !delivered {
        return false;
    }

    let save = schedule_delivery_save(
        app.clone(),
        session_arc.clone(),
        source_root,
        statuses,
        vec![],
    )
    .await;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), save).await;

    if wake {
        let idle = {
            let session = session_arc.lock().await;
            matches!(
                session.runtime.state,
                SessionState::Idle | SessionState::Completed | SessionState::WaitingUserInput
            )
        };
        if idle && !processor_running.swap(true, Ordering::SeqCst) {
            tokio::spawn(process_command_queue(app, session_arc, processor_running));
        } else {
            queue_notify.notify_one();
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::EngineChatSessionFacade;
    use refact_runtime_api::ChatSessionFacade;
    use crate::call_validation::ChatMessage;

    struct LegacyFacade(std::sync::atomic::AtomicUsize);

    #[async_trait::async_trait]
    impl ChatSessionFacade for LegacyFacade {
        async fn session_snapshot(
            &self,
            _: &str,
        ) -> Result<refact_runtime_api::ChatSessionSnapshot, String> {
            unimplemented!()
        }
        async fn update_session(
            &self,
            _: &str,
            _: refact_runtime_api::ChatSessionUpdate,
        ) -> Result<(), String> {
            unimplemented!()
        }
        async fn create_session(
            &self,
            _: refact_runtime_api::CreateSessionRequest,
        ) -> Result<(), String> {
            unimplemented!()
        }
        async fn push_command(
            &self,
            _: &str,
            _: crate::chat::types::ChatCommand,
        ) -> Result<(), String> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn session_state(&self, _: &str) -> Result<Option<SessionState>, String> {
            Ok(None)
        }
        async fn maybe_save_session(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
        async fn save_trajectory_snapshot(
            &self,
            _: refact_chat_history::trajectory_snapshot::TrajectorySnapshot,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn legacy_runtime_refuses_receipt_without_delivering_then_allows_explicit_fallback() {
        let facade = LegacyFacade(Default::default());
        let delivery = PendingDelivery::with_id(
            "legacy",
            vec![ChatMessage::new("user".into(), "message".into())],
            PushMode::Append,
            "test",
            false,
        );
        let error = facade
            .deliver_messages_with_receipt("legacy", delivery.clone())
            .await
            .unwrap_err();
        assert_eq!(error, "deliver_messages_with_receipt is unsupported by this runtime; use deliver_messages without receipt guarantees");
        assert_eq!(facade.0.load(Ordering::SeqCst), 0);
        assert_eq!(
            facade.deliver_messages("legacy", delivery).await.unwrap(),
            DeliveryOutcome::Queued
        );
        assert_eq!(facade.0.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn delivery_failed_save_and_duplicate_retry_remain_unacknowledged() {
        for queued in [false, true] {
            let workspace = tempfile::tempdir().unwrap();
            let config = tempfile::tempdir().unwrap();
            let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
                workspace.path().join("cache"),
                config.path().to_path_buf(),
            )
            .await;
            *gcx.documents_state.workspace_folders.lock().unwrap() =
                vec![workspace.path().to_path_buf()];
            let app = AppState::from_gcx(gcx).await;
            let id = "durable-delivery";
            let session = Arc::new(AMutex::new(ChatSession::new(id.into())));
            session.lock().await.thread.title = "Delivery acceptance".into();
            if queued {
                session.lock().await.start_stream().unwrap();
            }
            app.chat
                .sessions
                .write()
                .await
                .insert(id.into(), session.clone());
            let directory = workspace
                .path()
                .join(".refact")
                .join("trajectories")
                .join(id);
            tokio::fs::create_dir_all(directory.parent().unwrap())
                .await
                .unwrap();
            tokio::fs::write(&directory, "block directory creation")
                .await
                .unwrap();
            let delivery = PendingDelivery::with_id(
                "retry",
                vec![ChatMessage::new("user".into(), "keep".into())],
                PushMode::Append,
                "test",
                false,
            );
            let accepted = deliver_to_chat_with_receipt(app.clone(), id, delivery.clone())
                .await
                .unwrap();
            assert_eq!(
                accepted.outcome,
                if queued {
                    DeliveryOutcome::Queued
                } else {
                    DeliveryOutcome::Delivered
                }
            );
            assert_eq!(
                accepted.trajectory_persistence,
                TrajectoryPersistence::Pending
            );
            assert_eq!(
                accepted.note.as_deref(),
                Some("Persistence pending; do not resend")
            );
            let duplicate = deliver_to_chat_with_receipt(app.clone(), id, delivery.clone())
                .await
                .unwrap();
            assert_eq!(duplicate.outcome, DeliveryOutcome::Duplicate);
            assert_eq!(
                duplicate.trajectory_persistence,
                TrajectoryPersistence::Pending
            );
            {
                let session = session.lock().await;
                assert_eq!(session.pending_deliveries.len(), usize::from(queued));
                assert_eq!(session.messages.len(), usize::from(!queued));
                assert!(session.trajectory_dirty);
            }
            tokio::fs::remove_file(directory).await.unwrap();
            assert_eq!(
                deliver_to_chat(app.clone(), id, delivery).await.unwrap(),
                DeliveryOutcome::Duplicate
            );
            assert!(tokio::time::timeout(
                std::time::Duration::from_secs(10),
                schedule_delivery_save(app.clone(), session.clone(), None, vec![], vec![]).await
            )
            .await
            .unwrap()
            .unwrap());
            let saved = crate::chat::trajectories::load_trajectory_for_chat(app.gcx.clone(), id)
                .await
                .unwrap();
            assert_eq!(saved.pending_deliveries.len(), usize::from(queued));
            assert_eq!(saved.messages.len(), usize::from(!queued));
            assert!(!session.lock().await.trajectory_dirty);
        }
    }

    #[tokio::test]
    async fn interrupted_delivery_does_not_wake_processor() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let workspace = tempfile::tempdir().unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];
        let app = AppState::from_gcx(gcx).await;
        let session = Arc::new(AMutex::new(ChatSession::new("interrupted-delivery".into())));
        crate::chat::session_runtime::interrupt_board_execution(
            &mut *session.lock().await,
            crate::tasks::admission::ExecutionUnavailable::Missing,
        );
        app.chat
            .sessions
            .write()
            .await
            .insert("interrupted-delivery".into(), session.clone());
        let receipt = deliver_to_chat_with_receipt(
            app,
            "interrupted-delivery",
            PendingDelivery::with_id(
                "interrupted-message",
                vec![ChatMessage::new("user".into(), "steer".into())],
                PushMode::Append,
                "test",
                true,
            ),
        )
        .await
        .unwrap();
        assert!(matches!(
            receipt.outcome,
            DeliveryOutcome::Delivered | DeliveryOutcome::Queued
        ));
        let session = session.lock().await;
        assert_eq!(session.runtime.state, SessionState::Interrupted);
        assert!(!session.queue_processor_running.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn delivery_records_causality_without_changing_delivery_semantics() {
        use crate::chat::types::CausalStamp;
        use crate::coordination::causal;

        let workspace = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            workspace.path().join("cache"),
            config.path().to_path_buf(),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];
        let app = AppState::from_gcx(gcx).await;
        let chat_id = format!("causal-delivery-{}", uuid::Uuid::new_v4());
        let session = Arc::new(AMutex::new(ChatSession::new(chat_id.clone())));
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.clone(), session.clone());

        let mut first = PendingDelivery::with_id(
            format!("{chat_id}-one"),
            vec![ChatMessage::new("user".into(), "cause".into())],
            PushMode::Append,
            "test",
            false,
        );
        first.causal = Some(CausalStamp::new(format!("delivery:{chat_id}:one"), vec![]));
        assert_eq!(
            deliver_to_chat(app.clone(), &chat_id, first).await.unwrap(),
            DeliveryOutcome::Delivered,
            "recording causality must not change a delivery outcome"
        );

        let mut second = PendingDelivery::with_id(
            format!("{chat_id}-two"),
            vec![ChatMessage::new("user".into(), "effect".into())],
            PushMode::Append,
            "test",
            false,
        );
        second.causal = Some(CausalStamp::new(
            format!("delivery:{chat_id}:two"),
            vec![format!("delivery:{chat_id}:one")],
        ));
        assert_eq!(
            deliver_to_chat(app.clone(), &chat_id, second)
                .await
                .unwrap(),
            DeliveryOutcome::Delivered
        );
        assert_eq!(session.lock().await.messages.len(), 2);

        let trace = causal::recorder().trace(&format!("delivery:{chat_id}:two"));
        let ids: Vec<String> = trace
            .events
            .iter()
            .map(|event| event.event_id.clone())
            .collect();
        assert!(ids.contains(&format!("delivery:{chat_id}:one")));
        assert!(ids.contains(&format!("delivery:{chat_id}:two")));
        assert!(
            !trace.complete,
            "the first delivery's own cause is unknown, and that gap is reported"
        );
    }

    #[tokio::test]
    async fn delivery_facade_reports_actual_outcomes_and_rejects_missing_target() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let workspace = tempfile::tempdir().unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];
        let app = AppState::from_gcx(gcx.clone()).await;
        let session = Arc::new(AMutex::new(ChatSession::new("delivery-facade".into())));
        app.chat
            .sessions
            .write()
            .await
            .insert("delivery-facade".into(), session.clone());
        let facade = EngineChatSessionFacade::new(gcx);
        let make = |id: &str| {
            PendingDelivery::with_id(
                id,
                vec![ChatMessage::new("user".into(), "delivery".into())],
                PushMode::Append,
                "test",
                false,
            )
        };
        assert!(facade
            .deliver_messages("missing-delivery-target", make("missing"))
            .await
            .is_err());
        assert_eq!(
            facade
                .deliver_messages("delivery-facade", make("one"))
                .await
                .unwrap(),
            DeliveryOutcome::Delivered
        );
        assert_eq!(
            facade
                .deliver_messages("delivery-facade", make("one"))
                .await
                .unwrap(),
            DeliveryOutcome::Duplicate
        );
        session.lock().await.start_stream().unwrap();
        assert_eq!(
            facade
                .deliver_messages("delivery-facade", make("two"))
                .await
                .unwrap(),
            DeliveryOutcome::Queued
        );
        assert!(!session.lock().await.abort_flag.load(Ordering::SeqCst));
        facade
            .update_pending_delivery("delivery-facade", "two", Some(PushMode::Preempt), false)
            .await
            .unwrap();
        let session = session.lock().await;
        assert!(session.pending_deliveries.is_empty());
        assert!(session.draft_message.is_none());
        assert!(session.abort_flag.load(Ordering::SeqCst));
    }
}
