use super::{assert_output_matches_schema, tool_text};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatMessage, ContextEnum};
use crate::tools::tools_description::Tool;
use refact_tool_api::tool_output::TOOL_OUTPUT_EXTRA_KEY;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

pub(super) async fn fixture() -> (tempfile::TempDir, Arc<Mutex<AtCommandsContext>>) {
    let dir = tempfile::Builder::new()
        .prefix("refact-read-contract-")
        .tempdir()
        .unwrap();
    let root = dunce::canonicalize(dir.path()).unwrap();
    std::fs::write(root.join("fixture.rs"), "alpha\nneedle here\nomega\n").unwrap();
    std::fs::write(root.join("other.txt"), "other\n").unwrap();
    let gcx = crate::global_context::tests::make_test_gcx().await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.clone()];
    *gcx.documents_state.workspace_files.lock().unwrap() =
        vec![root.join("fixture.rs"), root.join("other.txt")];
    *gcx.privacy_settings.write().unwrap() = Arc::new(crate::privacy::PrivacySettings {
        privacy_rules: crate::privacy::FilePrivacySettings {
            only_send_to_servers_I_control: vec![],
            blocked: vec![],
        },
        loaded_ts: u64::MAX / 2,
    });
    let ccx = AtCommandsContext::new_from_app(
        crate::app_state::AppState::from_gcx(gcx).await,
        4096,
        20,
        false,
        vec![],
        "read-contract".into(),
        None,
        "model".into(),
        None,
        None,
    )
    .await;
    (dir, Arc::new(Mutex::new(ccx)))
}

pub(super) async fn run(
    tool: &mut dyn Tool,
    ccx: Arc<Mutex<AtCommandsContext>>,
    args: Value,
) -> (Value, Vec<ChatMessage>) {
    let args: HashMap<String, Value> = serde_json::from_value(args).unwrap();
    let (_, results) = tool
        .tool_execute(ccx, &"read-call".into(), &args)
        .await
        .unwrap();
    let messages = results
        .into_iter()
        .filter_map(|r| match r {
            ContextEnum::ChatMessage(m) => Some(m),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_output_matches_schema(&tool.tool_description(), &messages);
    let message = messages
        .iter()
        .find(|m| m.extra.contains_key(TOOL_OUTPUT_EXTRA_KEY))
        .unwrap();
    (message.extra[TOOL_OUTPUT_EXTRA_KEY].clone(), messages)
}

fn counts(output: &Value, added: usize, removed: usize) {
    assert_eq!(output["files"][0]["lines_added"], added);
    assert_eq!(output["files"][0]["lines_removed"], removed);
}

fn assert_diff(messages: &[ChatMessage], added: &str, removed: &str) {
    let diff = messages.iter().find(|m| m.role == "diff").unwrap();
    let chunks: Vec<crate::call_validation::DiffChunk> =
        serde_json::from_str(&diff.content.content_text_only()).unwrap();
    assert_eq!(
        chunks
            .iter()
            .map(|c| c.lines_add.as_str())
            .collect::<String>(),
        added
    );
    assert_eq!(
        chunks
            .iter()
            .map(|c| c.lines_remove.as_str())
            .collect::<String>(),
        removed
    );
    assert_eq!(
        diff.content.content_text_only(),
        serde_json::to_string(&chunks).unwrap()
    );
    let without_payload: Vec<_> = messages
        .iter()
        .cloned()
        .map(|mut m| {
            m.extra.remove(TOOL_OUTPUT_EXTRA_KEY);
            m
        })
        .collect();
    assert_eq!(tool_text(messages), tool_text(&without_payload));
}

#[tokio::test]
async fn update_textdoc_output_contract() {
    let (dir, ccx) = fixture().await;
    let path = dunce::canonicalize(dir.path()).unwrap().join("fixture.rs");
    let mut args = json!({"old_str":"needle here", "replacement":"replacement"});
    args["path"] = json!(path);
    let mut tool = crate::tools::file_edit::tool_update_textdoc::ToolUpdateTextDoc {
        config_path: String::new(),
    };
    let (output, messages) = run(&mut tool, ccx, args).await;
    counts(&output, 1, 1);
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "alpha\nreplacement\nomega\n"
    );
    assert_diff(&messages, "replacement\n", "needle here\n");
}

#[tokio::test]
async fn update_textdoc_regex_output_contract() {
    let (dir, ccx) = fixture().await;
    let path = dunce::canonicalize(dir.path()).unwrap().join("fixture.rs");
    let mut args = json!({"pattern":"needle here", "replacement":"replacement"});
    args["path"] = json!(path);
    let mut tool = crate::tools::file_edit::tool_update_textdoc_regex::ToolUpdateTextDocRegex {
        config_path: String::new(),
    };
    let (output, messages) = run(&mut tool, ccx, args).await;
    counts(&output, 1, 1);
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "alpha\nreplacement\nomega\n"
    );
    assert_diff(&messages, "replacement\n", "needle here\n");
}

#[tokio::test]
async fn update_textdoc_by_lines_output_contract() {
    let (dir, ccx) = fixture().await;
    let path = dunce::canonicalize(dir.path()).unwrap().join("fixture.rs");
    let mut args = json!({"ranges":"2", "content":"replacement"});
    args["path"] = json!(path);
    let mut tool =
        crate::tools::file_edit::tool_update_textdoc_by_lines::ToolUpdateTextDocByLines {
            config_path: String::new(),
        };
    let (output, messages) = run(&mut tool, ccx, args).await;
    counts(&output, 1, 1);
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "alpha\nreplacement\nomega\n"
    );
    assert_diff(&messages, "replacement\n", "needle here\n");
}

