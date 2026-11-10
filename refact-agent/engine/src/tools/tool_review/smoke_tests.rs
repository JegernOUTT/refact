use super::*;
use crate::chat::types::{ChatSession, SessionState};

fn invocation(id: &str, name: &str, value: &Value) -> ChatMessage {
    let mut message = ChatMessage::new("assistant".into(), String::new());
    message.tool_calls = Some(vec![serde_json::from_value(json!({
        "id":id,"type":"function","function":{"name":name,"arguments":value.to_string()}
    }))
    .unwrap()]);
    message
}

struct Fixture {
    _temp: tempfile::TempDir,
    source: PathBuf,
    oid: String,
    ccx: Arc<Mutex<AtCommandsContext>>,
}

impl Fixture {
    async fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir(&source).unwrap();
        let oid = {
            let repo = git2::Repository::init(&source).unwrap();
            let mut tree = repo.treebuilder(None).unwrap();
            tree.insert(
                "tracked.py",
                repo.blob(b"assert 1 == 1\n").unwrap(),
                0o100644,
            )
            .unwrap();
            let tree = repo.find_tree(tree.write().unwrap()).unwrap();
            let sig = git2::Signature::now("Smoke", "smoke@localhost").unwrap();
            let oid = repo
                .commit(Some("HEAD"), &sig, &sig, "fixture", &tree, &[])
                .unwrap();
            repo.checkout_head(None).unwrap();
            oid.to_string()
        };
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            temp.path().join("cache"),
            temp.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        let mut live = ChatSession::new("review-tool-smoke".into());
        live.work_project_root = Some(source.clone());
        live.set_runtime_state(SessionState::ExecutingTools, None);
        let mut user = ChatMessage::new(
            "user".into(),
            "Review tracked.py and explain its source without claiming a product test verdict"
                .into(),
        );
        user.message_id = "smoke-user".into();
        live.add_message(user);
        // Captured native IO fixture with actual file content and provenance.
        live.add_message(invocation(
            "smoke-source",
            "cat",
            &json!({"paths":source.join("tracked.py")}),
        ));
        let mut source_result = ChatMessage::new(
            "tool".into(),
            std::fs::read_to_string(source.join("tracked.py")).unwrap(),
        );
        source_result.tool_call_id = "smoke-source".into();
        source_result.tool_failed = Some(false);
        crate::privacy::records::merge_records(
            &mut source_result,
            crate::privacy::records::declared_file_records(
                &app.gcx,
                vec![source.join("tracked.py")],
            )
            .unwrap(),
        );
        live.add_message(source_result);
        let abort = live.abort_flag.clone();
        app.chat
            .sessions
            .write()
            .await
            .insert(live.chat_id.clone(), Arc::new(Mutex::new(live)));
        let context = AtCommandsContext::new_with_abort(
            app,
            4096,
            10,
            false,
            vec![],
            "review-tool-smoke".into(),
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
            oid,
            ccx: Arc::new(Mutex::new(context)),
        }
    }

    async fn call(&self, kind: ReviewToolKind, value: Value) -> Result<Value, String> {
        self.call_with_id(kind, value).await.map(|(_, value)| value)
    }

    async fn call_with_id(
        &self,
        kind: ReviewToolKind,
        value: Value,
    ) -> Result<(String, Value), String> {
        let call_id = format!("smoke-{}", uuid::Uuid::new_v4());
        let app = self.ccx.lock().await.app.clone();
        let live = app
            .chat
            .sessions
            .read()
            .await
            .get("review-tool-smoke")
            .unwrap()
            .clone();
        live.lock()
            .await
            .add_message(invocation(&call_id, ToolReview(kind).name(), &value));
        let args = serde_json::from_value(value).unwrap();
        let execution = ToolReview(kind)
            .tool_execute(self.ccx.clone(), &call_id, &args)
            .await;
        let (_, messages) = match execution {
            Ok(result) => result,
            Err(error) => {
                let mut result = ChatMessage::new("tool".into(), error.clone());
                result.tool_call_id = call_id;
                result.tool_failed = Some(true);
                live.lock().await.add_message(result);
                return Err(error);
            }
        };
        let mut actual = Vec::new();
        for item in messages {
            if let ContextEnum::ChatMessage(mut message) = item {
                message.tool_failed = Some(false);
                live.lock().await.add_message(message.clone());
                actual.push(ContextEnum::ChatMessage(message));
            }
        }
        let value = actual
            .into_iter()
            .find_map(|item| match item {
                ContextEnum::ChatMessage(message) => match message.content {
                    ChatContent::SimpleText(text) => {
                        Some(serde_json::from_str(&text).map_err(|e| e.to_string()))
                    }
                    _ => None,
                },
                _ => None,
            })
            .ok_or_else(|| "No actual tool result".to_string())??;
        Ok((call_id, value))
    }

    async fn activate(&self) -> Value {
        self.call(
            ReviewToolKind::Context,
            json!({
                "repository":self.source,"base_ref":self.oid,"candidate_ref":self.oid,
                "request_key":"capture","title":"Smoke"
            }),
        )
        .await
        .unwrap()
    }

    async fn update(&self, operation: Value, key: &str) -> Value {
        let current = self.call(ReviewToolKind::Get, json!({})).await.unwrap();
        let request = json!({"expected_revision":current["revision"],"request_key":key,"operation":operation});
        self.call(ReviewToolKind::Update, request)
            .await
            .unwrap_or_else(|error| panic!("fixture update {key} failed: {error}"))
    }
}

