use std::sync::Arc;
use tokio::sync::Mutex;
use crate::app_state::AppState;
use crate::chat::types::ChatSession;
use crate::call_validation::ChatMessage;
use super::capsule::{prepare_work_context, CapsuleState, CAPSULE_MARKER_KEY};

#[tokio::test(flavor = "current_thread")]
async fn blocking_projection_does_not_starve_current_thread_executor() {
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let job = tokio::spawn(async move {
        super::service::projection_blocking(move || {
            let _ = ready_tx.send(());
            let _ = release_rx.blocking_recv();
            Ok::<_, super::error::WorkError>(())
        })
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), ready_rx)
        .await
        .expect("blocking closure did not start")
        .expect("ready signal dropped");
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            tokio::task::yield_now().await;
            17
        })
        .await
        .expect("current-thread executor was blocked"),
        17
    );
    release_tx.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), job)
        .await
        .expect("projection failed to join")
        .expect("projection task panicked")
        .expect("projection returned error");
}

async fn fixture() -> (AppState, tempfile::TempDir, Arc<Mutex<ChatSession>>) {
    let dir = tempfile::tempdir_in(dunce::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        dir.path().join("cache"),
        dir.path().join("config"),
    )
    .await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().into()];
    let mut caps = crate::caps::CodeAssistantCaps::default();
    for id in ["trusted/model", "other/model"] {
        caps.chat_models.insert(
            id.into(),
            Arc::new(crate::caps::ChatModelRecord {
                base: crate::caps::BaseModelRecord {
                    id: id.into(),
                    n_ctx: 200_000,
                    ..Default::default()
                },
                supports_tools: true,
                supports_agent: true,
                ..Default::default()
            }),
        );
    }
    let (project_root, _) = crate::caps::project_model_defaults_state(gcx.clone()).await;
    {
        let mut state = gcx.caps_state.write().await;
        state.caps = Some(Arc::new(caps));
        state.loaded_project_root = Some(project_root);
        state.last_attempted_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }
    let app = AppState::from_gcx(gcx).await;
    let mut caller = ChatSession::new("capsule-caller".into());
    caller.thread.model = "trusted/model".into();
    caller.thread.mode = "agent".into();
    caller.work_project_root = Some(dir.path().into());
    caller.add_message(ChatMessage::new(
        "user".into(),
        "keep this prefix byte-for-byte".into(),
    ));
    let caller = Arc::new(Mutex::new(caller));
    app.chat
        .sessions
        .write()
        .await
        .insert("capsule-caller".into(), caller.clone());
    let mut peer = ChatSession::new("capsule-peer".into());
    peer.work_project_root = Some(dir.path().into());
    peer.thread.title = "Permitted peer".into();
    app.chat
        .sessions
        .write()
        .await
        .insert("capsule-peer".into(), Arc::new(Mutex::new(peer)));
    (app, dir, caller)
}

#[serial_test::serial(runtime_settings)]
#[tokio::test]
async fn generation_appends_one_capsule_without_rewriting_existing_messages() {
    let (app, _dir, caller) = fixture().await;
    let before = serde_json::to_vec(&caller.lock().await.messages).unwrap();
    tokio::join!(
        crate::chat::prepare_project_work_context(&app, &caller),
        crate::chat::prepare_project_work_context(&app, &caller),
    );
    crate::chat::prepare_project_work_context(&app, &caller).await;
    let session = caller.lock().await;
    assert_eq!(
        session
            .messages
            .iter()
            .filter(|message| message.extra.contains_key(CAPSULE_MARKER_KEY))
            .count(),
        1
    );
    assert_eq!(serde_json::to_vec(&session.messages[..1]).unwrap(), before);
    let capsule = session.messages.last().unwrap().content.content_text_only();
    assert!(capsule.contains("Permitted peer"));
    assert!(capsule.contains("observed_at_ms"));
    assert!(capsule.contains("checkout"));
}

#[serial_test::serial(runtime_settings)]
#[tokio::test]
async fn reloaded_session_reuses_capsule_marker_and_removed_capsule_can_be_regenerated() {
    let (app, dir, caller) = fixture().await;
    crate::chat::prepare_project_work_context(&app, &caller).await;
    let before = caller.lock().await.messages.len();
    assert_eq!(before, 2);
    let (messages, thread) = {
        let session = caller.lock().await;
        (session.messages.clone(), session.thread.clone())
    };
    let mut restored = ChatSession::new_with_trajectory(
        "capsule-caller".into(),
        messages,
        thread,
        chrono::Utc::now().to_rfc3339(),
        None,
        vec![],
        None,
    );
    restored.work_project_root = Some(dir.path().into());
    *caller.lock().await = restored;
    crate::chat::prepare_project_work_context(&app, &caller).await;
    assert_eq!(caller.lock().await.messages.len(), before);
    caller
        .lock()
        .await
        .messages
        .retain(|message| !message.extra.contains_key(CAPSULE_MARKER_KEY));
    crate::chat::prepare_project_work_context(&app, &caller).await;
    assert_eq!(caller.lock().await.messages.len(), before);
}

