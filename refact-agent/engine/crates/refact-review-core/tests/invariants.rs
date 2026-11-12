use refact_review_core::*;
fn actor() -> Actor {
    Actor {
        kind: ActorKind::User,
        id: "user".into(),
    }
}
fn mutation(revision: u64, key: &str, operation: Operation) -> Mutation {
    Mutation {
        expected_revision: revision,
        request_key: key.into(),
        actor: actor(),
        at_ms: 1,
        operation,
    }
}
fn commit(repo: &git2::Repository, text: &str) -> String {
    std::fs::write(repo.workdir().unwrap().join("file.txt"), text).unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(std::path::Path::new("file.txt")).unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let signature = git2::Signature::now("Review", "review@example.invalid").unwrap();
    let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        "snapshot",
        &tree,
        &parent.iter().collect::<Vec<_>>(),
    )
    .unwrap()
    .to_string()
}
fn capture(store: &ReviewStore, repo: &git2::Repository, oid: &str) -> Candidate {
    store
        .capture(
            repo.workdir().unwrap(),
            CaptureRequest {
                base_oid: oid.into(),
                head_oid: oid.into(),
                policy: CapturePolicy::default(),
                origin: Origin::Local,
            },
        )
        .unwrap()
}
#[test]
fn cas_idempotency_and_restart_are_atomic() {
    let root = tempfile::tempdir().unwrap();
    let store = ReviewStore::open(root.path()).unwrap();
    let review = store
        .create("chat", "Initial", Origin::Local, "create")
        .unwrap();
    let m = mutation(
        0,
        "rename",
        Operation::Rename {
            title: "Changed".into(),
        },
    );
    let first = store.mutate(&review.id, m.clone()).unwrap();
    let mut retry = m.clone();
    retry.at_ms = 999;
    assert_eq!(store.mutate(&review.id, retry).unwrap(), first);
    assert!(matches!(
        store.mutate(
            &review.id,
            mutation(
                0,
                "stale",
                Operation::Rename {
                    title: "Lost".into()
                }
            )
        ),
        Err(Error::Conflict {
            actual: 1,
            ..
        })
    ));
    assert!(matches!(
        store.mutate(
            &review.id,
            mutation(
                0,
                "rename",
                Operation::Rename {
                    title: "Other".into()
                }
            )
        ),
        Err(Error::IdempotencyConflict)
    ));
    drop(store);
    let reopened = ReviewStore::open(root.path()).unwrap();
    assert_eq!(reopened.get_for_chat("chat").unwrap(), Some(first));
}
#[test]
fn canonical_comment_and_outbox_survive_restart_and_reject_overwrite() {
    let root = tempfile::tempdir().unwrap();
    let store = ReviewStore::open(root.path()).unwrap();
    let r = store
        .create("chat", "Review", Origin::Local, "create")
        .unwrap();
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
        thread_id: thread.id.clone(),
        actor: actor(),
        body: "Exact comment".into(),
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
                    thread: thread.clone(),
                    contribution,
                },
            ),
        )
        .unwrap();
    assert_eq!(r.threads[0].contributions.len(), 1);
    assert!(matches!(store
        .mutate(
            &r.id,
            mutation(
                1,
                "overwrite",
                Operation::PutThread {
                    thread
                }
            )
        )
        , Err(Error::Invalid(message)) if message == "contributions append-only"));
    drop(store);
    let store = ReviewStore::open(root.path()).unwrap();
    let pending = store.pending_deliveries(&r.id).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].contribution_id, "comment");
    store.settle_delivery(&pending[0].id).unwrap();
    assert!(store.pending_deliveries(&r.id).unwrap().is_empty());
    assert_eq!(store.load(&r.id).unwrap(), r);
}
#[test]
fn historical_snapshot_and_anchor_reconciliation_preserve_identity_without_guessing() {
    let root = tempfile::tempdir().unwrap();
    let checkout = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(checkout.path()).unwrap();
    let store = ReviewStore::open(root.path()).unwrap();
    let one = commit(&repo, "before\nneedle\nafter\n");
    let c1 = capture(&store, &repo, &one);
    let anchor = store.anchor(&c1, "file.txt", Side::Head, 2, 2).unwrap();
    let r = store
        .create("chat", "Review", Origin::Local, "create")
        .unwrap();
    let r = store
        .mutate(
            &r.id,
            mutation(
                0,
                "stage1",
                Operation::StageCandidate {
                    candidate: c1.clone(),
                },
            ),
        )
        .unwrap();
    let r = store
        .mutate(
            &r.id,
            mutation(
                r.revision,
                "activate1",
                Operation::ActivateCandidate {
                    candidate_id: c1.id.clone(),
                },
            ),
        )
        .unwrap();
    let step = TourStep {
        id: "stable-step".into(),
        current: TourVersion {
            html: None,
            blocks: vec![],
            candidate_id: None,
            version: 1,
            title: "Needle".into(),
            body: "Explain".into(),
            anchors: vec![anchor.clone()],
        },
        history: vec![],
    };
    let r = store
        .mutate(
            &r.id,
            mutation(
                r.revision,
                "tour",
                Operation::PutTourStep {
                    step,
                },
            ),
        )
        .unwrap();
    let two = commit(&repo, "new\nbefore\nneedle\nafter\n");
    let c2 = capture(&store, &repo, &two);
    let r = store
        .mutate(
            &r.id,
            mutation(
                r.revision,
                "stage2",
                Operation::StageCandidate {
                    candidate: c2.clone(),
                },
            ),
        )
        .unwrap();
    let r = store
        .mutate(
            &r.id,
            mutation(
                r.revision,
                "activate2",
                Operation::ActivateCandidate {
                    candidate_id: c2.id.clone(),
                },
            ),
        )
        .unwrap();
    assert_eq!(r.tour[0].id, "stable-step");
    assert_eq!(r.tour[0].current.anchors[0].position.start_line, 3);
    assert_eq!(r.tour[0].current.anchors[0].history, vec![anchor.position]);
    assert_eq!(r.tour[0].history.len(), 1);
    let three = commit(
        &repo,
        "new\nbefore\nneedle\nafter\nnew\nbefore\nneedle\nafter\n",
    );
    let c3 = capture(&store, &repo, &three);
    let r = store
        .mutate(
            &r.id,
            mutation(
                r.revision,
                "stage3",
                Operation::StageCandidate {
                    candidate: c3.clone(),
                },
            ),
        )
        .unwrap();
    let r = store
        .mutate(
            &r.id,
            mutation(
                r.revision,
                "activate3",
                Operation::ActivateCandidate {
                    candidate_id: c3.id,
                },
            ),
        )
        .unwrap();
    assert_eq!(r.tour[0].current.anchors[0].status, AnchorStatus::Stalled);
    drop(repo);
    drop(checkout);
    drop(store);
    let store = ReviewStore::open(root.path()).unwrap();
    assert_eq!(
        store.file(&c1, "file.txt").unwrap().head.unwrap(),
        b"before\nneedle\nafter\n"
    );
    assert_eq!(store.load(&r.id).unwrap().candidates.len(), 3);
    let mut forged = c1;
    forged.files[0].path = "../escape".into();
    assert!(store.file(&forged, "../escape").is_err());
}
#[test]
#[cfg(unix)]
fn symlink_sources_and_store_roots_are_refused() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join(".refact")).unwrap();
    assert!(ReviewStore::open(root.path()).is_err());
    let store = ReviewStore::open(outside.path()).unwrap();
    let checkout = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(checkout.path()).unwrap();
    let oid = commit(&repo, "safe\n");
    std::fs::remove_file(checkout.path().join("file.txt")).unwrap();
    let forbidden = outside.path().join("forbidden.txt");
    std::fs::write(&forbidden, "private bytes must never be captured").unwrap();
    std::os::unix::fs::symlink(&forbidden, checkout.path().join("file.txt")).unwrap();
    assert!(std::fs::symlink_metadata(checkout.path().join("file.txt"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(store
        .capture(
            checkout.path(),
            CaptureRequest {
                base_oid: oid.clone(),
                head_oid: oid,
                policy: CapturePolicy {
                    working: true,
                    ..Default::default()
                },
                origin: Origin::Local
            }
        )
        .is_err());
}

#[test]
fn stale_claims_and_annotation_history_are_enforced() {
    let root = tempfile::tempdir().unwrap();
    let checkout = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(checkout.path()).unwrap();
    let store = ReviewStore::open(root.path()).unwrap();
    let oid = commit(&repo, "before\nneedle\nafter\n");
    let c1 = capture(&store, &repo, &oid);
    let c2 = capture(&store, &repo, &oid);
    let mut r = store
        .create("chat", "Review", Origin::Local, "create")
        .unwrap();
    for (key, c) in [("one", c1.clone()), ("two", c2.clone())] {
        r = store
            .mutate(
                &r.id,
                mutation(
                    r.revision,
                    key,
                    Operation::StageCandidate {
                        candidate: c,
                    },
                ),
            )
            .unwrap();
    }
    r = store
        .mutate(
            &r.id,
            mutation(
                r.revision,
                "activate",
                Operation::ActivateCandidate {
                    candidate_id: c2.id.clone(),
                },
            ),
        )
        .unwrap();
    let old_anchor = store.anchor(&c1, "file.txt", Side::Head, 2, 2).unwrap();
    let mut annotation = Annotation {
        id: "finding".into(),
        kind: AnnotationKind::Finding,
        title: "Original".into(),
        body: "Original body".into(),
        anchors: vec![old_anchor],
        severity: Severity::High,
        confidence: Confidence::Low,
        assessment: Assessment::Unassessed,
        disposition: Disposition::Open,
        applicability: Applicability::Current,
        evidence_ids: vec![],
        run_id: None,
    };
    assert!(matches!(store
        .mutate(
            &r.id,
            mutation(
                r.revision,
                "stale",
                Operation::PutAnnotation {
                    annotation: annotation.clone()
                }
            )
        )
        , Err(Error::Invalid(message)) if message == "annotation cannot claim current applicability"));
    annotation.anchors = vec![store.anchor(&c2, "file.txt", Side::Head, 2, 2).unwrap()];
    annotation.anchors[0].status = AnchorStatus::Stalled;
    annotation.anchors[0].stalled_reason = Some("uncertain".into());
    assert!(matches!(store
        .mutate(
            &r.id,
            mutation(
                r.revision,
                "stalled",
                Operation::PutAnnotation {
                    annotation: annotation.clone()
                }
            )
        )
        , Err(Error::Invalid(message)) if message == "annotation cannot claim current applicability"));
    annotation.anchors = vec![store.anchor(&c1, "file.txt", Side::Head, 2, 2).unwrap()];
    annotation.applicability = Applicability::Historical;
    r = store
        .mutate(
            &r.id,
            mutation(
                r.revision,
                "insert",
                Operation::PutAnnotation {
                    annotation: annotation.clone(),
                },
            ),
        )
        .unwrap();
    annotation.title = "Changed".into();
    let old_position = annotation.anchors[0].position.clone();
    annotation.anchors = vec![store.anchor(&c2, "file.txt", Side::Head, 2, 2).unwrap()];
    annotation.anchors[0].history.push(old_position);
    r = store
        .mutate(
            &r.id,
            mutation(
                r.revision,
                "change",
                Operation::PutAnnotation {
                    annotation: annotation.clone(),
                },
            ),
        )
        .unwrap();
    assert_eq!(r.annotations[0].anchors[0].history.len(), 1);
    annotation.anchors[0].history.clear();
    r = store
        .mutate(
            &r.id,
            mutation(
                r.revision,
                "clear",
                Operation::PutAnnotation {
                    annotation,
                },
            ),
        )
        .unwrap();
    assert_eq!(r.annotations[0].anchors[0].history.len(), 1);
    let evidence = Evidence {
        id: "old".into(),
        candidate_id: c1.id,
        material_id: None,
        run_id: None,
        source: EvidenceSource::External {
            url: "https://example.invalid".into(),
        },
        summary: "Old".into(),
        applicability: Applicability::Current,
        created_at_ms: 1,
    };
    assert!(matches!(store
        .mutate(
            &r.id,
            mutation(
                r.revision,
                "evidence",
                Operation::AddEvidence {
                    evidence
                }
            )
        )
        , Err(Error::Invalid(message)) if message == "evidence cannot claim current applicability"));
    drop(store);
    let store = ReviewStore::open(root.path()).unwrap();
    assert!(store.object_history(&r.id).unwrap().iter().any(|h| matches!(&h.object,HistoricalObject::Annotation(a) if a.title=="Original" && a.body=="Original body")));
}
#[test]
fn contribution_provenance_cannot_impersonate_another_actor() {
    let root = tempfile::tempdir().unwrap();
    let store = ReviewStore::open(root.path()).unwrap();
    let r = store
        .create("chat", "Review", Origin::Local, "create")
        .unwrap();
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
        id: "forged".into(),
        thread_id: thread.id.clone(),
        actor: Actor {
            kind: ActorKind::Agent,
            id: "other-agent".into(),
        },
        body: "Spoof".into(),
        created_at_ms: 1,
        chat_message_id: None,
        context: None,
    };
    assert!(matches!(store
        .mutate(
            &r.id,
            mutation(
                0,
                "spoof",
                Operation::CreateThread {
                    thread,
                    contribution
                }
            )
        )
        , Err(Error::Invalid(message)) if message == "contribution actor must equal mutation actor"));
    assert!(store.load(&r.id).unwrap().threads.is_empty());
    assert!(store.pending_deliveries(&r.id).unwrap().is_empty());
}
#[test]
fn corruption_aborts_activation_instead_of_stalling_and_metadata_overlay_is_excluded() {
    let root = tempfile::tempdir().unwrap();
    let checkout = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(checkout.path()).unwrap();
    let oid = commit(&repo, "before\nneedle\nafter\n");
    std::fs::create_dir(checkout.path().join(".refact")).unwrap();
    std::fs::write(checkout.path().join(".refact/local"), "metadata").unwrap();
    let mut index = repo.index().unwrap();
    index
        .add_path(std::path::Path::new(".refact/local"))
        .unwrap();
    index.write().unwrap();
    let store = ReviewStore::open(root.path()).unwrap();
    let c1 = capture(&store, &repo, &oid);
    let mut r = store
        .create("chat", "Review", Origin::Local, "create")
        .unwrap();
    r = store
        .mutate(
            &r.id,
            mutation(
                0,
                "stage",
                Operation::StageCandidate {
                    candidate: c1.clone(),
                },
            ),
        )
        .unwrap();
    let anchor = store.anchor(&c1, "file.txt", Side::Head, 2, 2).unwrap();
    let step = TourStep {
        id: "step".into(),
        current: TourVersion {
            html: None,
            blocks: vec![],
            candidate_id: None,
            version: 1,
            title: "Title".into(),
            body: "Body".into(),
            anchors: vec![anchor],
        },
        history: vec![],
    };
    r = store
        .mutate(
            &r.id,
            mutation(
                r.revision,
                "tour",
                Operation::PutTourStep {
                    step,
                },
            ),
        )
        .unwrap();
    std::fs::write(
        checkout.path().join("file.txt"),
        "new\nbefore\nneedle\nafter\n",
    )
    .unwrap();
    let c2 = store
        .capture(
            checkout.path(),
            CaptureRequest {
                base_oid: oid.clone(),
                head_oid: oid,
                policy: CapturePolicy {
                    staged: true,
                    working: true,
                    untracked: true,
                },
                origin: Origin::Local,
            },
        )
        .unwrap();
    assert!(c2.files.iter().all(|f| !f.path.starts_with(".refact")));
    r = store
        .mutate(
            &r.id,
            mutation(
                r.revision,
                "stage2",
                Operation::StageCandidate {
                    candidate: c2.clone(),
                },
            ),
        )
        .unwrap();
    let db =
        rusqlite::Connection::open(root.path().join(".refact/reviews/reviews.sqlite")).unwrap();
    db.execute(
        "UPDATE blobs SET data=?1 WHERE digest=?2",
        rusqlite::params![
            b"corrupt".as_slice(),
            c2.files[0].head.as_ref().unwrap().digest
        ],
    )
    .unwrap();
    assert!(matches!(
        store.mutate(
            &r.id,
            mutation(
                r.revision,
                "activate",
                Operation::ActivateCandidate {
                    candidate_id: c2.id
                }
            )
        ),
        Err(Error::Integrity(_))
    ));
    assert_eq!(store.load(&r.id).unwrap(), r);
}

fn apply_operation(store: &ReviewStore, r: Review, key: &str, operation: Operation) -> Review {
    store
        .mutate(&r.id, mutation(r.revision, key, operation))
        .unwrap()
}
fn run_for(candidate: &Candidate) -> Run {
    Run {
        id: "run".into(),
        candidate_id: candidate.id.clone(),
        agent_id: "agent".into(),
        purpose: "investigate".into(),
        model: None,
        domain: None,
        status: RunStatus::Running,
        started_at_ms: 1,
        finished_at_ms: None,
        error: None,
    }
}
#[test]
fn cancellation_intent_cannot_be_superseded_by_worker_updates() {
    let root = tempfile::tempdir().unwrap();
    let checkout = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(checkout.path()).unwrap();
    let oid = commit(&repo, "needle\n");
    let store = ReviewStore::open(root.path()).unwrap();
    let c = capture(&store, &repo, &oid);
    let r = store
        .create("chat", "Review", Origin::Local, "create")
        .unwrap();
    let r = apply_operation(
        &store,
        r,
        "stage",
        Operation::StageCandidate {
            candidate: c.clone(),
        },
    );
    let mut run = run_for(&c);
    let r = apply_operation(
        &store,
        r,
        "start",
        Operation::PutRun {
            run: run.clone(),
        },
    );
    assert_eq!(r.runs[0].status, RunStatus::Running);
    let r = apply_operation(
        &store,
        r,
        "cancel",
        Operation::RequestRunCancel {
            run_id: run.id.clone(),
        },
    );
    assert_eq!(r.runs[0].status, RunStatus::CancelRequested);
    for (key, status) in [
        ("stale-running", RunStatus::Running),
        ("stale-queued", RunStatus::Queued),
    ] {
        run.status = status;
        assert!(
            matches!(store.mutate(&r.id, mutation(r.revision, key, Operation::PutRun { run: run.clone() })),
            Err(Error::Invalid(message)) if message == "run provenance/terminal state immutable")
        );
        assert_eq!(store.load(&r.id).unwrap(), r);
    }
    run.status = RunStatus::Cancelled;
    run.finished_at_ms = Some(2);
    let r = apply_operation(
        &store,
        r,
        "settle",
        Operation::PutRun {
            run,
        },
    );
    drop(store);
    assert_eq!(
        ReviewStore::open(root.path())
            .unwrap()
            .load(&r.id)
            .unwrap()
            .runs[0]
            .status,
        RunStatus::Cancelled
    );
}
#[test]
fn stalled_source_evidence_is_not_promoted_to_current_on_activation() {
    let root = tempfile::tempdir().unwrap();
    let checkout = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(checkout.path()).unwrap();
    let oid = commit(&repo, "needle\n");
    let store = ReviewStore::open(root.path()).unwrap();
    let c = capture(&store, &repo, &oid);
    let r = store
        .create("chat", "Review", Origin::Local, "create")
        .unwrap();
    let r = apply_operation(
        &store,
        r,
        "stage",
        Operation::StageCandidate {
            candidate: c.clone(),
        },
    );
    let mut anchor = store.anchor(&c, "file.txt", Side::Head, 1, 1).unwrap();
    anchor.status = AnchorStatus::Stalled;
    anchor.stalled_reason = Some("requires revalidation".into());
    let evidence = Evidence {
        id: "source".into(),
        candidate_id: c.id.clone(),
        material_id: None,
        run_id: None,
        source: EvidenceSource::Source {
            path: "file.txt".into(),
            anchor: Some(anchor.clone()),
        },
        summary: "Unverified source".into(),
        applicability: Applicability::Historical,
        created_at_ms: 1,
    };
    let r = apply_operation(
        &store,
        r,
        "evidence",
        Operation::AddEvidence {
            evidence,
        },
    );
    assert_eq!(r.current_candidate_id, None);
    let r = apply_operation(
        &store,
        r,
        "activate",
        Operation::ActivateCandidate {
            candidate_id: c.id.clone(),
        },
    );
    assert_eq!(r.current_candidate_id, Some(c.id));
    assert_eq!(
        r.evidence[0].applicability,
        Applicability::NeedsRevalidation
    );
    assert_eq!(
        r.evidence[0].source,
        EvidenceSource::Source {
            path: "file.txt".into(),
            anchor: Some(anchor)
        }
    );
}
#[test]
fn exact_rename_reconciles_but_preexisting_identical_copy_does_not() {
    for existing_copy in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let checkout = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(checkout.path()).unwrap();
        let oid = commit(&repo, "before\nneedle\nafter\n");
        if existing_copy {
            std::fs::copy(
                checkout.path().join("file.txt"),
                checkout.path().join("copy.txt"),
            )
            .unwrap();
        }
        let store = ReviewStore::open(root.path()).unwrap();
        let request = || CaptureRequest {
            base_oid: oid.clone(),
            head_oid: oid.clone(),
            policy: CapturePolicy {
                working: true,
                staged: false,
                untracked: true,
            },
            origin: Origin::Local,
        };
        let c1 = store.capture(checkout.path(), request()).unwrap();
        assert_eq!(c1.files.iter().any(|f| f.path == "copy.txt"), existing_copy);
        let anchor = store.anchor(&c1, "file.txt", Side::Head, 2, 2).unwrap();
        let r = store
            .create("chat", "Review", Origin::Local, "create")
            .unwrap();
        let r = apply_operation(
            &store,
            r,
            "stage1",
            Operation::StageCandidate {
                candidate: c1.clone(),
            },
        );
        let r = apply_operation(
            &store,
            r,
            "tour",
            Operation::PutTourStep {
                step: TourStep {
                    id: "step".into(),
                    current: TourVersion {
                        html: None,
                        blocks: vec![],
                        candidate_id: None,
                        version: 1,
                        title: "Needle".into(),
                        body: "Body".into(),
                        anchors: vec![anchor.clone()],
                    },
                    history: vec![],
                },
            },
        );
        if existing_copy {
            std::fs::remove_file(checkout.path().join("file.txt")).unwrap();
        } else {
            std::fs::rename(
                checkout.path().join("file.txt"),
                checkout.path().join("copy.txt"),
            )
            .unwrap();
        }
        // Use a new committed tree so the deleted path is absent on both sides.
        let mut index = repo.index().unwrap();
        index.remove_path(std::path::Path::new("file.txt")).unwrap();
        index.add_path(std::path::Path::new("copy.txt")).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("Review", "review@example.invalid").unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        let next = repo
            .commit(Some("HEAD"), &sig, &sig, "move", &tree, &[&parent])
            .unwrap()
            .to_string();
        let c2 = capture(&store, &repo, &next);
        assert!(!c2.files.iter().any(|f| f.path == "file.txt"));
        assert_eq!(
            c1.files.iter().find(|f| f.path == "file.txt").unwrap().head,
            c2.files.iter().find(|f| f.path == "copy.txt").unwrap().head
        );
        let r = apply_operation(
            &store,
            r,
            "stage2",
            Operation::StageCandidate {
                candidate: c2.clone(),
            },
        );
        let r = apply_operation(
            &store,
            r,
            "activate",
            Operation::ActivateCandidate {
                candidate_id: c2.id.clone(),
            },
        );
        let actual = &r.tour[0].current.anchors[0];
        if existing_copy {
            assert_eq!(actual.status, AnchorStatus::Stalled);
            assert_eq!(actual.position, anchor.position);
            assert_eq!(
                actual.stalled_reason.as_deref(),
                Some("path/side unavailable or non-text")
            );
        } else {
            assert_eq!(actual.status, AnchorStatus::Current);
            assert_eq!(actual.position.path, "copy.txt");
            assert_eq!(actual.position.candidate_id, c2.id);
            assert_eq!(actual.history, vec![anchor.position]);
        }
    }
}
#[test]
fn contribution_context_survives_restart_and_retry_without_rewriting_provenance() {
    let root = tempfile::tempdir().unwrap();
    let checkout = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(checkout.path()).unwrap();
    let oid = commit(&repo, "needle\n");
    let store = ReviewStore::open(root.path()).unwrap();
    let c = capture(&store, &repo, &oid);
    let r = store
        .create("chat", "Review", Origin::Local, "create")
        .unwrap();
    let r = apply_operation(
        &store,
        r,
        "stage",
        Operation::StageCandidate {
            candidate: c.clone(),
        },
    );
    let r = apply_operation(
        &store,
        r,
        "run",
        Operation::PutRun {
            run: run_for(&c),
        },
    );
    let context = ContributionContext {
        candidate_id: Some(c.id),
        run_id: Some("run".into()),
        applicability: Applicability::NeedsRevalidation,
        privacy: vec![refact_privacy::FileRecord {
            path: "file.txt".into(),
            zone: "private".into(),
            attribution: refact_privacy::Attribution::Observed,
        }],
    };
    let contribution = Contribution {
        blocks: vec![],
        id: "comment".into(),
        thread_id: "thread".into(),
        actor: actor(),
        body: "Exact comment".into(),
        created_at_ms: 1,
        chat_message_id: Some("message-original".into()),
        context: Some(context.clone()),
    };
    let mut request = mutation(
        r.revision,
        "comment",
        Operation::CreateThread {
            thread: Thread {
                target: None,
                id: "thread".into(),
                anchors: vec![],
                annotation_id: None,
                resolved: false,
                contributions: vec![],
            },
            contribution,
        },
    );
    let r = store.mutate(&r.id, request.clone()).unwrap();
    drop(store);
    let store = ReviewStore::open(root.path()).unwrap();
    assert_eq!(
        store.load(&r.id).unwrap().threads[0].contributions[0].context,
        Some(context)
    );
    if let Operation::CreateThread {
        contribution, ..
    } = &mut request.operation
    {
        contribution.context = None;
        contribution.created_at_ms = 99;
        contribution.chat_message_id = Some("retry-message".into());
    }
    assert_eq!(store.mutate(&r.id, request).unwrap(), r);
    assert_eq!(store.pending_deliveries(&r.id).unwrap().len(), 1);
    let mut changed = r.threads[0].contributions[0].clone();
    changed.context = None;
    assert!(
        matches!(store.mutate(&r.id, mutation(r.revision, "rewrite", Operation::AddContribution { contribution: changed })),
        Err(Error::Invalid(message)) if message == "contribution immutable")
    );
    assert_eq!(store.load(&r.id).unwrap(), r);
}

