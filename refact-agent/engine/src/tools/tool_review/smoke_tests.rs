use super::*;
use std::path::PathBuf;

use crate::chat::types::{ChatSession, SessionState};
use refact_review_core::{ActorKind, ReviewStore, StackScope};

const CHAT: &str = "review-tool-smoke";

fn commit(repo: &git2::Repository, parent: Option<git2::Oid>, files: &[(&str, &str)]) -> git2::Oid {
    let mut tree = repo.treebuilder(None).unwrap();
    for (name, text) in files {
        tree.insert(name, repo.blob(text.as_bytes()).unwrap(), 0o100644)
            .unwrap();
    }
    let tree = repo.find_tree(tree.write().unwrap()).unwrap();
    let sig = git2::Signature::now("Smoke", "smoke@localhost").unwrap();
    let parents: Vec<git2::Commit> = parent
        .map(|id| repo.find_commit(id).unwrap())
        .into_iter()
        .collect();
    let refs: Vec<&git2::Commit> = parents.iter().collect();
    repo.commit(None, &sig, &sig, "fixture", &tree, &refs)
        .unwrap()
}

struct Fixture {
    _temp: tempfile::TempDir,
    source: PathBuf,
    oids: Vec<String>,
    ccx: Arc<Mutex<AtCommandsContext>>,
}

impl Fixture {
    async fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir(&source).unwrap();
        let oids = {
            let repo = git2::Repository::init(&source).unwrap();
            let same = ("unrelated.txt", "same\n");
            let base = commit(
                &repo,
                None,
                &[("tracked.py", "a = 1\nb = 2\nc = 3\nd = 4\n"), same],
            );
            let first = commit(
                &repo,
                Some(base),
                &[
                    ("tracked.py", "a = 1\nb = 20\nc = 3\nd = 4\ne = 5\n"),
                    ("added.py", "new = True\n"),
                    same,
                ],
            );
            let second = commit(
                &repo,
                Some(first),
                &[
                    ("tracked.py", "a = 1\nb = 20\nc = 30\nd = 4\ne = 5\n"),
                    ("added.py", "new = True\n"),
                    same,
                ],
            );
            for (name, oid) in [("first", first), ("second", second), ("main", first)] {
                repo.reference(&format!("refs/heads/{name}"), oid, true, "fixture")
                    .unwrap();
            }
            repo.set_head("refs/heads/main").unwrap();
            repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
                .unwrap();
            vec![base.to_string(), first.to_string(), second.to_string()]
        };
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            temp.path().join("cache"),
            temp.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        let mut live = ChatSession::new(CHAT.into());
        live.work_project_root = Some(source.clone());
        live.set_runtime_state(SessionState::ExecutingTools, None);
        let mut user = ChatMessage::new("user".into(), "Review this change".into());
        user.message_id = "smoke-user".into();
        live.add_message(user);
        let abort = live.abort_flag.clone();
        app.chat
            .sessions
            .write()
            .await
            .insert(CHAT.into(), Arc::new(Mutex::new(live)));
        let context = AtCommandsContext::new_with_abort(
            app,
            4096,
            10,
            false,
            vec![],
            CHAT.into(),
            None,
            String::new(),
            None,
            None,
            Some(abort),
        )
        .await;
        Self {
            _temp: temp,
            source,
            oids,
            ccx: Arc::new(Mutex::new(context)),
        }
    }

    async fn session(&self) -> Arc<Mutex<ChatSession>> {
        let app = self.ccx.lock().await.app.clone();
        let sessions = app.chat.sessions.read().await;
        sessions.get(CHAT).unwrap().clone()
    }

    /// Runs one tool the way the chat does and returns the stored tool message.
    async fn message(&self, kind: ReviewToolKind, value: Value) -> Result<ChatMessage, String> {
        let call_id = format!("smoke-{}", uuid::Uuid::new_v4());
        let args = serde_json::from_value(value).unwrap();
        let (_, messages) = ToolReview(kind)
            .tool_execute(self.ccx.clone(), &call_id, &args)
            .await?;
        let message = messages
            .into_iter()
            .find_map(|item| match item {
                ContextEnum::ChatMessage(message) => Some(message),
                _ => None,
            })
            .ok_or("no tool message")?;
        self.session()
            .await
            .lock()
            .await
            .add_message(message.clone());
        Ok(message)
    }

    async fn call(&self, kind: ReviewToolKind, value: Value) -> Result<Value, String> {
        let message = self.message(kind, value).await?;
        serde_json::from_str(&message.content.content_text_only()).map_err(|e| e.to_string())
    }

    async fn open(&self) -> Value {
        self.call(
            ReviewToolKind::Open,
            json!({"base":self.oids[0],"head":self.oids[1],"title":"Smoke"}),
        )
        .await
        .unwrap()
    }

    fn store(&self) -> ReviewStore {
        ReviewStore::open(&self.source).unwrap()
    }

    fn active(&self) -> Review {
        self.store().get_for_chat(CHAT).unwrap().unwrap()
    }
}

