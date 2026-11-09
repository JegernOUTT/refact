use refact_review_core::*;

fn actor(kind: ActorKind) -> Actor {
    Actor {
        kind,
        id: "owner".into(),
    }
}
fn mutation(r: &Review, key: &str, kind: ActorKind, operation: Operation) -> Mutation {
    Mutation {
        expected_revision: r.revision,
        request_key: key.into(),
        actor: actor(kind),
        at_ms: 42,
        operation,
    }
}
fn run(s: &ReviewStore, r: &Review, key: &str, kind: ActorKind, op: Operation) -> Review {
    s.mutate(&r.id, mutation(r, key, kind, op)).unwrap()
}
fn fixture() -> (tempfile::TempDir, ReviewStore, Review, TourDiscussionTarget) {
    let root = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(root.path()).unwrap();
    let tree_id = repo.index().unwrap().write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let sig = git2::Signature::new("Test", "test@localhost", &git2::Time::new(1, 0)).unwrap();
    let oid = repo
        .commit(Some("HEAD"), &sig, &sig, "fixture", &tree, &[])
        .unwrap()
        .to_string();
    let s = ReviewStore::open(root.path()).unwrap();
    let candidate = s
        .capture(
            root.path(),
            CaptureRequest {
                base_oid: oid.clone(),
                head_oid: oid,
                policy: CapturePolicy::default(),
                origin: Origin::Local,
            },
        )
        .unwrap();
    let r = s
        .create("owner", "review", Origin::Local, "create")
        .unwrap();
    let r = run(
        &s,
        &r,
        "stage",
        ActorKind::User,
        Operation::StageCandidate {
            candidate: candidate.clone(),
        },
    );
    let r = run(
        &s,
        &r,
        "activate",
        ActorKind::User,
        Operation::ActivateCandidate {
            candidate_id: candidate.id.clone(),
        },
    );
    let version = TourVersion {
        context_intake: None,
        candidate_id: Some(candidate.id.clone()),
        version: 1,
        title: "Topic".into(),
        body: "Original".into(),
        anchors: vec![],
        blocks: vec![RichBlock {
            id: "block".into(),
            revision: 1,
            content: RichBlockContent::Markdown {
                body: "Original block".into(),
            },
        }],
    };
    let r = run(
        &s,
        &r,
        "tour",
        ActorKind::Agent,
        Operation::PutTourStep {
            step: TourStep {
                id: "topic".into(),
                current: version,
                history: vec![],
            },
        },
    );
    let target = TourDiscussionTarget {
        candidate_id: candidate.id,
        step_id: "topic".into(),
        version: 1,
        block_id: Some("block".into()),
        block_revision: Some(1),
    };
    let r = run(
        &s,
        &r,
        "thread",
        ActorKind::User,
        Operation::PutThread {
            thread: Thread {
                id: "thread".into(),
                target: Some(target.clone()),
                anchors: vec![],
                annotation_id: None,
                resolved: false,
                contributions: vec![],
            },
        },
    );
    (root, s, r, target)
}
fn propose(target: &TourDiscussionTarget) -> Operation {
    Operation::ProposeTourRevision {
        target: target.clone(),
        thread_id: "thread".into(),
        change_request: "Explain the trade-off".into(),
        body: "I propose explaining the trade-off.".into(),
        blocks: vec![],
    }
}
fn accept(r: &Review, target: &TourDiscussionTarget) -> Operation {
    Operation::AcceptTourRevision {
        proposal_id: r.tour_revision_proposals[0].id.clone(),
        target: target.clone(),
    }
}
fn apply(r: &Review, body: &str) -> Operation {
    let mut current = r.tour[0].current.clone();
    current.version += 1;
    current.body = body.into();
    Operation::ApplyTourRevision {
        proposal_id: r.tour_revision_proposals[0].id.clone(),
        current,
    }
}
#[test]
fn guarded_revision_checks_persisted_active_slot_before_operation_and_replay() {
    let (root, s, r, target) = fixture();
    let other =
        rusqlite::Connection::open(root.path().join(".refact/reviews/reviews.sqlite")).unwrap();
    let mut alternate = r.clone();
    alternate.id = "alternate-owner-review".into();
    other
        .execute(
            "INSERT INTO reviews VALUES(?1,?2,?3)",
            rusqlite::params![
                alternate.id,
                alternate.owner_chat_id,
                serde_json::to_string(&alternate).unwrap()
            ],
        )
        .unwrap();
    let switch = |id: &str| {
        other
            .execute(
                "UPDATE active_reviews SET review_id=?1 WHERE chat='owner'",
                [id],
            )
            .unwrap();
    };
    let proposal_request = mutation(&r, "guarded-propose", ActorKind::Agent, propose(&target));
    assert_eq!(s.get_for_chat("owner").unwrap().unwrap().id, r.id);
    switch(&alternate.id);
    assert!(s
        .mutate_current_tour_revision("owner", &r.id, proposal_request.clone(), None)
        .is_err());
    assert_eq!(s.load(&r.id).unwrap(), r);
    assert!(s.pending_deliveries(&r.id).unwrap().is_empty());
    switch(&r.id);
    let context = ContributionContext {
        candidate_id: Some(target.candidate_id.clone()),
        run_id: None,
        applicability: Applicability::Current,
        privacy: vec![],
    };
    let r = s
        .mutate_current_tour_revision(
            "owner",
            &r.id,
            proposal_request.clone(),
            Some(context.clone()),
        )
        .unwrap();
    assert_eq!(r.threads[0].contributions[0].context, Some(context));
    let accept_request = mutation(&r, "guarded-accept", ActorKind::User, accept(&r, &target));
    switch(&alternate.id);
    for request in [proposal_request.clone(), accept_request.clone()] {
        assert!(s
            .mutate_current_tour_revision("owner", &r.id, request, None)
            .is_err());
    }
    assert_eq!(s.load(&r.id).unwrap(), r);
    switch(&r.id);
    let r = s
        .mutate_current_tour_revision("owner", &r.id, accept_request.clone(), None)
        .unwrap();
    assert_eq!(
        s.mutate_current_tour_revision("owner", &r.id, accept_request.clone(), None)
            .unwrap(),
        r
    );
    let apply_request = mutation(
        &r,
        "guarded-apply",
        ActorKind::Agent,
        apply(&r, "Guarded authored content"),
    );
    switch(&alternate.id);
    assert!(s
        .mutate_current_tour_revision("owner", &r.id, apply_request.clone(), None)
        .is_err());
    assert!(s
        .mutate_current_tour_revision("owner", &r.id, accept_request.clone(), None)
        .is_err());
    assert_eq!(s.load(&r.id).unwrap(), r);
    switch(&r.id);
    let r = s
        .mutate_current_tour_revision("owner", &r.id, apply_request.clone(), None)
        .unwrap();
    for request in [proposal_request, accept_request, apply_request.clone()] {
        assert_eq!(
            s.mutate_current_tour_revision("owner", &r.id, request, None)
                .unwrap(),
            r
        );
    }
    assert!(s
        .mutate_current_tour_revision("foreign", &r.id, apply_request.clone(), None)
        .is_err());
    assert!(s
        .mutate_current_tour_revision(
            "owner",
            &r.id,
            mutation(
                &r,
                "not-revision",
                ActorKind::User,
                Operation::Rename {
                    title: "No".into()
                }
            ),
            None
        )
        .is_err());
    switch(&alternate.id);
    assert!(s
        .mutate_current_tour_revision("owner", &r.id, apply_request, None)
        .is_err());
    assert_eq!(s.load(&r.id).unwrap(), r);
    assert_eq!(s.get_for_chat("owner").unwrap().unwrap().id, alternate.id);
    assert_eq!(s.pending_deliveries(&r.id).unwrap().len(), 2);
}
#[test]
fn trusted_proposal_context_is_atomic_persisted_and_excluded_from_replay_fingerprint() {
    let (root, s, r, target) = fixture();
    let request = mutation(&r, "context-proposal", ActorKind::Agent, propose(&target));
    let context = ContributionContext {
        candidate_id: Some(target.candidate_id.clone()),
        run_id: None,
        applicability: Applicability::Current,
        privacy: vec![],
    };
    for invalid_context in [
        ContributionContext {
            candidate_id: Some("other".into()),
            ..context.clone()
        },
        ContributionContext {
            run_id: Some("run".into()),
            ..context.clone()
        },
        ContributionContext {
            applicability: Applicability::Historical,
            ..context.clone()
        },
    ] {
        assert!(s
            .mutate_tour_revision_with_context(&r.id, request.clone(), invalid_context)
            .is_err());
        assert_eq!(s.load(&r.id).unwrap(), r);
        assert!(s.pending_deliveries(&r.id).unwrap().is_empty());
    }
    let r = s
        .mutate_tour_revision_with_context(&r.id, request.clone(), context.clone())
        .unwrap();
    assert_eq!(r.threads[0].contributions[0].context, Some(context.clone()));
    let r = run(
        &s,
        &r,
        "accept-context",
        ActorKind::User,
        accept(&r, &target),
    );
    let r = run(
        &s,
        &r,
        "apply-context",
        ActorKind::Agent,
        apply(&r, "Authored"),
    );
    let changed_context = ContributionContext {
        candidate_id: Some("runtime-changed".into()),
        ..context.clone()
    };
    assert_eq!(
        s.mutate_tour_revision_with_context(&r.id, request, changed_context)
            .unwrap(),
        r
    );
    assert!(s
        .mutate_tour_revision_with_context(
            &r.id,
            mutation(
                &r,
                "bad-op",
                ActorKind::User,
                Operation::Rename {
                    title: "No".into()
                }
            ),
            context
        )
        .is_err());
    drop(s);
    assert_eq!(
        ReviewStore::open(root.path()).unwrap().load(&r.id).unwrap(),
        r
    );
}
#[test]
fn durable_chat_consent_authored_fulfilment_and_replay() {
    let (root, s, r, target) = fixture();
    let original = r.tour.clone();
    let proposal_request = mutation(&r, "propose", ActorKind::Agent, propose(&target));
    let r = s.mutate(&r.id, proposal_request.clone()).unwrap();
    assert_eq!(r.tour, original);
    assert_eq!(r.threads[0].contributions.len(), 1);
    assert_eq!(s.pending_deliveries(&r.id).unwrap().len(), 1);
    assert!(r.tour_revision_proposals[0].acceptance.is_none());
    assert!(s
        .mutate(
            &r.id,
            mutation(&r, "premature", ActorKind::Agent, apply(&r, "Changed"))
        )
        .is_err());
    let acceptance_request = mutation(&r, "accept", ActorKind::User, accept(&r, &target));
    let r = s.mutate(&r.id, acceptance_request.clone()).unwrap();
    assert_eq!(r.tour, original);
    assert_eq!(r.threads[0].contributions.len(), 2);
    assert!(r.tour_revision_proposals[0].acceptance.is_some());
    assert!(r.tour_revision_proposals[0].fulfilment.is_none());
    let apply_request = mutation(
        &r,
        "apply",
        ActorKind::Agent,
        apply(&r, "Authored trade-off"),
    );
    let r = s.mutate(&r.id, apply_request.clone()).unwrap();
    assert_eq!(r.tour[0].history, vec![original[0].current.clone()]);
    assert_eq!(
        r.tour_revision_proposals[0]
            .fulfilment
            .as_ref()
            .unwrap()
            .version,
        2
    );
    assert!(!rich_blocks::is_tour_revision_target_current(&r, &target));
    assert!(rich_blocks::resolve_tour_target(&r, &target).is_ok());
    assert!(s
        .object_history(&r.id)
        .unwrap()
        .iter()
        .any(|h| matches!(h.object, HistoricalObject::TourStep(_))));
    for request in [proposal_request.clone(), acceptance_request, apply_request] {
        assert_eq!(s.mutate(&r.id, request).unwrap(), r);
    }
    let mut changed = proposal_request;
    changed.operation = Operation::Rename {
        title: "different".into(),
    };
    assert!(matches!(
        s.mutate(&r.id, changed),
        Err(Error::IdempotencyConflict)
    ));
    assert_eq!(s.pending_deliveries(&r.id).unwrap().len(), 2);
    drop(s);
    let s = ReviewStore::open(root.path()).unwrap();
    assert_eq!(s.load(&r.id).unwrap(), r);
    assert_eq!(s.pending_deliveries(&r.id).unwrap().len(), 2);
}
#[test]
fn rejects_foreign_stale_noop_and_rolls_back_outbox() {
    let (_root, s, r, target) = fixture();
    let mut foreign = mutation(&r, "foreign", ActorKind::Agent, propose(&target));
    foreign.actor.id = "other".into();
    assert!(s.mutate(&r.id, foreign).is_err());
    let mut wrong = target.clone();
    wrong.block_revision = Some(99);
    assert!(s
        .mutate(
            &r.id,
            mutation(&r, "wrong", ActorKind::Agent, propose(&wrong))
        )
        .is_err());
    let mut invalid = propose(&target);
    if let Operation::ProposeTourRevision {
        blocks, ..
    } = &mut invalid
    {
        blocks.push(RichBlock {
            id: "bad".into(),
            revision: 1,
            content: RichBlockContent::Evidence {
                evidence_ids: vec!["missing".into()],
            },
        });
    }
    assert!(s
        .mutate(&r.id, mutation(&r, "proposal", ActorKind::Agent, invalid))
        .is_err());
    assert_eq!(s.load(&r.id).unwrap(), r);
    assert!(s.pending_deliveries(&r.id).unwrap().is_empty());
    let r = run(&s, &r, "proposal", ActorKind::Agent, propose(&target));
    assert!(s
        .mutate(
            &r.id,
            mutation(&r, "agent-accept", ActorKind::Agent, accept(&r, &target))
        )
        .is_err());
    assert!(s
        .mutate(
            &r.id,
            mutation(&r, "wrong-accept", ActorKind::User, accept(&r, &wrong))
        )
        .is_err());
    let r = run(&s, &r, "accept", ActorKind::User, accept(&r, &target));
    let mut noop = apply(&r, &r.tour[0].current.body);
    if let Operation::ApplyTourRevision {
        current, ..
    } = &mut noop
    {
        current.blocks[0].revision = 999;
    }
    assert!(s
        .mutate(&r.id, mutation(&r, "apply", ActorKind::Agent, noop))
        .is_err());
    assert_eq!(s.load(&r.id).unwrap(), r);
    let mut wrong_candidate = apply(&r, "Changed");
    if let Operation::ApplyTourRevision {
        current, ..
    } = &mut wrong_candidate
    {
        current.candidate_id = Some("foreign".into());
    }
    assert!(s
        .mutate(
            &r.id,
            mutation(&r, "apply", ActorKind::Agent, wrong_candidate)
        )
        .is_err());
    let mut ordinary = r.tour[0].clone();
    ordinary.current.version += 1;
    ordinary.current.body = "Ordinary update".into();
    let r = run(
        &s,
        &r,
        "ordinary",
        ActorKind::Agent,
        Operation::PutTourStep {
            step: ordinary,
        },
    );
    assert!(r.tour_revision_proposals[0].fulfilment.is_none());
    assert!(s
        .mutate(
            &r.id,
            mutation(&r, "apply", ActorKind::Agent, apply(&r, "Stale"))
        )
        .is_err());
    assert!(s
        .mutate(
            &r.id,
            mutation(&r, "stale-proposal", ActorKind::Agent, propose(&target))
        )
        .is_err());
    assert_eq!(s.load(&r.id).unwrap(), r);
}
#[test]
fn candidate_switch_stales_pending_without_rebinding_and_foreign_apply_is_rejected() {
    let (root, s, r, target) = fixture();
    let r = run(&s, &r, "proposal", ActorKind::Agent, propose(&target));
    let r = run(&s, &r, "accept", ActorKind::User, accept(&r, &target));
    assert!(s
        .mutate(
            &r.id,
            mutation(&r, "accept-again", ActorKind::User, accept(&r, &target))
        )
        .is_err());
    assert!(s
        .mutate(
            &r.id,
            mutation(&r, "user-apply", ActorKind::User, apply(&r, "Changed"))
        )
        .is_err());
    let mut foreign = mutation(&r, "foreign-apply", ActorKind::Agent, apply(&r, "Changed"));
    foreign.actor.id = "other".into();
    assert!(s.mutate(&r.id, foreign).is_err());
    let repo = git2::Repository::open(root.path()).unwrap();
    let oid = repo.head().unwrap().target().unwrap().to_string();
    let candidate = s
        .capture(
            root.path(),
            CaptureRequest {
                base_oid: oid.clone(),
                head_oid: oid,
                policy: CapturePolicy {
                    staged: true,
                    working: false,
                    untracked: false,
                },
                origin: Origin::Local,
            },
        )
        .unwrap();
    assert_ne!(candidate.id, target.candidate_id);
    let r = run(
        &s,
        &r,
        "stage-next",
        ActorKind::User,
        Operation::StageCandidate {
            candidate: candidate.clone(),
        },
    );
    let r = run(
        &s,
        &r,
        "activate-next",
        ActorKind::User,
        Operation::ActivateCandidate {
            candidate_id: candidate.id,
        },
    );
    assert!(!rich_blocks::is_tour_revision_target_current(&r, &target));
    assert_eq!(r.tour_revision_proposals[0].target, target);
    assert!(r.tour_revision_proposals[0].acceptance.is_some());
    assert!(r.tour_revision_proposals[0].fulfilment.is_none());
    assert!(s
        .mutate(
            &r.id,
            mutation(&r, "stale-apply", ActorKind::Agent, apply(&r, "Changed"))
        )
        .is_err());
    assert!(s
        .mutate(
            &r.id,
            mutation(&r, "stale-accept", ActorKind::User, accept(&r, &target))
        )
        .is_err());
    assert_eq!(s.load(&r.id).unwrap(), r);
}
#[test]
fn ordinary_replies_never_create_proposals_and_old_json_defaults() {
    let (_root, s, r, _target) = fixture();
    let r = run(
        &s,
        &r,
        "reply",
        ActorKind::Agent,
        Operation::AddContribution {
            contribution: Contribution {
                id: "reply".into(),
                thread_id: "thread".into(),
                actor: actor(ActorKind::Agent),
                body: "Could revise this.".into(),
                blocks: vec![],
                created_at_ms: 42,
                chat_message_id: None,
                context: None,
            },
        },
    );
    assert!(r.tour_revision_proposals.is_empty());
    let mut json = serde_json::to_value(&r).unwrap();
    json.as_object_mut()
        .unwrap()
        .remove("tour_revision_proposals");
    assert!(serde_json::from_value::<Review>(json)
        .unwrap()
        .tour_revision_proposals
        .is_empty());
    let json = serde_json::to_value(propose(r.threads[0].target.as_ref().unwrap())).unwrap();
    let mut legacy = json;
    legacy.as_object_mut().unwrap().remove("blocks");
    assert!(serde_json::from_value::<Operation>(legacy).is_ok());
}

