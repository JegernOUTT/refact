use std::sync::Arc;
use tokio::sync::Mutex;
use super::catalog_cache::neutral;
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
