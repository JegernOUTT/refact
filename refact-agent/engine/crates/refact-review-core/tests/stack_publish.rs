use refact_review_core::*;

fn fixture() -> (tempfile::TempDir, ReviewStore, StackManifest) {
    let root = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(root.path()).unwrap();
    let tree = repo
        .find_tree(repo.treebuilder(None).unwrap().write().unwrap())
        .unwrap();
    let sig = git2::Signature::now("Test", "test@example.invalid").unwrap();
    let oid = repo
        .commit(Some("HEAD"), &sig, &sig, "fixture", &tree, &[])
        .unwrap()
        .to_string();
    let store = ReviewStore::open(root.path()).unwrap();
    let ids: Vec<_> = (1..=2)
        .map(|n| format!("00000000-0000-4000-8000-{n:012}"))
        .collect();
    let manifest = StackManifest {
        schema_version: SCHEMA_VERSION,
        id: "stack".into(),
        owner_chat_id: "owner".into(),
        revision: 0,
        title: "Stack".into(),
        repository: root.path().to_str().unwrap().into(),
        source: StackSource {
            kind: "provided_manifest".into(),
            reference: Some("fixture".into()),
        },
        root_base_ref: oid.clone(),
        root_base_oid: Some(oid.clone()),
        scope_choice: StackScopeChoice {
            scope: StackScope::Whole,
            selected_member_id: ids[0].clone(),
            confirmed: true,
            explicit_request: true,
        },
        members: ids
            .into_iter()
            .enumerate()
            .map(|(i, id)| StackMember {
                id,
                branch_ref: format!("branch-{i}"),
                pr_identity: None,
                parent_member_id: None,
                declared_base_ref: oid.clone(),
                resolved_base_oid: Some(oid.clone()),
                head_oid: Some(oid.clone()),
                review_id: None,
                state: StackMemberState::Ready,
                reason: None,
            })
            .collect(),
    };
    (root, store, manifest)
}
fn capture(store: &ReviewStore, m: &StackManifest) -> Vec<(String, Candidate)> {
    m.members
        .iter()
        .filter(|member| {
            m.scope_choice.scope == StackScope::Whole
                || m.scope_choice.selected_member_id == member.id
        })
        .map(|member| {
            (
                member.id.clone(),
                store
                    .capture(
                        std::path::Path::new(&m.repository),
                        CaptureRequest {
                            base_oid: member.resolved_base_oid.clone().unwrap(),
                            head_oid: member.head_oid.clone().unwrap(),
                            policy: CapturePolicy::default(),
                            origin: Origin::Imported,
                        },
                    )
                    .unwrap(),
            )
        })
        .collect()
}
fn assert_empty(store: &ReviewStore) {
    assert!(store.get_stack_for_chat("owner").unwrap().is_none());
    assert!(store.list_for_chat("owner").unwrap().is_empty());
    assert!(!store.stack_scope_confirmed("owner").unwrap());
}

#[test]
fn superseding_preparation_between_capture_and_publish_rejects_without_partial_state() {
    let (_root, store, m) = fixture();
    let legacy = store
        .create("owner", "Legacy", Origin::Local, "legacy")
        .unwrap();
    assert!(store.stack_scope_confirmed("owner").unwrap());
    store.put_stack_preparation("owner", "A", &m).unwrap();
    let staged = capture(&store, &m);
    store.put_stack_preparation("owner", "B", &m).unwrap();
    assert!(matches!(
        store.publish_stack_import(m, "A", "publish", staged),
        Err(Error::Invalid(_))
    ));
    assert!(store.get_stack_for_chat("owner").unwrap().is_none());
    assert_eq!(store.list_for_chat("owner").unwrap(), vec![legacy.clone()]);
    assert_eq!(store.get_for_chat("owner").unwrap(), Some(legacy));
    assert!(!store.stack_scope_confirmed("owner").unwrap());
}

#[test]
fn retry_normalizes_capture_identity_but_fresh_same_id_preparation_blocks_even_replay() {
    let (_root, store, m) = fixture();
    let legacy = store
        .create("owner", "Legacy", Origin::Local, "legacy")
        .unwrap();
    store.put_stack_preparation("owner", "A", &m).unwrap();
    let first = capture(&store, &m);
    let response = store
        .publish_stack_import(m.clone(), "A", "publish", first.clone())
        .unwrap();
    let retry = capture(&store, &m);
    assert_ne!(first[0].1.id, retry[0].1.id);
    assert_eq!(
        store
            .publish_stack_import(m.clone(), "A", "publish", retry)
            .unwrap(),
        response
    );
    assert!(store.stack_scope_confirmed("owner").unwrap());
    assert_eq!(store.get_for_chat("owner").unwrap(), Some(legacy.clone()));
    for member in &response.members {
        let review = store.load(member.review_id.as_ref().unwrap()).unwrap();
        assert_eq!(review.candidates.len(), 1);
        assert_eq!(review.materials.len(), 1);
    }
    store.put_stack_preparation("owner", "B", &m).unwrap();
    assert!(!store.stack_scope_confirmed("owner").unwrap());
    assert!(store
        .publish_stack_import(m, "A", "publish", first)
        .is_err());
    assert!(store
        .set_active_review(
            "owner",
            response.members[0].review_id.as_ref().unwrap(),
            Some(&legacy.id),
            "activate"
        )
        .is_err());
    assert_eq!(store.get_for_chat("owner").unwrap(), Some(legacy));
}