#[test]
fn trusted_revision_intake_preserves_consent_history_and_rejects_receipt_only_noop() {
    let (root, store, r, target) = fixture();
    let r = run(
        &store,
        &r,
        "intake-propose",
        ActorKind::Agent,
        propose(&target),
    );
    let candidate = r.candidates[0].clone();
    let receipt = ContextIntakeReceipt {
        id: "intake".into(),
        phase: "before_tour".into(),
        owner_chat_id: "owner".into(),
        repository: candidate.repository,
        base_oid: candidate.base_oid,
        head_oid: candidate.head_oid,
        candidate_id: Some(candidate.id),
        captured_at_ms: 100,
        input_reference: ContextInputReference {
            kind: "user_directive".into(),
            message_id: Some("user".into()),
            tool_call_id: None,
            status: "cited_user_intent".into(),
            content_hash: content_hash(b"review"),
        },
        consultations: vec![],
        gaps: vec![],
        summary: "Context".into(),
        privacy: vec![],
    };
    let request = mutation(
        &r,
        "intake-premature",
        ActorKind::Agent,
        apply(&r, "Authored"),
    );
    assert!(store
        .mutate_current_tour_revision_with_intake("owner", &r.id, request, None, receipt.clone())
        .is_err());
    let r = run(
        &store,
        &r,
        "intake-accept",
        ActorKind::User,
        accept(&r, &target),
    );
    let noop = mutation(
        &r,
        "intake-noop",
        ActorKind::Agent,
        apply(&r, &r.tour[0].current.body),
    );
    assert!(store
        .mutate_current_tour_revision_with_intake("owner", &r.id, noop, None, receipt.clone())
        .is_err());
    let request = mutation(&r, "intake-apply", ActorKind::Agent, apply(&r, "Authored"));
    let r = store
        .mutate_current_tour_revision_with_intake(
            "owner",
            &r.id,
            request.clone(),
            None,
            receipt.clone(),
        )
        .unwrap();
    assert_eq!(r.tour[0].current.context_intake, Some(receipt.clone()));
    assert!(r.tour[0].history[0].context_intake.is_none());
    assert!(r.tour_revision_proposals[0].fulfilment.is_some());
    assert_eq!(
        store.intake_for_request(&r.id, "intake-apply").unwrap(),
        Some(receipt.clone())
    );
    drop(store);
    let store = ReviewStore::open(root.path()).unwrap();
    assert_eq!(
        store
            .mutate_current_tour_revision_with_intake("owner", &r.id, request, None, receipt)
            .unwrap(),
        r
    );
}