#[tokio::test]
async fn direct_edit_history_three_versions_exact_retries_and_tamper_rejection() {
    let f = Fixture::new().await;
    f.activate().await;
    let store = refact_review_core::ReviewStore::open(&f.source).unwrap();
    let mut retries = Vec::new();
    let mut snapshots = Vec::new();
    for version in 1..=3 {
        let before = store.get_for_chat("review-tool-smoke").unwrap().unwrap();
        let history = before
            .tour
            .first()
            .map(|s| s.history.clone())
            .unwrap_or_default();
        let operation = json!({"op":"put_tour_step","step":{"id":"history","current":{
            "version":version,"title":"History","body":format!("Explanation {version}"),"anchors":[]
        },"history":history}});
        let key = format!("history-{version}");
        f.update(operation.clone(), &key).await;
        let request =
            json!({"expected_revision":before.revision,"request_key":key,"operation":operation});
        retries.push(request);
        snapshots.push(store.get_for_chat("review-tool-smoke").unwrap().unwrap());
    }
    let persisted = snapshots.last().unwrap();
    assert_eq!(persisted.tour[0].history.len(), 2);
    for (index, version) in persisted.tour[0].history.iter().enumerate() {
        assert_eq!(version, &snapshots[index].tour[0].current);
    }
    let journal = store.object_history(&persisted.id).unwrap();
    for request in &retries {
        let replay = f
            .call(ReviewToolKind::Update, request.clone())
            .await
            .unwrap();
        assert_eq!(replay["revision"], persisted.revision);
    }
    for field in ["body", "extra", "short_fresh"] {
        let mut tampered = retries[2].clone();
        match field {
            "body" => tampered["operation"]["step"]["history"][0]["body"] = json!("forged"),
            "extra" => {
                let extra = tampered["operation"]["step"]["current"].clone();
                tampered["operation"]["step"]["history"]
                    .as_array_mut()
                    .unwrap()
                    .push(extra);
            }
            _ => {
                tampered["request_key"] = json!("fresh-short-prefix");
                tampered["expected_revision"] = json!(persisted.revision);
                tampered["operation"]["step"]["current"]["version"] = json!(4);
                tampered["operation"]["step"]["history"] = json!([]);
            }
        }
        assert!(
            f.call(ReviewToolKind::Update, tampered).await.is_err(),
            "{field}"
        );
    }
    assert_eq!(store.load(&persisted.id).unwrap(), *persisted);
    assert_eq!(store.object_history(&persisted.id).unwrap(), journal);
}

