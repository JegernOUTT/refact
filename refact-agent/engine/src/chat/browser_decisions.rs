use super::types::PendingBrowserMessage;
pub(crate) use refact_chat_history::trajectory_snapshot::PendingBrowserDecisionSnapshot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StaleBrowserDecision {
    NoPending,
    Mismatched {
        expected_pending_message_id: String,
        received_pending_message_id: String,
    },
}

pub(crate) fn take_matching_browser_message(
    pending: &mut Option<PendingBrowserMessage>,
    pending_message_id: &str,
) -> Result<PendingBrowserMessage, StaleBrowserDecision> {
    let Some(current) = pending.as_ref() else {
        return Err(StaleBrowserDecision::NoPending);
    };
    if current.pending_message_id != pending_message_id {
        return Err(StaleBrowserDecision::Mismatched {
            expected_pending_message_id: current.pending_message_id.clone(),
            received_pending_message_id: pending_message_id.to_string(),
        });
    }
    pending.take().ok_or(StaleBrowserDecision::NoPending)
}

pub(crate) fn cancel_pending_browser_message(
    pending: &mut Option<PendingBrowserMessage>,
) -> Option<PendingBrowserMessage> {
    pending.take()
}

pub(crate) fn snapshot_pending_browser_decision(
    pending: &PendingBrowserMessage,
    browser_runtime_id: Option<String>,
) -> PendingBrowserDecisionSnapshot {
    PendingBrowserDecisionSnapshot {
        version: 1,
        pending_message_id: pending.pending_message_id.clone(),
        client_message_id: pending.client_message_id.clone(),
        browser_runtime_id,
        requires_revalidation: true,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RestoredBrowserDecision {
    Expired {
        pending_message_id: String,
        client_message_id: Option<String>,
    },
}

pub(crate) fn restore_browser_decision(
    snapshot: PendingBrowserDecisionSnapshot,
) -> RestoredBrowserDecision {
    RestoredBrowserDecision::Expired {
        pending_message_id: snapshot.pending_message_id,
        client_message_id: snapshot.client_message_id,
    }
}

pub(crate) fn expire_restored_browser_decision(
    session: &mut super::types::ChatSession,
    snapshot: PendingBrowserDecisionSnapshot,
) {
    let RestoredBrowserDecision::Expired {
        pending_message_id,
        client_message_id,
    } = restore_browser_decision(snapshot);
    session.pending_browser_message = None;
    session.add_message(crate::chat::internal_roles::event(
        crate::chat::internal_roles::EventSubkind::SystemNotice,
        "chat.browser_decisions",
        serde_json::json!({
            "kind": "browser_decision_expired",
            "pending_message_id": pending_message_id,
            "client_message_id": client_message_id,
        }),
        "The pending browser-context decision expired after the chat was restored. No browser consent was replayed. Resend your message to choose fresh context.".to_string(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pending(id: &str) -> PendingBrowserMessage {
        PendingBrowserMessage {
            pending_message_id: id.to_string(),
            content: json!("send this once context is selected"),
            attachments: vec![],
            client_message_id: Some("optimistic-1".to_string()),
            checkpoints: vec![],
            context_files: vec![],
            suppress_auto_enrichment: false,
            skill_activation_name: None,
            skill_context_msg: None,
        }
    }

    #[test]
    fn stale_id_leaves_pending_message_intact() {
        let mut slot = Some(pending("fresh"));

        let result = take_matching_browser_message(&mut slot, "stale");

        assert!(matches!(
            result,
            Err(StaleBrowserDecision::Mismatched { .. })
        ));
        assert_eq!(slot.as_ref().unwrap().pending_message_id, "fresh");
    }

    #[test]
    fn duplicate_response_is_rejected_after_first_consumes() {
        let mut slot = Some(pending("once"));
        assert!(take_matching_browser_message(&mut slot, "once").is_ok());

        assert!(matches!(
            take_matching_browser_message(&mut slot, "once"),
            Err(StaleBrowserDecision::NoPending)
        ));
    }

    #[test]
    fn cancellation_removes_pending_message() {
        let mut slot = Some(pending("cancel-me"));

        let cancelled = cancel_pending_browser_message(&mut slot);

        assert_eq!(cancelled.unwrap().pending_message_id, "cancel-me");
        assert!(slot.is_none());
    }

    #[test]
    fn expired_browser_decision_leaves_a_notice_without_replaying_the_message() {
        let mut session = super::super::types::ChatSession::new("restored".into());
        let snapshot = snapshot_pending_browser_decision(&pending("expired"), None);

        expire_restored_browser_decision(&mut session, snapshot);

        assert!(session.pending_browser_message.is_none());
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.messages[0].role, "event");
        assert_eq!(
            session.messages[0].extra["event"]["payload"]["pending_message_id"],
            "expired"
        );
        assert!(session.messages[0]
            .content
            .content_text_only()
            .contains("Resend your message"));
        assert!(!session.messages[0]
            .content
            .content_text_only()
            .contains("send this once"));
    }

    #[test]
    fn restart_snapshot_roundtrips_but_never_replays_browser_decision() {
        let snapshot = snapshot_pending_browser_decision(
            &pending("restart"),
            Some("environment-tab-id".to_string()),
        );
        let serialized = serde_json::to_string(&snapshot).unwrap();
        let restored: PendingBrowserDecisionSnapshot = serde_json::from_str(&serialized).unwrap();

        assert!(restored.requires_revalidation);
        assert_eq!(
            restored.browser_runtime_id.as_deref(),
            Some("environment-tab-id")
        );
        assert_eq!(
            restore_browser_decision(restored),
            RestoredBrowserDecision::Expired {
                pending_message_id: "restart".to_string(),
                client_message_id: Some("optimistic-1".to_string()),
            }
        );
    }
}
