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
fn intent(turn: &str) -> ProposalTurnCompletionIntent {
    ProposalTurnCompletionIntent {
        owner_chat_id: "owner".into(),
        turn_id: turn.into(),
        tool_call_id: "call".into(),
        job_id: Some("job".into()),
        human_message_id: Some("human".into()),
    }
}
fn bound(
    s: &ReviewStore,
    r: &Review,
    target: &TourDiscussionTarget,
) -> (Review, ProposalTurnCompletion) {
    let r = s
        .mutate_current_tour_revision_with_completion(
            "owner",
            &r.id,
            mutation(r, "proposal", ActorKind::Agent, propose(target)),
            None,
            Some(intent("original")),
        )
        .unwrap();
    let record = s
        .proposal_completion("owner", &r.tour_revision_proposals[0].id)
        .unwrap()
        .unwrap();
    (r, record)
}
fn acknowledged(s: &ReviewStore, record: &ProposalTurnCompletion) -> ProposalTurnCompletion {
    s.acknowledge_proposal_result(record, &["native-success".into()])
        .unwrap();
    s.proposal_completion("owner", &record.proposal_id)
        .unwrap()
        .unwrap()
}
#[test]
fn completion_and_proposal_rollback_together_and_replay_never_rebinds() {
    let (_root, s, r, target) = fixture();
    let request = mutation(&r, "proposal", ActorKind::Agent, propose(&target));
    let mut bad = intent("bad");
    bad.owner_chat_id = "foreign".into();
    assert!(s
        .mutate_current_tour_revision_with_completion(
            "owner",
            &r.id,
            request.clone(),
            None,
            Some(bad)
        )
        .is_err());
    assert_eq!(s.load(&r.id).unwrap(), r);
    assert!(s.pending_deliveries(&r.id).unwrap().is_empty());
    assert!(s.proposal_completions("owner").unwrap().is_empty());
    let (r, original) = bound(&s, &r, &target);
    assert_eq!(original.review_id, r.id);
    assert_eq!(original.candidate_id, target.candidate_id);
    assert_eq!(
        original.contribution_id,
        r.tour_revision_proposals[0].proposal_contribution_id
    );
    assert_eq!(
        s.pending_deliveries(&r.id).unwrap()[0].id,
        original.delivery_id
    );
    assert_eq!(
        s.mutate_current_tour_revision_with_completion(
            "owner",
            &r.id,
            request,
            None,
            Some(intent("new"))
        )
        .unwrap(),
        r
    );
    assert_eq!(
        s.proposal_completion("owner", &original.proposal_id)
            .unwrap(),
        Some(original)
    );
    assert!(s
        .proposal_completion("foreign", &r.tour_revision_proposals[0].id)
        .unwrap()
        .is_none());
}
#[test]
fn legacy_replay_cannot_mint_completion() {
    let (_root, s, r, target) = fixture();
    let request = mutation(&r, "legacy", ActorKind::Agent, propose(&target));
    let r = s.mutate(&r.id, request.clone()).unwrap();
    s.mutate_current_tour_revision_with_completion(
        "owner",
        &r.id,
        request,
        None,
        Some(intent("new")),
    )
    .unwrap();
    assert!(s
        .proposal_completion("owner", &r.tour_revision_proposals[0].id)
        .unwrap()
        .is_none());
}
#[test]
fn immutable_ack_settled_delivery_exact_consume_once_and_restart() {
    let (root, s, r, target) = fixture();
    let (_r, record) = bound(&s, &r, &target);
    assert!(!s.consume_proposal_completion(&record).unwrap());
    assert!(s.acknowledge_proposal_result(&record, &[]).is_err());
    let mut foreign = record.clone();
    foreign.turn_id = "changed".into();
    assert!(s
        .acknowledge_proposal_result(&foreign, &["native-success".into()])
        .is_err());
    let ack = acknowledged(&s, &record);
    s.acknowledge_proposal_result(&record, &ack.result_message_ids)
        .unwrap();
    assert!(s
        .acknowledge_proposal_result(&record, &["changed".into()])
        .is_err());
    assert!(!s.consume_proposal_completion(&ack).unwrap());
    s.settle_delivery(&record.delivery_id).unwrap();
    assert!(!s.consume_proposal_completion(&record).unwrap());
    drop(s);
    let s = ReviewStore::open(root.path()).unwrap();
    assert_eq!(
        s.proposal_completion("owner", &ack.proposal_id).unwrap(),
        Some(ack.clone())
    );
    assert!(s.consume_proposal_completion(&ack).unwrap());
    assert!(!s.consume_proposal_completion(&ack).unwrap());
    drop(s);
    let s = ReviewStore::open(root.path()).unwrap();
    assert!(!s.consume_proposal_completion(&ack).unwrap());
    assert!(s.proposal_completions("owner").unwrap().is_empty());
}
#[test]
fn stale_accepted_fulfilled_and_foreign_never_consume() {
    for mode in [
        "stale",
        "accepted",
        "fulfilled",
        "foreign",
        "candidate",
        "actor",
    ] {
        let (root, s, r, target) = fixture();
        let (mut r, record) = bound(&s, &r, &target);
        let mut ack = acknowledged(&s, &record);
        s.settle_delivery(&record.delivery_id).unwrap();
        match mode {
            "accepted" | "fulfilled" => {
                r = run(&s, &r, "accept", ActorKind::User, accept(&r, &target));
                if mode == "fulfilled" {
                    run(&s, &r, "apply", ActorKind::Agent, apply(&r, "Changed"));
                }
            }
            "stale" => {
                let mut step = r.tour[0].clone();
                step.current.version += 1;
                step.current.body = "Changed".into();
                run(
                    &s,
                    &r,
                    "update",
                    ActorKind::Agent,
                    Operation::PutTourStep {
                        step,
                    },
                );
            }
            "foreign" => ack.owner_chat_id = "foreign".into(),
            "candidate" | "actor" => {
                if mode == "candidate" {
                    r.current_candidate_id = None;
                } else {
                    r.threads[0].contributions[0].actor.id = "foreign".into();
                }
                let c =
                    rusqlite::Connection::open(root.path().join(".refact/reviews/reviews.sqlite"))
                        .unwrap();
                c.execute(
                    "UPDATE reviews SET document=?1 WHERE id=?2",
                    rusqlite::params![serde_json::to_string(&r).unwrap(), r.id],
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        assert!(!s.consume_proposal_completion(&ack).unwrap(), "{mode}");
        assert_eq!(s.proposal_completions("owner").unwrap().len(), 1);
    }
}
#[test]
fn cross_connection_active_switch_committed_before_consume_is_observed() {
    let (root, s, r, target) = fixture();
    let (r, record) = bound(&s, &r, &target);
    let ack = acknowledged(&s, &record);
    s.settle_delivery(&record.delivery_id).unwrap();
    assert_eq!(s.get_for_chat("owner").unwrap().unwrap().id, r.id);
    let mut c =
        rusqlite::Connection::open(root.path().join(".refact/reviews/reviews.sqlite")).unwrap();
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    tx.execute("DELETE FROM active_reviews WHERE chat='owner'", [])
        .unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        send.send(()).unwrap();
        s.consume_proposal_completion(&ack).unwrap()
    });
    receive.recv().unwrap();
    tx.commit().unwrap();
    assert!(!worker.join().unwrap());
}