#[serial_test::serial(runtime_settings)]
#[tokio::test]
async fn many_rows_shrink_to_total_message_budget_and_supply_a_recoverable_inventory_cursor() {
    let (app, dir, _caller) = fixture().await;
    for number in 0..40 {
        let id = format!("peer-{number}");
        let mut peer = ChatSession::new(id.clone());
        peer.work_project_root = Some(dir.path().into());
        peer.thread.title = format!("Peer {number}: {}", "large title ".repeat(20));
        app.chat
            .sessions
            .write()
            .await
            .insert(id, Arc::new(Mutex::new(peer)));
    }
    let mut state = CapsuleState::default();
    let message = super::service::PROJECTION_BUILDS
        .scope(std::cell::Cell::new(0), async {
            let message = prepare_work_context(&app, &mut state, "capsule-caller", "trusted/model")
                .await
                .unwrap();
            assert_eq!(
                super::service::PROJECTION_BUILDS.with(|count| count.get()),
                1,
                "omitted-row inventory must reuse the capsule's cleared projection"
            );
            message
        })
        .await;
    let wire = refact_llm::adapters::render_extra::render_event_message(&message);
    assert!(
        crate::tokens::count_text_tokens(None, &wire).unwrap()
            <= crate::file_activity::settings().capsule_token_budget
    );
    let text = message.content.content_text_only();
    let token = text
        .split("work_overview(cursor=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let mut caller = super::scope::model_caller(&app, "capsule-caller", "trusted/model")
        .await
        .unwrap();
    let page = super::service::overview(
        &app,
        &caller,
        super::types::WorkOverviewRequest {
            cursor: Some(token.into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(page.total, 42);
    assert_eq!(page.rows.len(), 42);
    let ids: std::collections::HashSet<_> = page
        .rows
        .iter()
        .filter_map(|row| row.chat_id.as_deref())
        .collect();
    assert!(ids.contains("capsule-caller"));
    assert!(ids.contains("capsule-peer"));
    for number in 0..40 {
        assert!(ids.contains(format!("peer-{number}").as_str()));
    }
    let request = super::types::WorkOverviewRequest {
        cursor: Some(token.into()),
        ..Default::default()
    };
    assert!(super::capsule::still_cleared(&app, &message));
    let original_checkout = caller.checkout_root.clone();
    caller.checkout_root = dir.path().join("other-checkout");
    std::fs::create_dir(&caller.checkout_root).unwrap();
    assert_eq!(
        super::service::overview(&app, &caller, request.clone())
            .await
            .unwrap_err()
            .code,
        "invalid_cursor"
    );
    caller.checkout_root = original_checkout;
    app.gcx.privacy_policy_load.write().unwrap().policy = Arc::new(refact_privacy::PrivacyPolicy {
        zones: vec![refact_privacy::Zone {
            name: "guarded".into(),
            patterns: vec![dir.path().join("guarded.rs").to_string_lossy().into()],
            send_to: vec!["other".into()],
            on_shell_read: refact_privacy::ShellBehavior::Withhold,
        }],
        ..Default::default()
    });
    assert_eq!(
        super::service::overview(&app, &caller, request)
            .await
            .unwrap_err()
            .code,
        "invalid_cursor"
    );
    assert!(!super::capsule::still_cleared(&app, &message));
    app.gcx.privacy_policy_load.write().unwrap().error = Some("unavailable".into());
    assert!(!super::capsule::still_cleared(&app, &message));
}

#[serial_test::serial(runtime_settings)]
#[tokio::test]
async fn cached_tokenizer_branch_keeps_the_complete_capsule_within_budget() {
    let (app, dir, _caller) = fixture().await;
    let vocab: serde_json::Map<String, serde_json::Value> = (0u8..=127)
        .enumerate()
        .map(|(index, byte)| ((byte as char).to_string(), serde_json::json!(index)))
        .collect();
    let model: tokenizers::models::bpe::BPE = serde_json::from_str(
        &serde_json::json!({
            "type":"BPE", "vocab":vocab, "merges":[]
        })
        .to_string(),
    )
    .unwrap();
    let tokenizer = Arc::new(tokenizers::Tokenizer::new(model));
    for index in 0..40 {
        let id = format!("token-peer-{index}");
        let mut peer = ChatSession::new(id.clone());
        peer.work_project_root = Some(dir.path().into());
        peer.thread.title = "<&>".repeat(150);
        app.chat
            .sessions
            .write()
            .await
            .insert(id, Arc::new(Mutex::new(peer)));
    }
    app.gcx
        .tokenizer_state
        .write()
        .unwrap()
        .map
        .insert("trusted/model".into(), Some(tokenizer.clone()));
    let message = prepare_work_context(
        &app,
        &mut CapsuleState::default(),
        "capsule-caller",
        "trusted/model",
    )
    .await
    .unwrap();
    let text = refact_llm::adapters::render_extra::render_event_message(&message);
    assert!(message
        .content
        .content_text_only()
        .contains("work_overview(cursor="));
    assert!(crate::tokens::count_text_tokens(Some(tokenizer.clone()), &text).unwrap() > 0);
    assert!(
        crate::tokens::count_text_tokens(Some(tokenizer), &text).unwrap()
            <= crate::file_activity::settings().capsule_token_budget
    );
}

#[serial_test::serial(runtime_settings)]
#[tokio::test]
async fn escaped_peer_titles_remain_bounded_after_provider_xml_rendering() {
    let (app, dir, _caller) = fixture().await;
    for index in 0..10 {
        let id = format!("escaped-{index}");
        let mut peer = ChatSession::new(id.clone());
        peer.work_project_root = Some(dir.path().into());
        peer.thread.title = "<&>".repeat(200);
        app.chat
            .sessions
            .write()
            .await
            .insert(id, Arc::new(Mutex::new(peer)));
    }
    let message = prepare_work_context(
        &app,
        &mut CapsuleState::default(),
        "capsule-caller",
        "trusted/model",
    )
    .await
    .unwrap();
    let wire = refact_llm::adapters::render_extra::render_event_message(&message);
    assert!(
        crate::tokens::count_text_tokens(None, &wire).unwrap()
            <= crate::file_activity::settings().capsule_token_budget
    );
    assert!(message
        .content
        .content_text_only()
        .contains("work_overview(cursor="));
}

#[serial_test::serial(runtime_settings)]
#[tokio::test]
async fn provider_event_rendering_keeps_prior_capsule_prefix_and_hides_private_marker_metadata() {
    let (app, _dir, caller) = fixture().await;
    crate::chat::prepare_project_work_context(&app, &caller).await;
    let prior = caller.lock().await.messages.last().unwrap().clone();
    let rendered_before = refact_llm::adapters::render_extra::render_event_message(&prior);
    caller.lock().await.thread.model = "other/model".into();
    crate::chat::prepare_project_work_context(&app, &caller).await;
    let session = caller.lock().await;
    assert_eq!(session.messages.len(), 3);
    assert_eq!(
        refact_llm::adapters::render_extra::render_event_message(&session.messages[1]),
        rendered_before
    );
    let current = refact_llm::adapters::render_extra::render_event_message(&session.messages[2]);
    assert!(current.contains("coordination.work"));
    assert!(!current.contains("policy_key"));
    assert!(!current.contains("resolved_model"));
    assert!(!current.contains(CAPSULE_MARKER_KEY));
}

#[serial_test::serial(runtime_settings)]
#[tokio::test]
async fn model_policy_changes_force_recheck_inside_debounce_and_never_disclose_denied_peer() {
    let (app, dir, caller) = fixture().await;
    let path = dir.path().join("guarded.rs");
    std::fs::write(&path, "source").unwrap();
    app.gcx.privacy_policy_load.write().unwrap().policy = Arc::new(refact_privacy::PrivacyPolicy {
        zones: vec![refact_privacy::Zone {
            name: "guarded".into(),
            patterns: vec![path.to_string_lossy().into()],
            send_to: vec!["trusted".into()],
            on_shell_read: refact_privacy::ShellBehavior::Withhold,
        }],
        ..Default::default()
    });
    let peer = app
        .chat
        .sessions
        .read()
        .await
        .get("capsule-peer")
        .cloned()
        .unwrap();
    let mut source = ChatMessage::new("tool".into(), "source".into());
    crate::privacy::records::merge_records(
        &mut source,
        [refact_privacy::FileRecord {
            path: path.to_string_lossy().into(),
            zone: "guarded".into(),
            attribution: refact_privacy::Attribution::Declared,
        }],
    );
    peer.lock().await.add_message(source);
    let mut state = CapsuleState::default();
    let first = prepare_work_context(&app, &mut state, "capsule-caller", "trusted/model")
        .await
        .unwrap();
    assert!(first.content.content_text_only().contains("Permitted peer"));
    caller.lock().await.thread.model = "other/model".into();
    let second = prepare_work_context(&app, &mut state, "capsule-caller", "other/model")
        .await
        .unwrap();
    assert!(!second
        .content
        .content_text_only()
        .contains("Permitted peer"));
    assert!(!second.content.content_text_only().contains("guarded"));
    assert!(crate::privacy::records::records_to_carry(&[second])
        .unwrap()
        .is_empty());
    assert!(
        prepare_work_context(&app, &mut state, "capsule-caller", "other/model")
            .await
            .is_none()
    );
}

#[serial_test::serial(runtime_settings)]
#[tokio::test]
async fn policy_revocation_after_blocking_projection_prevents_returned_metadata() {
    let (app, dir, _caller) = fixture().await;
    let caller = super::scope::model_caller(&app, "capsule-caller", "trusted/model")
        .await
        .unwrap();
    let gate_app = app.clone();
    let gate_caller = caller.clone();
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(super::service::PROJECTION_POST_WORK_GATE.scope(
        std::cell::RefCell::new(Some((entered_tx, release_rx))),
        async move { super::service::visible_projection(&gate_app, &gate_caller).await },
    ));
    tokio::time::timeout(std::time::Duration::from_secs(10), entered_rx)
        .await
        .expect("projection did not reach gate")
        .expect("gate dropped");
    let path = dir.path().join("guarded.rs");
    app.gcx.privacy_policy_load.write().unwrap().policy = Arc::new(refact_privacy::PrivacyPolicy {
        zones: vec![refact_privacy::Zone {
            name: "guarded".into(),
            patterns: vec![path.to_string_lossy().into()],
            send_to: vec!["other".into()],
            on_shell_read: refact_privacy::ShellBehavior::Withhold,
        }],
        ..Default::default()
    });
    release_tx.send(()).unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), task)
        .await
        .expect("projection did not finish")
        .expect("projection panicked");
    match result {
        Err(error) => assert_eq!(error.code, "policy_unavailable"),
        Ok(_) => panic!("revoked disclosure must not return catalog metadata"),
    }
}

#[cfg(feature = "perf-tests")]
#[serial_test::serial(runtime_settings)]
#[tokio::test]
#[ignore = "opt-in synthetic capsule benchmark; run with --features perf-tests -- --ignored"]
async fn benchmark_provenance_rich_capsule_omission_inventory() {
    let (app, dir, _caller) = fixture().await;
    let files: Vec<_> = (0..10)
        .map(|n| {
            let path = dir.path().join(format!("synthetic-{n}.rs"));
            std::fs::write(&path, format!("// synthetic benchmark file {n}\n")).unwrap();
            path
        })
        .collect();
    for count in [100usize, 1000] {
        if count == 1000 {
            app.chat
                .sessions
                .write()
                .await
                .retain(|id, _| !id.starts_with("bench-100-"));
        }
        for index in 0..count {
            let id = format!("bench-{count}-{index}");
            let mut peer = ChatSession::new(id.clone());
            peer.work_project_root = Some(dir.path().into());
            peer.thread.title = format!(
                "Synthetic inventory work {index} — {}",
                "details ".repeat(5)
            );
            let mut records_message =
                ChatMessage::new("tool".into(), "synthetic declared files".into());
            crate::privacy::records::merge_records(
                &mut records_message,
                files.iter().map(|path| refact_privacy::FileRecord {
                    path: path.to_string_lossy().into_owned(),
                    zone: "normal".into(),
                    attribution: refact_privacy::Attribution::Declared,
                }),
            );
            peer.add_message(records_message);
            app.chat
                .sessions
                .write()
                .await
                .insert(id, Arc::new(Mutex::new(peer)));
        }
        let build = async {
            let start = std::time::Instant::now();
            let message = prepare_work_context(
                &app,
                &mut CapsuleState::default(),
                "capsule-caller",
                "trusted/model",
            )
            .await
            .expect("synthetic capsule should be produced");
            (message, start.elapsed())
        };
        let (message, elapsed) = build.await;
        let body = message.content.content_text_only();
        assert!(
            body.contains("rows hidden"),
            "expected capsule omission for {count} rows"
        );
        let token = body
            .split("work_overview(cursor=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        let caller = super::scope::model_caller(&app, "capsule-caller", "trusted/model")
            .await
            .unwrap();
        let inventory_start = std::time::Instant::now();
        let page = super::service::overview(
            &app,
            &caller,
            super::types::WorkOverviewRequest {
                cursor: Some(token.into()),
                ..Default::default()
            },
        )
        .await
        .expect("inventory cursor should recover inventory");
        let inventory_elapsed = inventory_start.elapsed();
        assert!(
            page.source_records.len() >= 10,
            "provenance must be real before counting benchmark valid"
        );
        assert_eq!(page.total, count + 2);
        eprintln!("capsule synthetic rows={count} records={} capsule_stage_ms={} inventory_stage_ms={} inventory_page_rows={}", page.source_records.len(), elapsed.as_millis(), inventory_elapsed.as_millis(), page.rows.len());
    }
}
