use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Mutex, OnceLock},
};
use refact_core::chat_types::{DeliveryOutcome, PendingDelivery, PushMode};
use refact_review_core::{ActorKind, Review, Side};
use serde::{Deserialize, Serialize};
use serde_json::json;
use crate::{app_state::AppState, call_validation::ChatMessage};
use super::service::with_store;

pub async fn invalidate(app: &AppState, review: &Review) {
    let session = app
        .chat
        .sessions
        .read()
        .await
        .get(&review.owner_chat_id)
        .cloned();
    if let Some(session) = session {
        session.lock().await.emit(crate::chat::types::ChatEvent::ThreadUpdated { params: json!({"review": {"id": review.id, "revision": review.revision, "current_candidate_id": review.current_candidate_id}}) });
    }
}

pub async fn flush(app: &AppState, root: PathBuf, review: &Review) -> Result<(), String> {
    flush_with_wake(app, root, review, true).await
}

/// Read recovery delivers receipts, never starts generation.
pub async fn flush_read_only(app: &AppState, root: PathBuf, review: &Review) -> Result<(), String> {
    flush_with_wake(app, root, review, false).await
}

async fn flush_with_wake(
    app: &AppState,
    root: PathBuf,
    review: &Review,
    allow_wake: bool,
) -> Result<(), String> {
    let rid = review.id.clone();
    let owner = review.owner_chat_id.clone();
    let (review, pending) = with_store(root.clone(), move |s| {
        let review = s.load(&rid)?;
        if review.owner_chat_id != owner {
            return Err(refact_review_core::Error::NotFound);
        }
        let pending = s.pending_deliveries(&rid)?;
        Ok((review, pending))
    })
    .await
    .map_err(|e| e.to_string())?;
    for item in pending {
        if item.owner_chat_id != review.owner_chat_id || item.review_id != review.id {
            return Err("review outbox identity mismatch".into());
        }
        let Some((thread, contribution)) = review.threads.iter().find_map(|t| {
            t.contributions
                .iter()
                .find(|c| c.id == item.contribution_id)
                .map(|c| (t, c))
        }) else {
            return Err("review outbox contribution missing".into());
        };
        let role = if contribution.actor.kind == ActorKind::Agent {
            "assistant"
        } else {
            "user"
        };
        let mut message = ChatMessage::new(role.into(), contribution.body.clone());
        if let Some(id) = &contribution.chat_message_id {
            message.message_id = id.clone();
        }
        message.extra.insert(
            "review".into(),
            json!({"id":review.id,"thread_id":thread.id,"contribution_id":contribution.id,"target":thread.target,"blocks":contribution.blocks,"contribution":contribution,"tour_revision_proposal_id":super::proposals::contribution_proposal_id(&review, &contribution.id)}),
        );
        let mut paths = Vec::new();
        for anchor in &thread.anchors {
            let p = &anchor.position;
            paths.push(root.join(&p.path));
            if let Some(candidate) = review.candidates.iter().find(|c| c.id == p.candidate_id) {
                paths.push(PathBuf::from(&candidate.repository).join(&p.path));
            }
        }
        paths.extend(super::service::rich_paths(
            &root,
            &review,
            &contribution.blocks,
        ));
        if let Some(target) = &thread.target {
            let (version, _) =
                refact_review_core::rich_blocks::resolve_tour_target(&review, target)
                    .map_err(|e| e.to_string())?;
            paths.extend(super::service::rich_paths(&root, &review, &version.blocks));
        }
        let records = crate::privacy::records::declared_file_records(&app.gcx, paths)?;
        crate::privacy::records::merge_records(&mut message, records);
        crate::privacy::records::merge_records(
            &mut message,
            super::intake_runtime::privacy(&review),
        );
        if let Some(context) = &contribution.context {
            crate::privacy::records::merge_records(&mut message, context.privacy.clone());
        }
        let mut messages = Vec::new();
        let inspected = contribution
            .context
            .as_ref()
            .and_then(|c| c.candidate_id.as_deref());
        let anchors = thread.anchors.iter().filter_map(|a| {
            let p = if inspected.is_none() || inspected == Some(a.position.candidate_id.as_str()) { Some(&a.position) } else { a.history.iter().find(|p| Some(p.candidate_id.as_str()) == inspected) }?;
            Some(json!({"candidate_id":p.candidate_id,"path":p.path,"side":p.side,"start_line":p.start_line,"end_line":p.end_line}))
        }).take(8).collect::<Vec<_>>();
        let effective_applicability = contribution.context.as_ref().map(|c| {
            if c.candidate_id.is_some() && c.candidate_id != review.current_candidate_id {
                refact_review_core::Applicability::Historical
            } else {
                c.applicability.clone()
            }
        });
        let payload = json!({"review_id":review.id,"thread_id":thread.id,"contribution_id":contribution.id,"candidate_id":inspected,"run_id":contribution.context.as_ref().and_then(|c| c.run_id.as_deref()),"applicability":effective_applicability,"target":thread.target,"blocks":contribution.blocks,"anchors":anchors});
        let mut context = crate::chat::internal_roles::event(
            crate::chat::internal_roles::EventSubkind::SystemNotice,
            "review.comment",
            payload.clone(),
            format!("Review contribution context: {payload}"),
        );
        crate::privacy::records::merge_message_records(&mut context, &[message.clone()])
            .map_err(|e| e.to_string())?;
        messages.push(context);
        messages.push(message);
        let user = contribution.actor.kind == ActorKind::User;
        let mut delivery = PendingDelivery::with_id(
            &item.id,
            messages,
            PushMode::Append,
            if user {
                "review.comment.user"
            } else {
                "review.comment.reply"
            },
            allow_wake && super::proposals::contribution_wakes(&review, contribution),
        );
        delivery.set_ttl_secs(u64::MAX);
        let receipt = crate::chat::delivery::deliver_to_chat_with_receipt(
            app.clone(),
            &review.owner_chat_id,
            delivery,
        )
        .await?;
        if matches!(
            receipt.outcome,
            DeliveryOutcome::Delivered | DeliveryOutcome::Queued | DeliveryOutcome::Duplicate
        ) && (receipt.outbox_durable
            || receipt.trajectory_persistence
                == crate::chat::delivery::TrajectoryPersistence::Committed)
        {
            let id = item.id;
            with_store(root.clone(), move |s| s.settle_delivery(&id))
                .await
                .map_err(|e| e.to_string())?;
        } else {
            return Err(format!("review delivery pending: {:?}", receipt.outcome));
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActivityRequest {
    pub candidate_id: String,
    pub action: String,
    pub path: Option<String>,
    pub side: Option<Side>,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
    pub object_id: Option<String>,
    pub request_key: String,
}
fn selections() -> &'static Mutex<HashMap<String, ActivityRequest>> {
    static SELECTIONS: OnceLock<Mutex<HashMap<String, ActivityRequest>>> = OnceLock::new();
    SELECTIONS.get_or_init(|| Mutex::new(HashMap::new()))
}
pub async fn activity(
    root: PathBuf,
    review: &Review,
    request: ActivityRequest,
) -> refact_review_core::Result<()> {
    if request.request_key.is_empty()
        || request.request_key.len() > 256
        || request.action.len() > 128
        || !review
            .candidates
            .iter()
            .any(|c| c.id == request.candidate_id)
    {
        return Err(refact_review_core::Error::Invalid(
            "invalid selection".into(),
        ));
    }
    if let Some(path) = &request.path {
        let candidate = review
            .candidates
            .iter()
            .find(|c| c.id == request.candidate_id)
            .ok_or(refact_review_core::Error::NotFound)?;
        if !candidate.files.iter().any(|f| &f.path == path) {
            return Err(refact_review_core::Error::NotFound);
        }
        if let (Some(side), Some(start), Some(end)) =
            (request.side.clone(), request.start_line, request.end_line)
        {
            let candidate = candidate.clone();
            let path = path.clone();
            with_store(root, move |s| s.anchor(&candidate, &path, side, start, end)).await?;
        } else if request.start_line.is_some() || request.end_line.is_some() {
            return Err(refact_review_core::Error::Invalid(
                "incomplete line selection".into(),
            ));
        }
    } else if request.side.is_some() || request.start_line.is_some() || request.end_line.is_some() {
        return Err(refact_review_core::Error::Invalid(
            "selection path required".into(),
        ));
    }
    let mut selections = selections()
        .lock()
        .map_err(|_| refact_review_core::Error::Invalid("selection lock poisoned".into()))?;
    if selections.len() >= 1024 && !selections.contains_key(&review.id) {
        selections.clear();
    }
    selections.insert(review.id.clone(), request);
    Ok(())
}
pub async fn context_for_chat(
    app: &AppState,
    chat_id: &str,
) -> Result<Option<ChatMessage>, String> {
    let root = super::service::trusted_root(app, chat_id).await?;
    let Some(review) = super::service::get_for_chat(root.clone(), chat_id.to_owned())
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    let selection = selections()
        .lock()
        .map_err(|_| "selection lock poisoned")?
        .get(&review.id)
        .cloned()
        .filter(|s| review.candidates.iter().any(|c| c.id == s.candidate_id));
    let payload = json!({"id":review.id,"revision":review.revision,"current_candidate_id":review.current_candidate_id,"selection":selection,"pending_tour_revision_actions":super::proposals::pending(&review),"tour_revision_notice":"Acceptance records pending intent only. Never self-accept or automatically author on load; only owning foreground authoring can fulfil an accepted proposal."});
    let mut message = crate::chat::internal_roles::event(
        crate::chat::internal_roles::EventSubkind::SystemNotice,
        "review.context",
        payload.clone(),
        format!("Current review context: {payload}"),
    );
    let mut paths = Vec::new();
    if let Some(selection) = &selection {
        if let Some(path) = &selection.path {
            paths.push(root.join(path));
            if let Some(c) = review
                .candidates
                .iter()
                .find(|c| c.id == selection.candidate_id)
            {
                paths.push(PathBuf::from(&c.repository).join(path));
            }
        }
    }
    crate::privacy::records::merge_records(
        &mut message,
        crate::privacy::records::declared_file_records(&app.gcx, paths)?,
    );
    crate::privacy::records::merge_records(&mut message, super::intake_runtime::privacy(&review));
    Ok(Some(message))
}
