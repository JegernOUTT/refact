//! Explicit user consent records intent only; it never schedules authoring.
use crate::app_state::AppState;
use refact_review_core::{Actor, ActorKind, Error, Mutation, Operation, Review, TourDiscussionTarget};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptRequest {
    pub owner_chat_id: String,
    pub expected_revision: u64,
    pub request_key: String,
    pub target: TourDiscussionTarget,
}

pub async fn accept_owned(
    root: PathBuf,
    review_id: String,
    request: AcceptRequest,
    proposal_id: String,
) -> refact_review_core::Result<Review> {
    super::service::with_store(root, move |store| {
        let review = store.load(&review_id)?;
        if review.owner_chat_id != request.owner_chat_id
            || store
                .get_for_chat(&request.owner_chat_id)?
                .as_ref()
                .map(|r| &r.id)
                != Some(&review_id)
        {
            return Err(Error::NotFound);
        }
        // Do not prevalidate current version: retries must reach durable replay.
        store.mutate_current_tour_revision(
            &request.owner_chat_id,
            &review_id,
            Mutation {
                expected_revision: request.expected_revision,
                request_key: request.request_key,
                actor: Actor {
                    kind: ActorKind::User,
                    id: "user".into(),
                },
                at_ms: super::service::now_ms(),
                operation: Operation::AcceptTourRevision {
                    proposal_id,
                    target: request.target,
                },
            },
            None,
        )
    })
    .await
}

pub async fn accept(
    app: &AppState,
    review_id: String,
    proposal_id: String,
    request: AcceptRequest,
) -> refact_review_core::Result<Review> {
    let root = super::service::trusted_root(app, &request.owner_chat_id)
        .await
        .map_err(Error::Invalid)?;
    let review = accept_owned(root.clone(), review_id, request, proposal_id).await?;
    super::chat_bridge::invalidate(app, &review).await;
    if let Err(error) = super::chat_bridge::flush_read_only(app, root, &review).await {
        tracing::warn!(%error, "Tour revision acceptance delivery pending");
    }
    Ok(review)
}

/// Read-only status, not a dispatch queue or authority to execute.
pub fn pending(review: &Review) -> Vec<Value> {
    review.tour_revision_proposals.iter().filter(|p| p.fulfilment.is_none()).map(|p| {
        json!({"id":p.id,"target":p.target,"thread_id":p.thread_id,"change_request":p.change_request,
            "acceptance":p.acceptance,"status":if !refact_review_core::rich_blocks::is_tour_revision_target_current(review, &p.target) { "stale" } else if p.acceptance.is_some() { "accepted_pending" } else { "awaiting_user_acceptance" }})
    }).collect()
}

pub fn contribution_proposal_id<'a>(review: &'a Review, contribution_id: &str) -> Option<&'a str> {
    review
        .tour_revision_proposals
        .iter()
        .find(|p| {
            p.proposal_contribution_id == contribution_id
                || p.acceptance
                    .as_ref()
                    .is_some_and(|a| a.contribution_id == contribution_id)
        })
        .map(|p| p.id.as_str())
}

pub fn contribution_wakes(
    review: &Review,
    contribution: &refact_review_core::Contribution,
) -> bool {
    contribution.actor.kind == ActorKind::User
        && contribution_proposal_id(review, &contribution.id).is_none()
}
