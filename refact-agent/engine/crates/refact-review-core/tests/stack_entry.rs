use refact_review_core::*;

fn discovery(repository: &str) -> StackManifest {
    StackManifest {
        schema_version: SCHEMA_VERSION,
        id: uuid::Uuid::new_v4().to_string(),
        owner_chat_id: "owner".into(),
        revision: 0,
        title: "Recorded merged PRs".into(),
        repository: dunce::canonicalize(repository)
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        source: StackSource {
            kind: "provided_manifest".into(),
            reference: Some("Explicit recorded comparison evidence".into()),
        },
        root_base_ref: String::new(),
        root_base_oid: None,
        scope_choice: StackScopeChoice {
            scope: StackScope::Single,
            selected_member_id: "00000000-0000-4000-8000-000000000001".into(),
            confirmed: false,
            explicit_request: false,
        },
        members: (1..=2)
            .map(|n| StackMember {
                id: format!("00000000-0000-4000-8000-{n:012}"),
                branch_ref: format!("missing-peer-{n}"),
                pr_identity: None,
                parent_member_id: if n == 2 {
                    Some("00000000-0000-4000-8000-000000000001".into())
                } else {
                    None
                },
                declared_base_ref: String::new(),
                resolved_base_oid: None,
                head_oid: None,
                review_id: None,
                state: StackMemberState::Missing,
                reason: Some(format!("entry:recorded_comparisons:member-{n}")),
            })
            .collect(),
    }
}
fn fixture() -> (tempfile::TempDir, ReviewStore, String) {
    let base = std::env::temp_dir();
    #[cfg(unix)]
    let base = base.canonicalize().unwrap();
    let root = tempfile::tempdir_in(base).unwrap();
    let repo = git2::Repository::init(root.path()).unwrap();
    let sig = git2::Signature::now("Test", "test@localhost").unwrap();
    let tree = repo
        .find_tree(repo.treebuilder(None).unwrap().write().unwrap())
        .unwrap();
    let oid = repo
        .commit(None, &sig, &sig, "fixture", &tree, &[])
        .unwrap()
        .to_string();
    let store = ReviewStore::open(root.path()).unwrap();
    (root, store, oid)
}
fn comparison(store: &ReviewStore, root: &std::path::Path, oid: &str) -> Candidate {
    store
        .capture(
            root,
            CaptureRequest {
                base_oid: oid.into(),
                head_oid: oid.into(),
                policy: CapturePolicy::default(),
                origin: Origin::Imported,
            },
        )
        .unwrap()
}
fn acquired(mut m: StackManifest, oid: &str, whole: bool) -> StackManifest {
    m.scope_choice.confirmed = true;
    m.scope_choice.explicit_request = true;
    if whole {
        m.scope_choice.scope = StackScope::Whole;
    }
    for (i, member) in m.members.iter_mut().enumerate() {
        if whole || i == 0 {
            member.declared_base_ref = oid.into();
            member.resolved_base_oid = Some(oid.into());
            member.head_oid = Some(oid.into());
            member.state = StackMemberState::Ready;
        }
    }
    m
}
#[test]
fn single_import_ignores_inaccessible_peer_and_preserves_active_execution_and_retry() {
    let (root, store, oid) = fixture();
    let legacy = store
        .create("owner", "Previous", Origin::Local, "previous")
        .unwrap();
    let m = discovery(&root.path().to_string_lossy());
    store
        .put_stack_preparation("owner", "entry:nonce", &m)
        .unwrap();
    let ready = acquired(m.clone(), &oid, false);
    let candidate = comparison(&store, root.path(), &oid);
    let input = vec![(ready.members[0].id.clone(), candidate)];
    let published = store
        .publish_stack_entry_import(ready.clone(), "entry:nonce", "import", input.clone())
        .unwrap();
    assert_eq!(published.members[1], m.members[1]);
    assert_eq!(store.get_for_chat("owner").unwrap(), Some(legacy.clone()));
    assert_eq!(store.load(&legacy.id).unwrap(), legacy);
    assert_eq!(
        store
            .publish_stack_entry_import(ready.clone(), "entry:nonce", "import", input.clone())
            .unwrap(),
        published
    );
    let review = store
        .load(published.members[0].review_id.as_ref().unwrap())
        .unwrap();
    assert_eq!(review.candidates.len(), 1);
    assert_eq!(review.materials.len(), 1);
    assert_eq!(review.revision, 3);
    store
        .put_stack_preparation("owner", "entry:new-nonce", &m)
        .unwrap();
    assert!(
        store
            .publish_stack_entry_import(ready, "entry:nonce", "import", input)
            .is_err(),
        "supersession gate must precede replay"
    );
}
#[test]
fn whole_historical_atomic_failure_retry_and_immutable_disclosure() {
    let (root, store, oid) = fixture();
    let m = discovery(&root.path().to_string_lossy());
    store
        .put_stack_preparation("owner", "entry:nonce", &m)
        .unwrap();
    let ready = acquired(m.clone(), &oid, true);
    let candidates: Vec<_> = ready
        .members
        .iter()
        .map(|m| (m.id.clone(), comparison(&store, root.path(), &oid)))
        .collect();
    let db =
        rusqlite::Connection::open(root.path().join(".refact/reviews/reviews.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER fail_entry BEFORE INSERT ON stack_revisions BEGIN SELECT RAISE(ABORT,'failpoint'); END;").unwrap();
    assert!(store
        .publish_stack_entry_import(ready.clone(), "entry:nonce", "import", candidates.clone())
        .is_err());
    assert!(store.list_for_chat("owner").unwrap().is_empty());
    assert!(store.get_stack_for_chat("owner").unwrap().is_none());
    db.execute_batch("DROP TRIGGER fail_entry;").unwrap();
    let published = store
        .publish_stack_entry_import(ready.clone(), "entry:nonce", "import", candidates)
        .unwrap();
    assert_eq!(store.list_for_chat("owner").unwrap().len(), 2);
    let mut relabeled = published.clone();
    relabeled.members[0].reason = None;
    assert!(store
        .update_stack(relabeled, published.revision, "relabel")
        .is_err());
    let mut repinned = published.clone();
    repinned.members[1].head_oid = Some("different".into());
    assert!(store
        .update_stack(repinned, published.revision, "repin")
        .is_err());
    assert_eq!(store.get_stack_for_chat("owner").unwrap(), Some(published));
}
#[test]
fn discovery_binding_rejects_peer_edits_extra_candidates_and_scope_forgery() {
    let (root, store, oid) = fixture();
    let m = discovery(&root.path().to_string_lossy());
    store
        .put_stack_preparation("owner", "entry:nonce", &m)
        .unwrap();
    let ready = acquired(m.clone(), &oid, false);
    let input = vec![(
        ready.members[0].id.clone(),
        comparison(&store, root.path(), &oid),
    )];
    let mut forged = ready.clone();
    forged.members[1].branch_ref = "different".into();
    assert!(store
        .publish_stack_entry_import(forged, "entry:nonce", "import", input.clone())
        .is_err());
    let mut forged = ready.clone();
    forged.scope_choice.selected_member_id = m.members[1].id.clone();
    assert!(store
        .publish_stack_entry_import(forged, "entry:nonce", "import", input.clone())
        .is_err());
    let mut extra = input.clone();
    extra.push((
        ready.members[1].id.clone(),
        comparison(&store, root.path(), &oid),
    ));
    assert!(store
        .publish_stack_entry_import(ready.clone(), "entry:nonce", "import", extra)
        .is_err());
    assert!(store.list_for_chat("owner").unwrap().is_empty());
    store
        .publish_stack_entry_import(ready, "entry:nonce", "import", input)
        .unwrap();
}
