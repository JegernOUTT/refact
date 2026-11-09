use super::service::{self, CommentRequest};
use refact_review_core::{Origin, ReviewStore, Error};
use std::sync::Arc;
use tokio::sync::Mutex;

#[tokio::test]
async fn canonical_comment_survives_retry_and_rejects_wrong_owner() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let store = ReviewStore::open(&root).unwrap();
    let review = store
        .create("review-owner", "Review", Origin::Local, "create")
        .unwrap();
    drop(store);
    let gcx = crate::global_context::tests::make_test_gcx().await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.clone()];
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    let mut session = crate::chat::types::ChatSession::new("review-owner".into());
    session.work_project_root = Some(root.clone());
    let session = Arc::new(Mutex::new(session));
    app.chat
        .sessions
        .write()
        .await
        .insert("review-owner".into(), session.clone());
    let request = CommentRequest {
        target: None,
        blocks: vec![],
        expected_revision: 0,
        request_key: "once".into(),
        thread_id: None,
        annotation_id: None,
        anchor: None,
        body: "A canonical comment".into(),
    };
    let first = service::comment(
        app.clone(),
        root.clone(),
        review.owner_chat_id.clone(),
        review.id.clone(),
        request.clone(),
    )
    .await
    .unwrap();
    let second = service::comment(
        app.clone(),
        root.clone(),
        review.owner_chat_id.clone(),
        review.id.clone(),
        request.clone(),
    )
    .await
    .unwrap();
    assert_eq!(first, second);
    assert_eq!(first.threads.len(), 1);
    assert_eq!(first.threads[0].contributions.len(), 1);
    let loaded = service::load_owned(root.clone(), "review-owner".into(), review.id.clone())
        .await
        .unwrap();
    assert_eq!(loaded, first);
    assert!(matches!(
        service::load_owned(root.clone(), "other".into(), review.id.clone()).await,
        Err(Error::NotFound)
    ));
    let messages = session.lock().await.messages.clone();
    let matching: Vec<_> = messages
        .iter()
        .filter(|m| m.extra.get("review").is_some())
        .collect();
    assert_eq!(matching.len(), 1);
    assert_eq!(
        matching[0].extra["review"]["contribution_id"],
        first.threads[0].contributions[0].id
    );
    assert!(
        messages
            .iter()
            .any(|m| m.role == "event"
                && m.content.content_text_only().contains(&first.threads[0].id))
    );
    let privacy = refact_privacy::FileRecord {
        path: "/private/secret.rs".into(),
        zone: "secret".into(),
        attribution: refact_privacy::Attribution::Declared,
    };
    let reply_request = CommentRequest {
        target: None,
        blocks: vec![],
        expected_revision: first.revision,
        request_key: "agent-reply".into(),
        thread_id: Some(first.threads[0].id.clone()),
        annotation_id: None,
        anchor: None,
        body: "reply".into(),
    };
    let replied = service::reply(
        app.clone(),
        root.clone(),
        "review-owner".into(),
        review.id.clone(),
        "review-owner".into(),
        reply_request.clone(),
        vec![privacy.clone()],
    )
    .await
    .unwrap();
    let replay = service::reply(
        app.clone(),
        root.clone(),
        "review-owner".into(),
        review.id.clone(),
        "review-owner".into(),
        reply_request,
        vec![],
    )
    .await
    .unwrap();
    assert_eq!(replied, replay);
    let loaded = service::load_owned(root.clone(), "review-owner".into(), review.id.clone())
        .await
        .unwrap();
    assert_eq!(
        loaded.threads[0].contributions[1]
            .context
            .as_ref()
            .unwrap()
            .privacy,
        vec![privacy]
    );
    let session_messages = session.lock().await.messages.clone();
    let reply = session_messages
        .iter()
        .find(|m| m.role == "assistant" && m.content.content_text_only() == "reply")
        .unwrap();
    assert_eq!(reply.extra["privacy"]["files"][0]["zone"], "secret");
    let mut changed = request;
    changed.body = "different".into();
    assert!(matches!(
        service::comment(app, root, "review-owner".into(), review.id, changed).await,
        Err(Error::IdempotencyConflict)
    ));
}