#[test]
fn atomic_snapshot_activation_rolls_back_and_retries_original_request() {
    let root = tempfile::tempdir().unwrap();
    let checkout = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(checkout.path()).unwrap();
    let oid = commit(&repo, "needle\n");
    let store = ReviewStore::open(root.path()).unwrap();
    let candidate = capture(&store, &repo, &oid);
    let r = store
        .create("chat", "Review", Origin::Local, "create")
        .unwrap();
    let material = |id: &str, kind| Material {
        id: id.into(),
        kind,
        candidate_id: candidate.id.clone(),
        agent_id: None,
        description: "Captured context".into(),
        content_digest: candidate.content_digest.clone(),
        input_digests: Default::default(),
        environment: None,
        worktree_id: None,
        worktree_root: None,
        artifacts: vec![],
    };
    let imported = material("imported", MaterialKind::Imported);
    let mut investigation = material(
        &format!("activation:{}", candidate.id),
        MaterialKind::Investigation,
    );
    investigation.worktree_id = Some("isolated-worktree".into());
    investigation.worktree_root = Some(checkout.path().to_string_lossy().into_owned());
    let request = mutation(
        0,
        "activate",
        Operation::ActivateSnapshot {
            candidate: candidate.clone(),
            imported: imported.clone(),
            investigation: investigation.clone(),
            target_reference: "target".into(),
        },
    );
    let db =
        rusqlite::Connection::open(root.path().join(".refact/reviews/reviews.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER fail_activation BEFORE UPDATE ON reviews BEGIN SELECT RAISE(ABORT, 'activation failpoint'); END;").unwrap();
    assert!(
        matches!(store.mutate(&r.id, request.clone()), Err(Error::Sql(rusqlite::Error::SqliteFailure(_, Some(message)))) if message == "activation failpoint")
    );
    assert_eq!(store.load(&r.id).unwrap(), r);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM requests WHERE scope=?1 AND key='activate'",
            [&r.id],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM object_revisions WHERE review_id=?1",
            [&r.id],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    db.execute_batch("DROP TRIGGER fail_activation;").unwrap();
    let activated = store.mutate(&r.id, request.clone()).unwrap();
    assert_eq!(activated.revision, 1);
    assert_eq!(activated.candidates, vec![candidate.clone()]);
    assert_eq!(activated.materials, vec![imported, investigation]);
    assert_eq!(activated.target_reference, "target");
    assert_eq!(activated.current_candidate_id, Some(candidate.id));
    drop(store);
    let store = ReviewStore::open(root.path()).unwrap();
    assert_eq!(store.mutate(&r.id, request).unwrap(), activated);
    assert_eq!(store.load(&r.id).unwrap(), activated);
}

#[test]
fn rich_topics_keep_exact_removed_block_targets_and_revisions() {
    let root = tempfile::tempdir().unwrap();
    let checkout = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(checkout.path()).unwrap();
    let oid = commit(&repo, "needle\n");
    let store = ReviewStore::open(root.path()).unwrap();
    let c = capture(&store, &repo, &oid);
    let other = capture(&store, &repo, &oid);
    let mut r = store
        .create("rich", "Rich", Origin::Local, "create")
        .unwrap();
    for (key, candidate) in [("stage", c.clone()), ("other", other.clone())] {
        r = apply_operation(
            &store,
            r,
            key,
            Operation::StageCandidate {
                candidate,
            },
        );
    }
    let anchor = store.anchor(&c, "file.txt", Side::Head, 1, 1).unwrap();
    let block = RichBlock {
        id: "code".into(),
        revision: 999,
        content: RichBlockContent::Code {
            anchor: anchor.clone(),
        },
    };
    let mut step = TourStep {
        id: "topic".into(),
        current: TourVersion {
            html: None,
            version: 1,
            title: "Topic".into(),
            body: String::new(),
            anchors: vec![],
            candidate_id: Some(c.id.clone()),
            blocks: vec![block],
        },
        history: vec![],
    };
    let request = mutation(
        r.revision,
        "rich",
        Operation::PutTourStep {
            step: step.clone(),
        },
    );
    r = store.mutate(&r.id, request.clone()).unwrap();
    assert_eq!(r.tour[0].current.blocks[0].revision, 1);
    assert_eq!(store.mutate(&r.id, request).unwrap(), r);
    step = r.tour[0].clone();
    step.current.version = 2;
    r = apply_operation(
        &store,
        r,
        "unchanged",
        Operation::PutTourStep {
            step: step.clone(),
        },
    );
    assert_eq!(r.tour[0].current.blocks[0].revision, 1);
    let target = TourDiscussionTarget {
        candidate_id: c.id.clone(),
        step_id: "topic".into(),
        version: 2,
        block_id: Some("code".into()),
        block_revision: Some(1),
    };
    let thread = Thread {
        id: "exact".into(),
        target: Some(target.clone()),
        anchors: vec![anchor],
        annotation_id: None,
        resolved: false,
        contributions: vec![],
    };
    r = apply_operation(
        &store,
        r,
        "thread",
        Operation::PutThread {
            thread,
        },
    );
    step.current.version = 3;
    step.current.blocks[0].content = RichBlockContent::Markdown {
        body: "changed".into(),
    };
    r = apply_operation(
        &store,
        r,
        "changed",
        Operation::PutTourStep {
            step: step.clone(),
        },
    );
    assert_eq!(r.tour[0].current.blocks[0].revision, 2);
    step.current.version = 4;
    step.current.blocks.clear();
    r = apply_operation(
        &store,
        r,
        "remove",
        Operation::PutTourStep {
            step: step.clone(),
        },
    );
    assert_eq!(
        rich_blocks::resolve_tour_target(&r, &target)
            .unwrap()
            .1
            .unwrap()
            .revision,
        1
    );
    for bad in [
        TourDiscussionTarget {
            candidate_id: other.id.clone(),
            ..target.clone()
        },
        TourDiscussionTarget {
            block_revision: Some(2),
            ..target.clone()
        },
        TourDiscussionTarget {
            block_revision: None,
            ..target.clone()
        },
    ] {
        assert!(rich_blocks::resolve_tour_target(&r, &bad).is_err());
        let mut thread = r.threads[0].clone();
        thread.id = format!("bad{:?}", bad);
        thread.target = Some(bad);
        assert!(store
            .mutate(
                &r.id,
                mutation(
                    r.revision,
                    &thread.id,
                    Operation::PutThread {
                        thread: thread.clone()
                    }
                )
            )
            .is_err());
    }
    r = apply_operation(
        &store,
        r,
        "activate-other",
        Operation::ActivateCandidate {
            candidate_id: other.id,
        },
    );
    assert_eq!(r.threads[0].target.as_ref(), Some(&target));
    assert_eq!(r.tour[0].current.candidate_id, Some(c.id));
    drop(store);
    let store = ReviewStore::open(root.path()).unwrap();
    assert_eq!(store.load(&r.id).unwrap(), r);
    assert!(rich_blocks::resolve_tour_target(&store.load(&r.id).unwrap(), &target).is_ok());
}

#[test]
fn rich_cross_candidate_paths_evidence_and_partial_writes_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let checkout = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(checkout.path()).unwrap();
    let oid = commit(&repo, "needle\n");
    let store = ReviewStore::open(root.path()).unwrap();
    let c = capture(&store, &repo, &oid);
    let other = capture(&store, &repo, &oid);
    let mut r = store
        .create("invalid-rich", "Rich", Origin::Local, "create")
        .unwrap();
    for (key, candidate) in [("stage", c.clone()), ("other", other.clone())] {
        r = apply_operation(
            &store,
            r,
            key,
            Operation::StageCandidate {
                candidate,
            },
        );
    }
    let evidence = Evidence {
        id: "source".into(),
        candidate_id: c.id.clone(),
        material_id: None,
        run_id: None,
        source: EvidenceSource::Source {
            path: "missing.txt".into(),
            anchor: None,
        },
        summary: String::new(),
        applicability: Applicability::Historical,
        created_at_ms: 1,
    };
    assert!(store
        .mutate(
            &r.id,
            mutation(
                r.revision,
                "bad-source",
                Operation::AddEvidence {
                    evidence: evidence.clone()
                }
            )
        )
        .is_err());
    let mut evidence = evidence;
    evidence.source = EvidenceSource::Source {
        path: "file.txt".into(),
        anchor: None,
    };
    r = apply_operation(
        &store,
        r,
        "source",
        Operation::AddEvidence {
            evidence,
        },
    );
    let invalid = vec![
        RichBlockContent::Code {
            anchor: store.anchor(&other, "file.txt", Side::Head, 1, 1).unwrap(),
        },
        RichBlockContent::Diff {
            candidate_id: c.id.clone(),
            path: "missing.txt".into(),
            anchors: vec![],
        },
        RichBlockContent::Evidence {
            evidence_ids: vec!["source".into()],
        },
        RichBlockContent::Sources {
            evidence_ids: vec!["missing".into()],
            relation: SourceRelation::Supports,
        },
    ];
    for (i, content) in invalid.into_iter().enumerate() {
        let contribution = Contribution {
            id: format!("bad-{i}"),
            thread_id: "partial".into(),
            actor: actor(),
            body: String::new(),
            blocks: vec![RichBlock {
                id: "bad".into(),
                revision: 1,
                content,
            }],
            created_at_ms: 1,
            chat_message_id: None,
            context: Some(ContributionContext {
                candidate_id: Some(c.id.clone()),
                run_id: None,
                applicability: Applicability::Historical,
                privacy: vec![],
            }),
        };
        let thread = Thread {
            id: "partial".into(),
            target: None,
            anchors: vec![],
            annotation_id: None,
            resolved: false,
            contributions: vec![],
        };
        assert!(store
            .mutate(
                &r.id,
                mutation(
                    r.revision,
                    &format!("bad-{i}"),
                    Operation::CreateThread {
                        thread,
                        contribution
                    }
                )
            )
            .is_err());
        assert_eq!(store.load(&r.id).unwrap(), r);
        assert!(store.pending_deliveries(&r.id).unwrap().is_empty());
        assert!(store.object_history(&r.id).unwrap().is_empty());
    }
}

