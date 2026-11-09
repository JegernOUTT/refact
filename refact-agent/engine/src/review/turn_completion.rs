use std::{
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
};
use refact_review_core::{ProposalTurnCompletion, Review};
use tokio::sync::Mutex;
use crate::{
    app_state::AppState,
    call_validation::ChatMessage,
    chat::types::{ChatSession, SessionState, TrajectoryCommitIntent},
};

type LiveOrigins = std::collections::HashMap<
    String,
    (
        std::sync::Weak<Mutex<ChatSession>>,
        std::sync::Weak<std::sync::atomic::AtomicBool>,
    ),
>;
pub(super) fn origins() -> &'static std::sync::Mutex<LiveOrigins> {
    static ORIGINS: std::sync::OnceLock<std::sync::Mutex<LiveOrigins>> = std::sync::OnceLock::new();
    ORIGINS.get_or_init(Default::default)
}

pub(crate) async fn capture(
    app: &AppState,
    owner: &str,
    call: &str,
) -> Option<(
    refact_review_core::ProposalTurnCompletionIntent,
    std::sync::Weak<Mutex<ChatSession>>,
    std::sync::Weak<std::sync::atomic::AtomicBool>,
)> {
    let session = app.chat.sessions.read().await.get(owner).cloned()?;
    let locked = session.lock().await;
    let turn = locked.messages.iter().rev().find(|m| {
        m.role == "assistant"
            && m.tool_calls
                .as_ref()
                .is_some_and(|calls| calls.iter().any(|c| c.id == call))
    })?;
    let intent = refact_review_core::ProposalTurnCompletionIntent {
        owner_chat_id: owner.into(),
        turn_id: turn.message_id.clone(),
        tool_call_id: call.into(),
        job_id: app
            .runtime
            .tool_jobs
            .proposal_job(owner, call)
            .map(|j| j.job_id),
        human_message_id: human_id(&locked),
    };
    let origin = Arc::downgrade(&locked.execution_abort);
    drop(locked);
    Some((intent, Arc::downgrade(&session), origin))
}

pub(crate) async fn register(
    root: PathBuf,
    review: &Review,
    intent: &refact_review_core::ProposalTurnCompletionIntent,
    proposal: &str,
    session: std::sync::Weak<Mutex<ChatSession>>,
    origin: std::sync::Weak<std::sync::atomic::AtomicBool>,
) -> Result<(), String> {
    let owner = intent.owner_chat_id.clone();
    let proposal_id = proposal.to_owned();
    let lookup_id = proposal_id.clone();
    let record =
        super::service::with_store(root, move |s| s.proposal_completion(&owner, &lookup_id))
            .await
            .map_err(|e| e.to_string())?;
    let Some(record) = record else {
        return Ok(());
    };
    if record.review_id != review.id
        || record.owner_chat_id != intent.owner_chat_id
        || record.turn_id != intent.turn_id
        || record.tool_call_id != intent.tool_call_id
        || record.job_id != intent.job_id
        || record.human_message_id != intent.human_message_id
    {
        return Ok(());
    }
    let mut origins = origins()
        .lock()
        .map_err(|_| "proposal origin lock poisoned")?;
    origins.retain(|_, (session, origin)| session.strong_count() > 0 && origin.strong_count() > 0);
    origins.entry(proposal_id).or_insert((session, origin));
    Ok(())
}

pub(crate) async fn await_proposal_result(
    app: &AppState,
    session: &Arc<Mutex<ChatSession>>,
    job: &crate::tool_jobs::JobReceipt,
) -> Result<(), String> {
    let Some(snapshot) = app
        .runtime
        .tool_jobs
        .wait_terminal(&job.binding.chat_id, &job.job_id)
        .await
    else {
        return Ok(());
    };
    let delivery_id = format!("tool_job:{}:{}", job.job_id, snapshot.revision);
    let inline = {
        let locked = session.lock().await;
        locked.messages.iter().any(|m| {
            m.role == "tool"
                && m.tool_call_id == job.binding.tool_call_id
                && m.tool_failed == Some(false)
        })
    };
    if inline {
        return Ok(());
    }
    let mut pending = crate::chat::types::PendingDelivery::with_id(
        delivery_id,
        vec![],
        crate::chat::types::PushMode::Append,
        "tool_job.completed",
        true,
    );
    pending.after_tool_call_id = Some(job.binding.tool_call_id.clone());
    pending.deferred_job = Some(refact_core::chat_types::DeferredJobRef {
        job_id: job.job_id.clone(),
        revision: snapshot.revision,
        chat_id: job.binding.chat_id.clone(),
        tool_call_id: job.binding.tool_call_id.clone(),
        tool_name: job.binding.tool_name.clone(),
        model: job.binding.model.clone(),
        turn_id: job.binding.turn_id.clone(),
    });
    {
        let mut locked = session.lock().await;
        if !locked.delivered_delivery_ids.contains(&pending.id)
            && !locked
                .pending_deliveries
                .iter()
                .any(|item| item.id == pending.id)
        {
            locked.enqueue_delivery(pending)?;
        }
    }
    crate::chat::tool_job_delivery::land_deferred_jobs(app, session).await;
    Ok(())
}

