#![cfg_attr(not(unix), allow(unused_imports))]

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatMessage, ContextEnum};
use crate::tools::tool_python::tests::{call, execute, fixture, with_tools};
use crate::tools::tools_description::Tool;

#[cfg(unix)]
async fn run_code(ccx: Arc<AMutex<AtCommandsContext>>, code: &str) -> ChatMessage {
    let (_, messages) = crate::tools::tool_python::ToolPython::default()
        .tool_execute(
            ccx,
            &uuid::Uuid::new_v4().to_string(),
            &HashMap::from([("code".to_string(), json!(code))]),
        )
        .await
        .unwrap();
    match messages.into_iter().next().unwrap() {
        ContextEnum::ChatMessage(message) => message,
        _ => panic!("expected a message"),
    }
}

#[cfg(unix)]
const RUN_SCRIPT: &str = r#"r = await run("printf 'a\\nerror: boom\\n'; exit 3", tail=5)
t = await run("sleep 5", timeout=1)
big = await run("head -c 3000000 /dev/zero | tr '\\000' x; echo; echo done")
log = open(r["log"]).read()
big_log = open(big["log"]).read()
str([
    r["ok"] is False,
    r["exit_code"] == 3,
    r["status"] == "exited",
    r["problems"] == ["error: boom"],
    r["tail"] == "a\nerror: boom",
    log == "a\nerror: boom\n",
    r["truncated"] is False,
    t["status"] == "timed_out" and t["ok"] is False,
    big["ok"] is True and big["exit_code"] == 0,
    big["tail"].endswith("done"),
    big["truncated"] is False and len(big_log) == 3000006 and big_log.startswith("xxx"),
    "run" in tools(),
])"#;

#[cfg(unix)]
#[tokio::test]
async fn run_waits_for_the_real_shell_and_returns_exit_tail_problems_and_log() {
    let (_dir, _app, ccx) = fixture().await;
    let message = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        run_code(ccx, RUN_SCRIPT),
    )
    .await
    .unwrap();
    let text = message.content.content_text_only();
    assert_eq!(message.tool_failed, Some(false), "{text}");
    assert!(
        text.contains("[True, True, True, True, True, True, True, True, True, True, True, True]"),
        "{text}"
    );
    assert!(
        message.extra["python"].get("read_only").is_none(),
        "{:?}",
        message.extra["python"]
    );
    let calls = message.extra["python"]["calls"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(calls.len(), 3, "{calls:?}");
    assert!(
        calls.iter().all(|call| call["tool"] == "shell"),
        "{calls:?}"
    );
    let summary = calls[0]["summary"].as_str().unwrap();
    assert!(
        summary.starts_with("exit 3 · ") && summary.ends_with(" · 1 problem"),
        "{summary}"
    );
    assert!(
        calls[1]["summary"]
            .as_str()
            .unwrap()
            .starts_with("timed out · "),
        "{calls:?}"
    );
    let run_dir = std::path::PathBuf::from(message.extra["python"]["run_dir"].as_str().unwrap());
    let artifact: Value =
        serde_json::from_slice(&std::fs::read(run_dir.join("calls/1_shell.json")).unwrap())
            .unwrap();
    assert_eq!(artifact["run_kwargs"]["tail"], 5);
    assert_eq!(artifact["kwargs"]["timeout"], Value::Null);
    assert!(
        std::fs::metadata(run_dir.join("calls/3_shell.json"))
            .unwrap()
            .len()
            < 64 * 1024
    );
}

#[tokio::test]
async fn run_is_not_bound_in_a_chat_without_the_shell_tool() {
    let (_dir, app, _) = fixture().await;
    let app = with_tools(app);
    let messages = execute(
        app,
        &[call(
            "no-shell",
            "bound = 'run' in tools()\ntry:\n    run\n    error = None\nexcept NameError:\n    error = 'NameError'\nstr((bound, error))",
            false,
        )],
    )
    .await;
    let text = messages[0].content.content_text_only();
    assert!(text.contains("(False, 'NameError')"), "{text}");
}

#[tokio::test]
async fn script_writes_follow_the_mode_the_allowed_tools_and_the_child_ceiling() {
    let (_dir, app, ccx) = fixture().await;
    for (mode, writable) in [
        ("explore", false),
        ("ask", false),
        ("plan", false),
        ("agent", true),
        ("debug", true),
        ("buddy", true),
        ("shell", true),
    ] {
        let catalog = app
            .tool_registry
            .acquire_tool_catalog(mode, Some("model"), None)
            .await;
        assert!(
            catalog.index.tools.iter().any(|tool| tool.name == "python"),
            "{mode}"
        );
        assert_eq!(
            super::script_writes(&app, &ccx, &catalog, &[], mode, "model").await,
            writable,
            "{mode}"
        );
    }
    let agent = app
        .tool_registry
        .acquire_tool_catalog("agent", Some("model"), None)
        .await;
    let read_tools = vec!["cat".to_string(), "tree".into(), "python".into()];
    assert!(!super::script_writes(&app, &ccx, &agent, &read_tools, "agent", "model").await);
    ccx.lock().await.child_tool_ceiling =
        Some(crate::subchat::ToolsPolicy::Only(read_tools.clone()));
    assert!(!super::script_writes(&app, &ccx, &agent, &[], "agent", "model").await);
    ccx.lock().await.child_tool_ceiling = Some(crate::subchat::ToolsPolicy::Only(vec![
        "cat".into(),
        "python".into(),
        "shell".into(),
    ]));
    assert!(super::script_writes(&app, &ccx, &agent, &[], "agent", "model").await);
    ccx.lock().await.child_tool_ceiling = None;
    let mut explore = (*app
        .tool_registry
        .acquire_tool_catalog("explore", Some("model"), None)
        .await)
        .clone();
    let mut mcp_writer = explore.index.tools[0].clone();
    mcp_writer.name = "shell".into();
    mcp_writer.source.source_type = refact_tool_api::ToolSourceType::Integration;
    explore.index.tools.push(mcp_writer);
    assert!(!super::script_writes(&app, &ccx, &explore, &[], "explore", "model").await);
}

#[tokio::test]
async fn a_read_only_chat_refuses_script_writes_outside_artifacts_with_the_read_only_text() {
    let (dir, app, _) = fixture().await;
    let app = with_tools(app);
    let messages = execute(
        app,
        &[call(
            "read-only-write",
            "from pathlib import Path\nPath(ARTIFACTS_DIR + '/ok.txt').write_text('ok')\ntry:\n    Path(PROJECT_ROOT + '/x.txt').write_text('x')\n    r = 'written'\nexcept PermissionError as e:\n    r = str(e)\nr",
            false,
        )],
    )
    .await;
    let text = messages[0].content.content_text_only();
    assert!(
        text.contains("this chat is read-only: scripts may write files only under ARTIFACTS_DIR"),
        "{text}"
    );
    assert!(!dir.path().join("project/x.txt").exists());
    assert_eq!(messages[0].extra["python"]["read_only"], true);
}
