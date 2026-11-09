use super::{proposals, service};
use refact_review_core::*;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;
use tower::ServiceExt;

#[tokio::test]
async fn fulfilment_preserves_replay_and_refuses_inactive_review_and_foreign_blocks() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let (review, target) = fixture(&root);
    let proposal = mutation(
        &review,
        "propose",
        Operation::ProposeTourRevision {
            target: target.clone(),
            thread_id: "discussion".into(),
            change_request: "Clarify".into(),
            body: "Proposal".into(),
            blocks: vec![],
        },
    );
    let privacy = refact_privacy::FileRecord {
        path: "/private/source.rs".into(),
        zone: "private".into(),
        attribution: refact_privacy::Attribution::Declared,
    };
    let proposed = service::model_mutate_with_privacy(
        root.clone(),
        "owner".into(),
        review.id.clone(),
        "owner".into(),
        proposal.clone(),
        vec![privacy.clone()],
    )
    .await
    .unwrap();
    assert_eq!(
        proposed.threads[0].contributions[0]
            .context
            .as_ref()
            .unwrap()
            .privacy,
        vec![privacy]
    );
    let id = proposed.tour_revision_proposals[0].id.clone();
    let request = proposals::AcceptRequest {
        owner_chat_id: "owner".into(),
        expected_revision: proposed.revision,
        request_key: "accept".into(),
        target,
    };
    let accepted =
        proposals::accept_owned(root.clone(), review.id.clone(), request.clone(), id.clone())
            .await
            .unwrap();
    let mut current = accepted.tour[0].current.clone();
    current.version += 1;
    current.body = "Clarified".into();
    let mut foreign = current.clone();
    foreign.blocks = serde_json::from_value(json!([{"id":"foreign","revision":1,"content":{"kind":"diff","candidate_id":"foreign","path":"file.rs","anchors":[]}}])).unwrap();
    let invalid = mutation(
        &accepted,
        "invalid",
        Operation::ApplyTourRevision {
            proposal_id: id.clone(),
            current: foreign,
        },
    );
    assert!(service::model_mutate(
        root.clone(),
        "owner".into(),
        review.id.clone(),
        "owner".into(),
        invalid
    )
    .await
    .is_err());
    let apply = mutation(
        &accepted,
        "apply",
        Operation::ApplyTourRevision {
            proposal_id: id.clone(),
            current,
        },
    );
    assert!(service::user_mutate(
        root.clone(),
        "owner".into(),
        review.id.clone(),
        apply.clone()
    )
    .await
    .is_err());
    let fulfilled = service::model_mutate(
        root.clone(),
        "owner".into(),
        review.id.clone(),
        "owner".into(),
        apply.clone(),
    )
    .await
    .unwrap();
    assert!(proposals::pending(&fulfilled).is_empty());
    assert_eq!(fulfilled.tour[0].history.len(), 1);
    assert_eq!(
        fulfilled,
        proposals::accept_owned(root.clone(), review.id.clone(), request.clone(), id.clone())
            .await
            .unwrap()
    );
    assert_eq!(
        fulfilled,
        service::model_mutate(
            root.clone(),
            "owner".into(),
            review.id.clone(),
            "owner".into(),
            proposal.clone()
        )
        .await
        .unwrap()
    );
    assert_eq!(
        fulfilled,
        service::model_mutate(
            root.clone(),
            "owner".into(),
            review.id.clone(),
            "owner".into(),
            apply
        )
        .await
        .unwrap()
    );
    let mut alternate = fulfilled.clone();
    alternate.id = "alternate-owner-review".into();
    let connection =
        rusqlite::Connection::open(root.join(".refact/reviews/reviews.sqlite")).unwrap();
    connection
        .execute(
            "INSERT INTO reviews VALUES(?1,?2,?3)",
            rusqlite::params![
                alternate.id,
                alternate.owner_chat_id,
                serde_json::to_string(&alternate).unwrap()
            ],
        )
        .unwrap();
    assert_eq!(
        connection
            .execute(
                "UPDATE active_reviews SET review_id=?1 WHERE chat='owner'",
                [&alternate.id],
            )
            .unwrap(),
        1
    );
    assert_eq!(
        ReviewStore::open(&root)
            .unwrap()
            .get_for_chat("owner")
            .unwrap()
            .unwrap()
            .id,
        alternate.id
    );
    assert!(
        proposals::accept_owned(root.clone(), review.id.clone(), request, id)
            .await
            .is_err()
    );
    assert!(
        service::model_mutate(root, "owner".into(), review.id, "owner".into(), proposal)
            .await
            .is_err()
    );
}