fn human_id(session: &ChatSession) -> Option<String> {
    session
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.message_id.clone())
}

pub(crate) async fn result_completions(
    app: &AppState,
    reference: &refact_core::chat_types::DeferredJobRef,
) -> Vec<ProposalTurnCompletion> {
    if reference.tool_name != "review_update" {
        return Vec::new();
    }
    let has_origin = origins().lock().is_ok_and(|entries| {
        entries
            .values()
            .any(|(session, _)| session.strong_count() > 0)
    });
    if !has_origin {
        return Vec::new();
    }
    let Ok(root) = super::service::trusted_root(app, &reference.chat_id).await else {
        return Vec::new();
    };
    let owner = reference.chat_id.clone();
    super::service::with_store(root, move |store| store.proposal_completions(&owner))
        .await
        .unwrap_or_default()
}

pub(crate) fn owns_result_wake(
    session_arc: &Arc<Mutex<ChatSession>>,
    session: &ChatSession,
    reference: &refact_core::chat_types::DeferredJobRef,
    records: &[ProposalTurnCompletion],
) -> bool {
    if session.turn_depth == 0
        || session.closed
        || session.abort_flag.load(Ordering::Acquire)
        || session.user_interrupt_flag.load(Ordering::Acquire)
        || !session.command_queue.is_empty()
        || session.pending_browser_message.is_some()
    {
        return false;
    }
    records.iter().any(|record| {
        if record.owner_chat_id != session.chat_id
            || record.job_id.as_deref() != Some(reference.job_id.as_str())
            || record.tool_call_id != reference.tool_call_id
            || record.turn_id != reference.turn_id
            || record.human_message_id != human_id(session)
        {
            return false;
        }
        let origin = origins()
            .lock()
            .ok()
            .and_then(|entries| entries.get(&record.proposal_id).cloned());
        let Some((owner, origin)) = origin else {
            return false;
        };
        if !owner
            .upgrade()
            .is_some_and(|owner| Arc::ptr_eq(&owner, session_arc))
            || !origin.upgrade().is_some_and(|origin| {
                Arc::ptr_eq(&origin, &session.execution_abort) && !origin.load(Ordering::Acquire)
            })
        {
            return false;
        }
        session
            .messages
            .iter()
            .find(|message| message.message_id == record.turn_id)
            .and_then(|message| message.tool_calls.as_ref())
            .is_some_and(|calls| calls.len() == 1 && calls[0].id == record.tool_call_id)
    })
}

