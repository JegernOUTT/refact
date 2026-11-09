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

fn intake(retrieval_id: &str) -> Value {
    let gaps = ["mcp", "web", "messenger", "knowledge", "trajectories", "tasks", "memory"]
        .map(|category| json!({"category":category,"status":"unavailable","reason":"Not provided by this source-only smoke fixture"}));
    json!({"user_message_id":"smoke-user","retrieval_tool_call_ids":[retrieval_id],
        "gaps":gaps,"summary":"Inspect the fixture source; no external context or execution verdict asserted"})
}

struct Fixture {
    _temp: tempfile::TempDir,
    source: PathBuf,
    oid: String,
    ccx: Arc<Mutex<AtCommandsContext>>,
    tour_intakes: Mutex<std::collections::HashMap<String, Value>>,
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
        // Captured native IO fixture: real file bytes, not a mocked intake decision.
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
            tour_intakes: Mutex::new(std::collections::HashMap::new()),
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
                "request_key":"capture","title":"Smoke","context_intake":intake("smoke-source")
            }),
        )
        .await
        .unwrap()
    }

    async fn update(&self, operation: Value, key: &str) -> Value {
        let current = self.call(ReviewToolKind::Get, json!({})).await.unwrap();
        let mut request = json!({"expected_revision":current["revision"],"request_key":key,"operation":operation});
        if request["operation"]["op"] == "put_tour_step" {
            let saved = self.tour_intakes.lock().await.get(key).cloned();
            request["context_intake"] = if let Some(saved) = saved {
                saved
            } else {
                let (read_id, _) = self
                    .call_with_id(
                        ReviewToolKind::Get,
                        json!({"path":"tracked.py","side":"head","start_line":1,"end_line":1}),
                    )
                    .await
                    .unwrap();
                let intake = intake(&read_id);
                self.tour_intakes
                    .lock()
                    .await
                    .insert(key.into(), intake.clone());
                intake
            };
        }
        self.call(ReviewToolKind::Update, request)
            .await
            .unwrap_or_else(|error| panic!("fixture update {key} failed: {error}"))
    }
}