#[tokio::test]
async fn reply_older_key_retry_returns_original_contribution_without_store_changes() {
    let f = Fixture::new().await;
    f.activate().await;
    let store = refact_review_core::ReviewStore::open(&f.source).unwrap();
    let current = store.get_for_chat("review-tool-smoke").unwrap().unwrap();
    let app = f.ccx.lock().await.app.clone();
    let seeded = crate::review::service::comment(
        app,
        f.source.clone(),
        "review-tool-smoke".into(),
        current.id.clone(),
        crate::review::service::CommentRequest {
            expected_revision: current.revision,
            request_key: "retry-question".into(),
            thread_id: None,
            annotation_id: None,
            anchor: None,
            target: None,
            blocks: vec![],
            body: "Explain this source".into(),
        },
    )
    .await
    .unwrap();
    let thread_id = &seeded.threads[0].id;
    let first_request = json!({
        "expected_revision":seeded.revision,"request_key":"reply-k1",
        "thread_id":thread_id,"body":"First answer"
    });
    let first = f
        .call(ReviewToolKind::Reply, first_request.clone())
        .await
        .unwrap();
    let second = f
        .call(
            ReviewToolKind::Reply,
            json!({"expected_revision":first["revision"],"request_key":"reply-k2",
                "thread_id":thread_id,"body":"Second answer"}),
        )
        .await
        .unwrap();
    let before = store.load(&seeded.id).unwrap();
    let history = store.object_history(&seeded.id).unwrap();
    let contributions = &before.threads[0].contributions;
    assert_eq!(contributions.len(), 3);
    assert_eq!(contributions[1].body, "First answer");
    assert_eq!(contributions[2].body, "Second answer");
    assert_eq!(first["contribution_id"], contributions[1].id);
    assert_eq!(second["contribution_id"], contributions[2].id);
    assert_ne!(first["contribution_id"], second["contribution_id"]);
    assert!(second["revision"].as_u64().unwrap() > first["revision"].as_u64().unwrap());
    let retry = f.call(ReviewToolKind::Reply, first_request).await.unwrap();
    assert_eq!(retry["contribution_id"], contributions[1].id);
    assert_eq!(retry["revision"], before.revision);
    assert_eq!(store.load(&seeded.id).unwrap(), before);
    assert_eq!(store.object_history(&seeded.id).unwrap(), history);
}

