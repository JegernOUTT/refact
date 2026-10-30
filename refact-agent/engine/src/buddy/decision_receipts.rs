//! Durable native intent receipts. These are snapshot metadata, never model messages.
use crate::chat::types::ChatSession;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MAX_RECEIPTS: usize = 512;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionReceipt {
    pub schema_version: u16,
    pub project_root: String,
    pub engine_identity: String,
    pub chat_id: String,
    pub client_request_id: String,
    pub request_identity: String,
    pub target_id: String,
    pub accepted: Option<bool>,
    pub source_user_id: Option<String>,
    pub source_version: String,
    pub payload_hash: String,
    pub status: String,
    pub followup: String,
    pub reason: Option<String>,
    pub message_id: Option<String>,
    pub version: u64,
    #[serde(default)]
    pub decision_version: u64,
}

pub fn digest(value: &serde_json::Value) -> String {
    format!("{:x}", Sha256::digest(value.to_string().as_bytes()))
}

pub fn find(session: &ChatSession, id: &str) -> Result<Option<DecisionReceipt>, String> {
    for raw in &session.native_decision_receipts {
        // Preserve unknown entries, but never silently treat them as permission to reuse an ID.
        if raw.get("client_request_id").and_then(|v| v.as_str()) == Some(id) {
            return serde_json::from_value(raw.clone())
                .map(Some)
                .map_err(|_| "Receipt schema unavailable; this identity cannot be reused".into());
        }
    }
    Ok(None)
}

pub fn reserve(session: &mut ChatSession, receipt: DecisionReceipt) -> Result<bool, String> {
    if let Some(old) = find(session, &receipt.client_request_id)? {
        if old.payload_hash != receipt.payload_hash || old.project_root != receipt.project_root {
            return Err("Client request identity is already bound to a different decision".into());
        }
        return Ok(false);
    }
    if session.native_decision_receipts.iter().any(|raw| {
        raw.get("request_identity").and_then(|v| v.as_str())
            == Some(receipt.request_identity.as_str())
            && !matches!(
                raw.get("status").and_then(|v| v.as_str()),
                Some("stale" | "rejected")
            )
    }) {
        return Err("This native request already has an immutable user decision".into());
    }
    // Never evict uncertain evidence or permit an old identity to be reused. Capacity is an
    // explicit fail-closed admission bound, not an LRU whose deletion would defeat deduplication.
    if session.native_decision_receipts.len() >= MAX_RECEIPTS {
        return Err("Native receipt retention capacity reached; no evidence was deleted".into());
    }
    session.increment_version();
    let mut receipt = receipt;
    receipt.version = session.trajectory_version;
    session
        .native_decision_receipts
        .push(serde_json::to_value(receipt).map_err(|e| e.to_string())?);
    Ok(true)
}

pub fn transition(session: &mut ChatSession, id: &str, status: &str, reason: Option<String>) {
    let Ok(Some(mut receipt)) = find(session, id) else {
        return;
    };
    if receipt.status != "queued" {
        return;
    }
    session.increment_version();
    receipt.version = session.trajectory_version;
    receipt.decision_version = session.trajectory_version;
    receipt.status = status.into();
    receipt.reason = reason;
    if status == "applied" && receipt.followup == "pending" {
        if session
            .goal
            .as_ref()
            .is_some_and(|goal| goal.active && goal.status == refact_chat_api::GoalStatus::Active)
        {
            session.goal_set_status_reason(
                refact_chat_api::GoalStatus::Paused,
                "user_refusal_followup",
            );
        }
        // Dispatch remains separate and may only occur at a proven safe boundary.
        receipt.message_id = Some(format!(
            "refusal-{}",
            digest(&serde_json::json!([
                receipt.project_root,
                receipt.chat_id,
                receipt.client_request_id,
                receipt.payload_hash
            ]))
        ));
    }
    replace(session, receipt);
}

pub fn replace(session: &mut ChatSession, receipt: DecisionReceipt) {
    if let Some(slot) = session.native_decision_receipts.iter_mut().find(|v| {
        v.get("client_request_id").and_then(|v| v.as_str()) == Some(&receipt.client_request_id)
    }) {
        if let Ok(value) = serde_json::to_value(receipt) {
            *slot = value;
        }
    }
}

pub fn visible(
    session: &ChatSession,
    mut receipt: DecisionReceipt,
    engine: &str,
) -> DecisionReceipt {
    if receipt.version > session.trajectory_committed_version && receipt.followup != "not_requested"
    {
        receipt.followup = "unknown".into();
    }
    let decision_version = if receipt.decision_version == 0 {
        receipt.version
    } else {
        receipt.decision_version
    };
    if decision_version > session.trajectory_committed_version {
        receipt.status = "unknown".into();
        receipt.reason = Some("Outcome has not been durably committed".into());
    } else if receipt.status == "queued" && receipt.engine_identity != engine {
        receipt.status = "unknown".into();
        receipt.reason = Some(
            "Engine restarted before an authoritative outcome; never automatically replayed".into(),
        );
    }
    if receipt.followup == "pending" && receipt.engine_identity != engine {
        receipt.followup = "unavailable".into();
        receipt.reason =
            Some("Restored followup was not dispatched; explicit new interaction required".into());
    }
    if receipt.followup == "pending"
        && (session.closed
            || session.abort_flag.load(std::sync::atomic::Ordering::SeqCst)
            || session.goal.as_ref().is_some_and(|g| {
                matches!(
                    g.status,
                    refact_chat_api::GoalStatus::Stopped
                        | refact_chat_api::GoalStatus::Completed
                        | refact_chat_api::GoalStatus::Transferred
                )
            }))
    {
        receipt.followup = "unavailable".into();
        receipt.reason = Some("Requesting source stopped or cancelled".into());
    }
    if receipt.followup == "dispatched" && receipt.engine_identity != engine {
        receipt.followup = "unknown".into();
    }
    receipt
}

/// Rebase only receipts loaded from an authoritative snapshot. Live unsaved receipts must never
/// pass through this path; their own version remains subject to commit acknowledgement.
pub fn restored(mut values: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
    for value in &mut values {
        if let Ok(mut receipt) = serde_json::from_value::<DecisionReceipt>(value.clone()) {
            receipt.version = 0;
            receipt.decision_version = 0;
            if let Ok(raw) = serde_json::to_value(receipt) {
                *value = raw;
            }
        }
    }
    values
}