#[test]
fn legacy_rich_fields_are_omitted_and_retry_fingerprint_survives_reload() {
    let raw = r#"{"op":"put_tour_step","step":{"id":"legacy","current":{"version":1,"title":"Legacy","body":"Body","anchors":[]},"history":[]}}"#;
    let op: Operation = serde_json::from_str(raw).unwrap();
    assert_eq!(serde_json::to_string(&op).unwrap(), raw);
    let root = tempfile::tempdir().unwrap();
    let store = ReviewStore::open(root.path()).unwrap();
    let r = store
        .create("legacy-rich", "Legacy", Origin::Local, "create")
        .unwrap();
    let request = mutation(0, "legacy", op);
    let first = store.mutate(&r.id, request.clone()).unwrap();
    let db =
        rusqlite::Connection::open(root.path().join(".refact/reviews/reviews.sqlite")).unwrap();
    let bytes = format!(
        "[0,\"legacy\",{},{}]",
        serde_json::to_string(&actor()).unwrap(),
        raw
    );
    let stored: String = db
        .query_row(
            "SELECT fingerprint FROM requests WHERE scope=?1 AND key='legacy'",
            [&r.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored, content_hash(bytes.as_bytes()));
    drop(store);
    let store = ReviewStore::open(root.path()).unwrap();
    assert_eq!(store.mutate(&r.id, request).unwrap(), first);
    assert_eq!(store.load(&r.id).unwrap(), first);
}
#[test]
fn captured_repository_identity_is_the_non_verbatim_canonical_root() {
    let root = tempfile::tempdir().unwrap();
    let checkout = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(checkout.path()).unwrap();
    let oid = commit(&repo, "one\n");
    let store = ReviewStore::open(root.path()).unwrap();
    let expected = dunce::canonicalize(checkout.path())
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(!expected.starts_with(r"\\?\"));
    let committed = capture(&store, &repo, &oid);
    assert_eq!(committed.repository, expected);
    let overlay = store
        .capture(
            checkout.path(),
            CaptureRequest {
                base_oid: oid.clone(),
                head_oid: oid,
                policy: CapturePolicy {
                    working: true,
                    staged: false,
                    untracked: false,
                },
                origin: Origin::Local,
            },
        )
        .unwrap();
    assert_eq!(overlay.repository, expected);
}
