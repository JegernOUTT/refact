use super::*;
use serde_json::json;

#[tokio::test]
async fn shutdown_refuses_new_runtime_admission() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx().await;
    let host = gcx
        .apps
        .get(first.path().into(), gcx.clone())
        .await
        .unwrap();
    gcx.shutdown_flag
        .store(true, std::sync::atomic::Ordering::SeqCst);
    gcx.apps.shutdown().await;
    let result = gcx.apps.get(second.path().into(), gcx.clone()).await;
    let refused = matches!(&result, Err(error) if error.contains("shut"));
    gcx.apps.shutdown().await;
    drop(host);
    assert!(refused, "registry admitted a runtime after shutdown");
}

#[tokio::test]
async fn published_document_and_view_state_use_immutable_bundle_and_exact_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx().await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().into()];
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    let created = execute(
        &app,
        Caller::Host,
        Command::Create {
            slug: "demo".into(),
            name: "Demo".into(),
        },
    )
    .await
    .unwrap();
    let id = serde_json::from_value(created["manifest"]["app_id"].clone()).unwrap();
    execute(
        &app,
        Caller::Host,
        Command::Publish {
            app_id: id,
            expected_revision: created["revision"].as_str().unwrap().into(),
        },
    )
    .await
    .unwrap();
    execute(
        &app,
        Caller::Host,
        Command::WriteFile {
            app_id: id,
            path: "ui/index.html".into(),
            content: "draft only".into(),
            expected_revision: created["revision"].as_str().unwrap().into(),
        },
    )
    .await
    .unwrap();
    let doc = document(&app, id, None, None).await.unwrap();
    assert!(!doc["html"].as_str().unwrap().contains("draft only"));
    let view = execute(
        &app,
        Caller::Host,
        Command::ViewRegister {
            app_id: id,
            chat_id: None,
            binding_id: None,
        },
    )
    .await
    .unwrap();
    let view_id = serde_json::from_value(view["view_id"].clone()).unwrap();
    let command = |token: String| Command::StateSet {
        app_id: id,
        scope: refact_apps_core::StateScope::Standalone,
        chat_id: None,
        view_id: Some(view_id),
        view_token: Some(token),
        expected_revision: 0,
        value: json!({"ok":true}),
    };
    assert!(execute(&app, Caller::Host, command("wrong".into()))
        .await
        .is_err());
    let state = execute(
        &app,
        Caller::Host,
        command(view["view_token"].as_str().unwrap().into()),
    )
    .await
    .unwrap();
    assert_eq!(state["revision"], 1);
    assert!(execute(
        &app,
        Caller::Host,
        command(view["view_token"].as_str().unwrap().into())
    )
    .await
    .unwrap_err()
    .contains("conflict"));
    app.gcx.apps.shutdown().await;
}
