use std::sync::Arc;
use tokio::sync::Mutex;
use crate::app_state::AppState;
use crate::chat::types::ChatSession;
use crate::call_validation::ChatMessage;
use super::capsule::{prepare_work_context, CapsuleState};

async fn fixture() -> (AppState, tempfile::TempDir) {
    let dir = tempfile::tempdir_in(dunce::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    let cache_dir = dir.path().join("cache");
    let source = dunce::canonicalize(dir.path()).unwrap();
    let project_hash = refact_worktrees::service::project_hash_for_path(&source);
    let registry_dir = cache_dir.join("worktrees").join(&project_hash);
    std::fs::create_dir_all(&registry_dir).unwrap();
    let mut mappings = Vec::new();
    for index in 0..50 {
        let id = format!("mapping-{index}");
        let root = registry_dir.join(&id);
        std::fs::create_dir_all(&root).unwrap();
        let root = dunce::canonicalize(root).unwrap();
        mappings.push(refact_worktrees::types::WorktreeRegistryRecord {
            creator_chat_id: None,
            meta: refact_worktrees::types::WorktreeMeta {
                id,
                kind: "chat".into(),
                root,
                source_workspace_root: source.clone(),
                repo_root: source.clone(),
                branch: Some(format!("bench-{index}")),
                base_branch: Some("main".into()),
                base_commit: None,
                task_id: None,
                card_id: None,
                agent_id: None,
                enforce: false,
            },
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            last_seen_at: None,
            references: Vec::new(),
            last_known_status: None,
        });
    }
    let registry = refact_worktrees::types::WorktreeRegistry {
        schema_version: 1,
        source_workspace_root: source,
        project_hash,
        records: mappings,
    };
    std::fs::write(
        registry_dir.join("index.json"),
        serde_json::to_vec(&registry).unwrap(),
    )
    .unwrap();
    let gcx =
        crate::global_context::tests::make_test_gcx_with_dirs(cache_dir, dir.path().join("config"))
            .await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().into()];
    let mut caps = crate::caps::CodeAssistantCaps::default();
    caps.chat_models.insert(
        "trusted/model".into(),
        Arc::new(crate::caps::ChatModelRecord {
            base: crate::caps::BaseModelRecord {
                id: "trusted/model".into(),
                n_ctx: 200_000,
                ..Default::default()
            },
            supports_tools: true,
            supports_agent: true,
            ..Default::default()
        }),
    );
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
    let mut caller = ChatSession::new("load-caller".into());
    caller.thread.model = "trusted/model".into();
    caller.thread.mode = "agent".into();
    caller.work_project_root = Some(dir.path().into());
    app.chat
        .sessions
        .write()
        .await
        .insert("load-caller".into(), Arc::new(Mutex::new(caller)));
    (app, dir)
}

