use refact_review_core::*;
#[test]
fn bound_member_branch_swap_cannot_reattribute_review_histories() {
    let root = tempfile::tempdir().unwrap();
    let store = ReviewStore::open(root.path()).unwrap();
    store.create_stack(manifest(), "create").unwrap();
    let reviews = bind(&store);
    let original = store.load_stack("stack").unwrap();
    let history = store.stack_history("stack").unwrap();
    let mut swapped = original.clone();
    let first = swapped.members[0].branch_ref.clone();
    swapped.members[0].branch_ref = swapped.members[1].branch_ref.clone();
    swapped.members[1].branch_ref = first;
    assert!(matches!(
        store.update_stack(swapped, original.revision, "swap"),
        Err(Error::Invalid(_))
    ));
    assert_eq!(store.load_stack("stack").unwrap(), original);
    assert_eq!(store.stack_history("stack").unwrap(), history);
    for review in reviews {
        assert_eq!(store.load(&review.id).unwrap(), review);
    }
}

#[test]
fn grouped_same_member_sequential_cas_commits_in_order_or_rolls_back() {
    let root = tempfile::tempdir().unwrap();
    let store = ReviewStore::open(root.path()).unwrap();
    let review = store
        .create("chat", "Original", Origin::Local, "create")
        .unwrap();
    let batch = |second_revision| {
        vec![
            (
                review.id.clone(),
                mutation(
                    0,
                    "first",
                    Operation::Rename {
                        title: "First".into(),
                    },
                ),
            ),
            (
                review.id.clone(),
                mutation(
                    second_revision,
                    "second",
                    Operation::Rename {
                        title: "Second".into(),
                    },
                ),
            ),
        ]
    };
    assert!(matches!(
        store.mutate_many(batch(0)),
        Err(Error::Conflict {
            actual: 1,
            ..
        })
    ));
    assert_eq!(store.load(&review.id).unwrap(), review);
    let result = store.mutate_many(batch(1)).unwrap();
    assert_eq!(result[0].revision, 1);
    assert_eq!(result[0].title, "First");
    assert_eq!(result[1].revision, 2);
    assert_eq!(result[1].title, "Second");
    assert_eq!(store.load(&review.id).unwrap(), result[1]);
}

#[test]
fn preparation_is_durable_without_scope_consent_or_review_creation() {
    let root = tempfile::tempdir().unwrap();
    let store = ReviewStore::open(root.path()).unwrap();
    let mut pending = manifest();
    pending.scope_choice.confirmed = false;
    store
        .put_stack_preparation("chat", "discovery-fingerprint", &pending)
        .unwrap();
    assert!(matches!(
        store.put_stack_preparation("wrong", "key", &pending),
        Err(Error::Invalid(_))
    ));
    drop(store);
    let store = ReviewStore::open(root.path()).unwrap();
    assert_eq!(
        store.get_stack_preparation("chat").unwrap(),
        Some(("discovery-fingerprint".into(), pending))
    );
    assert_eq!(store.get_stack_for_chat("chat").unwrap(), None);
    assert_eq!(store.get_for_chat("chat").unwrap(), None);
}

#[test]
fn grouped_mutations_roll_back_every_member_and_allow_same_key_retry() {
    let root = tempfile::tempdir().unwrap();
    let store = ReviewStore::open(root.path()).unwrap();
    store.create_stack(manifest(), "create").unwrap();
    let reviews = bind(&store);
    let request = |bad| {
        vec![
            (
                reviews[0].id.clone(),
                mutation(
                    0,
                    "group-a",
                    Operation::Rename {
                        title: "A changed".into(),
                    },
                ),
            ),
            (
                reviews[1].id.clone(),
                mutation(
                    if bad { 1 } else { 0 },
                    "group-b",
                    Operation::Rename {
                        title: "B changed".into(),
                    },
                ),
            ),
        ]
    };
    assert!(matches!(
        store.mutate_many(request(true)),
        Err(Error::Conflict { .. })
    ));
    assert_eq!(store.load(&reviews[0].id).unwrap(), reviews[0]);
    assert_eq!(store.load(&reviews[1].id).unwrap(), reviews[1]);
    let result = store.mutate_many(request(false)).unwrap();
    assert_eq!(result[0].title, "A changed");
    assert_eq!(result[1].title, "B changed");
    assert_eq!(store.mutate_many(request(false)).unwrap(), result);
    assert_eq!(store.load(&reviews[2].id).unwrap(), reviews[2]);
    assert_eq!(store.get_for_chat("chat").unwrap(), None);
}

