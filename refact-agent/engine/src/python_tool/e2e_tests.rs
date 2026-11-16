use crate::tools::tool_python::tests::{call, execute, fixture, with_tools};
use refact_chat_api::{ChatCommand, CommandRequest, PythonApprovalDecisionKind};
use std::time::Duration;

#[tokio::test]
async fn parallel_artifact_deny_approval_session_and_detached_delivery() {
    let (_dir, app, _) = fixture().await;
    let app = with_tools(app);
    let mut registry =
        crate::yaml_configs::customization_registry::get_project_registry(app.gcx.clone())
            .await
            .unwrap();
    registry.modes.get_mut("agent").unwrap().tool_confirm.rules = vec![
        crate::yaml_configs::customization_types::ToolConfirmRule {
            match_pattern: "fixture".into(),
            action: "ask".into(),
        },
        crate::yaml_configs::customization_types::ToolConfirmRule {
            match_pattern: "serial_fixture".into(),
            action: "deny".into(),
        },
    ];
    let root = crate::files_correction::get_project_dirs(app.gcx.clone())
        .await
        .remove(0);
    app.gcx
        .project_registry_cache
        .write()
        .unwrap()
        .insert(root, registry);
    app.gcx.tool_catalog_generations.advance_customization();
    let script = "import asyncio\nimport json\nfrom pathlib import Path\ntry:\n    await serial_fixture()\nexcept PermissionError:\n    denied = True\nresults = await asyncio.gather(fixture(), fixture(), fixture())\nPath(ARTIFACTS_DIR + '/out.json').write_text(json.dumps(results))\nanswer = 42\nanswer";
    let task = {
        let app = app.clone();
        tokio::spawn(async move { execute(app, &[call("e2e", script, false)]).await })
    };
    tokio::time::timeout(Duration::from_secs(20), async {
        while !task.is_finished() {
            let session = app.chat.sessions.read().await["python-test"].clone();
            let requests = session.lock().await.runtime.python_approvals.clone();
            for request in requests {
                let command = CommandRequest {
                    native_approval_guard: None,
                    supervision_guard: None,
                    client_request_id: uuid::Uuid::new_v4().to_string(),
                    priority: false,
                    command: ChatCommand::PythonApprovalDecision {
                        request_id: request.request_id,
                        decision: PythonApprovalDecisionKind::AllowOnce,
                    },
                };
                crate::chat::handle_v1_chat_command(
                    axum::extract::State(app.clone()),
                    axum::extract::Path("python-test".into()),
                    serde_json::to_vec(&command).unwrap().into(),
                )
                .await
                .unwrap();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let messages = task.await.unwrap();
    assert_eq!(messages[0].tool_failed, Some(false), "{:?}", messages[0]);
    let workspace = super::workspace::PythonWorkspace::for_chat(&app, "python-test")
        .await
        .unwrap();
    assert!(workspace.artifacts_dir().join("out.json").exists());
    let messages = execute(app.clone(), &[call("reuse", "answer", false)]).await;
    assert!(messages[0].content.content_text_only().contains("42"));
    app.runtime
        .tool_jobs
        .set_test_inline_budget(Some(Duration::from_millis(10)));
    let messages = execute(
        app.clone(),
        &[call(
            "detach",
            "import time\ntime.sleep(0.1)\nanswer",
            false,
        )],
    )
    .await;
    let job = messages[0].extra["tool_job"]["job_id"].as_str().unwrap();
    let terminal = tokio::time::timeout(
        Duration::from_secs(20),
        app.runtime.tool_jobs.wait_terminal("python-test", job),
    )
    .await
    .expect("background e2e exceeded 20 seconds")
    .unwrap();
    assert_eq!(
        terminal.result.unwrap().messages[0].tool_failed,
        Some(false)
    );
}
