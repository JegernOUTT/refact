use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use refact_chat_api::PythonApprovalDecisionKind as Decision;
use serde_json::{json, Value};

use crate::app_state::AppState;
use crate::call_validation::{ChatMessage, ContextEnum};
use crate::tools::tools_description::Tool;

const BUDGET: Duration = Duration::from_secs(60);

pub(super) async fn fixture() -> (
    tempfile::TempDir,
    AppState,
    Arc<tokio::sync::Mutex<crate::at_commands::at_commands::AtCommandsContext>>,
) {
    crate::tools::tool_python::tests::fixture().await
}

pub(super) fn coordination(dir: &tempfile::TempDir, yaml: &str) {
    std::fs::write(dir.path().join("project/.refact/coordination.yaml"), yaml).unwrap();
}

pub(super) async fn run(app: &AppState, code: &str) -> ChatMessage {
    let ccx = Arc::new(tokio::sync::Mutex::new(
        crate::at_commands::at_commands::AtCommandsContext::new_from_app(
            app.clone(),
            16384,
            20,
            false,
            vec![],
            "python-test".into(),
            None,
            "model".into(),
            None,
            None,
        )
        .await,
    ));
    run_ccx(ccx, code).await
}

pub(super) async fn run_ccx(
    ccx: Arc<tokio::sync::Mutex<crate::at_commands::at_commands::AtCommandsContext>>,
    code: &str,
) -> ChatMessage {
    let (_, messages) = tokio::time::timeout(
        BUDGET,
        crate::tools::tool_python::ToolPython::default().tool_execute(
            ccx,
            &uuid::Uuid::new_v4().to_string(),
            &HashMap::from([("code".into(), json!(code))]),
        ),
    )
    .await
    .expect("python run exceeded its budget")
    .unwrap();
    match messages.into_iter().next().unwrap() {
        ContextEnum::ChatMessage(m) => m,
        _ => panic!("expected a message"),
    }
}

pub(super) fn result(m: &ChatMessage) -> Value {
    let text = std::fs::read_to_string(
        std::path::Path::new(m.extra["python"]["run_dir"].as_str().unwrap()).join("result.json"),
    )
    .unwrap();
    serde_json::from_str(&text).unwrap()
}

pub(super) async fn session(
    app: &AppState,
) -> Arc<tokio::sync::Mutex<crate::chat::types::ChatSession>> {
    app.chat.sessions.read().await["python-test"].clone()
}

/// Spawns a run and returns its handle; the caller decides approvals.
pub(super) fn spawn(app: &AppState, code: &str) -> tokio::task::JoinHandle<ChatMessage> {
    let app = app.clone();
    let code = code.to_string();
    tokio::spawn(async move { run(&app, &code).await })
}

pub(super) async fn next_request(
    app: &AppState,
    task: &tokio::task::JoinHandle<ChatMessage>,
) -> refact_chat_api::PythonApprovalRequest {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Some(request) = session(app)
                .await
                .lock()
                .await
                .runtime
                .python_approvals
                .first()
            {
                break request.clone();
            }
            assert!(!task.is_finished(), "run ended without asking");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("no approval request")
}

pub(super) async fn decide(app: &AppState, id: &str, decision: Decision) -> Result<(), String> {
    app.python_tool
        .approvals
        .decide(app, "python-test", id, decision)
        .await
}

pub(super) async fn finish(task: tokio::task::JoinHandle<ChatMessage>) -> ChatMessage {
    tokio::time::timeout(BUDGET, task)
        .await
        .expect("run did not finish")
        .unwrap()
}

mod agents_fns;
mod chat;
mod chat_changes;
mod chats;
mod chats_changes;
mod chats_risky;
mod config_changes;
mod config_fns;
mod config_risky;
mod group_b;
mod group_b_agents;
mod misc_fns;
mod models_providers;
mod pipeline;
mod project_fns;
mod safety;
mod settings_fns;
mod system_fns;