#[test]
fn create_cannot_replace_stack_owner_or_create_implicit_execution_slot() {
    let root = tempfile::tempdir().unwrap();
    let store = ReviewStore::open(root.path()).unwrap();
    let original = store.create_stack(manifest(), "create").unwrap();
    let mut forged = manifest();
    forged.owner_chat_id = "other".into();
    assert!(matches!(
        store.create_stack(forged, "replace"),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        store.create("chat", "Implicit", Origin::Local, "implicit"),
        Err(Error::Invalid(_))
    ));
    assert_eq!(store.load_stack("stack").unwrap(), original);
    assert_eq!(store.get_for_chat("chat").unwrap(), None);
}

#[test]
fn captured_candidates_refresh_only_the_addressed_member() {
    let root = tempfile::tempdir().unwrap();
    let checkout = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(checkout.path()).unwrap();
    std::fs::write(checkout.path().join("file.txt"), "initial\n").unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(std::path::Path::new("file.txt")).unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let sig =
        git2::Signature::new("Review", "review@example.invalid", &git2::Time::new(1, 0)).unwrap();
    let oid = repo
        .commit(Some("HEAD"), &sig, &sig, "fixture", &tree, &[])
        .unwrap()
        .to_string();
    let store = ReviewStore::open(root.path()).unwrap();
    store.create_stack(manifest(), "create").unwrap();
    let mut reviews = bind(&store);
    let capture = || {
        store
            .capture(
                checkout.path(),
                CaptureRequest {
                    base_oid: oid.clone(),
                    head_oid: oid.clone(),
                    policy: CapturePolicy {
                        working: true,
                        ..Default::default()
                    },
                    origin: Origin::Local,
                },
            )
            .unwrap()
    };
    for r in &mut reviews {
        let candidate = capture();
        *r = store
            .mutate(
                &r.id,
                mutation(
                    0,
                    "stage",
                    Operation::StageCandidate {
                        candidate: candidate.clone(),
                    },
                ),
            )
            .unwrap();
        *r = store
            .mutate(
                &r.id,
                mutation(
                    1,
                    "activate",
                    Operation::ActivateCandidate {
                        candidate_id: candidate.id,
                    },
                ),
            )
            .unwrap();
    }
    let before = reviews.clone();
    std::fs::write(checkout.path().join("file.txt"), "B refreshed\n").unwrap();
    let candidate = capture();
    let b = &mut reviews[1];
    *b = store
        .mutate(
            &b.id,
            mutation(
                2,
                "refresh-stage",
                Operation::StageCandidate {
                    candidate: candidate.clone(),
                },
            ),
        )
        .unwrap();
    *b = store
        .mutate(
            &b.id,
            mutation(
                3,
                "refresh-activate",
                Operation::ActivateCandidate {
                    candidate_id: candidate.id.clone(),
                },
            ),
        )
        .unwrap();
    assert_eq!(b.current_candidate_id, Some(candidate.id));
    assert_eq!(b.candidates.len(), 2);
    assert_eq!(store.load(&before[0].id).unwrap(), before[0]);
    assert_eq!(store.load(&before[2].id).unwrap(), before[2]);
    assert_eq!(before[1].candidates[0], b.candidates[0]);
}

