














fn legacy_fixture_draft() -> crate::buddy::types::BuddyDraft {
    let now = chrono::Utc::now();
    crate::buddy::types::BuddyDraft {
        id: "editor-1".into(),
        kind: crate::buddy::types::DraftKind::AgentsMd,
        title: "Durable proposal".into(),
        yaml_or_json: "# Proposed instructions\nKeep this after editor expiry.".into(),
        explanation: "User review required".into(),
        created_at: now - chrono::Duration::days(2),
        expires_at: now - chrono::Duration::days(1),
    }
}

#[test]
fn legacy_migration_survives_expired_editor_restart_and_preserves_attention() {
    use crate::buddy::{legacy_work, operator_store::OperatorStore};
    use refact_buddy_core::operator_types::{ProjectIdentity, WorkerIdentity};
    let root = tempfile::tempdir().unwrap();
    let old_file = root.path().join("old-draft.json");
    let draft = legacy_fixture_draft();
    let old_bytes = serde_json::to_vec(&draft).unwrap();
    std::fs::write(&old_file, &old_bytes).unwrap();
    let project = ProjectIdentity {
        project_id: "migration".into(),
        canonical_root: root
            .path()
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
    };
    let worker = WorkerIdentity {
        worker_id: "worker".into(),
        engine_instance_id: "engine".into(),
        epoch: 1,
    };
    let mut store = OperatorStore::open(root.path(), &project).unwrap();
    assert_eq!(
        legacy_work::migrate_batch(&mut store, &worker, &[], &[draft.clone()])
            .unwrap()
            .imported,
        1
    );
    let work = store
        .list_work_items(None, 20)
        .unwrap()
        .entries
        .pop()
        .unwrap();
    let artifact_id = work.value.current_artifact_id.clone().unwrap();
    store
        .mark_work_read(&work.value.id, &artifact_id, 1)
        .unwrap();
    store
        .snooze_work(&work.value.id, &artifact_id, 1, Some(60))
        .unwrap();
    let read = store.get_read_state(&work.value.id).unwrap();
    let snooze = store.get_work_snooze(&work.value.id).unwrap();
    drop(store);
    let mut store = OperatorStore::open(root.path(), &project).unwrap();
    assert_eq!(
        legacy_work::migrate_batch(&mut store, &worker, &[], &[draft.clone()])
            .unwrap()
            .already_imported,
        1
    );
    assert_eq!(store.get_work_item(&work.value.id).unwrap().unwrap(), work);
    assert_eq!(store.get_work_snooze(&work.value.id).unwrap(), snooze);
    assert_eq!(
        legacy_work::migrate_batch(&mut store, &worker, &[], &[draft.clone()])
            .unwrap()
            .already_imported,
        1
    );
    assert_eq!(store.get_work_item(&work.value.id).unwrap().unwrap(), work);
    assert_eq!(store.get_work_snooze(&work.value.id).unwrap(), snooze);
    let mut changed_legacy = draft;
    changed_legacy.yaml_or_json = "Changed proposal remains unverified".into();
    assert_eq!(
        legacy_work::migrate_batch(&mut store, &worker, &[], &[changed_legacy])
            .unwrap()
            .imported,
        1
    );
    let revised = store.get_work_item(&work.value.id).unwrap().unwrap();
    assert_eq!(revised.value.current_artifact_revision, Some(2));
    assert_eq!(revised.value.state, work.value.state);
    assert_eq!(store.list_work_items(None, 20).unwrap().entries.len(), 1);
    assert_eq!(
        store
            .list_work_outcomes(&work.value.id, None, 20)
            .unwrap()
            .entries
            .len(),
        2
    );
    assert_eq!(store.get_read_state(&work.value.id).unwrap(), read);
    assert!(store.get_work_snooze(&work.value.id).unwrap().is_none());
    let artifact = store
        .get_artifact_revision(&artifact_id, Some(1))
        .unwrap()
        .unwrap();
    assert!(artifact
        .value
        .body
        .contains("Keep this after editor expiry"));
    assert!(artifact.value.proposed_action_ids.is_empty());
    assert_eq!(std::fs::read(old_file).unwrap(), old_bytes);
    let projection = legacy_work::projection_page(&store, None, 20).unwrap().0;
    assert_eq!(projection.len(), 1);
    assert_eq!(
        projection[0].status,
        crate::buddy::types::OpportunityStatus::New
    );
}