#[tokio::test]
async fn review_comments_wait_for_boundary_and_precede_routine_fifo() {
    use refact_core::chat_types::{PendingDelivery, PushMode};
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let store = ReviewStore::open(&root).unwrap();
    let review = store
        .create("priority-owner", "Review", Origin::Local, "create")
        .unwrap();
    drop(store);
    let gcx = crate::global_context::tests::make_test_gcx().await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.clone()];
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    let mut s = crate::chat::types::ChatSession::new("priority-owner".into());
    s.work_project_root = Some(root.clone());
    s.start_stream().unwrap();
    let session = Arc::new(Mutex::new(s));
    app.chat
        .sessions
        .write()
        .await
        .insert("priority-owner".into(), session.clone());
    let routine = PendingDelivery::with_id(
        "routine",
        vec![crate::call_validation::ChatMessage::new(
            "assistant".into(),
            "routine".into(),
        )],
        PushMode::Append,
        "background",
        false,
    );
    crate::chat::delivery::deliver_to_chat_with_receipt(app.clone(), "priority-owner", routine)
        .await
        .unwrap();
    for (key, revision) in [("first", 0), ("second", 1)] {
        let req = CommentRequest {
            target: None,
            blocks: vec![],
            expected_revision: revision,
            request_key: key.into(),
            thread_id: None,
            annotation_id: None,
            anchor: None,
            body: key.into(),
        };
        service::comment(
            app.clone(),
            root.clone(),
            review.owner_chat_id.clone(),
            review.id.clone(),
            req,
        )
        .await
        .unwrap();
    }
    let mut s = session.lock().await;
    assert!(s.messages.is_empty());
    let sources: Vec<_> = s
        .pending_deliveries
        .iter()
        .map(|d| d.source.as_str())
        .collect();
    assert_eq!(
        sources,
        vec!["review.comment.user", "review.comment.user", "background"]
    );
    let bodies: Vec<_> = s
        .pending_deliveries
        .iter()
        .take(2)
        .map(|d| d.messages.last().unwrap().content.content_text_only())
        .collect();
    assert_eq!(bodies, vec!["first", "second"]);
    let mut persisted = s.pending_deliveries[0].clone();
    assert_eq!(persisted.expires_at_ms, Some(u64::MAX));
    persisted.enqueued_at_ms = 1_600_000_000_000;
    let encoded = serde_json::to_vec(&persisted).unwrap();
    let restored = serde_json::from_slice(&encoded).unwrap();
    let mut recovered = crate::chat::types::ChatSession::new("recovered".into());
    recovered.turn_depth = 1;
    recovered.restore_pending_deliveries(vec![restored]);
    assert_eq!(recovered.pending_deliveries.len(), 1);
    assert_eq!(
        recovered.pending_deliveries[0].expires_at_ms,
        Some(u64::MAX)
    );
    s.finish_stream(None);
    s.drain_pending_deliveries();
    let delivered: Vec<_> = s
        .messages
        .iter()
        .filter(|m| m.extra.contains_key("review"))
        .map(|m| m.content.content_text_only())
        .collect();
    assert_eq!(delivered, vec!["first", "second"]);
    assert_eq!(
        s.messages.last().unwrap().content.content_text_only(),
        "routine"
    );
}

