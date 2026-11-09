use std::sync::atomic::Ordering;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::app_state::AppState;
use crate::call_validation::ChatMessage;
use crate::chat::types::ChatSession;

use super::capsule::{prepare_work_context, CapsuleState};

async fn fixture() -> (AppState, tempfile::TempDir) {
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
    for index in 0..5 {
        let id = format!("shared-caller-{index}");
        let mut session = ChatSession::new(id.clone());
        session.thread.model = "trusted/model".into();
        session.thread.mode = "agent".into();
        session.work_project_root = Some(dir.path().into());
        session.set_title(format!("Peer {index}"), false);
        app.chat
            .sessions
            .write()
            .await
            .insert(id, Arc::new(Mutex::new(session)));
    }
    (app, dir)
}

#[serial_test::serial(runtime_settings)]
#[tokio::test]
async fn repeated_messages_across_chats_reuse_catalog_and_file_classification() {
    let (app, _dir) = fixture().await;
    super::catalog_cache::CATALOG_BUILDS
        .scope(std::cell::Cell::new(0), async {
            super::capsule::ROW_DERIVATIONS
                .scope(std::cell::Cell::new(0), async {
                    let mut states = vec![CapsuleState::default(); 5];
                    for (index, state) in states.iter_mut().enumerate() {
                        assert!(prepare_work_context(
                            &app,
                            state,
                            &format!("shared-caller-{index}"),
                            "trusted/model",
                        )
                        .await
                        .is_some());
                    }
                    let builds = super::catalog_cache::CATALOG_BUILDS.with(|count| count.get());
                    assert_eq!(builds, 1);
                    let derivations = super::capsule::ROW_DERIVATIONS.with(|count| count.get());
                    assert_eq!(derivations, 5, "each cold caller must derive rows once");
                    let warm_states = states.clone();
                    let compiles = app
                        .gcx
                        .privacy_classification
                        .compiles
                        .load(Ordering::SeqCst);
                    let classifications = app
                        .gcx
                        .privacy_classification
                        .classifications
                        .load(Ordering::SeqCst);
                    assert!(classifications > 0);
                    for turn in 0..3 {
                        for (index, state) in states.iter_mut().enumerate() {
                            let id = format!("shared-caller-{index}");
                            let session = app.chat.sessions.read().await[&id].clone();
                            session.lock().await.add_message(ChatMessage::new(
                                "user".into(),
                                format!("ordinary message {turn}"),
                            ));
                            assert!(prepare_work_context(&app, state, &id, "trusted/model")
                                .await
                                .is_none());
                        }
                    }
                    assert_eq!(
                        super::catalog_cache::CATALOG_BUILDS.with(|count| count.get()),
                        builds
                    );
                    assert_eq!(
                        super::capsule::ROW_DERIVATIONS.with(|count| count.get()),
                        derivations,
                        "warm ordinary messages must not derive rows, lineage, or overlap"
                    );
                    assert_eq!(
                        states, warm_states,
                        "warm skip must not mutate capsule state"
                    );
                    assert_eq!(
                        app.gcx
                            .privacy_classification
                            .compiles
                            .load(Ordering::SeqCst),
                        compiles
                    );
                    assert_eq!(
                        app.gcx
                            .privacy_classification
                            .classifications
                            .load(Ordering::SeqCst),
                        classifications
                    );
                })
                .await;
        })
        .await;
}
