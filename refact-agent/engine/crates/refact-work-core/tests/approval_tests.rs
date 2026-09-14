use refact_work_core::{ApprovalDecision, ApprovalError, ApprovalRegistry, PendingApproval};
use serde_json::json;

fn pending(id: &str) -> PendingApproval {
    PendingApproval {
        id: id.into(),
        child_work_id: "child".into(),
        parent_chat_id: "parent".into(),
        run_generation: 7,
        tool_call_id: "call".into(),
        tool_name: "shell".into(),
        args: json!({"x": 1}),
        digest: None,
        created_at: 0,
        expires_at: 10,
        execution: None,
    }
}

#[test]
fn owner_and_generation_are_checked() {
    let r = ApprovalRegistry::new(4);
    r.register(pending("a"), 0).unwrap();
    assert_eq!(
        r.decide("other", 7, "a", true, 1),
        Err(ApprovalError::NotOwner)
    );
    assert_eq!(
        r.decide("parent", 8, "a", true, 1),
        Err(ApprovalError::GenerationMismatch)
    );
    assert_eq!(
        r.decide("parent", 7, "a", true, 1),
        Ok(ApprovalDecision::Approved)
    );
}

#[test]
fn expiry_blocks_decision() {
    let r = ApprovalRegistry::new(4);
    r.register(pending("a"), 0).unwrap();
    r.expire(10);
    assert_eq!(
        r.decide("parent", 7, "a", true, 10),
        Err(ApprovalError::Expired)
    );
}

#[test]
fn consume_checks_binding() {
    let r = ApprovalRegistry::new(4);
    r.register(pending("a"), 0).unwrap();
    r.decide("parent", 7, "a", true, 1).unwrap();
    assert_eq!(
        r.consume("a", "wrong", &json!({"x": 1}), 2),
        Err(ApprovalError::BindingMismatch)
    );
    assert_eq!(
        r.consume("a", "call", &json!({"x": 2}), 2),
        Err(ApprovalError::BindingMismatch)
    );
}

#[test]
fn duplicate_decisions_are_idempotent_but_conflicts_rejected() {
    let r = ApprovalRegistry::new(4);
    r.register(pending("a"), 0).unwrap();
    assert_eq!(
        r.decide("parent", 7, "a", true, 1),
        Ok(ApprovalDecision::Approved)
    );
    assert_eq!(
        r.decide("parent", 7, "a", true, 2),
        Ok(ApprovalDecision::Approved)
    );
    assert_eq!(
        r.decide("parent", 7, "a", false, 2),
        Err(ApprovalError::AlreadyDecided)
    );
}

#[test]
fn consume_is_once() {
    let r = ApprovalRegistry::new(4);
    r.register(pending("a"), 0).unwrap();
    r.decide("parent", 7, "a", true, 1).unwrap();
    assert_eq!(r.consume("a", "call", &json!({"x": 1}), 2).unwrap().id, "a");
    assert_eq!(
        r.consume("a", "call", &json!({"x": 1}), 2),
        Err(ApprovalError::Consumed)
    );
}

#[test]
fn cancel_and_capacity() {
    let r = ApprovalRegistry::new(1);
    r.register(pending("a"), 0).unwrap();
    let mut second = pending("b");
    second.tool_call_id = "another-call".into();
    assert_eq!(r.register(second, 0), Err(ApprovalError::Capacity));
    r.cancel("parent", 7, "a", 1).unwrap();
    assert_eq!(r.wait_now("a", 1), Ok(ApprovalDecision::Cancelled));
}

#[tokio::test]
async fn wait_observes_decision() {
    let r = ApprovalRegistry::new(2);
    r.register(pending("a"), 0).unwrap();
    let waiter = {
        let r = r.clone();
        tokio::spawn(async move { r.wait("a", 1).await })
    };
    r.decide("parent", 7, "a", false, 2).unwrap();
    assert_eq!(waiter.await.unwrap(), Ok(ApprovalDecision::Rejected));
}

#[tokio::test(start_paused = true)]
async fn wait_expires_without_external_expire() {
    let r = ApprovalRegistry::new(1);
    r.register(pending("a"), 0).unwrap();
    let waiter = {
        let r = r.clone();
        tokio::spawn(async move { r.wait("a", 0).await })
    };
    tokio::task::yield_now().await;
    tokio::time::advance(std::time::Duration::from_millis(10)).await;
    assert_eq!(waiter.await.unwrap(), Ok(ApprovalDecision::Expired));
}

#[test]
fn approval_at_deadline_cannot_be_consumed() {
    let r = ApprovalRegistry::new(1);
    r.register(pending("a"), 0).unwrap();
    assert_eq!(
        r.decide("parent", 7, "a", true, 9),
        Ok(ApprovalDecision::Approved)
    );
    assert_eq!(
        r.consume("a", "call", &json!({"x": 1}), 10),
        Err(ApprovalError::Expired)
    );
}

#[test]
fn parent_listing_filters_owners() {
    let mut other = pending("b");
    other.parent_chat_id = "other".into();
    let r = ApprovalRegistry::new(3);
    r.register(pending("a"), 0).unwrap();
    r.register(other, 0).unwrap();
    assert_eq!(r.get("parent", "a", 1).unwrap().id, "a");
    assert_eq!(r.get("other", "a", 1), Err(ApprovalError::NotOwner));
    assert_eq!(
        r.list_for_parent("parent", 1)
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        vec!["a"]
    );
}