#[tokio::test]
async fn update_textdoc_anchored_output_contract() {
    let (dir, ccx) = fixture().await;
    let path = dunce::canonicalize(dir.path()).unwrap().join("fixture.rs");
    let mut args = json!({"mode":"replace_between", "anchor_before":"alpha\n", "anchor_after":"omega", "content":"replacement\n"});
    args["path"] = json!(path);
    let mut tool =
        crate::tools::file_edit::tool_update_textdoc_anchored::ToolUpdateTextDocAnchored {
            config_path: String::new(),
        };
    let (output, messages) = run(&mut tool, ccx, args).await;
    counts(&output, 1, 1);
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "alpha\nreplacement\nomega\n"
    );
    assert_diff(&messages, "replacement\n", "needle here\n");
}

#[tokio::test]
async fn create_textdoc_output_contract() {
    let (dir, ccx) = fixture().await;
    let path = dunce::canonicalize(dir.path()).unwrap().join("new.txt");
    let mut tool = crate::tools::file_edit::tool_create_textdoc::ToolCreateTextDoc {
        config_path: String::new(),
    };
    let (output, messages) =
        run(&mut tool, ccx, json!({"path":path,"content":"new\ntext\n"})).await;
    counts(&output, 2, 0);
    assert_eq!(output["files"][0]["action"], "created");
    assert_eq!(std::fs::read_to_string(path).unwrap(), "new\ntext\n");
    assert_diff(&messages, "new\ntext\n\n", "");
}

#[tokio::test]
async fn undo_textdoc_output_contract() {
    let (dir, ccx) = fixture().await;
    let path = dunce::canonicalize(dir.path()).unwrap().join("fixture.rs");
    let mut update = crate::tools::file_edit::tool_update_textdoc::ToolUpdateTextDoc {
        config_path: String::new(),
    };
    run(
        &mut update,
        ccx.clone(),
        json!({"path":path,"old_str":"needle here","replacement":"replacement"}),
    )
    .await;
    let mut tool = crate::tools::file_edit::tool_undo_textdoc::ToolUndoTextDoc {
        config_path: String::new(),
    };
    let (output, messages) = run(&mut tool, ccx, json!({"path":path})).await;
    counts(&output, 1, 1);
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "alpha\nneedle here\nomega\n"
    );
    assert_diff(&messages, "needle here\n", "replacement\n");
}

#[tokio::test]
async fn apply_patch_output_contract() {
    let (dir, ccx) = fixture().await;
    let root = dunce::canonicalize(dir.path()).unwrap();
    let patch = format!("*** Begin Patch\n*** Add File: {}/new.txt\n+new\n*** Update File: {}/fixture.rs\n@@\n alpha\n-needle here\n+replacement\n omega\n*** Delete File: {}/other.txt\n*** End Patch", root.display(), root.display(), root.display());
    let mut tool = crate::tools::file_edit::tool_apply_patch::ToolApplyPatch {
        config_path: String::new(),
    };
    let (output, messages) = run(&mut tool, ccx, json!({"patch":patch})).await;
    assert_eq!(output["files"].as_array().unwrap().len(), 3);
    counts(&output, 1, 0);
    assert_eq!(output["files"][1]["lines_added"], 1);
    assert_eq!(output["files"][1]["lines_removed"], 1);
    assert_eq!(output["files"][2]["lines_removed"], 1);
    assert_eq!(
        std::fs::read_to_string(root.join("new.txt")).unwrap(),
        "new\n"
    );
    assert!(!root.join("other.txt").exists());
    assert_diff(&messages, "new\n\nreplacement\n", "needle here\nother\n");
}

#[tokio::test]
async fn rm_output_contract() {
    let (dir, ccx) = fixture().await;
    let path = dunce::canonicalize(dir.path()).unwrap().join("other.txt");
    let mut tool = crate::tools::tool_rm::ToolRm {
        config_path: String::new(),
    };
    let (output, messages) = run(&mut tool, ccx.clone(), json!({"path":path,"dry_run":true})).await;
    assert_eq!(output, json!({"removed":[], "dry_run":true}));
    assert!(tool_text(&messages).contains("[Dry run] Would remove file"));
    assert!(path.exists());
    let (output, messages) = run(&mut tool, ccx, json!({"path":path})).await;
    assert_eq!(output, json!({"removed":[path], "dry_run":false}));
    assert!(!path.exists());
    assert_diff(&messages, "", "other\n");
}

#[tokio::test]
async fn mv_output_contract() {
    let (dir, ccx) = fixture().await;
    let root = dunce::canonicalize(dir.path()).unwrap();
    let source = root.join("other.txt");
    let destination = root.join("fixture.rs");
    let mut tool = crate::tools::tool_mv::ToolMv {
        config_path: String::new(),
    };
    let (output, messages) = run(
        &mut tool,
        ccx,
        json!({"source":source,"destination":destination,"overwrite":true}),
    )
    .await;
    assert_eq!(
        output,
        json!({"source":source,"destination":destination,"overwritten":true})
    );
    assert!(!source.exists());
    assert_eq!(std::fs::read_to_string(destination).unwrap(), "other\n");
    assert_diff(&messages, "other\n", "other\nalpha\nneedle here\nomega\n");
}