fn paths(message: &ChatMessage) -> Vec<String> {
    crate::privacy::records::records_to_carry(std::slice::from_ref(message))
        .unwrap()
        .into_iter()
        .map(|record| record.path)
        .collect()
}

#[tokio::test]
async fn reading_before_a_review_exists_reports_the_state_instead_of_failing() {
    let f = Fixture::new().await;
    let value = f.call(ReviewToolKind::Read, json!({})).await.unwrap();
    assert!(value["review"].is_null());
    assert!(value["next"].as_str().unwrap().contains("review_open"));
    let error = f
        .call(
            ReviewToolKind::Tour,
            json!({"id":"p","title":"t","summary":"s","html":"<p>x</p>"}),
        )
        .await
        .unwrap_err();
    assert!(error.contains("review_open"), "{error}");
}

#[tokio::test]
async fn pages_get_versions_history_and_anchored_inserts_from_tags_alone() {
    let f = Fixture::new().await;
    let opened = f.open().await;
    assert_eq!(opened["changed_files_total"], 2);
    assert_eq!(
        f.session()
            .await
            .lock()
            .await
            .thread
            .worktree
            .as_ref()
            .unwrap()
            .kind,
        "review"
    );
    let html = r#"<h1>Flow</h1><refact-code path="tracked.py" lines="2-999" id="core" highlight="2"></refact-code><refact-diff path="tracked.py"></refact-diff>"#;
    let first = f
        .call(
            ReviewToolKind::Tour,
            json!({"id":"flow","title":"Flow","summary":"How it flows.","html":html}),
        )
        .await
        .unwrap();
    assert_eq!(first["page"]["version"], 1);
    assert_eq!(first["inserts"].as_array().unwrap().len(), 2);
    let step = f.active().tour[0].clone();
    let stored = step.current.html.clone().unwrap();
    assert!(stored.contains(r#"<refact-slot data-block="core" data-highlight="2"></refact-slot>"#));
    assert!(!stored.contains("<refact-code") && !stored.contains("<refact-diff"));
    assert_eq!(step.current.candidate_id, f.active().current_candidate_id);
    let refact_review_core::RichBlockContent::Code {
        anchor,
    } = &step.current.blocks[0].content
    else {
        panic!("first insert is code");
    };
    assert_eq!(
        (anchor.position.start_line, anchor.position.end_line),
        (2, 5)
    );

    let edited = f
        .call(
            ReviewToolKind::Tour,
            json!({"id":"flow","edits":[{"old":"<h1>Flow</h1>","new":"<h1>Data flow</h1>"}]}),
        )
        .await
        .unwrap();
    assert_eq!(edited["page"]["version"], 2);
    let step = f.active().tour[0].clone();
    assert!(step
        .current
        .html
        .as_deref()
        .unwrap()
        .starts_with("<h1>Data flow</h1>"));
    assert_eq!(step.current.title, "Flow");
    assert_eq!(step.current.blocks.len(), 2);
    assert_eq!(step.history.len(), 1);
    assert_eq!(step.history[0].version, 1);

    let replaced = f
        .call(
            ReviewToolKind::Tour,
            json!({"id":"flow","title":"Flow, again","html":"<p>short</p>"}),
        )
        .await
        .unwrap();
    assert_eq!(replaced["page"]["version"], 3);
    assert_eq!(f.active().tour[0].history.len(), 2);
    assert!(f.active().tour[0].current.blocks.is_empty());

    for (html, expected) in [
        (
            r#"<refact-code path="trackd.py" lines="1"></refact-code>"#,
            "is not a file in this review",
        ),
        (
            r#"<refact-code path="tracked.py" lines="40"></refact-code>"#,
            "has 5 lines",
        ),
        (
            r#"<refact-diff path="unrelated.txt"></refact-diff>"#,
            "unchanged in the comparison",
        ),
        (
            r#"<refact-slot data-block="ghost"></refact-slot>"#,
            "matches no insert",
        ),
    ] {
        let error = f
            .call(
                ReviewToolKind::Tour,
                json!({"id":"bad","title":"t","summary":"s","html":html}),
            )
            .await
            .unwrap_err();
        assert!(error.contains(expected), "{html}: {error}");
    }
    let error = f
        .call(
            ReviewToolKind::Tour,
            json!({"id":"flow","edits":[{"old":"absent","new":"x"}]}),
        )
        .await
        .unwrap_err();
    assert!(error.contains("`edits[0].old` does not occur"), "{error}");
    assert_eq!(f.active().tour.len(), 1);
}

#[tokio::test]
async fn results_stay_compact_and_declare_only_the_files_they_quote() {
    let f = Fixture::new().await;
    let opened = f
        .message(
            ReviewToolKind::Open,
            json!({"base":f.oids[0],"head":f.oids[1]}),
        )
        .await
        .unwrap();
    let declared = paths(&opened);
    assert!(declared.iter().any(|path| path.ends_with("tracked.py")));
    assert!(
        !declared.iter().any(|path| path.ends_with("unrelated.txt")),
        "{declared:?}"
    );

    let added = f
        .call(ReviewToolKind::Finding, json!({"title":"b jumps","body":"b is now 20.","path":"tracked.py","lines":"2","severity":"high"}))
        .await
        .unwrap();
    let id = added["finding"]["id"].as_str().unwrap().to_string();
    assert_eq!(added["finding"]["at"], "tracked.py:2");
    let updated = f
        .call(
            ReviewToolKind::Finding,
            json!({"id":id,"status":"dismissed","lines":"2-3","path":"tracked.py"}),
        )
        .await
        .unwrap();
    assert_eq!(updated["finding"]["title"], "b jumps");
    assert_eq!(updated["finding"]["severity"], "high");
    assert_eq!(updated["finding"]["status"], "dismissed");
    assert_eq!(updated["findings_total"], 1);
    let annotation = f.active().annotations[0].clone();
    assert_eq!(annotation.anchors[0].history.len(), 1);

    let source = f
        .call(
            ReviewToolKind::Source,
            json!({"summary":"The spec asks for 20.","url":"https://example.invalid/spec"}),
        )
        .await
        .unwrap();
    let source_id = source["source"]["id"].as_str().unwrap();
    let cited = format!(r#"<refact-evidence ref="{source_id}"></refact-evidence>"#);
    f.call(
        ReviewToolKind::Tour,
        json!({"id":"why","title":"Why","summary":"Why 20.","html":cited}),
    )
    .await
    .unwrap();

    let overview = f.message(ReviewToolKind::Read, json!({})).await.unwrap();
    let text = overview.content.content_text_only();
    assert!(text.len() < 4000, "{}", text.len());
    assert!(text.contains("b jumps") && text.contains("\"why\""));
    assert!(serde_json::to_string(&overview).unwrap().len() < 8000);
    assert!(!paths(&overview)
        .iter()
        .any(|path| path.ends_with("unrelated.txt")));

    let file = f
        .call(
            ReviewToolKind::Read,
            json!({"path":"tracked.py","lines":"2-3","side":"base"}),
        )
        .await
        .unwrap();
    assert_eq!(file["text"], "    2 | b = 2\n    3 | c = 3");
    let diff = f
        .call(
            ReviewToolKind::Read,
            json!({"path":"tracked.py","diff":true}),
        )
        .await
        .unwrap();
    assert!(diff["diff"].as_str().unwrap().contains("+b = 20"));
}

#[tokio::test]
async fn finding_evidence_replaces_deduplicates_preserves_and_clears() {
    let f = Fixture::new().await;
    f.open().await;
    for id in ["spec", "context"] {
        f.call(
            ReviewToolKind::Source,
            json!({"id":id,"summary":"The required value.","path":"tracked.py","lines":"2"}),
        )
        .await
        .unwrap();
    }
    let created = f
        .call(
            ReviewToolKind::Finding,
            json!({"id":"value","title":"Wrong value","body":"Use the required value.","path":"tracked.py","lines":"2","evidence":["spec"]}),
        )
        .await
        .unwrap();
    assert_eq!(created["evidence_total"], 1);
    assert_eq!(f.active().annotations[0].evidence_ids, ["spec"]);

    f.call(
        ReviewToolKind::Finding,
        json!({"id":"value","evidence":["context","spec","context","spec"]}),
    )
    .await
    .unwrap();
    assert_eq!(f.active().annotations[0].evidence_ids, ["context", "spec"]);

    f.call(
        ReviewToolKind::Finding,
        json!({"id":"value","title":"Updated title"}),
    )
    .await
    .unwrap();
    let kept = f.active().annotations[0].clone();
    assert_eq!(kept.title, "Updated title");
    assert_eq!(kept.evidence_ids, ["context", "spec"]);

    let cleared = f
        .call(ReviewToolKind::Finding, json!({"id":"value","evidence":[]}))
        .await
        .unwrap();
    assert_eq!(cleared["evidence_total"], 0);
    assert!(f.active().annotations[0].evidence_ids.is_empty());
}

#[tokio::test]
async fn unknown_finding_evidence_leaves_creates_and_updates_unwritten() {
    let f = Fixture::new().await;
    f.open().await;
    f.call(
        ReviewToolKind::Source,
        json!({"id":"spec","summary":"The required value.","path":"tracked.py"}),
    )
    .await
    .unwrap();
    let before_create = serde_json::to_value(f.active()).unwrap();
    let error = f
        .call(
            ReviewToolKind::Finding,
            json!({"id":"value","title":"Wrong value","body":"Use the required value.","path":"tracked.py","lines":"2","evidence":["spec","missing"]}),
        )
        .await
        .unwrap_err();
    assert_eq!(error, "Evidence or source `missing` is not in this review");
    assert_eq!(serde_json::to_value(f.active()).unwrap(), before_create);

    f.call(
        ReviewToolKind::Finding,
        json!({"id":"value","title":"Wrong value","body":"Use the required value.","path":"tracked.py","lines":"2","evidence":["spec"]}),
    )
    .await
    .unwrap();
    let before_update = serde_json::to_value(f.active()).unwrap();
    let error = f
        .call(
            ReviewToolKind::Finding,
            json!({"id":"value","title":"Must not be written","evidence":["missing"]}),
        )
        .await
        .unwrap_err();
    assert_eq!(error, "Evidence or source `missing` is not in this review");
    assert_eq!(serde_json::to_value(f.active()).unwrap(), before_update);
}

#[tokio::test]
async fn source_needing_revalidation_is_rejected_without_writing() {
    use refact_review_core::{Actor, AnchorStatus, Applicability, EvidenceSource, Mutation, Operation};

    let f = Fixture::new().await;
    f.open().await;
    f.call(
        ReviewToolKind::Source,
        json!({"id":"spec","summary":"The anchored specification.","path":"tracked.py","lines":"2"}),
    )
    .await
    .unwrap();
    let review = f.active();
    let mut evidence = review
        .evidence
        .iter()
        .find(|item| item.id == "spec")
        .unwrap()
        .clone();
    assert_eq!(evidence.applicability, Applicability::Current);
    let EvidenceSource::Source {
        anchor: Some(anchor),
        ..
    } = &mut evidence.source
    else {
        panic!("source must have a file anchor");
    };
    assert_eq!(anchor.status, AnchorStatus::Current);
    anchor.status = AnchorStatus::Stalled;
    anchor.stalled_reason = Some("requires revalidation".into());
    evidence.id = "stalled-spec".into();
    evidence.applicability = Applicability::NeedsRevalidation;
    let changed = f
        .store()
        .mutate(
            &review.id,
            Mutation {
                expected_revision: review.revision,
                request_key: "stalled-source".into(),
                actor: Actor {
                    kind: ActorKind::System,
                    id: "test".into(),
                },
                at_ms: 1,
                operation: Operation::AddEvidence {
                    evidence,
                },
            },
        )
        .unwrap();
    let stalled = changed
        .evidence
        .iter()
        .find(|item| item.id == "stalled-spec")
        .unwrap();
    assert_eq!(stalled.applicability, Applicability::NeedsRevalidation);
    assert!(matches!(
        stalled.source,
        EvidenceSource::Source {
            anchor: Some(_),
            ..
        }
    ));
    let before = serde_json::to_value(changed).unwrap();
    let error = f.call(
        ReviewToolKind::Finding,
        json!({"id":"value","title":"Wrong value","body":"Use the required value.","path":"tracked.py","lines":"2","evidence":["stalled-spec"]}),
    ).await.unwrap_err();
    assert_eq!(error, "Source `stalled-spec` needs revalidation against the current comparison; record it again with review_source before citing it");
    assert_eq!(serde_json::to_value(f.active()).unwrap(), before);
}

#[tokio::test]
async fn execution_finding_evidence_is_rejected_without_writing() {
    use crate::exec::{ExecOwnerMeta, ExecSpawnRequest};
    use refact_review_core::{Applicability, EvidenceSource};
    use std::time::Duration;

    let f = Fixture::new().await;
    f.open().await;
    let (app, cwd) = {
        let context = f.ccx.lock().await;
        (
            context.app.clone(),
            context.execution_scope_worktree().unwrap().root,
        )
    };
    let spawned = app
        .gcx
        .exec_registry
        .spawn(
            ExecSpawnRequest::foreground("echo review-evidence")
                .with_cwd(cwd)
                .with_owner(ExecOwnerMeta {
                    chat_id: Some(CHAT.into()),
                    ..Default::default()
                })
                .with_timeout(Duration::from_secs(10)),
        )
        .await
        .unwrap();
    let process_id = spawned.snapshot.meta.process_id;
    tokio::time::timeout(
        Duration::from_secs(15),
        app.gcx.exec_registry.wait(&process_id),
    )
    .await
    .unwrap()
    .unwrap();
    let recorded = f
        .call(
            ReviewToolKind::RecordEvidence,
            json!({"process_id":process_id.0,"summary":"The execution completed."}),
        )
        .await
        .unwrap();
    let id = recorded["evidence"]["id"].as_str().unwrap();
    let review = f.active();
    let evidence = review.evidence.iter().find(|item| item.id == id).unwrap();
    assert!(matches!(evidence.source, EvidenceSource::Executed { .. }));
    assert_eq!(evidence.applicability, Applicability::Unknown);
    let before = serde_json::to_value(review).unwrap();
    let error = f
        .call(
            ReviewToolKind::Finding,
            json!({"id":"value","title":"Wrong value","body":"Use the required value.","path":"tracked.py","lines":"2","evidence":[id]}),
        )
        .await
        .unwrap_err();
    assert_eq!(error, format!("Evidence `{id}` is an execution record, which cannot be attributed to the current comparison; describe the run in the finding body or cite it on a tour page"));
    assert_eq!(serde_json::to_value(f.active()).unwrap(), before);
}

#[tokio::test]
async fn historical_finding_evidence_is_rejected_without_writing() {
    use refact_review_core::{
        Actor, Applicability, CapturePolicy, CaptureRequest, Mutation, Operation, Origin,
    };

    let f = Fixture::new().await;
    f.open().await;
    f.call(
        ReviewToolKind::Source,
        json!({"id":"spec","summary":"The original comparison.","path":"tracked.py"}),
    )
    .await
    .unwrap();
    let store = f.store();
    let mut target = authoring::require(&f.ccx, None).await.unwrap();
    let mut review = f.active();
    assert_eq!(review.evidence[0].applicability, Applicability::Current);
    let newer = store
        .capture(
            &f.source,
            CaptureRequest {
                base_oid: f.oids[0].clone(),
                head_oid: f.oids[2].clone(),
                policy: CapturePolicy::default(),
                origin: Origin::Local,
            },
        )
        .unwrap();
    for (key, operation) in [
        (
            "stage-newer",
            Operation::StageCandidate {
                candidate: newer.clone(),
            },
        ),
        (
            "activate-newer",
            Operation::ActivateCandidate {
                candidate_id: newer.id.clone(),
            },
        ),
    ] {
        review = store
            .mutate(
                &review.id,
                Mutation {
                    expected_revision: review.revision,
                    request_key: key.into(),
                    actor: Actor {
                        kind: ActorKind::System,
                        id: "test".into(),
                    },
                    at_ms: 1,
                    operation,
                },
            )
            .unwrap();
    }
    assert_eq!(
        review.current_candidate_id.as_deref(),
        Some(newer.id.as_str())
    );
    assert_eq!(review.evidence[0].applicability, Applicability::Historical);
    target.review = review;
    let before = serde_json::to_value(f.active()).unwrap();
    let error = write::finding(
        &f.ccx,
        &target,
        write::FindingInput {
            id: Some("value".into()),
            title: Some("Wrong value".into()),
            body: Some("Use the required value.".into()),
            path: Some("tracked.py".into()),
            lines: Some("2".into()),
            evidence: Some(vec!["spec".into()]),
            ..Default::default()
        },
        "historical-finding",
    )
    .await
    .unwrap_err();
    assert_eq!(error, "Evidence `spec` was recorded for an earlier version of the change; cite evidence for the current comparison");
    assert_eq!(serde_json::to_value(f.active()).unwrap(), before);
}

#[tokio::test]
async fn finding_evidence_is_scoped_to_the_selected_stack_member() {
    let f = Fixture::new().await;
    f.call(
        ReviewToolKind::Open,
        json!({"stack":[{"pr":1,"branch":"first","base":f.oids[0]},{"pr":2,"branch":"second"}]}),
    )
    .await
    .unwrap();
    f.call(
        ReviewToolKind::Source,
        json!({"member":"second","id":"top-spec","summary":"Top member context.","path":"tracked.py"}),
    )
    .await
    .unwrap();
    let stack = f.store().get_stack_for_chat(CHAT).unwrap().unwrap();
    let second_id = stack.members[1].review_id.as_ref().unwrap();
    let before_first = serde_json::to_value(f.active()).unwrap();
    let before_second = serde_json::to_value(f.store().load(second_id).unwrap()).unwrap();
    let error = f
        .call(
            ReviewToolKind::Finding,
            json!({"member":"first","id":"value","title":"Wrong value","body":"Use the required value.","path":"tracked.py","lines":"2","evidence":["top-spec"]}),
        )
        .await
        .unwrap_err();
    assert_eq!(error, "Evidence or source `top-spec` is not in this review");
    assert_eq!(serde_json::to_value(f.active()).unwrap(), before_first);
    assert_eq!(
        serde_json::to_value(f.store().load(second_id).unwrap()).unwrap(),
        before_second
    );
    f.call(
        ReviewToolKind::Finding,
        json!({"member":"second","id":"value","title":"Wrong value","body":"Use the required value.","path":"tracked.py","lines":"3","evidence":["top-spec"]}),
    )
    .await
    .unwrap();
    assert_eq!(
        f.store().load(second_id).unwrap().annotations[0].evidence_ids,
        ["top-spec"]
    );
    assert_eq!(serde_json::to_value(f.active()).unwrap(), before_first);
}

#[tokio::test]
async fn the_canvas_document_lists_changed_and_referenced_files_only() {
    let f = Fixture::new().await;
    f.open().await;
    let listed = |review: Review| {
        let mut paths: Vec<String> = crate::review::http::slim(review).candidates[0]
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect();
        paths.sort();
        paths
    };
    assert_eq!(f.active().candidates[0].files.len(), 3);
    assert_eq!(listed(f.active()), ["added.py", "tracked.py"]);
    f.call(
        ReviewToolKind::Source,
        json!({"summary":"Untouched neighbour.","path":"unrelated.txt","lines":"1"}),
    )
    .await
    .unwrap();
    assert_eq!(
        listed(f.active()),
        ["added.py", "tracked.py", "unrelated.txt"]
    );
    assert!(crate::review::http::slim(f.active()).privacy.is_empty());
}

#[tokio::test]
async fn simultaneous_writers_all_land_without_a_revision_error() {
    let f = Fixture::new().await;
    f.open().await;
    let writes = (0..6).map(|index| {
        let id = format!("page-{index}");
        f.call(
            ReviewToolKind::Tour,
            json!({"id":id,"title":id,"summary":"s","html":"<p>concurrent</p>"}),
        )
    });
    let results = futures::future::join_all(writes).await;
    for result in &results {
        assert!(result.is_ok(), "{result:?}");
    }
    assert_eq!(f.active().tour.len(), 6);
}

#[tokio::test]
async fn remote_comments_import_once_and_marked_items_leave_once() {
    let f = Fixture::new().await;
    f.open().await;
    let comments = json!([
        {"id":"100","author":"alice","body":"Why 20?","path":"tracked.py","line":2},
        {"id":"101","author":"bob","body":"Spec says so.","reply_to":"100"},
        {"id":"200","author":"alice","body":"Looks good overall."},
    ]);
    let before = f.session().await.lock().await.messages.len();
    let imported = f
        .call(ReviewToolKind::ImportComments, json!({"comments":comments}))
        .await
        .unwrap();
    assert_eq!(imported["imported"], 3);
    let review = f.active();
    assert_eq!(review.threads.len(), 2);
    let thread = review
        .threads
        .iter()
        .find(|thread| thread.contributions.len() == 2)
        .unwrap();
    assert_eq!(thread.anchors[0].position.start_line, 2);
    assert!(thread
        .contributions
        .iter()
        .all(|item| item.actor.kind == ActorKind::External));
    assert_eq!(thread.contributions[1].actor.id, "github:bob");
    assert!(f.store().pending_deliveries(&review.id).unwrap().is_empty());
    // Other people's comments never enter the chat as user turns.
    assert_eq!(f.session().await.lock().await.messages.len(), before + 1);

    let again = f
        .call(ReviewToolKind::ImportComments, json!({"comments":comments}))
        .await
        .unwrap();
    assert_eq!(
        (
            again["imported"].as_u64(),
            again["already_present"].as_u64()
        ),
        (Some(0), Some(3))
    );
    assert_eq!(f.active().threads.len(), 2);

    let app = f.ccx.lock().await.app.clone();
    let commented = crate::review::service::comment(
        app,
        f.source.clone(),
        CHAT.into(),
        review.id.clone(),
        crate::review::service::CommentRequest {
            expected_revision: 0,
            request_key: "user-reply".into(),
            thread_id: Some(thread.id.clone()),
            annotation_id: None,
            anchor: None,
            target: None,
            blocks: vec![],
            body: "Then name the constant.".into(),
        },
    )
    .await
    .unwrap();
    let mine = commented
        .threads
        .iter()
        .find(|item| item.id == thread.id)
        .unwrap()
        .contributions
        .last()
        .unwrap()
        .id
        .clone();
    assert!(
        f.call(ReviewToolKind::Outbox, json!({})).await.unwrap()["waiting"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let waiting = f
        .call(ReviewToolKind::Outbox, json!({"request":[mine]}))
        .await
        .unwrap();
    let item = &waiting["waiting"][0];
    assert_eq!(item["body"], "Then name the constant.");
    assert_eq!(item["reply_to_remote_id"], "100");
    assert_eq!(item["at"]["path"], "tracked.py");
    let done = f
        .call(
            ReviewToolKind::Outbox,
            json!({"published":[{"id":mine,"remote_id":"300"}],"request":[mine]}),
        )
        .await
        .unwrap();
    assert!(done["waiting"].as_array().unwrap().is_empty());
    let error = f
        .call(ReviewToolKind::Outbox, json!({"request":["nope"]}))
        .await
        .unwrap_err();
    assert!(error.contains("is not a finding or comment"), "{error}");
}

#[tokio::test]
async fn a_stack_opens_whole_in_one_call_and_any_member_takes_pages() {
    let f = Fixture::new().await;
    let opened = f
        .call(
            ReviewToolKind::Open,
            json!({
                "target":"https://app.graphite.com/github/pr/acme/widgets/1/Title",
                "stack":[{"pr":1,"branch":"first","base":f.oids[0]},{"pr":2,"branch":"second"}],
            }),
        )
        .await
        .unwrap();
    assert_eq!(opened["stack"]["members"].as_array().unwrap().len(), 2);
    assert!(f.session().await.lock().await.pending_questions.is_empty());
    let stack = f.store().get_stack_for_chat(CHAT).unwrap().unwrap();
    assert!(stack.scope_choice.confirmed);
    assert_eq!(stack.scope_choice.scope, StackScope::Whole);
    assert!(stack
        .members
        .iter()
        .all(|member| member.review_id.is_some()));
    let pr = stack.members[1].pr_identity.clone().unwrap();
    assert_eq!((pr.repository.as_str(), pr.number), ("acme/widgets", 2));
    let first = f.active();
    assert_eq!(Some(first.id.clone()), stack.members[0].review_id);
    let checkout = f.session().await.lock().await.thread.worktree.clone();

    f.call(
        ReviewToolKind::Tour,
        json!({"member":"2","id":"top","title":"Top","summary":"s","html":"<refact-diff path=\"tracked.py\"></refact-diff>"}),
    )
    .await
    .unwrap();
    let second = f
        .store()
        .load(stack.members[1].review_id.as_ref().unwrap())
        .unwrap();
    assert_eq!(second.tour.len(), 1);
    assert!(f.active().tour.is_empty());
    assert_eq!(f.active().id, first.id);
    assert_eq!(f.session().await.lock().await.thread.worktree, checkout);

    let overview = f.call(ReviewToolKind::Read, json!({})).await.unwrap();
    assert_eq!(overview["stack"]["members"][1]["pages"], 1);
    assert_eq!(overview["stack"]["members"][0]["open_here"], true);
    let error = f
        .call(ReviewToolKind::Read, json!({"member":"9"}))
        .await
        .unwrap_err();
    assert!(
        error.contains("#1 first") && error.contains("#2 second"),
        "{error}"
    );
}

/// A pull request that keeps what is posted to it, like the real one.
#[derive(Default)]
struct Hub {
    line: std::sync::Mutex<Vec<Value>>,
    general: std::sync::Mutex<Vec<Value>>,
}

#[async_trait]
impl remote::Forge for Hub {
    async fn get(&self, path: &str) -> authoring::Outcome<Value> {
        Ok(if path.contains("/pulls/7/comments?") {
            json!(*self.line.lock().unwrap())
        } else if path.contains("/issues/7/comments?") {
            json!(*self.general.lock().unwrap())
        } else {
            json!([])
        })
    }
    async fn post(&self, path: &str, body: Value) -> authoring::Outcome<Value> {
        let (mut line, mut general) = (self.line.lock().unwrap(), self.general.lock().unwrap());
        let id = 500 + line.len() + general.len();
        let mut stored = body;
        stored["id"] = json!(id);
        stored["user"] = json!({"login":"me"});
        stored["created_at"] = json!("2026-02-01T00:00:00Z");
        if let Some(parent) = path
            .strip_suffix("/replies")
            .and_then(|path| path.rsplit('/').next())
        {
            stored["in_reply_to_id"] = json!(parent.parse::<u64>().unwrap());
        }
        if path.contains("/pulls/7/comments") {
            line.push(stored);
        } else {
            general.push(stored);
        }
        Ok(json!({"id": id, "html_url": format!("https://example.invalid/{id}")}))
    }
}

#[tokio::test]
async fn a_pull_request_syncs_both_ways_with_no_model_and_nothing_twice() {
    let f = Fixture::new().await;
    f.open().await;
    let pr = refact_review_core::PrIdentity {
        host: "github.com".into(),
        repository: "acme/widgets".into(),
        number: 7,
    };
    let hub = Hub::default();
    hub.line.lock().unwrap().push(json!({
        "id":1,"user":{"login":"alice"},"body":"Why 20?","path":"tracked.py","line":2,
        "created_at":"2026-01-01T00:00:00Z",
    }));
    hub.general.lock().unwrap().push(json!({
        "id":10,"user":{"login":"carol"},"body":"LGTM","created_at":"2026-01-02T00:00:00Z",
    }));
    let mut target = authoring::require(&f.ccx, None).await.unwrap();
    let first = remote::sync_with(&mut target, &hub, &pr, remote::Direction::BOTH).await;
    assert_eq!(
        (first.available, first.imported, first.published),
        (true, 2, 0)
    );
    assert_eq!(target.review.threads.len(), 2);

    let finding = f
        .call(
            ReviewToolKind::Finding,
            json!({"title":"b jumps","body":"b is now 20.","path":"tracked.py","lines":"2-3"}),
        )
        .await
        .unwrap()["finding"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let thread = target
        .review
        .threads
        .iter()
        .find(|thread| !thread.anchors.is_empty())
        .unwrap()
        .id
        .clone();
    let app = f.ccx.lock().await.app.clone();
    crate::review::service::comment(
        app,
        f.source.clone(),
        CHAT.into(),
        target.review.id.clone(),
        crate::review::service::CommentRequest {
            expected_revision: 0,
            request_key: "mine".into(),
            thread_id: Some(thread),
            annotation_id: None,
            anchor: None,
            target: None,
            blocks: vec![],
            body: "Name the constant.".into(),
        },
    )
    .await
    .unwrap();
    let reply = crate::review::service::reply_contribution_id(&target.review.id, "mine");
    f.call(ReviewToolKind::Outbox, json!({"request":[finding, reply]}))
        .await
        .unwrap();

    let second = remote::sync_with(&mut target, &hub, &pr, remote::Direction::BOTH).await;
    assert_eq!(
        (second.imported, second.published, second.failed.len()),
        (0, 2, 0)
    );
    {
        let line = hub.line.lock().unwrap();
        assert_eq!(line.len(), 3);
        assert_eq!(
            (&line[1]["path"], &line[1]["start_line"], &line[1]["line"]),
            (&json!("tracked.py"), &json!(2), &json!(3))
        );
        assert_eq!(line[1]["commit_id"], f.oids[1]);
        assert!(line[1]["body"]
            .as_str()
            .unwrap()
            .starts_with("**b jumps**\n\nb is now 20."));
        assert_eq!(line[2]["in_reply_to_id"], 1);
        assert!(line[2]["body"]
            .as_str()
            .unwrap()
            .starts_with("Name the constant."));
    }

    // Their own return trip is recognised, and nothing waits to be posted again.
    let third = remote::sync_with(&mut target, &hub, &pr, remote::Direction::BOTH).await;
    assert_eq!((third.imported, third.published), (0, 0));
    assert_eq!(hub.line.lock().unwrap().len(), 3);
    assert_eq!(target.review.threads.len(), 2);
    assert!(comments::outgoing(&target.review).is_empty());

    // Posted, but the engine stopped before recording it: the comment itself is the record.
    let lost = f
        .call(
            ReviewToolKind::Finding,
            json!({"title":"c","body":"c","path":"tracked.py","lines":"3"}),
        )
        .await
        .unwrap()["finding"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    f.call(ReviewToolKind::Outbox, json!({"request":[lost]}))
        .await
        .unwrap();
    hub.general.lock().unwrap().push(json!({
        "id":77,"user":{"login":"me"},"created_at":"2026-02-02T00:00:00Z",
        "body":format!("**c**\n\nc\n\n<!-- refact-review:{lost} -->"),
    }));
    let fourth = remote::sync_with(&mut target, &hub, &pr, remote::Direction::BOTH).await;
    assert_eq!((fourth.imported, fourth.published), (0, 0));
    assert_eq!(
        hub.line.lock().unwrap().len() + hub.general.lock().unwrap().len(),
        5
    );
    let link = target
        .review
        .remote_links
        .iter()
        .find(|link| link.local_id == lost)
        .unwrap();
    assert_eq!(link.state, refact_review_core::RemoteLinkState::Published);
    assert_eq!(link.remote_id.as_deref(), Some("issue_comment:77"));
}