#[test]
fn linked_legacy_card_and_expired_editor_share_one_durable_proposal() {
    use crate::buddy::{legacy_work, operator_store::OperatorStore};
    use crate::buddy::types::{
        BuddyAction, BuddyOpportunity, BuddyOpportunityKind, BuddyOpportunityLinks, BuddyPriority,
        OpportunityStatus,
    };
    use refact_buddy_core::operator_types::{ProjectIdentity, WorkerIdentity};
    let root = tempfile::tempdir().unwrap();
    let project = ProjectIdentity {
        project_id: "linked".into(),
        canonical_root: root
            .path()
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
    };
    let worker = WorkerIdentity {
        worker_id: "worker".into(),
        engine_instance_id: "engine".into(),
        epoch: 1,
    };
    let draft = legacy_fixture_draft();
    let card = BuddyOpportunity {
        id: "card-1".into(),
        kind: BuddyOpportunityKind::AgentsMdGap,
        summary: "Linked draft".into(),
        priority: BuddyPriority::Normal,
        confidence: 0.8,
        fact_keys: vec![],
        cooldown_key: "linked".into(),
        cooldown_secs: 60,
        status: OpportunityStatus::Expired,
        proposed_actions: vec![BuddyAction::ApplyConfigPatch {
            draft_id: draft.id.clone(),
            target_path: "AGENTS.md".into(),
        }],
        humor: None,
        humor_allowed: false,
        related: BuddyOpportunityLinks::default(),
        created_at: draft.created_at,
        expires_at: draft.expires_at,
        resolved_at: None,
    };
    let mut store = OperatorStore::open(root.path(), &project).unwrap();
    legacy_work::migrate_batch(&mut store, &worker, &[card.clone()], &[draft]).unwrap();
    legacy_work::migrate_batch(&mut store, &worker, &[card], &[]).unwrap();
    let page = store.list_work_items(None, 20).unwrap();
    assert_eq!(page.entries.len(), 1);
    let work = &page.entries[0];
    assert_eq!(
        work.value.state,
        refact_buddy_core::operator_types::WorkItemState::NeedsAttention
    );
    let artifact = store
        .get_artifact_revision(work.value.current_artifact_id.as_ref().unwrap(), Some(1))
        .unwrap()
        .unwrap();
    assert!(artifact
        .value
        .body
        .contains("Keep this after editor expiry"));
    assert_eq!(
        legacy_work::projection_page(&store, None, 20)
            .unwrap()
            .0
            .len(),
        1
    );
}

#[test]
fn oversized_migration_batch_rejects_before_any_write() {
    use crate::buddy::{legacy_work, operator_store::OperatorStore};
    use refact_buddy_core::operator_types::{ProjectIdentity, WorkerIdentity};
    let root = tempfile::tempdir().unwrap();
    let project = ProjectIdentity {
        project_id: "bounded".into(),
        canonical_root: root
            .path()
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
    };
    let worker = WorkerIdentity {
        worker_id: "worker".into(),
        engine_instance_id: "engine".into(),
        epoch: 1,
    };
    let mut store = OperatorStore::open(root.path(), &project).unwrap();
    assert!(legacy_work::migrate_batch(
        &mut store,
        &worker,
        &[],
        &vec![legacy_fixture_draft(); legacy_work::MAX_MIGRATION_BATCH + 1]
    )
    .is_err());
    assert!(store.list_work_items(None, 20).unwrap().entries.is_empty());
}