fn manifest() -> StackManifest {
    let ids: Vec<_> = (1..=3)
        .map(|n| format!("00000000-0000-4000-8000-{n:012}"))
        .collect();
    StackManifest {
        schema_version: SCHEMA_VERSION,
        id: "stack".into(),
        owner_chat_id: "chat".into(),
        revision: 0,
        title: "Fork".into(),
        repository: "local-repo".into(),
        source: StackSource {
            kind: "graphite".into(),
            reference: None,
        },
        root_base_ref: "main".into(),
        root_base_oid: Some("base".into()),
        scope_choice: StackScopeChoice {
            scope: StackScope::Whole,
            selected_member_id: ids[0].clone(),
            confirmed: true,
            explicit_request: false,
        },
        members: ids
            .iter()
            .enumerate()
            .map(|(n, id)| StackMember {
                id: id.clone(),
                branch_ref: format!("branch-{n}"),
                pr_identity: None,
                parent_member_id: (n > 0).then(|| ids[0].clone()),
                declared_base_ref: if n == 0 {
                    "main".into()
                } else {
                    "branch-0".into()
                },
                resolved_base_oid: Some("base".into()),
                head_oid: Some(format!("head-{n}")),
                review_id: None,
                state: StackMemberState::Ready,
                reason: None,
            })
            .collect(),
    }
}
fn bind(store: &ReviewStore) -> Vec<Review> {
    (0..3)
        .map(|n| {
            let m = store.get_stack_for_chat("chat").unwrap().unwrap();
            store
                .create_member_review(
                    &m.id,
                    &m.members[n].id,
                    m.revision,
                    "Member",
                    Origin::Local,
                    &format!("bind-{n}"),
                )
                .unwrap()
        })
        .collect()
}
fn mutation(revision: u64, key: &str, operation: Operation) -> Mutation {
    Mutation {
        expected_revision: revision,
        request_key: key.into(),
        actor: Actor {
            kind: ActorKind::User,
            id: "user".into(),
        },
        at_ms: 1,
        operation,
    }
}
#[test]
fn fork_refresh_reorder_and_tombstones_preserve_independent_reviews() {
    let root = tempfile::tempdir().unwrap();
    let store = ReviewStore::open(root.path()).unwrap();
    store.create_stack(manifest(), "create").unwrap();
    let reviews = bind(&store);
    assert_eq!(store.get_for_chat("chat").unwrap(), None);
    let b = store
        .mutate(
            &reviews[1].id,
            mutation(
                0,
                "b-only",
                Operation::Rename {
                    title: "B refreshed".into(),
                },
            ),
        )
        .unwrap();
    let mut m = store.get_stack_for_chat("chat").unwrap().unwrap();
    m.members[1].head_oid = Some("restacked-b".into());
    m.members.swap(1, 2);
    let m = store
        .update_stack(m.clone(), m.revision, "reorder")
        .unwrap();
    assert_eq!(
        store
            .stack_review_for_member(&m.id, &manifest().members[1].id)
            .unwrap(),
        Some(b)
    );
    assert_eq!(store.load(&reviews[0].id).unwrap(), reviews[0]);
    assert_eq!(store.load(&reviews[2].id).unwrap(), reviews[2]);
    let mut missing = m.clone();
    missing.members[1].state = StackMemberState::Missing;
    let missing = store.update_stack(missing, m.revision, "missing").unwrap();
    let mut removed = missing.clone();
    removed.members[1].state = StackMemberState::Removed;
    let removed = store
        .update_stack(removed, missing.revision, "removed")
        .unwrap();
    assert_eq!(store.load(&reviews[2].id).unwrap(), reviews[2]);
    let mut erased = removed.clone();
    erased.members.remove(1);
    assert!(matches!(
        store.update_stack(erased, removed.revision, "erase"),
        Err(Error::Invalid(_))
    ));
    drop(store);
    let store = ReviewStore::open(root.path()).unwrap();
    assert_eq!(store.get_stack_for_chat("chat").unwrap(), Some(removed));
}
#[test]
fn invalid_dag_duplicate_identity_and_unconfirmed_scope_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let store = ReviewStore::open(root.path()).unwrap();
    let mut cycle = manifest();
    cycle.members[0].parent_member_id = Some(cycle.members[1].id.clone());
    assert!(matches!(
        store.create_stack(cycle, "cycle"),
        Err(Error::Invalid(_))
    ));
    let mut duplicate = manifest();
    duplicate.members[2].id = duplicate.members[1].id.clone();
    assert!(matches!(
        store.create_stack(duplicate, "duplicate"),
        Err(Error::Invalid(_))
    ));
    let mut self_parent = manifest();
    self_parent.members[0].parent_member_id = Some(self_parent.members[0].id.clone());
    assert!(matches!(
        store.create_stack(self_parent, "self"),
        Err(Error::Invalid(_))
    ));
    let mut unconfirmed = manifest();
    unconfirmed.scope_choice.confirmed = false;
    assert!(matches!(
        store.create_stack(unconfirmed, "scope"),
        Err(Error::Invalid(_))
    ));
    assert_eq!(store.get_stack_for_chat("chat").unwrap(), None);
}
#[test]
fn single_scope_cannot_widen_or_bind_other_members() {
    let root = tempfile::tempdir().unwrap();
    let store = ReviewStore::open(root.path()).unwrap();
    let mut m = manifest();
    m.scope_choice.scope = StackScope::Single;
    let m = store.create_stack(m, "create").unwrap();
    assert!(matches!(
        store.create_member_review(&m.id, &m.members[1].id, 0, "No", Origin::Local, "outside"),
        Err(Error::Invalid(_))
    ));
    let mut wider = m.clone();
    wider.scope_choice.scope = StackScope::Whole;
    assert!(matches!(
        store.update_stack(wider, 0, "widen"),
        Err(Error::Invalid(_))
    ));
    assert!(store.list_for_chat("chat").unwrap().is_empty());
}
#[test]
fn legacy_migration_preserves_ids_keys_and_selected_slot() {
    let root = tempfile::tempdir().unwrap();
    let store = ReviewStore::open(root.path()).unwrap();
    let old = store
        .create("chat", "Legacy", Origin::Local, "original-key")
        .unwrap();
    let old = store
        .mutate(
            &old.id,
            mutation(
                0,
                "legacy-rename",
                Operation::Rename {
                    title: "Saved".into(),
                },
            ),
        )
        .unwrap();
    drop(store);
    let db =
        rusqlite::Connection::open(root.path().join(".refact/reviews/reviews.sqlite")).unwrap();
    db.execute_batch("DROP TABLE active_reviews; DROP TABLE singleton_reviews; DELETE FROM review_store_migrations; ALTER TABLE reviews RENAME TO newer; CREATE TABLE reviews(id TEXT PRIMARY KEY,chat TEXT NOT NULL UNIQUE,document TEXT NOT NULL); INSERT INTO reviews SELECT * FROM newer; DROP TABLE newer;").unwrap();
    drop(db);
    let store = ReviewStore::open(root.path()).unwrap();
    assert_eq!(store.get_for_chat("chat").unwrap(), Some(old.clone()));
    assert_eq!(
        store
            .mutate(
                &old.id,
                mutation(
                    0,
                    "legacy-rename",
                    Operation::Rename {
                        title: "Saved".into()
                    }
                )
            )
            .unwrap(),
        old
    );
    store.create_stack(manifest(), "stack").unwrap();
    let reviews = bind(&store);
    assert_ne!(reviews[0].id, old.id);
    assert_ne!(reviews[1].id, old.id);
    let imported = store
        .mutate(
            &reviews[0].id,
            mutation(
                0,
                "import-only",
                Operation::Rename {
                    title: "Imported member".into(),
                },
            ),
        )
        .unwrap();
    assert_ne!(imported.title, old.title);
    assert_eq!(store.load(&old.id).unwrap(), old);
    assert_eq!(store.get_for_chat("chat").unwrap(), Some(old.clone()));
    assert_eq!(
        store
            .create("chat", "Other", Origin::Local, "new-singleton-key")
            .unwrap(),
        old
    );
    assert!(matches!(
        store.create("chat", "Changed", Origin::Local, "original-key"),
        Err(Error::IdempotencyConflict)
    ));
}
#[test]
fn manifest_cas_retry_and_sql_rollback_do_not_consume_keys() {
    let root = tempfile::tempdir().unwrap();
    let store = ReviewStore::open(root.path()).unwrap();
    let m = store.create_stack(manifest(), "create").unwrap();
    let mut next = m.clone();
    next.title = "Updated".into();
    let db =
        rusqlite::Connection::open(root.path().join(".refact/reviews/reviews.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER fail_history BEFORE INSERT ON stack_revisions BEGIN SELECT RAISE(ABORT,'failpoint'); END;").unwrap();
    assert!(matches!(
        store.update_stack(next.clone(), 0, "update"),
        Err(Error::Sql(_))
    ));
    assert_eq!(store.load_stack(&m.id).unwrap(), m);
    assert_eq!(store.stack_history(&m.id).unwrap(), vec![m.clone()]);
    db.execute_batch("DROP TRIGGER fail_history;").unwrap();
    let updated = store.update_stack(next.clone(), 0, "update").unwrap();
    assert_eq!(
        store.update_stack(next.clone(), 0, "update").unwrap(),
        updated
    );
    next.title = "Key misuse".into();
    assert!(matches!(
        store.update_stack(next.clone(), 0, "update"),
        Err(Error::IdempotencyConflict)
    ));
    assert!(matches!(
        store.update_stack(next, 0, "stale"),
        Err(Error::Conflict {
            actual: 1,
            ..
        })
    ));
    assert_eq!(store.stack_history(&m.id).unwrap(), vec![m, updated]);
}
#[test]
fn activation_checks_owner_membership_cas_and_rolls_back_journal() {
    let root = tempfile::tempdir().unwrap();
    let store = ReviewStore::open(root.path()).unwrap();
    store.create_stack(manifest(), "create").unwrap();
    let reviews = bind(&store);
    let outsider = store
        .create("other-chat", "Outside", Origin::Local, "other")
        .unwrap();
    assert!(matches!(
        store.set_active_review("unowned-chat", &outsider.id, None, "owner-only"),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        store.set_active_review("chat", &outsider.id, None, "forged"),
        Err(Error::Invalid(_))
    ));
    store
        .set_active_review("chat", &reviews[0].id, None, "first")
        .unwrap();
    assert!(matches!(
        store.set_active_review("chat", &reviews[1].id, None, "stale"),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        store.mutate_and_set_active(
            "chat",
            &reviews[1].id,
            Some(&reviews[0].id),
            mutation(
                1,
                "bad-revision",
                Operation::Rename {
                    title: "No".into()
                }
            )
        ),
        Err(Error::Conflict { .. })
    ));
    assert_eq!(
        store.get_for_chat("chat").unwrap(),
        Some(reviews[0].clone())
    );
    let b = store
        .mutate_and_set_active(
            "chat",
            &reviews[1].id,
            Some(&reviews[0].id),
            mutation(
                0,
                "activate",
                Operation::Rename {
                    title: "B".into(),
                },
            ),
        )
        .unwrap();
    assert_eq!(store.get_for_chat("chat").unwrap(), Some(b.clone()));
    store
        .stack_review_for_member("stack", &manifest().members[0].id)
        .unwrap();
    assert_eq!(store.get_for_chat("chat").unwrap(), Some(b));
}
#[test]
fn coherent_handoff_includes_all_member_object_histories_and_settled_deliveries() {
    let root = tempfile::tempdir().unwrap();
    let store = ReviewStore::open(root.path()).unwrap();
    store.create_stack(manifest(), "create").unwrap();
    let reviews = bind(&store);
    for r in &reviews {
        let thread = Thread {
            target: None,
            id: "thread".into(),
            anchors: vec![],
            annotation_id: None,
            resolved: false,
            contributions: vec![],
        };
        let contribution = Contribution {
            blocks: vec![],
            id: "comment".into(),
            thread_id: "thread".into(),
            actor: Actor {
                kind: ActorKind::User,
                id: "user".into(),
            },
            body: format!("Comment {}", r.id),
            created_at_ms: 1,
            chat_message_id: None,
            context: None,
        };
        let r = store
            .mutate(
                &r.id,
                mutation(
                    0,
                    "comment",
                    Operation::CreateThread {
                        thread,
                        contribution,
                    },
                ),
            )
            .unwrap();
        let mut thread = r.threads[0].clone();
        thread.resolved = true;
        store
            .mutate(
                &r.id,
                mutation(
                    1,
                    "resolve",
                    Operation::PutThread {
                        thread,
                    },
                ),
            )
            .unwrap();
        store.settle_delivery(&format!("{}:comment", r.id)).unwrap();
    }
    let snapshot = store.handoff_snapshot(&reviews[1].id).unwrap();
    assert_eq!(snapshot.review.revision, 2);
    assert_eq!(snapshot.stack_history.len(), 4);
    assert_eq!(snapshot.stack_member_reviews.len(), 3);
    for member in snapshot.stack_member_reviews {
        assert_eq!(member.review.revision, 2);
        assert!(member.review.threads[0].resolved);
        assert_eq!(member.object_history.len(), 1);
        assert!(
            matches!(&member.object_history[0].object,HistoricalObject::Thread(t) if !t.resolved)
        );
        assert_eq!(member.delivery_statuses.len(), 1);
        assert!(member.delivery_statuses[0].delivered);
    }
    assert_eq!(store.get_for_chat("chat").unwrap(), None);
}
