use std::sync::Arc;
use tokio::sync::Mutex;
use super::catalog_cache::{neutral, CATALOG_BUILDS};
use crate::chat::types::ChatSession;

#[tokio::test]
async fn ordinary_session_scope_move_keeps_local_scheduler_fallback() {
    let project = tempfile::tempdir().unwrap();
    let foreign = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx().await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![project.path().into()];
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    let mut session = ChatSession::new("scoped-session".into());
    session.work_project_root = Some(project.path().into());
    session.set_title("Local".into(), false);
    let session = Arc::new(Mutex::new(session));
    app.chat
        .sessions
        .write()
        .await
        .insert("scoped-session".into(), session.clone());
    let caller = super::scope::operator_caller(&app).await.unwrap();
    let initial = neutral(&app, &caller).await.unwrap();
    let ticket = app
        .work_scheduler
        .reserve(refact_work_core::AdmissionRequest::new(
            refact_work_core::WorkKey::trajectory("scoped-session").run(1),
            refact_work_core::WorkScope::single(initial.project.clone()),
        ))
        .unwrap();
    let permit = ticket.acquire().await.unwrap();
    let _parked = permit.park().unwrap();
    assert!(neutral(&app, &caller)
        .await
        .unwrap()
        .find("scoped-session")
        .is_some());
    for root in [Some(foreign.path().to_path_buf()), None] {
        {
            let mut session = session.lock().await;
            session.work_project_root = root;
            session.set_title("Moved".into(), false);
        }
        let updated = neutral(&app, &caller).await.unwrap();
        let cold = super::catalog::build_catalog_for_destination(
            &app,
            Some(&caller.source_root.to_string_lossy()),
            true,
        )
        .await
        .unwrap();
        let row = updated.find("scoped-session").unwrap();
        assert_eq!(row.state, super::types::WorkStateTag::Parked);
        assert!(row.title.is_none());
        assert_eq!(Some(row), cold.find("scoped-session"));
    }
}