#[tokio::test]
async fn executor_rich_authoring_discussion_reply_and_reload() {
    let f = Fixture::new().await;
    let activation = f.activate().await;
    let candidate = activation["candidate_id"].clone();
    let anchor = f
        .call(
            ReviewToolKind::Anchor,
            json!({"path":"tracked.py","side":"head","start_line":1,"end_line":1}),
        )
        .await
        .unwrap();
    f.update(json!({"op":"add_evidence","evidence":{"id":"consulted","candidate_id":candidate,"source":{"kind":"source","path":"tracked.py","anchor":anchor},"summary":"Consulted immutable source","applicability":"current","created_at_ms":1}}), "rich-source").await;
    let prepared = f
        .call(
            ReviewToolKind::PrepareExecution,
            json!({"request_key":"rich-inputs"}),
        )
        .await
        .unwrap();
    let app = f.ccx.lock().await.app.clone();
    let checkout = f.ccx.lock().await.execution_scope_worktree().unwrap().root;
    let meta = crate::exec::ExecProcessMeta::new(
        crate::exec::ExecMode::Foreground,
        "rich-fixture-registry-result".into(),
    )
    .with_chat_id("review-tool-smoke")
    .with_cwd(checkout);
    let process_id = meta.process_id.clone();
    app.gcx.exec_registry.register(meta, 4096).await;
    app.gcx
        .exec_registry
        .mark_started(&process_id)
        .await
        .unwrap();
    app.gcx
        .exec_registry
        .mark_exited(&process_id, Some(0))
        .await
        .unwrap();
    let recorded = f.call(ReviewToolKind::RecordEvidence, json!({"process_id":process_id,"summary":"Synthetic completed registry process; not a product test verdict","retained_kind":"investigation","prepared_material_id":prepared["id"]})).await.unwrap();
    let executed_id = recorded["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["source"]["kind"] == "executed")
        .unwrap()["id"]
        .clone();
    let blocks = json!([
        {"id":"text","revision":1,"content":{"kind":"markdown","body":"Narrative"}},
        {"id":"visual","revision":1,"content":{"kind":"html","html":"<script>local()</script>","anchors":[anchor]}},
        {"id":"code","revision":1,"content":{"kind":"code","anchor":anchor}},
        {"id":"diff","revision":1,"content":{"kind":"diff","candidate_id":candidate,"path":"tracked.py"}},
        {"id":"sources","revision":1,"content":{"kind":"sources","evidence_ids":["consulted"],"relation":"consulted"}},
        {"id":"support","revision":1,"content":{"kind":"evidence","evidence_ids":[executed_id]}}
    ]);
    let authored = f.update(json!({"op":"put_tour_step","step":{"id":"rich","current":{"version":1,"title":"Rich tour","body":"Accessible fallback","anchors":[],"blocks":blocks},"history":[]}}), "rich-tour").await;
    assert_eq!(authored["tour"][0]["current"]["candidate_id"], candidate);
    assert_eq!(authored["tour"][0]["current"]["blocks"]["total"], 6);
    let exact = f
        .call(
            ReviewToolKind::Get,
            json!({"object_kind":"tour_step","object_id":"rich"}),
        )
        .await
        .unwrap();
    assert_eq!(exact["current"]["blocks"], blocks);
    let app = f.ccx.lock().await.app.clone();
    let request = crate::review::service::CommentRequest {
        expected_revision: authored["revision"].as_u64().unwrap(), request_key:"rich-question".into(),
        thread_id:None, annotation_id:None, anchor:None,
        target:Some(serde_json::from_value(json!({"candidate_id":candidate,"step_id":"rich","version":1,"block_id":"code","block_revision":1})).unwrap()),
        blocks:vec![], body:"Explain this native code block".into(),
    };
    let commented = crate::review::service::comment(
        app.clone(),
        f.source.clone(),
        "review-tool-smoke".into(),
        authored["id"].as_str().unwrap().into(),
        request,
    )
    .await
    .unwrap();
    let thread = &commented.threads[0].id;
    f.call(ReviewToolKind::Reply, json!({"expected_revision":commented.revision,"request_key":"rich-answer","thread_id":thread,"body":"Accessible rich answer","blocks":blocks})).await.unwrap();
    let reloaded = refact_review_core::ReviewStore::open(&f.source)
        .unwrap()
        .load(&commented.id)
        .unwrap();
    let reply = &reloaded.threads[0].contributions[1];
    assert_eq!(serde_json::to_value(&reply.blocks).unwrap(), blocks);
    let snapshot = refact_review_core::ReviewStore::open(&f.source)
        .unwrap()
        .handoff_snapshot(&commented.id)
        .unwrap();
    let export = crate::review::handoff::format_snapshot(&snapshot).unwrap();
    assert!(export.markdown.contains("Quoted html:"));
    assert!(export.markdown.contains("\"block_revision\": 1"));
    let live = app
        .chat
        .sessions
        .read()
        .await
        .get("review-tool-smoke")
        .unwrap()
        .clone();
    let live = live.lock().await;
    let deliveries = live
        .pending_deliveries
        .iter()
        .flat_map(|d| d.messages.iter())
        .chain(live.messages.iter())
        .filter(|m| m.extra.get("review").is_some())
        .collect::<Vec<_>>();
    assert!(deliveries
        .iter()
        .any(|m| m.extra["review"]["contribution_id"] == reply.id
            && m.extra["review"]["blocks"] == blocks));
}

#[tokio::test]
async fn executor_all_seven_tools_and_cas_preserve_immutable_source() {
    let f = Fixture::new().await;
    let activation = f.activate().await;
    let candidate = activation["candidate_id"].clone();
    let checkout = PathBuf::from(activation["worktree"]["root"].as_str().unwrap());
    let anchor = f
        .call(
            ReviewToolKind::Anchor,
            json!({"path":"tracked.py","side":"head","start_line":1,"end_line":1}),
        )
        .await
        .unwrap();
    let first = f
        .update(json!({"op":"rename","title":"Renamed"}), "rename")
        .await;
    let retry = f.call(ReviewToolKind::Update, json!({"expected_revision":first["revision"].as_u64().unwrap()-1,"request_key":"rename","operation":{"op":"rename","title":"Renamed"}})).await.unwrap();
    assert_eq!(retry["revision"], first["revision"]);
    f.update(
        json!({"op":"set_target_reference","target_reference":"fixture-local"}),
        "target",
    )
    .await;
    assert!(f.call(ReviewToolKind::Update, json!({"expected_revision":0,"request_key":"stale","operation":{"op":"rename","title":"Lost"}})).await.is_err());
    f.update(json!({"op":"put_tour_step","step":{"id":"tour","current":{"version":1,"title":"Tour","body":"Source explanation","anchors":[anchor]},"history":[]}}), "tour").await;
    f.update(json!({"op":"put_annotation","annotation":{"id":"finding","kind":"finding","title":"Finding","body":"Source claim only","anchors":[anchor],"severity":"low","confidence":"high","assessment":"supported","disposition":"open","applicability":"current","evidence_ids":[],"run_id":null}}), "finding").await;
    let app = f.ccx.lock().await.app.clone();
    let current: Review =
        crate::review::service::get_for_chat(f.source.clone(), "review-tool-smoke".into())
            .await
            .unwrap()
            .unwrap();
    let seeded = crate::review::service::comment(
        app,
        f.source.clone(),
        "review-tool-smoke".into(),
        current.id.clone(),
        crate::review::service::CommentRequest {
            target: None,
            blocks: vec![],
            expected_revision: current.revision,
            request_key: "fixture-user-comment".into(),
            thread_id: None,
            annotation_id: Some("finding".into()),
            anchor: Some(serde_json::from_value(anchor.clone()).unwrap()),
            body: "Simulated fixture user question".into(),
        },
    )
    .await
    .unwrap();
    let thread_id = seeded.threads[0].id.clone();
    let mut thread = serde_json::to_value(&seeded.threads[0]).unwrap();
    thread["resolved"] = json!(true);
    f.update(json!({"op":"put_thread","thread":thread}), "thread")
        .await;
    f.update(json!({"op":"add_evidence","evidence":{"id":"source-evidence","candidate_id":candidate,"material_id":null,"run_id":null,"source":{"kind":"source","path":"tracked.py","anchor":anchor},"summary":"Source assertion, not execution","applicability":"current","created_at_ms":1}}), "source").await;
    let current = f.call(ReviewToolKind::Get, json!({})).await.unwrap();
    let replied = f.call(ReviewToolKind::Reply, json!({"expected_revision":current["revision"],"request_key":"reply","thread_id":thread_id,"body":"Fixture reply"})).await.unwrap();
    let exact = f
        .call(
            ReviewToolKind::Get,
            json!({"object_kind":"thread","object_id":thread_id}),
        )
        .await
        .unwrap();
    assert_eq!(exact["contributions"][1]["body"], "Fixture reply");
    assert_eq!(exact["contributions"].as_array().unwrap().len(), 2);
    assert_eq!(replied.as_object().unwrap().len(), 5);
    assert_eq!(replied["review_id"], current["id"]);
    assert_eq!(
        replied["revision"],
        current["revision"].as_u64().unwrap() + 1
    );
    assert_eq!(replied["thread_id"], thread_id);
    assert_eq!(replied["contribution_id"], exact["contributions"][1]["id"]);
    let next = replied["next"].as_str().unwrap();
    assert!(next.contains("saved in the thread and delivered to chat as its linked message"));
    assert!(!next.contains("already shown"));
    assert!(next.contains("Do not repeat its content"));
    assert!(next.contains("Continue other requested work"));
    assert!(next.contains("end the turn with one short sentence, never an empty message"));
    assert!(!replied.to_string().contains("changed_files"));
    let store = refact_review_core::ReviewStore::open(&f.source).unwrap();
    let before = store.load(current["id"].as_str().unwrap()).unwrap();
    let journal = store.object_history(&before.id).unwrap();
    let mut forged = exact.clone();
    let mut extra = exact["contributions"][1].clone();
    extra["id"] = json!("forged-contribution");
    extra["body"] = json!("Unlinked answer");
    forged["contributions"].as_array_mut().unwrap().push(extra);
    let error = f
        .call(
            ReviewToolKind::Update,
            json!({
                "expected_revision":before.revision,"request_key":"forged-answer",
                "operation":{"op":"put_thread","thread":forged}
            }),
        )
        .await
        .unwrap_err();
    assert!(error.contains("contributions are append-only"));
    assert!(error.contains("review_reply(thread_id, body)"));
    assert!(error.contains("put_thread only re-anchors or resolves existing threads"));
    assert_eq!(store.load(&before.id).unwrap(), before);
    assert_eq!(store.object_history(&before.id).unwrap(), journal);
    std::fs::write(checkout.join("tracked.py"), b"assert 2 == 2\n").unwrap();
    let source = f
        .call(
            ReviewToolKind::Get,
            json!({"path":"tracked.py","side":"head","start_line":1,"end_line":1}),
        )
        .await
        .unwrap();
    assert_eq!(source["lines"][0]["text"], "assert 1 == 1");
    let prepared = f
        .call(
            ReviewToolKind::PrepareExecution,
            json!({"request_key":"before"}),
        )
        .await
        .unwrap();
    let app = f.ccx.lock().await.app.clone();
    let meta = crate::exec::ExecProcessMeta::new(
        crate::exec::ExecMode::Foreground,
        "fixture-registry-result".into(),
    )
    .with_chat_id("review-tool-smoke")
    .with_cwd(checkout.clone());
    let id = meta.process_id.clone();
    app.gcx.exec_registry.register(meta, 4096).await;
    app.gcx.exec_registry.mark_started(&id).await.unwrap();
    app.gcx
        .exec_registry
        .mark_exited(&id, Some(0))
        .await
        .unwrap();
    let recorded = f.call(ReviewToolKind::RecordEvidence, json!({"process_id":id.as_str(),"summary":"Simulated registry fixture, not product success","retained_kind":"repair","prepared_material_id":prepared["id"]})).await.unwrap();
    let evidence = recorded["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["source"]["kind"] == "executed")
        .unwrap();
    assert_eq!(evidence["source"]["exit_code"], 0);
    assert_eq!(evidence["candidate_id"], candidate);
    assert_ne!(evidence["applicability"], "current");
    assert!(recorded["materials"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["kind"] == "repair"));
    assert_eq!(
        std::fs::read(f.source.join("tracked.py")).unwrap(),
        b"assert 1 == 1\n"
    );
}

#[tokio::test]
async fn executor_rejects_invalid_arguments_ownership_and_fabricated_execution() {
    let f = Fixture::new().await;
    assert!(f.call(ReviewToolKind::Context, json!({})).await.is_err());
    assert!(!f.source.join(".refact/reviews").exists());
    assert!(f
        .call(ReviewToolKind::Context, json!({"action":"unknown"}))
        .await
        .is_err());
    let missing = f
        .call(
            ReviewToolKind::Context,
            json!({"action":"stack_prepare","discovery":{}}),
        )
        .await
        .unwrap_err();
    assert!(
        missing.contains("title")
            && missing.contains("members")
            && missing.contains("selected_member")
    );
    let prepared = f.call(ReviewToolKind::Context, json!({"action":"stack_prepare","discovery":{"title":"Pending fixture","source":{"kind":"provided_manifest","reference":"simulated fixture known membership"},"selected_member":"one","members":[{"key":"one","branch_ref":"missing-unresolved"}],"comparison_mode":"live_chain"}})).await.unwrap();
    assert_eq!(prepared["status"], "pending_scope_choice");
    assert!(f.call(ReviewToolKind::Context, json!({"repository":f.source,"base_ref":f.oid,"candidate_ref":f.oid,"request_key":"before-consent"})).await.is_err());
    let app = f.ccx.lock().await.app.clone();
    assert!(
        crate::review::service::get_for_chat(f.source.clone(), "review-tool-smoke".into())
            .await
            .unwrap()
            .is_none()
    );
    assert!(app
        .chat
        .sessions
        .read()
        .await
        .get("review-tool-smoke")
        .unwrap()
        .lock()
        .await
        .thread
        .worktree
        .is_none());
    let f = Fixture::new().await;
    f.activate().await;
    for (kind, args) in [
        (ReviewToolKind::Anchor, json!({"path":"tracked.py"})),
        (
            ReviewToolKind::Update,
            json!({"operation":{"op":"rename","title":"x"}}),
        ),
        (ReviewToolKind::Reply, json!({"body":"x"})),
        (ReviewToolKind::PrepareExecution, json!({})),
        (
            ReviewToolKind::RecordEvidence,
            json!({"process_id":"invented"}),
        ),
        (ReviewToolKind::Get, json!({"review_id":"foreign"})),
        (ReviewToolKind::Get, json!({"object_kind":"thread"})),
        (ReviewToolKind::Get, json!({"side":"head"})),
        (
            ReviewToolKind::Get,
            json!({"object_kind":"thread","object_id":"x","path":"tracked.py"}),
        ),
        (
            ReviewToolKind::Update,
            json!({"expected_revision":1,"request_key":"bad-op","operation":{"op":"unknown"}}),
        ),
        (
            ReviewToolKind::RecordEvidence,
            json!({"process_id":"invented","summary":"fake","retained_kind":"repair"}),
        ),
    ] {
        assert!(f.call(kind, args).await.is_err());
    }
    let app = f.ccx.lock().await.app.clone();
    let checkout = f.ccx.lock().await.execution_scope_worktree().unwrap().root;
    let meta =
        crate::exec::ExecProcessMeta::new(crate::exec::ExecMode::Foreground, "foreign".into())
            .with_chat_id("foreign")
            .with_cwd(checkout);
    let id = meta.process_id.clone();
    app.gcx.exec_registry.register(meta, 4096).await;
    app.gcx.exec_registry.mark_started(&id).await.unwrap();
    app.gcx
        .exec_registry
        .mark_exited(&id, Some(0))
        .await
        .unwrap();
    assert!(f
        .call(
            ReviewToolKind::RecordEvidence,
            json!({"process_id":id.as_str(),"summary":"foreign process","retained_kind":"repair"})
        )
        .await
        .is_err());
}

#[tokio::test]
async fn direct_capture_and_tour_edits_need_no_intake_proposal_or_thread() {
    let f = Fixture::new().await;
    let capture = json!({"repository":f.source,"base_ref":f.oid,"candidate_ref":f.oid,
        "request_key":"direct-capture","title":"Direct workspace"});
    let activated = f
        .call(ReviewToolKind::Context, capture.clone())
        .await
        .unwrap();
    let recaptured = f.call(ReviewToolKind::Context, capture).await.unwrap();
    assert_eq!(
        activated["review"]["revision"],
        recaptured["review"]["revision"]
    );
    let first_request = json!({"expected_revision":activated["review"]["revision"],
        "request_key":"direct-first","operation":{"op":"put_tour_step","step":{
        "id":"page","current":{"version":1,"title":"Flow","body":"Short accessible fallback",
        "html":"<main><h1>Flow</h1><p>Source-linked explanation</p></main>","anchors":[]},"history":[]}}});
    let first = f
        .call(ReviewToolKind::Update, first_request.clone())
        .await
        .unwrap();
    let mut exact = f
        .call(
            ReviewToolKind::Get,
            json!({"object_kind":"tour_step","object_id":"page"}),
        )
        .await
        .unwrap();
    assert_eq!(
        exact["current"]["html"],
        first_request["operation"]["step"]["current"]["html"]
    );
    exact["current"]["version"] = json!(2);
    exact["current"]["html"] = json!("<main><h1>Updated flow</h1></main>");
    let second_request = json!({"expected_revision":first["revision"],"request_key":"direct-second",
        "operation":{"op":"put_tour_step","step":exact}});
    let second = f
        .call(ReviewToolKind::Update, second_request.clone())
        .await
        .unwrap();
    for request in [first_request.clone(), second_request.clone()] {
        assert_eq!(
            f.call(ReviewToolKind::Update, request).await.unwrap()["revision"],
            second["revision"]
        );
    }
    let store = refact_review_core::ReviewStore::open(&f.source).unwrap();
    let persisted = store.load(second["id"].as_str().unwrap()).unwrap();
    assert!(persisted.threads.is_empty());
    assert!(persisted
        .privacy
        .iter()
        .any(|record| record.path.ends_with("tracked.py")));
    let app = f.ccx.lock().await.app.clone();
    let live = app
        .chat
        .sessions
        .read()
        .await
        .get("review-tool-smoke")
        .unwrap()
        .clone();
    let live = live.lock().await;
    let written = live
        .messages
        .iter()
        .rev()
        .find(|message| message.role == "tool" && message.extra.contains_key("review"))
        .unwrap();
    let carried = crate::privacy::records::records_to_carry(&[written.clone()]).unwrap();
    for record in &persisted.privacy {
        assert!(
            carried.contains(record),
            "Review tool result lost source privacy"
        );
    }
    drop(live);
    assert_eq!(persisted.tour[0].history.len(), 1);
    assert_eq!(persisted.tour[0].history[0].version, 1);
    let journal = store.object_history(&persisted.id).unwrap();
    for failure in [
        "cas",
        "candidate",
        "foreign",
        "version",
        "history",
        "changed_retry",
    ] {
        let mut request = second_request.clone();
        request["request_key"] = json!(format!("rejected-{failure}"));
        request["expected_revision"] = second["revision"].clone();
        request["operation"]["step"]["history"] =
            serde_json::to_value(&persisted.tour[0].history).unwrap();
        request["operation"]["step"]["current"]["version"] = json!(3);
        match failure {
            "cas" => request["expected_revision"] = json!(0),
            "candidate" => {
                request["operation"]["step"]["current"]["candidate_id"] = json!("foreign")
            }
            "foreign" => request["review_id"] = json!("foreign"),
            "version" => request["operation"]["step"]["current"]["version"] = json!(7),
            "history" => request["operation"]["step"]["history"] = json!([]),
            _ => {
                request = second_request.clone();
                request["operation"]["step"]["current"]["body"] = json!("forged retry");
            }
        }
        assert!(
            f.call(ReviewToolKind::Update, request).await.is_err(),
            "{failure}"
        );
    }
    assert_eq!(store.load(&persisted.id).unwrap(), persisted);
    assert_eq!(store.object_history(&persisted.id).unwrap(), journal);
    let reloaded = f
        .call(
            ReviewToolKind::Get,
            json!({"object_kind":"tour_step","object_id":"page"}),
        )
        .await
        .unwrap();
    assert_eq!(reloaded, serde_json::to_value(&persisted.tour[0]).unwrap());
}

#[test]
fn advertised_workspace_contract_has_no_intake_actions_or_receipts() {
    let context = ToolReview(ReviewToolKind::Context)
        .tool_description()
        .input_schema;
    let text = context.to_string();
    assert!(!text.contains("context_question") && !text.contains("context_sources"));
    assert!(context["properties"].get("context_intake").is_none());
    let update = ToolReview(ReviewToolKind::Update)
        .tool_description()
        .input_schema;
    assert!(update["properties"].get("context_intake").is_none());
}

#[test]
fn reply_description_requires_a_brief_turn_closing() {
    let description = ToolReview(ReviewToolKind::Reply)
        .tool_description()
        .description;
    assert!(description.contains("only way to answer a discussion"));
    assert!(description.contains("saved in the thread and delivered to chat as its linked message"));
    assert!(!description.contains("appears inline") && !description.contains("already shown"));
    assert!(description.contains("Do not repeat its content"));
    assert!(description.contains("Continue other requested work"));
    assert!(description.contains("end the turn with one short sentence, never an empty message"));
}