#[tokio::test]
async fn late_reply_keeps_inspected_candidate_and_privacy() {
    use refact_review_core::*;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let repo = git2::Repository::init(&root).unwrap();
    let sig = git2::Signature::now("review", "review@example.test").unwrap();
    let oid = |text: &str| {
        std::fs::write(root.join("file.txt"), text).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("file.txt")).unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(None, &sig, &sig, "snapshot", &tree, &[])
            .unwrap()
            .to_string()
    };
    let a_oid = oid("first\nneedle\nlast\n");
    let b_oid = oid("first\nneedle\nlast\nextra\n");
    let store = ReviewStore::open(&root).unwrap();
    let capture = |oid: &str| {
        store
            .capture(
                &root,
                CaptureRequest {
                    base_oid: oid.into(),
                    head_oid: oid.into(),
                    policy: CapturePolicy::default(),
                    origin: Origin::Local,
                },
            )
            .unwrap()
    };
    let a = capture(&a_oid);
    let b = capture(&b_oid);
    let mut review = store
        .create("late-owner", "Review", Origin::Local, "create")
        .unwrap();
    let actor = Actor {
        kind: ActorKind::System,
        id: "test".into(),
    };
    let apply = |r: &Review, key: &str, operation| {
        store
            .mutate(
                &r.id,
                Mutation {
                    expected_revision: r.revision,
                    request_key: key.into(),
                    actor: actor.clone(),
                    at_ms: 1,
                    operation,
                },
            )
            .unwrap()
    };
    review = apply(
        &review,
        "stage-a",
        Operation::StageCandidate {
            candidate: a.clone(),
        },
    );
    review = apply(
        &review,
        "activate-a",
        Operation::ActivateCandidate {
            candidate_id: a.id.clone(),
        },
    );
    let gcx = crate::global_context::tests::make_test_gcx().await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.clone()];
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    let mut session = crate::chat::types::ChatSession::new("late-owner".into());
    session.work_project_root = Some(root.clone());
    let session = Arc::new(Mutex::new(session));
    app.chat
        .sessions
        .write()
        .await
        .insert("late-owner".into(), session.clone());
    let anchor = store.anchor(&a, "file.txt", Side::Head, 2, 2).unwrap();
    review = service::comment(
        app.clone(),
        root.clone(),
        "late-owner".into(),
        review.id.clone(),
        CommentRequest {
            target: None,
            blocks: vec![],
            expected_revision: review.revision,
            request_key: "initial".into(),
            thread_id: None,
            annotation_id: None,
            anchor: Some(anchor),
            body: "question".into(),
        },
    )
    .await
    .unwrap();
    let run = Run {
        id: "run-a".into(),
        candidate_id: a.id.clone(),
        agent_id: "reviewer".into(),
        purpose: "review".into(),
        model: None,
        domain: None,
        status: RunStatus::Running,
        started_at_ms: 1,
        finished_at_ms: None,
        error: None,
    };
    review = apply(
        &review,
        "run",
        Operation::PutRun {
            run,
        },
    );
    review = apply(
        &review,
        "stage-b",
        Operation::StageCandidate {
            candidate: b.clone(),
        },
    );
    review = apply(
        &review,
        "activate-b",
        Operation::ActivateCandidate {
            candidate_id: b.id.clone(),
        },
    );
    assert_eq!(review.threads[0].anchors[0].position.candidate_id, b.id);
    let privacy = refact_privacy::FileRecord {
        path: "/private/secret.rs".into(),
        zone: "secret".into(),
        attribution: refact_privacy::Attribution::Declared,
    };
    let updated = service::reply(
        app,
        root.clone(),
        "late-owner".into(),
        review.id.clone(),
        "reviewer".into(),
        CommentRequest {
            target: None,
            blocks: vec![],
            expected_revision: review.revision,
            request_key: "late".into(),
            thread_id: Some(review.threads[0].id.clone()),
            annotation_id: None,
            anchor: None,
            body: "old finding".into(),
        },
        vec![privacy.clone()],
    )
    .await
    .unwrap();
    let stored = updated.threads[0]
        .contributions
        .last()
        .unwrap()
        .context
        .as_ref()
        .unwrap();
    assert_eq!(stored.candidate_id.as_deref(), Some(a.id.as_str()));
    assert_eq!(stored.run_id.as_deref(), Some("run-a"));
    assert_eq!(stored.applicability, Applicability::Historical);
    assert_eq!(stored.privacy, vec![privacy]);
    let messages = session.lock().await.messages.clone();
    let event = messages
        .iter()
        .rev()
        .find(|m| m.role == "event" && m.content.content_text_only().contains("run-a"))
        .unwrap();
    let body = event.content.content_text_only();
    assert!(body.contains(&a.id));
    assert!(body.contains("historical"));
    assert!(!body.contains(&b.id));
    assert_eq!(
        event.extra["privacy"]["files"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["zone"] == "secret")
            .count(),
        1
    );
}