#[tokio::test]
async fn folded_agent_child_refresh_matches_cold_build_without_registry_scan() {
    let dir = tempfile::tempdir_in(dunce::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    let gcx = crate::global_context::tests::make_test_gcx().await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().into()];
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    let mut child = ChatSession::new("folded-child".into());
    child.work_project_root = Some(dir.path().into());
    child.set_title("A".into(), false);
    let child = Arc::new(Mutex::new(child));
    app.chat
        .sessions
        .write()
        .await
        .insert("folded-child".into(), child.clone());
    let agent: crate::agents::types::BackgroundAgent = serde_json::from_value(serde_json::json!({
        "schema_version":1, "agent_id":"folded-agent", "parent_chat_id":"parent",
        "child_chat_id":"folded-child", "kind":"subagent", "config_name":"subagent",
        "title":"", "prompt":"", "target_files":[], "status":"completed",
        "step_count":0, "edited_files":[], "model":"model",
        "created_at":"2026-01-01T00:00:00Z", "last_update_at":"2026-01-01T00:00:00Z",
        "change_seq":1
    }))
    .unwrap();
    app.agents
        .records_write_guard_for_test()
        .await
        .insert(agent.agent_id.clone(), agent);
    let caller = super::scope::operator_caller(&app).await.unwrap();
    CATALOG_BUILDS
        .scope(std::cell::Cell::new(0), async {
            let initial = neutral(&app, &caller).await.unwrap();
            assert_eq!(initial.find("folded-agent").unwrap().queue_size, 0);
            for (queue, title) in [(2, "A"), (0, "B")] {
                {
                    let mut child = child.lock().await;
                    child.runtime.queue_size = queue;
                    child.set_title(title.into(), false);
                    let mut source =
                        crate::call_validation::ChatMessage::new("tool".into(), "source".into());
                    source.extra.insert("privacy".into(), serde_json::json!({"files":[{
                    "path":dir.path().join(title), "zone":"guarded", "attribution":"declared"
                }]}));
                    child.replace_messages(vec![source]);
                }
                let scans = app.agents.list_all_calls_for_test();
                let updated = neutral(&app, &caller).await.unwrap();
                assert_eq!(app.agents.list_all_calls_for_test(), scans);
                assert_eq!(CATALOG_BUILDS.with(|count| count.get()), 1);
                let cold = super::catalog::build_catalog_for_destination(
                    &app,
                    Some(&caller.source_root.to_string_lossy()),
                    true,
                )
                .await
                .unwrap();
                let row = updated.find("folded-agent").unwrap();
                assert_eq!(row.queue_size, queue);
                assert_eq!(row.title.as_deref(), Some(title));
                assert_eq!(row, cold.find("folded-agent").unwrap());
                assert!(row.provenance_complete);
                assert_eq!(
                    row.source_records[0].path,
                    dir.path().join(title).to_string_lossy()
                );
            }
            {
                let mut records = app.agents.records_write_guard_for_test().await;
                let agent = records.get_mut("folded-agent").unwrap();
                agent.title = "Registry wins".into();
                agent.pending_deliveries = serde_json::from_value(serde_json::json!([
                    {"id":"one"}, {"id":"two"}
                ]))
                .unwrap();
            }
            let ticket = app
                .work_scheduler
                .reserve(refact_work_core::AdmissionRequest::new(
                    refact_work_core::WorkKey::background_agent("folded-agent").run(1),
                    refact_work_core::WorkScope::single(initial.project.clone()),
                ))
                .unwrap();
            let permit = ticket.acquire().await.unwrap();
            let _parked = permit.park().unwrap();
            neutral(&app, &caller).await.unwrap();
            child.lock().await.set_title("C".into(), false);
            let scans = app.agents.list_all_calls_for_test();
            let updated = neutral(&app, &caller).await.unwrap();
            assert_eq!(app.agents.list_all_calls_for_test(), scans);
            assert_eq!(CATALOG_BUILDS.with(|count| count.get()), 2);
            let cold = super::catalog::build_catalog_for_destination(
                &app,
                Some(&caller.source_root.to_string_lossy()),
                true,
            )
            .await
            .unwrap();
            let row = updated.find("folded-agent").unwrap();
            assert_eq!(row.queue_size, 2);
            assert_eq!(row.title.as_deref(), Some("Registry wins"));
            assert_eq!(row.state, super::types::WorkStateTag::Parked);
            assert!(row.metadata.priority.is_some());
            assert_eq!(row, cold.find("folded-agent").unwrap());
            let foreign = tempfile::tempdir().unwrap();
            {
                let mut child = child.lock().await;
                child.work_project_root = Some(foreign.path().into());
                child.set_title("Moved".into(), false);
            }
            let moved = neutral(&app, &caller).await.unwrap();
            assert_eq!(CATALOG_BUILDS.with(|count| count.get()), 3);
            let cold = super::catalog::build_catalog_for_destination(
                &app,
                Some(&caller.source_root.to_string_lossy()),
                true,
            )
            .await
            .unwrap();
            assert_eq!(moved.find("folded-agent"), cold.find("folded-agent"));
            app.chat.sessions.write().await.remove("folded-child");
            let removed = neutral(&app, &caller).await.unwrap();
            assert_eq!(CATALOG_BUILDS.with(|count| count.get()), 4);
            let cold = super::catalog::build_catalog_for_destination(
                &app,
                Some(&caller.source_root.to_string_lossy()),
                true,
            )
            .await
            .unwrap();
            assert_eq!(removed.find("folded-agent"), cold.find("folded-agent"));
        })
        .await;
}

