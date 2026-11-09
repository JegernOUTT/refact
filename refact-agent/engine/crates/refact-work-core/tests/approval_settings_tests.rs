use refact_work_core::{
    coordination_settings, install_coordination_settings, ApprovalDecision, ApprovalError,
    ApprovalRegistry, CoordinationSettings, PendingApproval,
};

struct RestoreSettings(CoordinationSettings);

impl Drop for RestoreSettings {
    fn drop(&mut self) {
        install_coordination_settings(self.0.clone()).unwrap();
    }
}

fn request(id: &str) -> PendingApproval {
    PendingApproval {
        id: id.into(),
        child_work_id: "child".into(),
        parent_chat_id: "parent".into(),
        run_generation: 1,
        tool_call_id: id.into(),
        tool_name: "shell".into(),
        args: serde_json::json!({"command":"echo accepted"}),
        requires_human: false,
        digest: None,
        created_at: 0,
        expires_at: 100,
        execution: None,
    }
}

#[test]
fn settings_backed_approval_capacity_changes_without_discarding_accepted_requests() {
    let _restore = RestoreSettings(coordination_settings());
    let registry = ApprovalRegistry::from_settings();
    let shared = registry.clone();
    let mut settings = coordination_settings();
    settings.max_pending_approvals = 2;
    install_coordination_settings(settings.clone()).unwrap();
    registry.register(request("a"), 0).unwrap();
    registry.register(request("b"), 0).unwrap();
    assert_eq!(
        shared.register(request("c"), 0),
        Err(ApprovalError::Capacity)
    );
    settings.max_pending_approvals = 1;
    install_coordination_settings(settings.clone()).unwrap();
    assert_eq!(
        registry.register(request("a"), 1),
        Err(ApprovalError::Duplicate)
    );
    assert_eq!(registry.len(), 2);
    assert_eq!(
        shared.register(request("c"), 1),
        Err(ApprovalError::Capacity)
    );
    assert_eq!(
        registry.decide("parent", 1, "a", true, 1),
        Ok(ApprovalDecision::Approved)
    );
    shared.consume("a", "a", &request("a").args, 2).unwrap();
    assert_eq!(
        shared.decide("parent", 1, "a", true, 3),
        Ok(ApprovalDecision::Approved)
    );
    shared.cancel("parent", 1, "b", 3).unwrap();
    assert_eq!(registry.wait_now("b", 3), Ok(ApprovalDecision::Cancelled));
    settings.max_pending_approvals = 3;
    install_coordination_settings(settings.clone()).unwrap();
    shared.register(request("c"), 4).unwrap();
    let rows = registry.list_for_parent("parent", 4);
    assert_eq!(
        rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
    assert!(rows.iter().all(|row| row.expires_at == 100));
    assert_eq!(
        registry.consume("a", "a", &request("a").args, 5),
        Err(ApprovalError::Consumed)
    );
    settings.max_pending_approvals = 0;
    assert!(install_coordination_settings(settings).is_err());
    assert_eq!(coordination_settings().max_pending_approvals, 3);
    registry.expire(100);
    let mut next = request("next");
    next.expires_at = 200;
    registry.register(next, 100).unwrap();
}
