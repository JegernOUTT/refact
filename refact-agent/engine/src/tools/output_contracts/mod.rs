#![allow(dead_code)]

use crate::app_state::AppState;
use crate::call_validation::{ChatMessage, ChatToolCall, ChatToolFunction};
use crate::chat::tools::{execute_tools, ExecuteToolsOptions};
use crate::chat::types::ThreadParams;
use refact_tool_api::tool_output::TOOL_OUTPUT_EXTRA_KEY;
use refact_tool_api::ToolDesc;

mod agents_mcp_tests;
mod codegraph_tests;
mod edit_tests;
mod exec_tests;
mod read_tests;

pub(crate) async fn run_tool_for_contract(
    app: AppState,
    tool_name: &str,
    args_json: serde_json::Value,
) -> Vec<ChatMessage> {
    let dir = tempfile::tempdir_in(dunce::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    *app.gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
    let call = ChatToolCall {
        id: uuid::Uuid::new_v4().to_string(),
        function: ChatToolFunction {
            name: tool_name.into(),
            arguments: args_json.to_string(),
        },
        index: None,
        tool_type: "function".into(),
        extra_content: None,
        started_at_ms: None,
        completed_at_ms: None,
    };
    execute_tools(
        app,
        &[call],
        &[],
        &ThreadParams {
            id: format!("contract-{}", uuid::Uuid::new_v4()),
            model: "model".into(),
            ..Default::default()
        },
        "agent",
        None,
        ExecuteToolsOptions {
            structured_output: true,
            wait_until_terminal: true,
            ..Default::default()
        },
    )
    .await
    .0
}

pub(crate) fn assert_output_matches_schema(desc: &ToolDesc, messages: &[ChatMessage]) {
    let outputs: Vec<_> = messages
        .iter()
        .filter_map(|m| m.extra.get(TOOL_OUTPUT_EXTRA_KEY))
        .collect();
    match &desc.output_schema {
        Some(schema) => {
            assert!(
                !outputs.is_empty(),
                "{} declares output_schema but returned no tool_output",
                desc.name
            );
            for output in outputs {
                refact_tool_api::schema_validate::validate(schema, output)
                    .unwrap_or_else(|errors| panic!("{} output contract: {errors:?}", desc.name));
            }
        }
        None => assert!(
            outputs.is_empty(),
            "{} returned tool_output without output_schema",
            desc.name
        ),
    }
}

pub(crate) fn tool_text(messages: &[ChatMessage]) -> String {
    messages
        .iter()
        .map(|m| m.content.content_text_only())
        .collect::<Vec<_>>()
        .join("")
}

#[test]
fn output_contracts_harness_smoke() {
    let message = ChatMessage::new("tool".into(), "unchanged\n".into());
    assert_eq!(tool_text(&[message]), "unchanged\n");
}

#[test]
fn output_contracts_validates_payload() {
    let mut desc = ToolDesc {
        name: "fixture".into(),
        experimental: false,
        allow_parallel: true,
        description: String::new(),
        input_schema: serde_json::json!({}),
        output_schema: Some(
            serde_json::json!({"type":"object","required":["ok"],"properties":{"ok":{"type":"boolean"}}}),
        ),
        annotations: None,
        display_name: String::new(),
        source: refact_tool_api::ToolSource {
            source_type: refact_tool_api::ToolSourceType::Builtin,
            config_path: String::new(),
        },
    };
    let mut message = ChatMessage::new("tool".into(), "text".into());
    assert!(
        std::panic::catch_unwind(|| assert_output_matches_schema(&desc, &[message.clone()]))
            .is_err()
    );
    message
        .extra
        .insert(TOOL_OUTPUT_EXTRA_KEY.into(), serde_json::json!({"ok":true}));
    assert_output_matches_schema(&desc, &[message.clone()]);
    message.extra.insert(
        TOOL_OUTPUT_EXTRA_KEY.into(),
        serde_json::json!({"ok":"wrong"}),
    );
    assert!(
        std::panic::catch_unwind(|| assert_output_matches_schema(&desc, &[message.clone()]))
            .is_err()
    );
    desc.output_schema = None;
    assert!(
        std::panic::catch_unwind(|| assert_output_matches_schema(&desc, &[message.clone()]))
            .is_err()
    );
    message.extra.clear();
    assert_output_matches_schema(&desc, &[message]);
}
