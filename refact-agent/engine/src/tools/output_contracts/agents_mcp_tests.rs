use super::{assert_output_matches_schema, run_tool_for_contract, tool_text};
use crate::call_validation::{ChatMessage, ContextEnum};
use crate::tools::tool_wait::{attach_wait_output, ToolWait};
use crate::tools::tools_description::Tool;
use refact_tool_api::tool_output::TOOL_OUTPUT_EXTRA_KEY;
use serde_json::json;

#[tokio::test]
async fn wait_and_subagent_contracts() {
    let app =
        crate::app_state::AppState::from_gcx(crate::global_context::tests::make_test_gcx().await)
            .await;
    let messages = run_tool_for_contract(
        app,
        "wait",
        json!({
            "target":{"kind":"duration","duration_ms":100,"description":"contract"}
        }),
    )
    .await;
    let desc = ToolWait {
        config_path: String::new(),
    }
    .tool_description();
    assert_output_matches_schema(&desc, &messages);
    let detail: serde_json::Value = serde_json::from_str(&tool_text(&messages)).unwrap();
    assert_eq!(
        messages[0].extra[TOOL_OUTPUT_EXTRA_KEY],
        json!({
            "kind":"duration", "outcome":"completed", "detail":detail
        })
    );
}

#[test]
fn wait_outputs_preserve_all_target_renderings() {
    let desc = ToolWait {
        config_path: String::new(),
    }
    .tool_description();
    for (kind, body) in [
        (
            "duration",
            json!({"target":"duration","outcome":"interrupted","slept_ms":9}),
        ),
        (
            "agents",
            json!({"target":"agents","outcome":"timed_out","evidence":"still running"}),
        ),
        (
            "task_agents",
            json!({"target":"task_agents","outcome":"parked","card_ids":["a"]}),
        ),
        (
            "task_agents",
            json!({"target":"task_agents","outcome":"no_matching_agents"}),
        ),
    ] {
        let text = body.to_string();
        let mut messages = vec![ContextEnum::ChatMessage(ChatMessage::new(
            "tool".into(),
            text.clone(),
        ))];
        attach_wait_output(kind, &mut messages);
        let ContextEnum::ChatMessage(message) = &messages[0] else {
            panic!()
        };
        assert_eq!(message.content.content_text_only(), text);
        assert_eq!(message.extra[TOOL_OUTPUT_EXTRA_KEY]["detail"], body);
        assert_output_matches_schema(&desc, &[message.clone()]);
    }
    let mut message = ChatMessage::new(
        "tool".into(),
        "outcome: timed_out\ntarget: process\nunchanged\n".into(),
    );
    message.extra.insert(
        "exec".into(),
        json!({"process_id":"p","wait_timed_out":true}),
    );
    let mut messages = vec![ContextEnum::ChatMessage(message)];
    attach_wait_output("process", &mut messages);
    let ContextEnum::ChatMessage(message) = &messages[0] else {
        panic!()
    };
    assert_eq!(
        message.content.content_text_only(),
        "outcome: timed_out\ntarget: process\nunchanged\n"
    );
    assert_eq!(message.extra[TOOL_OUTPUT_EXTRA_KEY]["outcome"], "timed_out");
    assert_eq!(
        message.extra[TOOL_OUTPUT_EXTRA_KEY]["detail"]["process_id"],
        "p"
    );
    assert_output_matches_schema(&desc, &[message.clone()]);
}

#[test]
fn subagent_receipt_output_contract_preserves_text() {
    use crate::tools::tool_subagent::{build_background_start_tool_result, ToolSubagent};
    let (_, completion_rx) = tokio::sync::oneshot::channel();
    let handle = crate::agents::spawn::SpawnHandle {
        agent_id: "agent".into(),
        child_chat_id: "child".into(),
        launch_revision: 1,
        launch_digest: "digest".into(),
        worktree_branch: None,
        auto_merge: None,
        completion_rx,
    };
    let ContextEnum::ChatMessage(message) =
        build_background_start_tool_result(&handle, "task", "parent", "model", None, "", "call")
    else {
        panic!()
    };
    assert_eq!(message.content.content_text_only(), "✓ Accepted background subagent: task\n- agent_id: agent\n- status: queued\n- child_chat_id: child\n- model: model\n\nOpen the child trajectory: [view](refact://chat/child)\n\nThe completion will be pushed back into this chat automatically.");
    assert_eq!(
        message.extra[TOOL_OUTPUT_EXTRA_KEY],
        json!({"agent_id":"agent","child_chat_id":"child","status":"queued","model":"model"})
    );
    assert_output_matches_schema(
        &ToolSubagent {
            config_path: String::new(),
        }
        .tool_description(),
        &[message],
    );
}

