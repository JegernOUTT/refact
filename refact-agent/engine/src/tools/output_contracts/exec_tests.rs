use super::{assert_output_matches_schema, tool_text};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::ContextEnum;
use crate::tools::tools_description::Tool;
use refact_tool_api::tool_output::TOOL_OUTPUT_EXTRA_KEY;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Mutex;

async fn context() -> Arc<Mutex<AtCommandsContext>> {
    let gcx = crate::global_context::tests::make_test_gcx().await;
    Arc::new(Mutex::new(
        AtCommandsContext::new_with_abort(
            crate::app_state::AppState::from_gcx(gcx).await,
            4096,
            20,
            false,
            vec![],
            "exec-contract".into(),
            None,
            "model".into(),
            None,
            None,
            None,
        )
        .await,
    ))
}

async fn call(
    tool: &mut dyn Tool,
    ccx: Arc<Mutex<AtCommandsContext>>,
    args: Value,
) -> Vec<crate::call_validation::ChatMessage> {
    let args = serde_json::from_value(args).unwrap();
    let (_, result) = tool
        .tool_execute(ccx, &"contract".into(), &args)
        .await
        .unwrap();
    let messages = result
        .into_iter()
        .filter_map(|m| match m {
            ContextEnum::ChatMessage(m) => Some(m),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_output_matches_schema(&tool.tool_description(), &messages);
    assert!(messages[0].extra.contains_key(TOOL_OUTPUT_EXTRA_KEY));
    messages
}

#[cfg(unix)]
#[tokio::test]
async fn shell_output_contract_exit_zero() {
    let mut tool = crate::tools::tool_shell::ToolShell::default();
    let messages = call(
        &mut tool,
        context().await,
        json!({"command":"printf hello", "description":"Print hello", "tty":false}),
    )
    .await;
    let output = &messages[0].extra[TOOL_OUTPUT_EXTRA_KEY];
    assert_eq!(output["status"], "exited");
    assert_eq!(output["exit_code"], 0);
    assert_eq!(output["stdout"], "hello\n");
    assert_eq!(output["stderr"], "");
    assert!(!output["output_withheld"].as_bool().unwrap());
}

#[cfg(unix)]
#[tokio::test]
async fn shell_output_contract_nonzero_exit() {
    let mut tool = crate::tools::tool_shell::ToolShell::default();
    let messages = call(&mut tool, context().await, json!({"command":"printf problem >&2; exit 7", "description":"Report failure", "tty":false})).await;
    assert_eq!(messages[0].extra[TOOL_OUTPUT_EXTRA_KEY]["exit_code"], 7);
    assert_eq!(
        messages[0].extra[TOOL_OUTPUT_EXTRA_KEY]["stderr"],
        "problem\n"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn process_family_contracts() {
    use crate::tools::tool_process::*;
    let ccx = context().await;
    let mut list = ToolProcessList {
        config_path: String::new(),
    };
    let empty = call(&mut list, ccx.clone(), json!({})).await;
    assert_eq!(
        tool_text(&empty),
        "Processes (status: running, scope: chat)\ncount: 0\n"
    );
    let mut start = ToolProcessStart {
        config_path: String::new(),
    };
    let started = call(
        &mut start,
        ccx.clone(),
        json!({"command":"cat", "description":"Echo input", "tty":true}),
    )
    .await;
    let id = started[0].extra[TOOL_OUTPUT_EXTRA_KEY]["process_id"].clone();
    let mut write = crate::tools::tool_process_write_stdin::ToolProcessWriteStdin {
        config_path: String::new(),
    };
    let written = call(
        &mut write,
        ccx.clone(),
        json!({"process_id":id,"chars":"hello\n","yield_time_ms":100}),
    )
    .await;
    assert_eq!(written[0].extra[TOOL_OUTPUT_EXTRA_KEY]["bytes_written"], 6);
    let mut read = ToolProcessRead {
        config_path: String::new(),
    };
    call(&mut read, ccx.clone(), json!({"process_id":id})).await;
    call(&mut list, ccx.clone(), json!({})).await;
    let mut kill = ToolProcessKill {
        config_path: String::new(),
    };
    let killed = call(&mut kill, ccx, json!({"process_id":id})).await;
    assert_eq!(killed[0].extra[TOOL_OUTPUT_EXTRA_KEY]["status"], "killed");
}

#[tokio::test]
async fn exec_cleanup_text_is_unchanged() {
    let mut clean = crate::tools::tool_clean_background_processes::ToolCleanBackgroundProcesses {
        config_path: String::new(),
    };
    let result = call(&mut clean, context().await, json!({})).await;
    assert_eq!(tool_text(&result), "{\"killed_count\":0,\"killed\":[]}");
}

#[cfg(unix)]
#[tokio::test]
async fn shell_output_contract_filters_both_streams() {
    let mut tool = crate::tools::tool_shell::ToolShell::default();
    let messages = call(
        &mut tool,
        context().await,
        json!({
            "command":"printf 'keep-out\nhidden-out\n'; printf 'keep-err\nhidden-err\n' >&2",
            "description":"Print filtered streams", "tty":false,
            "output_filter":"keep", "output_limit":"1"
        }),
    )
    .await;
    let output = &messages[0].extra[TOOL_OUTPUT_EXTRA_KEY];
    assert!(output["stdout"].as_str().unwrap().contains("keep-out"));
    assert!(output["stderr"].as_str().unwrap().contains("keep-err"));
    assert!(!output.to_string().contains("hidden-"));
}

#[tokio::test]
async fn process_read_output_contract_matches_stream_filter_and_disk_source() {
    use crate::exec::{ExecMode, ExecOutputStream, ExecProcessMeta};
    use crate::tools::tool_process::ToolProcessRead;
    let ccx = context().await;
    let gcx = ccx.lock().await.app.gcx.clone();
    let snapshot = gcx
        .exec_registry
        .register(
            ExecProcessMeta::new(ExecMode::Background, "test".into()).with_chat_id("exec-contract"),
            1024 * 1024,
        )
        .await;
    let id = snapshot.meta.process_id;
    let temp = tempfile::tempdir().unwrap();
    gcx.exec_registry
        .set_spill_target_for_test(
            &id,
            crate::exec::spill::SpillTarget::with_root(
                temp.path().to_path_buf(),
                "exec-contract",
                &id,
            ),
        )
        .await
        .unwrap();
    gcx.exec_registry.mark_started(&id).await.unwrap();
    for (stream, text) in [
        (ExecOutputStream::Stdout, "keep-out\nhidden-out\n"),
        (ExecOutputStream::Stderr, "keep-err\nhidden-err\n"),
    ] {
        gcx.exec_registry
            .append_output(&id, stream, text.into())
            .await
            .unwrap();
    }
    let mut read = ToolProcessRead {
        config_path: String::new(),
    };
    for stream in ["stdout", "stderr", "combined", "all"] {
        let messages = call(&mut read, ccx.clone(), json!({
            "process_id":id.as_str(), "stream":stream, "output_filter":"keep", "output_limit":"1"
        })).await;
        let output = &messages[0].extra[TOOL_OUTPUT_EXTRA_KEY];
        assert!(!output.to_string().contains("hidden-"), "{output}");
        if stream == "stdout" {
            assert!(!output.to_string().contains("keep-err"));
        }
        if stream == "stderr" {
            assert!(!output.to_string().contains("keep-out"));
        }
        for chunk in output["chunks"].as_array().unwrap() {
            assert!(tool_text(&messages).contains(chunk["text"].as_str().unwrap()));
        }
    }
    gcx.exec_registry
        .append_output(&id, ExecOutputStream::Stdout, "x".repeat(300 * 1024))
        .await
        .unwrap();
    let path = gcx.exec_registry.disk_log_path(&id).await.unwrap();
    tokio::fs::write(path, "keep-disk\nhidden-disk\n")
        .await
        .unwrap();
    let messages = call(
        &mut read,
        ccx,
        json!({
            "process_id":id.as_str(), "from_disk":true, "output_filter":"keep", "output_limit":"1"
        }),
    )
    .await;
    let output = &messages[0].extra[TOOL_OUTPUT_EXTRA_KEY];
    assert_eq!(output["chunks"].as_array().unwrap().len(), 1);
    assert!(output["chunks"][0]["text"]
        .as_str()
        .unwrap()
        .contains("keep-disk"));
    assert!(!output.to_string().contains("hidden-disk"));
    assert!(!output.to_string().contains("keep-out"));
}