fn mutation(review: &Review, key: &str, operation: Operation) -> Mutation {
    Mutation {
        expected_revision: review.revision,
        request_key: key.into(),
        actor: Actor {
            kind: ActorKind::Agent,
            id: review.owner_chat_id.clone(),
        },
        at_ms: 1,
        operation,
    }
}
fn fixture(root: &std::path::Path) -> (Review, TourDiscussionTarget) {
    let repo = git2::Repository::init(root).unwrap();
    let tree_id = repo.index().unwrap().write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let sig = git2::Signature::now("test", "test@example.test").unwrap();
    let oid = repo
        .commit(Some("HEAD"), &sig, &sig, "fixture", &tree, &[])
        .unwrap()
        .to_string();
    let store = ReviewStore::open(root).unwrap();
    let candidate = store
        .capture(
            root,
            CaptureRequest {
                base_oid: oid.clone(),
                head_oid: oid,
                policy: CapturePolicy::default(),
                origin: Origin::Local,
            },
        )
        .unwrap();
    let mut review = store
        .create("owner", "Review", Origin::Local, "create")
        .unwrap();
    for (key, op) in [
        (
            "stage",
            Operation::StageCandidate {
                candidate: candidate.clone(),
            },
        ),
        (
            "activate",
            Operation::ActivateCandidate {
                candidate_id: candidate.id.clone(),
            },
        ),
    ] {
        review = store
            .mutate(&review.id, mutation(&review, key, op))
            .unwrap();
    }
    let step: TourStep = serde_json::from_value(json!({"id":"step","current":{"version":1,"title":"Title","body":"Original","anchors":[],"candidate_id":candidate.id},"history":[]})).unwrap();
    review = store
        .mutate(
            &review.id,
            mutation(
                &review,
                "tour",
                Operation::PutTourStep {
                    step,
                },
            ),
        )
        .unwrap();
    let target = TourDiscussionTarget {
        candidate_id: candidate.id,
        step_id: "step".into(),
        version: 1,
        block_id: None,
        block_revision: None,
    };
    let thread = Thread {
        id: "discussion".into(),
        anchors: vec![],
        annotation_id: None,
        target: Some(target.clone()),
        resolved: false,
        contributions: vec![],
    };
    review = store
        .mutate(
            &review.id,
            mutation(
                &review,
                "thread",
                Operation::PutThread {
                    thread,
                },
            ),
        )
        .unwrap();
    (review, target)
}