pub(crate) async fn acknowledge(
    app: &AppState,
    chat: &str,
    call: &str,
    job: Option<&str>,
    messages: &[ChatMessage],
) -> Result<(), String> {
    if messages.is_empty() || messages.iter().any(|m| m.tool_failed == Some(true)) {
        return Ok(());
    }
    let ids = messages
        .iter()
        .filter(|m| !m.message_id.is_empty())
        .map(|m| m.message_id.clone())
        .collect::<Vec<_>>();
    if ids.len() != messages.len() {
        return Ok(());
    }
    let root = super::service::trusted_root(app, chat).await?;
    let chat = chat.to_owned();
    let call = call.to_owned();
    let job = job.map(str::to_owned);
    super::service::with_store(root, move |s| {
        for record in s.proposal_completions(&chat)? {
            if record.tool_call_id == call
                && record.job_id == job
                && record.result_message_ids.is_empty()
            {
                s.acknowledge_proposal_result(&record, &ids)?;
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())
}

pub(super) fn eligible(
    app: &AppState,
    session: &ChatSession,
    record: &ProposalTurnCompletion,
) -> bool {
    let current_origin = origins()
        .lock()
        .ok()
        .and_then(|origins| origins.get(&record.proposal_id).cloned())
        .and_then(|(_, origin)| origin.upgrade());
    if !current_origin.is_some_and(|origin| {
        Arc::ptr_eq(&origin, &session.execution_abort) && !origin.load(Ordering::Acquire)
    }) {
        return false;
    }
    if session.closed
        || session.chat_id != record.owner_chat_id
        || human_id(session) != record.human_message_id
        || session.abort_flag.load(Ordering::Acquire)
        || session.user_interrupt_flag.load(Ordering::Acquire)
        || session.runtime.paused
        || !session.runtime.pause_reasons.is_empty()
        || session.planner_paused()
        || !session.command_queue.is_empty()
        || !session.delivery_wake_sources.is_empty()
        || !session.pending_deliveries.is_empty()
        || session.pending_browser_message.is_some()
        || !session.post_tool_side_effects.is_empty()
        || !session.delivered_delivery_ids.contains(&record.delivery_id)
        || record.result_message_ids.is_empty()
        || !record.result_message_ids.iter().all(|id| {
            session.messages.iter().any(|m| {
                &m.message_id == id
                    && m.tool_failed == Some(false)
                    && ((m.role == "tool"
                        && m.tool_call_id == record.tool_call_id
                        && !m.extra.contains_key("tool_job"))
                        || (m.role == "event"
                            && m.extra.get("tool_job_result").is_some_and(|v| {
                                v["job_id"].as_str() == record.job_id.as_deref()
                                    && v["tool_call_id"].as_str()
                                        == Some(record.tool_call_id.as_str())
                                    && v["original_role"] == "tool"
                            })))
            })
        })
        || !app
            .runtime
            .tool_jobs
            .proposal_boundary_clear(&session.chat_id, record.job_id.as_deref())
    {
        return false;
    }
    let Some(index) = session
        .messages
        .iter()
        .position(|m| m.message_id == record.turn_id)
    else {
        return false;
    };
    let Some(calls) = &session.messages[index].tool_calls else {
        return false;
    };
    // Conservatively leave mixed groups to the normal protocol continuation.
    calls.len() == 1
        && calls[0].id == record.tool_call_id
        && calls[0].function.name == "review_update"
        && !session.messages[index + 1..]
            .iter()
            .any(|m| m.role == "assistant" && m.tool_calls.as_ref().is_some_and(|c| !c.is_empty()))
}

fn canonical_landed(
    session: &ChatSession,
    record: &ProposalTurnCompletion,
    review: &Review,
) -> bool {
    let Some((thread, contribution)) = review.threads.iter().find_map(|t| {
        t.contributions
            .iter()
            .find(|c| c.id == record.contribution_id)
            .map(|c| (t, c))
    }) else {
        return false;
    };
    let expected = serde_json::json!({"id":review.id,"thread_id":thread.id,"contribution_id":contribution.id,"target":thread.target,"blocks":contribution.blocks,"contribution":contribution,"tour_revision_proposal_id":record.proposal_id});
    session
        .messages
        .iter()
        .filter(|m| {
            !m.message_id.is_empty()
                && contribution
                    .chat_message_id
                    .as_ref()
                    .is_none_or(|id| &m.message_id == id)
                && m.role == "assistant"
                && m.content.content_text_only() == contribution.body
                && m.extra.get("review") == Some(&expected)
        })
        .count()
        == 1
}

pub(crate) async fn stop_at_boundary(
    app: &AppState,
    session: &Arc<Mutex<ChatSession>>,
) -> Result<bool, String> {
    let chat = session.lock().await.chat_id.clone();
    let has_origin = origins()
        .lock()
        .map_err(|_| "proposal origin lock poisoned")?
        .values()
        .any(|(candidate, _)| {
            candidate
                .upgrade()
                .is_some_and(|candidate| Arc::ptr_eq(&candidate, session))
        });
    if !has_origin {
        return Ok(false);
    }
    let root = super::service::trusted_root(app, &chat).await?;
    if !root.join(".refact/reviews/reviews.sqlite").exists() {
        return Ok(false);
    }
    let owner = chat.clone();
    let records = super::service::with_store(root.clone(), move |s| s.proposal_completions(&owner))
        .await
        .map_err(|e| e.to_string())?;
    for record in records {
        let review_id = record.review_id.clone();
        let review = super::service::with_store(root.clone(), move |s| s.load(&review_id))
            .await
            .map_err(|e| e.to_string())?;
        {
            let locked = session.lock().await;
            if !eligible(app, &locked, &record) || !canonical_landed(&locked, &record, &review) {
                continue;
            }
        }
        if !crate::chat::trajectories::try_save_trajectory_with_intent(
            app.clone(),
            session.clone(),
            TrajectoryCommitIntent::Required,
        )
        .await?
        {
            return Ok(false);
        }
        let store_root = root.clone();
        let store = tokio::task::spawn_blocking(move || {
            let store = refact_review_core::ReviewStore::open(&store_root)?;
            store.set_completion_busy_timeout_zero()?;
            Ok::<_, refact_review_core::Error>(store)
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
        let sessions = app.chat.sessions.read().await;
        if !sessions.get(&chat).is_some_and(|s| Arc::ptr_eq(s, session)) {
            return Ok(false);
        }
        let mut locked = session.lock().await;
        if !eligible(app, &locked, &record) || !canonical_landed(&locked, &record, &review) {
            continue;
        }
        // No await between the final lifecycle check, durable single consumption,
        // and Idle. A crash after consumption never completes a later turn.
        if store
            .consume_proposal_completion(&record)
            .map_err(|e| e.to_string())?
        {
            locked.set_runtime_state(SessionState::Idle, None);
            locked.release_turn_only_state();
            if let Ok(mut origins) = origins().lock() {
                origins.remove(&record.proposal_id);
            }
            return Ok(true);
        }
    }
    Ok(false)
}
