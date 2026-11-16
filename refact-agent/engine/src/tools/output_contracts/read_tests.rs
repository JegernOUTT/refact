use super::{assert_output_matches_schema, tool_text};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatMessage, ContextEnum};
use crate::tools::tools_description::Tool;
use refact_tool_api::tool_output::TOOL_OUTPUT_EXTRA_KEY;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

async fn fixture() -> (tempfile::TempDir, Arc<Mutex<AtCommandsContext>>) {
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

async fn run(
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
    let message = messages.iter().find(|m| m.role == "tool").unwrap();
    (message.extra[TOOL_OUTPUT_EXTRA_KEY].clone(), messages)
}

fn builder_contract(tool: &dyn Tool, output: impl serde::Serialize, text: &str) -> Value {
    let output = serde_json::to_value(output).unwrap();
    let mut message = ChatMessage::new("tool".into(), text.into());
    message
        .extra
        .insert(TOOL_OUTPUT_EXTRA_KEY.into(), output.clone());
    assert_output_matches_schema(&tool.tool_description(), &[message.clone()]);
    assert_eq!(tool_text(&[message]), text);
    output
}

#[tokio::test]
async fn cat_output_contract() {
    let (dir, ccx) = fixture().await;
    let path = dunce::canonicalize(dir.path())
        .unwrap()
        .join("fixture.rs")
        .to_string_lossy()
        .into_owned();
    let mut tool = crate::tools::tool_cat::ToolCat {
        config_path: String::new(),
    };
    let (out, messages) = run(&mut tool, ccx, json!({"paths":format!("{path}:2-2")})).await;
    assert_eq!(out["files"][0]["line1"], 2);
    assert_eq!(out["files"][0]["line2"], 2);
    assert_eq!(out["files"][0]["total_lines"], 3);
    assert_eq!(out["files"][0]["content"], "needle here\n");
    let display_path = refact_core::chat_types::normalize_file_name(path);
    assert_eq!(
        tool_text(&messages),
        format!("Paths found:\n{display_path}\n\n")
    );
}

#[tokio::test]
async fn glob_output_contract() {
    let (dir, ccx) = fixture().await;
    let mut tool = crate::tools::tool_glob::ToolGlob {
        config_path: String::new(),
    };
    let (out, messages) = run(
        &mut tool,
        ccx,
        json!({"pattern":"*.rs", "path":"workspace"}),
    )
    .await;
    assert_eq!(out["paths"].as_array().unwrap().len(), 1);
    let path = refact_core::chat_types::normalize_file_name(
        dunce::canonicalize(dir.path())
            .unwrap()
            .join("fixture.rs")
            .to_string_lossy()
            .into_owned(),
    );
    assert_eq!(out["paths"][0], path);
    assert_eq!(out["truncated"], false);
    assert_eq!(
        tool_text(&messages),
        format!("Glob matches for pattern '*.rs':\n  {}\n", path)
    );
}

#[tokio::test]
async fn tree_output_contract() {
    let (dir, ccx) = fixture().await;
    let root = dunce::canonicalize(dir.path())
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let mut tool = crate::tools::tool_tree::ToolTree {
        config_path: String::new(),
    };
    let (out, messages) = run(&mut tool, ccx.clone(), json!({"path":root})).await;
    let entry = out["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["path"].as_str().unwrap().ends_with("fixture.rs"))
        .unwrap();
    assert_eq!(entry["kind"], "file");
    assert_eq!(entry["size_bytes"], 24);
    assert_eq!(entry["lines"], 3);
    let (_, second) = run(&mut tool, ccx, json!({"path":root})).await;
    assert_eq!(tool_text(&messages), tool_text(&second));
    assert!(tool_text(&messages).contains("fixture.rs [24B] 3L"));
}

#[tokio::test]
async fn regex_search_output_contract() {
    let (_dir, ccx) = fixture().await;
    let mut tool = crate::tools::tool_regex_search::ToolRegexSearch {
        config_path: String::new(),
    };
    let args = json!({"pattern":"needle", "scope":"workspace", "context_lines":0});
    let (out, messages) = run(&mut tool, ccx.clone(), args.clone()).await;
    assert_eq!(out["matches"][0]["line"], 2);
    assert_eq!(out["matches"][0]["text"], "needle here");
    assert_eq!(out["files_matched"], 1);
    assert_eq!(out["total_matches"], 1);
    assert_eq!(out["truncated"], false);
    let (_, second) = run(&mut tool, ccx, args).await;
    assert_eq!(tool_text(&messages), tool_text(&second));
    assert!(tool_text(&messages).contains(">     2 | needle here"));
}

#[tokio::test]
async fn symbol_definition_output_contract() {
    let (dir, ccx) = fixture().await;
    let gcx = ccx.lock().await.app.gcx.clone();
    let service = Arc::new(refact_codegraph::CodeGraphService::open_in_memory().unwrap());
    let path = dunce::canonicalize(dir.path())
        .unwrap()
        .join("fixture.rs")
        .to_string_lossy()
        .into_owned();
    service
        .index_file(&path, "pub fn needle() {}\n", "rust")
        .await
        .unwrap();
    service.connect_usages().await.unwrap();
    *gcx.codegraph.lock().await = Some(service);
    let mut tool = crate::tools::tool_ast_definition::ToolAstDefinition {
        config_path: String::new(),
    };
    let (out, messages) = run(&mut tool, ccx.clone(), json!({"symbols":"needle,absent"})).await;
    assert_eq!(out["definitions"][0]["symbol"], "needle");
    assert_eq!(out["definitions"][0]["path"], path);
    assert_eq!(out["definitions"][0]["line1"], 1);
    assert_eq!(out["not_found"], json!(["absent"]));
    let (_, second) = run(&mut tool, ccx, json!({"symbols":"needle,absent"})).await;
    assert_eq!(tool_text(&messages), tool_text(&second));
}

#[test]
fn knowledge_output_contract() {
    use crate::tools::tool_knowledge::{knowledge_output, ToolGetKnowledge};
    let memory = crate::memories::MemoRecord {
        memid: "fixture".into(),
        tags: vec![],
        content: "known content".into(),
        file_path: Some("knowledge/fixture.md".into()),
        line_range: None,
        title: Some("Fixture".into()),
        created: None,
        kind: None,
        score: Some(0.5),
    };
    let out = builder_contract(
        &ToolGetKnowledge {
            config_path: String::new(),
        },
        knowledge_output(&[memory]),
        "unchanged knowledge\n",
    );
    assert_eq!(out["entries"][0]["content"], "known content");
    assert_eq!(out["entries"][0]["score"], 0.5);
    assert_eq!(out["entries"][0]["path"], "knowledge/fixture.md");
    assert_eq!(out["entries"][0]["title"], "Fixture");
}

#[test]
fn web_output_contract() {
    // Fetching always contacts public Jina first; exercise the payload builder without real network.
    use crate::tools::tool_web::{web_output, ToolWeb};
    let out = builder_contract(
        &ToolWeb {
            config_path: String::new(),
        },
        web_output("http://127.0.0.1/fixture", "local page\n"),
        "local page\n",
    );
    assert_eq!(
        out,
        json!({"url":"http://127.0.0.1/fixture", "content":"local page\n", "truncated":false})
    );
}