#[tokio::test]
async fn executor_receipt_history_three_versions_exact_retries_and_tamper_rejection() {
    let f = Fixture::new().await;
    f.activate().await;
    let store = refact_review_core::ReviewStore::open(&f.source).unwrap();
    let mut retries = Vec::new();
    let mut snapshots = Vec::new();
    for version in 1..=3 {
        let before = store.get_for_chat("review-tool-smoke").unwrap().unwrap();
        let mut history = before
            .tour
            .first()
            .map(|s| s.history.clone())
            .unwrap_or_default();
        for retained in &mut history {
            retained.context_intake = None;
        }
        let operation = json!({"op":"put_tour_step","step":{"id":"history","current":{
            "version":version,"title":"History","body":format!("Explanation {version}"),"anchors":[]
        },"history":history}});
        let key = format!("history-{version}");
        f.update(operation.clone(), &key).await;
        let saved = f.tour_intakes.lock().await.get(&key).cloned().unwrap();
        let request = json!({"expected_revision":before.revision,"request_key":key,
            "operation":operation,"context_intake":saved});
        retries.push(request);
        snapshots.push(store.get_for_chat("review-tool-smoke").unwrap().unwrap());
    }
    let persisted = snapshots.last().unwrap();
    assert_eq!(persisted.tour[0].history.len(), 2);
    for (index, version) in persisted.tour[0].history.iter().enumerate() {
        assert_eq!(version, &snapshots[index].tour[0].current);
        assert!(version.context_intake.is_some());
    }
    let journal = store.object_history(&persisted.id).unwrap();
    for request in &retries {
        let replay = f
            .call(ReviewToolKind::Update, request.clone())
            .await
            .unwrap();
        assert_eq!(replay["revision"], persisted.revision);
    }
    for field in ["body", "receipt", "current_receipt", "extra", "short_fresh"] {
        let mut tampered = retries[2].clone();
        let receipt = serde_json::to_value(&persisted.tour[0].history[0].context_intake).unwrap();
        match field {
            "body" => tampered["operation"]["step"]["history"][0]["body"] = json!("forged"),
            "receipt" => tampered["operation"]["step"]["history"][0]["context_intake"] = receipt,
            "current_receipt" => {
                tampered["operation"]["step"]["current"]["context_intake"] = receipt
            }
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
    assert_eq!(replied["threads"][0]["contributions_total"], 2);
    let exact = f
        .call(
            ReviewToolKind::Get,
            json!({"object_kind":"thread","object_id":thread_id}),
        )
        .await
        .unwrap();
    assert_eq!(exact["contributions"][1]["body"], "Fixture reply");
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
async fn context_sources_return_actual_ids_for_capture_and_tour_without_broadening_access() {
    let f = Fixture::new().await;
    let before = f
        .call(
            ReviewToolKind::Context,
            json!({"action":"context_sources","phase":"before_review"}),
        )
        .await
        .unwrap();
    assert_eq!(before["sources"].as_array().unwrap().len(), 1);
    let source = &before["sources"][0];
    assert_eq!(source["tool_call_id"], "smoke-source");
    assert_eq!(
        source["result_hash"],
        refact_review_core::content_hash(b"assert 1 == 1\n")
    );
    assert!(source.get("content").is_none() && source.get("arguments").is_none());
    let activation = f.call(ReviewToolKind::Context, json!({
        "repository":f.source,"base_ref":f.oid,"candidate_ref":f.oid,"request_key":"from-helper",
        "context_intake":intake(source["tool_call_id"].as_str().unwrap())
    })).await.unwrap();
    let empty = f
        .call(
            ReviewToolKind::Context,
            json!({"action":"context_sources","phase":"before_tour"}),
        )
        .await
        .unwrap();
    assert!(empty["sources"].as_array().unwrap().is_empty());
    let (read_id, _) = f
        .call_with_id(ReviewToolKind::Get, json!({"path":"tracked.py"}))
        .await
        .unwrap();
    let tour = f
        .call(
            ReviewToolKind::Context,
            json!({"action":"context_sources","phase":"before_tour"}),
        )
        .await
        .unwrap();
    assert_eq!(tour["sources"][0]["tool_call_id"], read_id);
    let authored = f.call(ReviewToolKind::Update, json!({"expected_revision":activation["review"]["revision"],
        "request_key":"helper-tour","context_intake":intake(tour["sources"][0]["tool_call_id"].as_str().unwrap()),
        "operation":{"op":"put_tour_step","step":{"id":"helper","current":{"version":1,"title":"Source","body":"Read candidate source","anchors":[]},"history":[]}}
    })).await.unwrap();
    assert_eq!(authored["tour_total"], 1);
    assert!(f
        .call(
            ReviewToolKind::Context,
            json!({"action":"context_sources","phase":"before_tour","review_id":"foreign"})
        )
        .await
        .is_err());
    assert!(f
        .call(
            ReviewToolKind::Context,
            json!({"action":"context_sources","phase":"before_review","repository":"/private"})
        )
        .await
        .is_err());
    let app = f.ccx.lock().await.app.clone();
    let live = app
        .chat
        .sessions
        .read()
        .await
        .get("review-tool-smoke")
        .unwrap()
        .clone();
    let mut user = ChatMessage::new("user".into(), "New ordinary directive".into());
    user.message_id = "new-user".into();
    live.lock().await.add_message(user);
    let empty = f
        .call(
            ReviewToolKind::Context,
            json!({"action":"context_sources","phase":"before_review"}),
        )
        .await
        .unwrap();
    assert!(empty["sources"].as_array().unwrap().is_empty());
    f.ccx.lock().await.chat_id = "unrelated".into();
    let args = serde_json::from_value(json!({"action":"context_sources","phase":"before_review"}))
        .unwrap();
    assert!(ToolReview(ReviewToolKind::Context)
        .tool_execute(f.ccx.clone(), &"unrelated-helper".into(), &args)
        .await
        .is_err());
    f.ccx.lock().await.chat_id = "review-tool-smoke".into();
    f.ccx.lock().await.subchat_depth = 1;
    assert!(f
        .call(
            ReviewToolKind::Context,
            json!({"action":"context_sources","phase":"before_review"})
        )
        .await
        .is_err());
}