#[test]
fn later_member_sql_failure_rolls_back_all_journals_bindings_confirmation_and_key() {
    let (root, store, m) = fixture();
    store.put_stack_preparation("owner", "A", &m).unwrap();
    let staged = capture(&store, &m);
    let db =
        rusqlite::Connection::open(root.path().join(".refact/reviews/reviews.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER fail_second BEFORE UPDATE ON reviews WHEN json_extract(NEW.document,'$.title')='branch-1' BEGIN SELECT RAISE(ABORT,'later member failpoint'); END;").unwrap();
    assert!(matches!(
        store.publish_stack_import(m.clone(), "A", "publish", staged.clone()),
        Err(Error::Sql(_))
    ));
    assert_empty(&store);
    for table in [
        "stacks",
        "stack_revisions",
        "requests",
        "stack_requests",
        "stack_confirmations",
        "active_reviews",
    ] {
        assert_eq!(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            0,
            "{table}"
        );
    }
    db.execute_batch("DROP TRIGGER fail_second").unwrap();
    store
        .publish_stack_import(m, "A", "publish", staged)
        .unwrap();
    assert!(store.stack_scope_confirmed("owner").unwrap());
}

#[test]
fn owner_identity_and_exact_scope_capture_set_are_required() {
    let (_root, store, m) = fixture();
    store.put_stack_preparation("owner", "A", &m).unwrap();
    let staged = capture(&store, &m);
    let mut wrong = m.clone();
    wrong.owner_chat_id = "other".into();
    assert!(store
        .publish_stack_import(wrong, "A", "wrong", staged.clone())
        .is_err());
    assert!(store
        .publish_stack_import(m.clone(), "A", "missing", staged[..1].to_vec())
        .is_err());
    let mut duplicate = staged.clone();
    duplicate.push(staged[0].clone());
    assert!(store
        .publish_stack_import(m.clone(), "A", "duplicate", duplicate)
        .is_err());
    let mut single = m.clone();
    single.scope_choice.scope = StackScope::Single;
    assert!(store
        .publish_stack_import(single.clone(), "A", "extra", staged.clone())
        .is_err());
    let mut changed = m.clone();
    changed.title = "not prepared".into();
    assert!(store
        .publish_stack_import(changed, "A", "changed", staged.clone())
        .is_err());
    assert_empty(&store);
    let response = store
        .publish_stack_import(single, "A", "single", staged[..1].to_vec())
        .unwrap();
    assert!(response.members[0].review_id.is_some());
    assert!(response.members[1].review_id.is_none());
}

#[test]
fn refresh_is_atomic_retries_without_duplicates_and_preserves_execution_slot() {
    let (root, store, m) = fixture();
    store.put_stack_preparation("owner", "A", &m).unwrap();
    let published = store
        .publish_stack_import(m.clone(), "A", "import", capture(&store, &m))
        .unwrap();
    let active_id = published.members[0].review_id.clone().unwrap();
    store
        .set_active_review("owner", &active_id, None, "activate")
        .unwrap();
    let before = store.list_for_chat("owner").unwrap();
    let repo = git2::Repository::open(root.path()).unwrap();
    let tree = repo
        .find_tree(repo.treebuilder(None).unwrap().write().unwrap())
        .unwrap();
    let sig = git2::Signature::now("Test", "test@example.invalid").unwrap();
    let parent = repo.head().unwrap().peel_to_commit().unwrap();
    let head = repo
        .commit(None, &sig, &sig, "next", &tree, &[&parent])
        .unwrap()
        .to_string();
    let mut next = published.clone();
    for member in &mut next.members {
        member.head_oid = Some(head.clone());
    }
    let candidates = capture(&store, &next);
    let db =
        rusqlite::Connection::open(root.path().join(".refact/reviews/reviews.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER fail_refresh BEFORE UPDATE ON reviews WHEN json_extract(NEW.document,'$.title')='branch-1' BEGIN SELECT RAISE(ABORT,'refresh failpoint'); END;").unwrap();
    assert!(matches!(
        store.publish_stack_refresh(next.clone(), 0, "refresh", candidates.clone()),
        Err(Error::Sql(_))
    ));
    assert_eq!(store.load_stack("stack").unwrap(), published);
    assert_eq!(store.list_for_chat("owner").unwrap(), before);
    db.execute_batch("DROP TRIGGER fail_refresh").unwrap();
    let updated = store
        .publish_stack_refresh(next.clone(), 0, "refresh", candidates)
        .unwrap();
    assert_eq!(
        store
            .publish_stack_refresh(next.clone(), 0, "refresh", capture(&store, &next))
            .unwrap(),
        updated
    );
    assert_eq!(store.get_for_chat("owner").unwrap().unwrap().id, active_id);
    for member in &updated.members {
        let review = store.load(member.review_id.as_ref().unwrap()).unwrap();
        assert_eq!(review.candidates.len(), 2);
        assert_eq!(review.materials.len(), 2);
        assert_eq!(
            review.candidates[0],
            before
                .iter()
                .find(|r| r.id == review.id)
                .unwrap()
                .candidates[0]
        );
    }
    let unchanged = store
        .publish_stack_refresh(updated.clone(), 1, "unchanged", capture(&store, &updated))
        .unwrap();
    assert_eq!(unchanged.revision, 2);
    assert_eq!(
        store
            .replay_stack_refresh("owner", "stack", "refresh", 0)
            .unwrap(),
        Some(updated.clone())
    );
    assert!(matches!(
        store.replay_stack_refresh("owner", "stack", "refresh", 1),
        Err(Error::IdempotencyConflict)
    ));
    assert!(store
        .replay_stack_refresh("other", "stack", "refresh", 0)
        .is_err());
    assert_eq!(
        store
            .replay_stack_refresh("owner", "stack", "new-key", 0)
            .unwrap(),
        None
    );
    for review in store.list_for_chat("owner").unwrap() {
        assert_eq!(review.candidates.len(), 2);
    }
}