#[tokio::test]
async fn cancellation_intent_replays_after_registry_stop() {
    use refact_review_core::*;
    use crate::agents::types::{BgAgentKind, CreateAgentRequest};
    use std::sync::atomic::Ordering;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let repo = git2::Repository::init(&root).unwrap();
    let tree_id = repo.index().unwrap().write_tree().unwrap();
    let sig = git2::Signature::now("review", "review@example.test").unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let oid = repo
        .commit(Some("HEAD"), &sig, &sig, "base", &tree, &[])
        .unwrap()
        .to_string();
    drop(tree);
    drop(repo);
    let store = ReviewStore::open(&root).unwrap();
    let mut review = store
        .create("cancel-owner", "Review", Origin::Local, "create")
        .unwrap();
    let candidate = store
        .capture(
            &root,
            CaptureRequest {
                base_oid: oid.clone(),
                head_oid: oid,
                policy: CapturePolicy::default(),
                origin: Origin::Local,
            },
        )
        .unwrap();
    let actor = Actor {
        kind: ActorKind::System,
        id: "test".into(),
    };
    let apply = |r: &Review, key: &str, operation| {
        store
            .mutate(
                &r.id,
                Mutation {
                    expected_revision: r.revision,
                    request_key: key.into(),
                    actor: actor.clone(),
                    at_ms: 1,
                    operation,
                },
            )
            .unwrap()
    };
    review = apply(
        &review,
        "stage",
        Operation::StageCandidate {
            candidate: candidate.clone(),
        },
    );
    let gcx = crate::global_context::tests::make_test_gcx().await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.clone()];
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    let (agent, abort, _) = app
        .agents
        .create(CreateAgentRequest {
            parent_chat_id: "cancel-owner".into(),
            parent_root_chat_id: Some("cancel-owner".into()),
            parent_tool_call_id: None,
            kind: BgAgentKind::Subagent,
            config_name: "subagent".into(),
            title: "reviewer".into(),
            prompt: "review".into(),
            target_files: vec![],
            model: "test/model".into(),
            model_type: None,
            goal_summary: None,
            plan_present: false,
            worktree_id: None,
            worktree_branch: None,
        })
        .await
        .unwrap();
    let run = Run {
        id: "run".into(),
        candidate_id: candidate.id,
        agent_id: agent.agent_id.clone(),
        purpose: "review".into(),
        model: Some("test/model".into()),
        domain: None,
        status: RunStatus::Queued,
        started_at_ms: agent.created_at.timestamp_millis(),
        finished_at_ms: None,
        error: None,
    };
    review = apply(
        &review,
        "run",
        Operation::PutRun {
            run,
        },
    );
    let mutation = Mutation {
        expected_revision: review.revision,
        request_key: "cancel-once".into(),
        actor: Actor {
            kind: ActorKind::User,
            id: "user".into(),
        },
        at_ms: service::now_ms(),
        operation: Operation::RequestRunCancel {
            run_id: "run".into(),
        },
    };
    let intent = store.mutate(&review.id, mutation.clone()).unwrap();
    assert_eq!(intent.runs[0].status, RunStatus::CancelRequested);
    drop(store);
    let settled = super::runs::refresh_runs(&app, root.clone(), intent)
        .await
        .unwrap();
    assert!(abort.load(Ordering::SeqCst));
    let replay = ReviewStore::open(&root)
        .unwrap()
        .mutate(&review.id, mutation)
        .unwrap();
    assert_eq!(replay, settled);
    assert_ne!(replay.runs[0].status, RunStatus::Running);
}