#[tokio::test]
async fn user_only_router_acceptance_reopens_pending_and_delivers_once_without_wake() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let (mut review, target) = fixture(&root);
    let mut ordinary = mutation(
        &review,
        "ordinary-pending",
        Operation::AddContribution {
            contribution: Contribution {
                blocks: vec![],
                id: "ordinary-user".into(),
                thread_id: "discussion".into(),
                actor: Actor {
                    kind: ActorKind::User,
                    id: "user".into(),
                },
                body: "Earlier ordinary question".into(),
                created_at_ms: 1,
                chat_message_id: None,
                context: None,
            },
        },
    );
    ordinary.actor.kind = ActorKind::User;
    ordinary.actor.id = "user".into();
    review = ReviewStore::open(&root)
        .unwrap()
        .mutate(&review.id, ordinary)
        .unwrap();
    assert!(proposals::contribution_wakes(
        &review,
        &review.threads[0].contributions[0]
    ));
    let proposal = mutation(
        &review,
        "proposal",
        Operation::ProposeTourRevision {
            target: target.clone(),
            thread_id: "discussion".into(),
            change_request: "Clarify scope".into(),
            body: "I propose clarifying scope".into(),
            blocks: vec![],
        },
    );
    assert!(service::user_mutate(
        root.clone(),
        "owner".into(),
        review.id.clone(),
        proposal.clone()
    )
    .await
    .is_err());
    assert!(service::model_mutate(
        root.clone(),
        "owner".into(),
        review.id.clone(),
        "child".into(),
        proposal.clone()
    )
    .await
    .is_err());
    let proposed = service::model_mutate(
        root.clone(),
        "owner".into(),
        review.id.clone(),
        "owner".into(),
        proposal.clone(),
    )
    .await
    .unwrap();
    let id = proposed.tour_revision_proposals[0].id.clone();
    let req = proposals::AcceptRequest {
        owner_chat_id: "owner".into(),
        expected_revision: proposed.revision,
        request_key: "accept".into(),
        target: target.clone(),
    };
    let forged = mutation(
        &proposed,
        "forged",
        Operation::AcceptTourRevision {
            proposal_id: id.clone(),
            target: target.clone(),
        },
    );
    assert!(service::user_mutate(
        root.clone(),
        "owner".into(),
        review.id.clone(),
        forged.clone()
    )
    .await
    .is_err());
    assert!(service::model_mutate(
        root.clone(),
        "owner".into(),
        review.id.clone(),
        "owner".into(),
        forged
    )
    .await
    .is_err());
    let mut wrong = req.clone();
    wrong.owner_chat_id = "other".into();
    assert!(
        proposals::accept_owned(root.clone(), review.id.clone(), wrong, id.clone())
            .await
            .is_err()
    );
    let mut stale = req.clone();
    stale.target.version += 1;
    assert!(
        proposals::accept_owned(root.clone(), review.id.clone(), stale, id.clone())
            .await
            .is_err()
    );
    let gcx = crate::global_context::tests::make_test_gcx().await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.clone()];
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    let mut session = crate::chat::types::ChatSession::new("owner".into());
    session.work_project_root = Some(root.clone());
    session.start_stream().unwrap();
    let session = Arc::new(Mutex::new(session));
    app.chat
        .sessions
        .write()
        .await
        .insert("owner".into(), session.clone());
    let router = super::http::router().with_state(app.clone());
    for _ in 0..2 {
        let response = router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/reviews/{}/tour-revision-proposals/{id}/accept",
                        review.id
                    ))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(serde_json::to_vec(&req).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), hyper::StatusCode::OK);
    }
    let accepted = ReviewStore::open(&root).unwrap().load(&review.id).unwrap();
    assert_eq!(
        proposals::pending(&accepted)[0]["status"],
        "accepted_pending"
    );
    assert_eq!(accepted.tour[0].current.body, "Original");
    assert_eq!(
        accepted,
        proposals::accept_owned(root.clone(), review.id.clone(), req, id.clone())
            .await
            .unwrap()
    );
    assert_eq!(
        accepted,
        service::model_mutate(
            root.clone(),
            "owner".into(),
            review.id.clone(),
            "owner".into(),
            proposal
        )
        .await
        .unwrap()
    );
    super::chat_bridge::flush_read_only(&app, root, &accepted)
        .await
        .unwrap();
    let mut session = session.lock().await;
    assert_eq!(session.pending_deliveries.len(), 3);
    for delivery in &session.pending_deliveries {
        assert!(!delivery.wake);
        let reference = &delivery.messages.last().unwrap().extra["review"];
        if reference["contribution_id"] == "ordinary-user" {
            assert!(reference["tour_revision_proposal_id"].is_null());
        } else {
            assert_eq!(reference["tour_revision_proposal_id"], id);
        }
    }
    session.finish_stream(None);
    session.drain_pending_deliveries();
    assert_eq!(
        session
            .messages
            .iter()
            .filter(|m| m.extra.contains_key("review"))
            .count(),
        3
    );
}
