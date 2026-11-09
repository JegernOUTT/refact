//! Real tool-executor regression: registry-owned child A must not see owner B.
use super::*;
use crate::chat::types::ChatSession;
use refact_review_core::{
    CapturePolicy, CaptureRequest, Material, MaterialKind, Origin, ReviewStore, Run, RunStatus,
    Thread, TourStep,
};

async fn call(
    ccx: &Arc<Mutex<AtCommandsContext>>,
    kind: ReviewToolKind,
    args: Value,
) -> Result<Value, String> {
    let (_, messages) = ToolReview(kind)
        .tool_execute(
            ccx.clone(),
            &"pinning-call".into(),
            &serde_json::from_value(args).unwrap(),
        )
        .await?;
    for item in messages {
        if let ContextEnum::ChatMessage(message) = item {
            return serde_json::from_str(&message.content.content_text_only())
                .map_err(|e| e.to_string());
        }
    }
    Err("Missing actual tool result".into())
}

#[tokio::test]
async fn registered_child_executor_filters_get_update_and_reply_after_owner_switch() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    std::fs::create_dir(&root).unwrap();
    let head = {
        let repo = git2::Repository::init(&root).unwrap();
        let mut builder = repo.treebuilder(None).unwrap();
        builder
            .insert(
                "tracked.rs",
                repo.blob(b"fn fixture() {}\n").unwrap(),
                0o100644,
            )
            .unwrap();
        let tree = repo.find_tree(builder.write().unwrap()).unwrap();
        let sig = git2::Signature::now("Fixture", "fixture@localhost").unwrap();
        let head = repo
            .commit(Some("HEAD"), &sig, &sig, "seed", &tree, &[])
            .unwrap();
        repo.checkout_head(None).unwrap();
        head.to_string()
    };
    let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        temp.path().join("cache"),
        temp.path().join("config"),
    )
    .await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.clone()];
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    let store = ReviewStore::open(&root).unwrap();
    let review = store
        .create("owner", "Pinned fixture", Origin::Imported, "create")
        .unwrap();
    let a = store
        .capture(
            &root,
            CaptureRequest {
                base_oid: head.clone(),
                head_oid: head.clone(),
                policy: CapturePolicy::default(),
                origin: Origin::Imported,
            },
        )
        .unwrap();
    crate::review::runs::apply(
        root.clone(),
        &review.id,
        Operation::StageCandidate {
            candidate: a.clone(),
        },
    )
    .await
    .unwrap();
    let anchor = store.anchor(&a, "tracked.rs", Side::Head, 1, 1).unwrap();
    let worktree = crate::worktrees::service::WorktreeService::new_async(
        app.gcx.cache_dir.clone(),
        root.clone(),
    )
    .await
    .unwrap()
    .create_review_snapshot("owner", &review.id, vec![])
    .await
    .unwrap()
    .worktree
    .meta;
    let (record, _, _) = app
        .agents
        .create(crate::agents::types::CreateAgentRequest {
            parent_chat_id: "owner".into(),
            parent_root_chat_id: Some("owner".into()),
            parent_tool_call_id: None,
            kind: crate::agents::types::BgAgentKind::Subagent,
            config_name: "subagent".into(),
            title: "Fixture worker".into(),
            prompt: "Inspect fixture".into(),
            target_files: vec![],
            model: "test/default".into(),
            model_type: None,
            goal_summary: None,
            plan_present: false,
            worktree_id: Some(worktree.id.clone()),
            worktree_branch: worktree.branch.clone(),
        })
        .await
        .unwrap();
    app.agents
        .mark_running(&record.agent_id, "child".into())
        .await
        .unwrap();
    app.agent_executions
        .remember_scope(&record.agent_id, root.clone());
    crate::review::runs::apply(
        root.clone(),
        &review.id,
        Operation::AddMaterial {
            material: Material {
                id: "accepted-input".into(),
                kind: MaterialKind::Investigation,
                candidate_id: a.id.clone(),
                agent_id: Some(record.agent_id.clone()),
                description: "Accepted fixture".into(),
                content_digest: a.content_digest.clone(),
                input_digests: Default::default(),
                environment: None,
                worktree_id: Some(worktree.id.clone()),
                worktree_root: Some(worktree.root.to_string_lossy().into()),
                artifacts: vec![],
            },
        },
    )
    .await
    .unwrap();
    crate::review::runs::apply(
        root.clone(),
        &review.id,
        Operation::PutRun {
            run: Run {
                id: "accepted-run".into(),
                candidate_id: a.id.clone(),
                agent_id: record.agent_id.clone(),
                purpose: "Inspect fixture".into(),
                model: None,
                domain: None,
                status: RunStatus::Running,
                started_at_ms: 1,
                finished_at_ms: None,
                error: None,
            },
        },
    )
    .await
    .unwrap();
    let mut session = ChatSession::new("child".into());
    session.work_project_root = Some(root.clone());
    session.thread.worktree = Some(worktree.clone());
    app.chat
        .sessions
        .write()
        .await
        .insert("child".into(), Arc::new(Mutex::new(session)));
    crate::review::runs::verify_child_binding(&app, "child")
        .await
        .unwrap();
    let mut context = AtCommandsContext::new_with_abort(
        app.clone(),
        4096,
        10,
        false,
        vec![],
        "child".into(),
        None,
        String::new(),
        None,
        Some(worktree),
        None,
    )
    .await;
    context.background_agent_id = Some(record.agent_id.clone());
    let ccx = Arc::new(Mutex::new(context));
    let step:TourStep = serde_json::from_value(json!({"id":"mixed-step","current":{"version":1,"candidate_id":a.id,"title":"A title","body":"A body","anchors":[anchor]},"history":[]})).unwrap();
    crate::review::runs::apply(
        root.clone(),
        &review.id,
        Operation::PutTourStep {
            step,
        },
    )
    .await
    .unwrap();
    let b = store
        .capture(
            &root,
            CaptureRequest {
                base_oid: head.clone(),
                head_oid: head,
                policy: CapturePolicy::default(),
                origin: Origin::Imported,
            },
        )
        .unwrap();
    assert_ne!(a.id, b.id);
    crate::review::runs::apply(
        root.clone(),
        &review.id,
        Operation::StageCandidate {
            candidate: b.clone(),
        },
    )
    .await
    .unwrap();
    crate::review::runs::apply(
        root.clone(),
        &review.id,
        Operation::ActivateCandidate {
            candidate_id: b.id.clone(),
        },
    )
    .await
    .unwrap();
    let current = store.load(&review.id).unwrap();
    let mut step = current.tour[0].clone();
    step.current.version += 1;
    step.current.candidate_id = Some(b.id.clone());
    step.current.anchors.clear();
    step.current.body = "FOREIGN_B_TOUR_MARKER".into();
    crate::review::runs::apply(
        root.clone(),
        &review.id,
        Operation::PutTourStep {
            step,
        },
    )
    .await
    .unwrap();
    let mut foreign:Thread = serde_json::from_value(json!({"id":"foreign-thread","anchors":[anchor],"annotation_id":null,"resolved":false,"contributions":[{"id":"foreign-reply","thread_id":"foreign-thread","actor":{"kind":"system","id":"review-runtime"},"body":"FOREIGN_B_REPLY_MARKER","created_at_ms":1,"context":{"candidate_id":b.id,"run_id":null,"applicability":"current","privacy":[]}}]})).unwrap();
    crate::review::runs::apply(
        root.clone(),
        &review.id,
        Operation::CreateThread {
            contribution: foreign.contributions.remove(0),
            thread: foreign,
        },
    )
    .await
    .unwrap();
    let own:Thread = serde_json::from_value(json!({"id":"own-thread","anchors":[anchor],"annotation_id":null,"resolved":false,"contributions":[]})).unwrap();
    crate::review::runs::apply(
        root.clone(),
        &review.id,
        Operation::PutThread {
            thread: own,
        },
    )
    .await
    .unwrap();
    let mut retained_anchor = anchor.clone();
    let mut foreign_position = retained_anchor.position.clone();
    foreign_position.candidate_id = b.id.clone();
    retained_anchor.history.push(foreign_position);
    let retained_step: TourStep = serde_json::from_value(json!({
        "id":"retained-rich-step",
        "current":{"version":1,"candidate_id":a.id,"title":"A retained code","body":"A narrative","anchors":[],"blocks":[{"id":"code","revision":1,"content":{"kind":"code","anchor":retained_anchor}}]},
        "history":[]
    })).unwrap();
    crate::review::runs::apply(
        root.clone(),
        &review.id,
        Operation::PutTourStep {
            step: retained_step,
        },
    )
    .await
    .unwrap();
    let retained_thread: Thread = serde_json::from_value(json!({
        "id":"retained-target-thread","anchors":[retained_anchor],"annotation_id":null,"resolved":false,"contributions":[],
        "target":{"candidate_id":a.id,"step_id":"retained-rich-step","version":1,"block_id":"code","block_revision":1}
    })).unwrap();
    crate::review::runs::apply(
        root.clone(),
        &review.id,
        Operation::PutThread {
            thread: retained_thread,
        },
    )
    .await
    .unwrap();
    let assert_pinned = |value: &Value| {
        let text = value.to_string();
        assert!(
            !text.contains("FOREIGN_B_"),
            "foreign candidate payload leaked: {text}"
        );
        assert_eq!(value["projection"], "pinned_summary");
        assert_eq!(value["current_candidate_id"], a.id);
        assert_eq!(value["candidates"].as_array().unwrap().len(), 1);
    };
    let summary = call(&ccx, ReviewToolKind::Get, json!({})).await.unwrap();
    assert_pinned(&summary);
    assert_eq!(summary["tour_total"], 0);
    assert_eq!(summary["threads_total"], 1);
    for (kind, id) in [
        ("tour_step", "mixed-step"),
        ("thread", "foreign-thread"),
        ("tour_step", "retained-rich-step"),
        ("thread", "retained-target-thread"),
    ] {
        let error = call(
            &ccx,
            ReviewToolKind::Get,
            json!({"object_kind":kind,"object_id":id}),
        )
        .await
        .unwrap_err();
        assert!(error.contains("not provably bound"), "{error}");
        assert!(!error.contains("FOREIGN_B_"));
    }
    let updated = call(&ccx, ReviewToolKind::Update, json!({"expected_revision":summary["revision"],"request_key":"child-annotation","operation":{"op":"put_annotation","annotation":{"id":"own-note","kind":"finding","title":"A finding","body":"A only","anchors":[anchor],"severity":"low","confidence":"high","assessment":"supported","disposition":"open","applicability":"historical","evidence_ids":[],"run_id":null}}})).await.unwrap();
    assert_pinned(&updated);
    assert_eq!(updated["annotations_total"], 1);
    let replied = call(&ccx, ReviewToolKind::Reply, json!({"expected_revision":updated["revision"],"request_key":"child-answer","thread_id":"own-thread","body":"A child answer"})).await.unwrap();
    assert_pinned(&replied);
    assert_eq!(
        replied["threads"][0]["contributions"][0]["body"],
        "A child answer"
    );
    let stored = store.load(&review.id).unwrap();
    assert_eq!(stored.current_candidate_id.as_deref(), Some(b.id.as_str()));
    assert!(serde_json::to_string(&stored)
        .unwrap()
        .contains("FOREIGN_B_TOUR_MARKER"));
    assert_eq!(
        stored
            .threads
            .iter()
            .find(|t| t.id == "own-thread")
            .unwrap()
            .contributions[0]
            .context
            .as_ref()
            .unwrap()
            .candidate_id
            .as_deref(),
        Some(a.id.as_str())
    );
    ccx.lock().await.execution_scope = None;
    assert!(call(&ccx, ReviewToolKind::Get, json!({})).await.is_err());
}