#[serial_test::serial(runtime_settings)]
#[tokio::test]
#[ignore = "opt-in realistic provenance scale validation benchmark"]
async fn realistic_provenance_scale_and_first_row_capacity_failure() {
    let (app, dir) = fixture().await;
    let policy = refact_privacy::PrivacyPolicy {
        zones: (0..20)
            .map(|i| refact_privacy::Zone {
                name: format!("zone-{i}"),
                patterns: vec![dir
                    .path()
                    .join(format!("z{i}-*"))
                    .to_string_lossy()
                    .into_owned()],
                send_to: vec!["trusted".into()],
                on_shell_read: refact_privacy::ShellBehavior::Withhold,
            })
            .collect(),
        ..Default::default()
    };
    app.gcx.privacy_policy_load.write().unwrap().policy = Arc::new(policy);
    assert_eq!(
        crate::files_correction::registered_worktree_path_mappings(&app.gcx.cache_dir).len(),
        50
    );
    for count in [100usize, 500] {
        let started = std::time::Instant::now();
        for index in 0..count {
            let mut peer = ChatSession::new(format!("load-{count}-{index}"));
            peer.work_project_root = Some(dir.path().into());
            let mut source = ChatMessage::new("tool".into(), "synthetic provenance".into());
            crate::privacy::records::merge_records(
                &mut source,
                (0..10).map(|n| {
                    let zone = n % 20;
                    let path = dir.path().join(format!("z{zone}-{index}-{n}.rs"));
                    std::fs::write(&path, "synthetic source\n").unwrap();
                    refact_privacy::FileRecord {
                        path: path.to_string_lossy().into_owned(),
                        zone: format!("zone-{zone}"),
                        attribution: refact_privacy::Attribution::Declared,
                    }
                }),
            );
            peer.add_message(source);
            app.chat
                .sessions
                .write()
                .await
                .insert(format!("load-{count}-{index}"), Arc::new(Mutex::new(peer)));
        }
        let build_start = std::time::Instant::now();
        let message = prepare_work_context(
            &app,
            &mut CapsuleState::default(),
            "load-caller",
            "trusted/model",
        )
        .await
        .expect("capsule generated");
        let build = build_start.elapsed();
        let text = message.content.content_text_only();
        let cursor = text
            .split("work_overview(cursor=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .expect("recoverable overview cursor");
        let caller = super::scope::model_caller(&app, "load-caller", "trusted/model")
            .await
            .unwrap();
        let page_start = std::time::Instant::now();
        let page = super::service::overview(
            &app,
            &caller,
            super::types::WorkOverviewRequest {
                cursor: Some(cursor.into()),
                ..Default::default()
            },
        )
        .await
        .expect("overview must accept recoverable cursor");
        let page_time = page_start.elapsed();
        assert!(!page.rows.is_empty());
        assert!(
            page.next_cursor.is_some(),
            "large inventory must continue beyond one page"
        );
        assert!(page.rows.iter().all(|row| row.provenance_complete));
        assert!(page.source_records.len() > 100);
        let setup = build_start.duration_since(started);
        eprintln!("realistic_load rows={count} policy_zones=20 worktree_mappings=50 records_per_row=10 record_inputs={} page_rows={} setup_ms={} capsule_ms={} overview_ms={} result=success", count * 10, page.rows.len(), setup.as_millis(), build.as_millis(), page_time.as_millis());
        app.chat
            .sessions
            .write()
            .await
            .retain(|id, _| id == "load-caller");
    }
    let mut peer = ChatSession::new("oversized-first-row".into());
    peer.work_project_root = Some(dir.path().into());
    let mut source = ChatMessage::new("tool".into(), "oversized provenance".into());
    crate::privacy::records::merge_records(
        &mut source,
        (0..257).map(|n| {
            let path = dir.path().join(format!("overflow-{n}.rs"));
            std::fs::write(&path, "synthetic overflow source\n").unwrap();
            refact_privacy::FileRecord {
                path: path.to_string_lossy().into_owned(),
                zone: "normal".into(),
                attribution: refact_privacy::Attribution::Declared,
            }
        }),
    );
    peer.add_message(source);
    app.chat
        .sessions
        .write()
        .await
        .insert("oversized-first-row".into(), Arc::new(Mutex::new(peer)));
    let caller = super::scope::model_caller(&app, "load-caller", "trusted/model")
        .await
        .unwrap();
    let start = std::time::Instant::now();
    let result = super::service::overview(
        &app,
        &caller,
        super::types::WorkOverviewRequest {
            rows: Some(50),
            ..Default::default()
        },
    )
    .await;
    let elapsed = start.elapsed();
    let error = result.expect_err("first row exceeding provenance record cap must fail closed");
    assert_eq!(
        error.cause,
        Some(super::error::WorkFailureCause::ProvenanceCapacity)
    );
    eprintln!("realistic_load rows=1 records_per_row=257 setup_ms=0 overview_ms={} result=error category=provenance_capacity", elapsed.as_millis());
}
