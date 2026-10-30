use refact_core::chat_types::{DeliveryOutcome, PendingDelivery, PushMode};

use crate::app_state::AppState;
use crate::coordination::peer_authority::PeerAuthority;

/// Consent is re-checked here, not only when the message was prepared: a grant
/// revoked or expired while the delivery sat in the queue stops it before it
/// can land. Deliveries without a peer stamp never needed peer authority.
pub fn peer_delivery_authorized(app: &AppState, delivery: &PendingDelivery) -> bool {
    let Some(stamp) = &delivery.peer_mail else {
        return true;
    };
    app.peer_grants.allows(
        &stamp.project,
        &stamp.sender_root,
        &stamp.recipient_root,
        PeerAuthority::Mail,
        chrono::Utc::now().timestamp_millis().max(0) as u64,
    )
}

/// Drop every drained delivery whose peer consent no longer holds, recording
/// it as a terminal `Failed` so its status never reads as delivered.
async fn retain_authorized(
    app: &AppState,
    agent_id: &str,
    deliveries: Vec<PendingDelivery>,
) -> Vec<PendingDelivery> {
    let mut authorized = Vec::with_capacity(deliveries.len());
    for delivery in deliveries {
        if crate::tools::tool_agent_planner_qna::is_planner_reply_delivery(&delivery) {
            tracing::warn!(%agent_id, delivery_id = %delivery.id, "Rejected runner planner reply: authenticated board acceptance required; not consumed");
        }
        if !crate::tools::tool_agent_planner_qna::is_planner_reply_delivery(&delivery)
            && peer_delivery_authorized(app, &delivery)
        {
            authorized.push(delivery);
            continue;
        }
        if let Err(error) = app
            .agents
            .record_delivery_status(
                agent_id,
                &delivery.id,
                refact_chat_api::DeliveryStatus::Failed,
            )
            .await
        {
            tracing::warn!(%agent_id, %error, "Failed to reject an unauthorized runner delivery");
        }
    }
    authorized
}

/// The only drain a runner may use: identical to the registry drain except a
/// revoked peer grant stops the delivery at the boundary.
pub async fn drain_authorized(
    app: &AppState,
    agent_id: &str,
    idle: bool,
) -> Result<Vec<PendingDelivery>, String> {
    let deliveries = app.agents.drain_deliveries(agent_id, idle).await?;
    Ok(retain_authorized(app, agent_id, deliveries).await)
}

pub async fn finish_authorized_turn(
    app: &AppState,
    agent_id: &str,
) -> Result<Vec<PendingDelivery>, String> {
    let deliveries = app.agents.finish_delivery_turn(agent_id).await?;
    Ok(retain_authorized(app, agent_id, deliveries).await)
}

async fn live_runner(app: &AppState, chat_id: &str) -> Option<String> {
    let agent_id = app.agents.find_agent_id_by_child_chat_id(chat_id).await?;
    app.agents.has_runtime(&agent_id).await.then_some(agent_id)
}

pub async fn route_to_runner(
    app: &AppState,
    chat_id: &str,
    delivery: PendingDelivery,
) -> Option<Result<DeliveryOutcome, String>> {
    let agent_id = live_runner(app, chat_id).await?;
    Some(deliver_to_agent(app.clone(), &agent_id, delivery).await)
}

pub async fn deliver_to_agent(
    app: AppState,
    agent_id: &str,
    delivery: PendingDelivery,
) -> Result<DeliveryOutcome, String> {
    if crate::tools::tool_agent_planner_qna::is_planner_reply_delivery(&delivery) {
        return Err(
            "Planner replies require authenticated board session acceptance, not a runner".into(),
        );
    }
    if delivery.thread_patch.is_some() {
        return Err(
            "thread parameter overrides are not supported by a live background runner".to_string(),
        );
    }
    let outcome = app.agents.enqueue_delivery(agent_id, delivery).await?;
    publish_runner_queue(&app, agent_id).await;
    Ok(outcome)
}

pub async fn update_runner_delivery(
    app: &AppState,
    chat_id: &str,
    id: &str,
    push: Option<PushMode>,
    cancel: bool,
) -> Option<Result<(), String>> {
    let agent_id = live_runner(app, chat_id).await?;
    let result = app
        .agents
        .update_pending_delivery(&agent_id, id, push, cancel)
        .await;
    if result.is_ok() {
        publish_runner_queue(app, &agent_id).await;
    }
    Some(result)
}

pub async fn publish_runner_queue(app: &AppState, agent_id: &str) {
    let Ok(record) = app.agents.get_any(agent_id).await else {
        return;
    };
    let Some(chat_id) = record.child_chat_id else {
        return;
    };
    let session = app.chat.sessions.read().await.get(&chat_id).cloned();
    if let Some(session) = session {
        let mut session = session.lock().await;
        session.set_runner_pending_deliveries(app.agents.pending_deliveries(agent_id).await);
    }
}

pub async fn recover_runner_deliveries(app: &AppState, chat_id: &str) -> Result<usize, String> {
    let Some(agent_id) = app.agents.find_agent_id_by_child_chat_id(chat_id).await else {
        return Ok(0);
    };
    if app.agents.has_runtime(&agent_id).await {
        return Ok(0);
    }
    let session = app.chat.sessions.read().await.get(chat_id).cloned();
    if let Some(session) = session {
        session
            .lock()
            .await
            .set_runner_pending_deliveries(Vec::new());
    }
    let mut recovered = 0;
    for mut delivery in retain_authorized(
        app,
        &agent_id,
        app.agents.pending_deliveries(&agent_id).await,
    )
    .await
    {
        let id = delivery.id.clone();
        delivery.wake = false;
        let outcome = Box::pin(crate::chat::delivery::deliver_to_chat(
            app.clone(),
            chat_id,
            delivery,
        ))
        .await?;
        match outcome {
            DeliveryOutcome::Queued | DeliveryOutcome::Delivered | DeliveryOutcome::Duplicate => {
                app.agents.consume_delivery(&agent_id, &id).await?;
                recovered += 1;
            }
            DeliveryOutcome::Expired => {
                app.agents
                    .record_delivery_status(
                        &agent_id,
                        &id,
                        refact_chat_api::DeliveryStatus::Expired,
                    )
                    .await?;
            }
            DeliveryOutcome::Superseded => {
                return Err("recovered delivery was superseded inconsistently".to_string());
            }
            DeliveryOutcome::Sealed => {
                app.agents
                    .record_delivery_status(&agent_id, &id, refact_chat_api::DeliveryStatus::Sealed)
                    .await?;
            }
        }
    }
    publish_runner_queue(app, &agent_id).await;
    Ok(recovered)
}