#[test]
fn mcp_output_schema_is_passed_through() {
    use crate::integrations::mcp::tool_mcp::{attach_structured_content, ToolMCP};
    let schema =
        json!({"type":"object","properties":{"count":{"type":"integer"}},"required":["count"]});
    let tool: rmcp::model::Tool = serde_json::from_value(json!({
        "name":"fixture", "inputSchema":{"type":"object"}, "outputSchema":schema
    }))
    .unwrap();
    let tool = ToolMCP {
        model_name: "fixture".into(),
        common: Default::default(),
        config_path: String::new(),
        mcp_client: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
        mcp_tool: tool,
        request_timeout: 10,
        init_timeout: 10,
        auto_approve: false,
    };
    let desc = tool.tool_description();
    assert_eq!(desc.output_schema, Some(schema));
    let mut message = ChatMessage::new("tool".into(), "unchanged MCP text\n".into());
    attach_structured_content(&mut message, Some(json!({"count":3})));
    assert_eq!(message.content.content_text_only(), "unchanged MCP text\n");
    assert_eq!(message.extra[TOOL_OUTPUT_EXTRA_KEY], json!({"count":3}));
    assert_output_matches_schema(&desc, &[message]);
    let mut plain = ChatMessage::new("tool".into(), "plain".into());
    attach_structured_content(&mut plain, None);
    assert!(!plain.extra.contains_key(TOOL_OUTPUT_EXTRA_KEY));
}

#[test]
fn subagent_preview_output_contract_has_no_fabricated_child() {
    use crate::tools::tool_subagent::{SubagentOutput, ToolSubagent};
    let output = SubagentOutput {
        agent_id: None,
        child_chat_id: None,
        status: "preview".into(),
        model: None,
    };
    let mut message = ChatMessage::new("tool".into(), "preview".into());
    message.extra.insert(
        TOOL_OUTPUT_EXTRA_KEY.into(),
        serde_json::to_value(output).unwrap(),
    );
    assert!(message.extra[TOOL_OUTPUT_EXTRA_KEY]["agent_id"].is_null());
    assert!(message.extra[TOOL_OUTPUT_EXTRA_KEY]["child_chat_id"].is_null());
    assert!(message.extra[TOOL_OUTPUT_EXTRA_KEY].get("model").is_none());
    assert_output_matches_schema(
        &ToolSubagent {
            config_path: String::new(),
        }
        .tool_description(),
        &[message],
    );
}

#[test]
fn mcp_structured_content_becomes_tool_output() {
    let result: rmcp::model::CallToolResult = serde_json::from_value(json!({
        "content":[{"type":"text","text":"existing text"}],
        "structuredContent":{"count":3}
    }))
    .unwrap();
    let mut message = ChatMessage::new("tool".into(), "existing text".into());
    crate::integrations::mcp::tool_mcp::attach_structured_content(
        &mut message,
        result.structured_content,
    );
    assert_eq!(message.content.content_text_only(), "existing text");
    assert_eq!(message.extra[TOOL_OUTPUT_EXTRA_KEY], json!({"count":3}));
}

#[test]
fn mcp_structured_content_obeys_output_size_cap() {
    use crate::integrations::mcp::tool_mcp::attach_structured_content;
    for (size, attached) in [(32, true), (200 * 1024, false)] {
        let content = json!({"value":"x".repeat(size)});
        let mut message = ChatMessage::new("tool".into(), "unchanged text".into());
        attach_structured_content(&mut message, Some(content.clone()));
        assert_eq!(message.content.content_text_only(), "unchanged text");
        assert_eq!(message.extra.contains_key(TOOL_OUTPUT_EXTRA_KEY), attached);
        if attached {
            assert_eq!(message.extra[TOOL_OUTPUT_EXTRA_KEY], content);
        }
    }
}