#[tokio::test]
async fn five_chat_principals_share_neutral_snapshot_without_rebuild_or_session_scan() {
    let dir = tempfile::tempdir_in(dunce::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    let foreign = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx().await;
    *gcx.documents_state.workspace_folders.lock().unwrap() =
        vec![dir.path().into(), foreign.path().into()];
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    for i in 0..5 {
        let mut session = ChatSession::new(format!("peer-{i}"));
        session.work_project_root = Some(dir.path().into());
        session.set_title(format!("Original {i}"), false);
        app.chat
            .sessions
            .write()
            .await
            .insert(session.chat_id.clone(), Arc::new(Mutex::new(session)));
    }
    let mut caller = super::scope::operator_caller(&app).await.unwrap();
    CATALOG_BUILDS
        .scope(std::cell::Cell::new(0), async {
            let first = neutral(&app, &caller).await.unwrap();
            assert_eq!(first.len(), 5);
            let sessions: Vec<_> = app.chat.sessions.read().await.values().cloned().collect();
            let mut guards = Vec::new();
            for session in &sessions {
                guards.push(session.lock().await);
            }
            for chat in 0..5 {
                caller.principal = format!("chat:peer-{chat}");
                for _ in 0..3 {
                    assert!(Arc::ptr_eq(&first, &neutral(&app, &caller).await.unwrap()));
                }
            }
            assert_eq!(CATALOG_BUILDS.with(|count| count.get()), 1);
            drop(guards);
            let peer = app.chat.sessions.read().await["peer-0"].clone();
            peer.lock().await.set_title("Changed".into(), false);
            let changed = neutral(&app, &caller).await.unwrap();
            assert_eq!(
                changed.find("peer-0").unwrap().title.as_deref(),
                Some("Changed")
            );
            assert_eq!(CATALOG_BUILDS.with(|count| count.get()), 1);
            let mut source =
                crate::call_validation::ChatMessage::new("tool".into(), "source".into());
            source.extra.insert(
                "privacy".into(),
                serde_json::json!({"files":[{
                    "path":dir.path().join("guarded"),"zone":"guarded","attribution":"declared"
                }]}),
            );
            peer.lock().await.add_message(source);
            let provenance = neutral(&app, &caller).await.unwrap();
            assert_eq!(provenance.find("peer-0").unwrap().source_records.len(), 1);
            assert_eq!(CATALOG_BUILDS.with(|count| count.get()), 1);
            for i in 0..3 {
                peer.lock()
                    .await
                    .add_message(crate::call_validation::ChatMessage::new(
                        "user".into(),
                        format!("text {i}"),
                    ));
                assert!(Arc::ptr_eq(
                    &provenance,
                    &neutral(&app, &caller).await.unwrap()
                ));
            }
            app.chat.sessions.write().await.remove("peer-0");
            assert!(neutral(&app, &caller)
                .await
                .unwrap()
                .find("peer-0")
                .is_none());
            assert_eq!(CATALOG_BUILDS.with(|count| count.get()), 2);
            caller.source_root = foreign.path().into();
            let foreign_catalog = neutral(&app, &caller).await.unwrap();
            assert!(foreign_catalog.is_empty());
            assert_ne!(foreign_catalog.project, first.project);
            caller.source_root = dir.path().into();
            let ticket = app
                .work_scheduler
                .reserve(refact_work_core::AdmissionRequest::new(
                    refact_work_core::WorkKey::trajectory("peer-1").run(1),
                    refact_work_core::WorkScope::single(first.project.clone()),
                ))
                .unwrap();
            let permit = ticket.acquire().await.unwrap();
            let _parked = permit.park().unwrap();
            let scheduled = neutral(&app, &caller).await.unwrap();
            assert_eq!(
                scheduled.find("peer-1").unwrap().state,
                super::types::WorkStateTag::Parked
            );
            let peer = app.chat.sessions.read().await["peer-1"].clone();
            peer.lock().await.set_title("Still parked".into(), false);
            let updated = neutral(&app, &caller).await.unwrap();
            assert_eq!(
                updated.find("peer-1").unwrap().state,
                super::types::WorkStateTag::Parked
            );
            assert_eq!(
                updated.find("peer-1").unwrap().metadata.run,
                scheduled.find("peer-1").unwrap().metadata.run
            );
        })
        .await;
}
