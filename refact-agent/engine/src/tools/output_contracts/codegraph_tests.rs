use super::{assert_output_matches_schema, tool_text};
use crate::tools::tool_codegraph::*;
use crate::tools::tools_description::Tool;
use refact_tool_api::tool_output::TOOL_OUTPUT_EXTRA_KEY;
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn every_codegraph_tool_declares_and_satisfies_output_schema() {
    let (dir, ccx) = super::edit_tests::fixture().await;
    let root = dunce::canonicalize(dir.path()).unwrap();
    let path = root.join("fixture.rs");
    let text = "pub fn helper() -> usize { 42 }\npub fn entry() -> usize { helper() }\n";
    std::fs::write(&path, text).unwrap();
    git2::Repository::init(&root).unwrap();
    let service = Arc::new(refact_codegraph::CodeGraphService::open_in_memory().unwrap());
    service
        .index_file(&path.to_string_lossy(), text, "rust")
        .await
        .unwrap();
    let gcx = ccx.lock().await.app.gcx.clone();
    *gcx.codegraph.lock().await = Some(service);
    let cases: Vec<(Box<dyn Tool + Send>, serde_json::Value)> = vec![
        (
            Box::new(ToolCodegraphOverview {
                config_path: String::new(),
            }),
            json!({}),
        ),
        (
            Box::new(ToolCodeHealth {
                config_path: String::new(),
            }),
            json!({"file_path":path}),
        ),
        (
            Box::new(ToolGitRisk {
                config_path: String::new(),
            }),
            json!({}),
        ),
        (
            Box::new(ToolCodeWhy {
                config_path: String::new(),
            }),
            json!({"query":"helper"}),
        ),
        (
            Box::new(ToolCodeDuplication {
                config_path: String::new(),
            }),
            json!({}),
        ),
        (
            Box::new(ToolDeadCode {
                config_path: String::new(),
            }),
            json!({}),
        ),
        (
            Box::new(ToolSecurityScan {
                config_path: String::new(),
            }),
            json!({"file_path":path}),
        ),
        (
            Box::new(ToolPrBlast {
                config_path: String::new(),
            }),
            json!({"changed_files":[path]}),
        ),
        (
            Box::new(ToolCodeMap {
                config_path: String::new(),
            }),
            json!({}),
        ),
    ];
    for (mut tool, args) in cases {
        assert!(tool.tool_description().output_schema.is_some());
        let (output, messages) = super::edit_tests::run(tool.as_mut(), ccx.clone(), args).await;
        assert_output_matches_schema(&tool.tool_description(), &messages);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&tool_text(&messages)).unwrap(),
            output
        );
        assert!(messages
            .iter()
            .any(|m| m.role == "tool" && m.extra.contains_key(TOOL_OUTPUT_EXTRA_KEY)));
    }
}
