use std::collections::HashMap;
use std::sync::Arc;

use crate::call_validation::{ChatMessage, ContextEnum};
use crate::chat::cache_guard::{is_append_only_prefix, sanitize_body_for_cache_guard};
use crate::chat::context_rebuild::fold_controls_for_rebuild;
use crate::chat::internal_roles;
use crate::chat::types::ChatSession;
use crate::llm::adapter::{AdapterSettings, LlmWireAdapter};
use crate::llm::adapters::anthropic::AnthropicAdapter;
use crate::llm::adapters::openai_chat::OpenAiChatAdapter;
use crate::llm::canonical::LlmRequest;
use crate::llm::params::CommonParams;
use crate::tools::tools_description::Tool;
use refact_agentic::mode_transition::{
    ParsedDecisions, TransitionContextBudget, assemble_reconstruction,
};
use refact_chat_api::role_projection::{CHECKPOINT_SOURCE, ProjectionLimits};
use refact_core::active_context::{active_context, make_reconstruction_report, ReconstructionMetadata};
use serde_json::{Value, json};
use tokio::sync::Mutex;

const BASE: &str = "# Approved plan\n\nKeep the approved scope verbatim.";
const RECONSTRUCTION_BUDGET: usize = 2_500;

fn user(text: &str) -> ChatMessage {
    ChatMessage::new("user".into(), text.into())
}

fn assistant(text: &str) -> ChatMessage {
    ChatMessage::new("assistant".into(), text.into())
}

fn source_messages(delta_count: usize) -> Vec<ChatMessage> {
    let mut messages = vec![
        ChatMessage::new("system".into(), "Stable system".into()),
        user("Please continue the approved work."),
        internal_roles::plan("agent", 1, BASE, None),
    ];
    messages.extend((1..=delta_count).map(|seq| {
        let mut message = internal_roles::plan_delta(
            "tool.update_plan",
            json!({"seq": seq}),
            format!("- update {seq}: touched another file"),
        );
        message.message_id = format!("plan-delta-{seq}");
        message
    }));
    messages.push(assistant("Completed the current iteration."));
    for (index, message) in messages.iter_mut().enumerate() {
        if message.message_id.is_empty() {
            message.message_id = format!("source-{index}");
        }
    }
    messages
}

fn limits() -> ProjectionLimits {
    ProjectionLimits {
        checkpoint_after_deltas: 24,
        max_chars: 32_768,
    }
}

fn budget() -> TransitionContextBudget {
    TransitionContextBudget {
        previous_symbols: 100_000,
        total_symbols: RECONSTRUCTION_BUDGET,
        files_symbols: 1_000,
        messages_symbols: 1_500,
        max_images: 0,
    }
}

async fn reconstruct(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    let decisions = ParsedDecisions {
        summary: "The task is underway.".into(),
        handoff_message: "Continue with the next update.".into(),
        ..Default::default()
    };
    let mut rebuilt = assemble_reconstruction(
        messages,
        &decisions,
        "agent",
        &[],
        budget(),
        true,
        &Default::default(),
    )
    .await
    .unwrap();
    for (index, message) in rebuilt.iter_mut().enumerate() {
        if message.message_id.is_empty() {
            message.message_id = format!("rebuilt-{index}");
        }
    }
    rebuilt
}

fn adapter_settings(model_name: &str, endpoint: &str) -> AdapterSettings {
    AdapterSettings {
        api_key: "test-key".into(),
        auth_token: String::new(),
        endpoint: endpoint.into(),
        extra_headers: Default::default(),
        model_name: model_name.into(),
        supports_tools: true,
        supports_reasoning: false,
        reasoning_type: None,
        supports_temperature: true,
        supports_max_completion_tokens: false,
        eof_is_done: false,
        supports_web_search: false,
        supports_cache_control: false,
    }
}

fn provider_bodies(messages: Vec<ChatMessage>) -> [Value; 2] {
    let request = LlmRequest::new("test/model".into(), messages).with_params(CommonParams {
        max_tokens: 1024,
        ..Default::default()
    });
    let openai = OpenAiChatAdapter
        .build_http(
            &refact_privacy::testing::cleared(request.clone()),
            &adapter_settings("gpt-4.1", "https://api.openai.com/v1/chat/completions"),
        )
        .unwrap()
        .body;
    let anthropic = AnthropicAdapter
        .build_http(
            &refact_privacy::testing::cleared(request),
            &adapter_settings("claude-3-sonnet", "https://api.anthropic.com/v1/messages"),
        )
        .unwrap()
        .body;
    [openai, anthropic]
}

async fn rebuilt_session(delta_count: usize) -> (ChatSession, Vec<ChatMessage>) {
    let original = source_messages(delta_count);
    let input = fold_controls_for_rebuild(original.clone(), limits());
    let mut session = ChatSession::new("plan-fold-compaction".into());
    session.messages = original.clone();
    let rebuilt = reconstruct(&input).await;
    let report = make_reconstruction_report(rebuilt, ReconstructionMetadata::default()).unwrap();
    session.add_message(report);
    (session, original)
}

fn active_after_report(session: &ChatSession) -> Vec<ChatMessage> {
    active_context(&session.messages).unwrap().messages
}

#[tokio::test]
async fn rebuild_folds_plan_control_before_reconstruction_and_bounds_both_wires() {
    assert!(include_str!("context_rebuild.rs").contains("messages: &rebuild_input,"));
    async fn make_wire(delta_count: usize) -> [String; 2] {
        let (session, _) = rebuilt_session(delta_count).await;
        let active = active_after_report(&session);
        let bodies = provider_bodies(active);
        bodies.map(|body| body.to_string())
    }

    let hundred = make_wire(100).await;
    let three_hundred = make_wire(300).await;
    let base_wire = BASE.replace('\n', "\\n");
    for (serialized, folded, latest_seq) in [(&hundred, 76, 100), (&three_hundred, 276, 300)]
        .into_iter()
        .flat_map(|(wires, folded, latest)| wires.iter().map(move |wire| (wire, folded, latest)))
    {
        assert!(
            serialized.contains(&base_wire),
            "approved base changed in provider body: {serialized}"
        );
        assert!(
            serialized.contains(&format!(
                "[checkpoint: {folded} earlier plan updates folded"
            )),
            "checkpoint missing: {serialized}"
        );
        assert!(serialized.contains(&format!("seq=\\\"{latest_seq}\\\"")));
        assert!(!serialized.contains("seq=\\\"1\\\""));
        assert!(serialized.len() < BASE.len() + 32_768 + 4_096);
    }
    assert!(three_hundred
        .iter()
        .zip(&hundred)
        .all(|(large, small)| large.len().saturating_sub(small.len()) <= 1_024));
}

#[tokio::test]
async fn ten_deltas_are_unchanged_and_unfolded_hundred_deltas_exceed_rebuild_budget() {
    let ten = source_messages(10);
    let ten_folded = fold_controls_for_rebuild(ten.clone(), limits());
    assert_eq!(
        serde_json::to_value(&ten_folded).unwrap(),
        serde_json::to_value(&ten).unwrap()
    );
    let ten_rebuilt = reconstruct(&ten_folded).await;
    let ten_ids: Vec<_> = ten
        .iter()
        .filter(|message| {
            message
                .extra
                .get("event")
                .and_then(|event| event.get("subkind"))
                == Some(&json!("plan_delta"))
        })
        .map(|message| message.message_id.as_str())
        .collect();
    for id in ten_ids {
        assert!(ten_rebuilt.iter().any(|message| message.message_id == id));
    }
    assert!(ten_rebuilt.iter().all(|message| {
        message
            .extra
            .get("event")
            .and_then(|event| event.get("source"))
            != Some(&json!(CHECKPOINT_SOURCE))
    }));

    let hundred = source_messages(100);
    let decisions = ParsedDecisions {
        summary: "The task is underway.".into(),
        handoff_message: "Continue with the next update.".into(),
        ..Default::default()
    };
    let error = assemble_reconstruction(
        &hundred,
        &decisions,
        "agent",
        &[],
        budget(),
        true,
        &Default::default(),
    )
    .await
    .unwrap_err();
    assert!(error.starts_with("Mandatory plan/goal context"), "{error}");
}

#[tokio::test]
async fn archive_stays_unchanged_and_update_plan_appends_after_rebuild() {
    let (session, original) = rebuilt_session(100).await;
    let report_index = session.messages.len() - 1;
    assert_eq!(
        serde_json::to_value(&session.messages[..report_index]).unwrap(),
        serde_json::to_value(&original).unwrap()
    );
    let mut baseline = active_after_report(&session);
    baseline.push(assistant("I continued after the rebuild."));
    let previous =
        provider_bodies(baseline.clone()).map(|body| sanitize_body_for_cache_guard(&body));

    let gcx = crate::global_context::tests::make_test_gcx().await;
    let session_arc = Arc::new(Mutex::new(session));
    gcx.chat_sessions
        .write()
        .await
        .insert("plan-fold-compaction".into(), session_arc.clone());
    let ccx = Arc::new(Mutex::new(
        crate::at_commands::at_commands::AtCommandsContext::new_from_app(
            crate::app_state::AppState::from_gcx(gcx.clone()).await,
            4096,
            20,
            false,
            vec![],
            "plan-fold-compaction".into(),
            None,
            "test/model".into(),
            None,
            None,
        )
        .await,
    ));
    let mut tool = crate::tools::tool_update_plan::ToolUpdatePlan {
        config_path: String::new(),
    };
    for index in 0..3 {
        let (_, result) = tool
            .tool_execute(
                ccx.clone(),
                &format!("call-{index}"),
                &HashMap::from([("note".into(), json!(format!("another change {index}")))]),
            )
            .await
            .unwrap();
        let ContextEnum::ChatMessage(result) = &result[0] else {
            panic!("expected update_plan tool result");
        };
        let result_text = result.content.content_text_only();
        let result: Value = serde_json::from_str(&result_text).unwrap();
        assert_eq!(result["seq"], json!(101 + index));
    }

    let pending_messages: Vec<_> = {
        let session = session_arc.lock().await;
        session
            .pending_deliveries
            .iter()
            .flat_map(|delivery| delivery.messages.iter().cloned())
            .collect()
    };
    let mut extended = baseline;
    extended.extend(pending_messages);
    extended.push(user("Continue after the three updates."));
    let next = provider_bodies(extended).map(|body| sanitize_body_for_cache_guard(&body));
    for (provider, (old, new)) in previous.iter().zip(next.iter()).enumerate() {
        assert!(is_append_only_prefix(old, new), "provider index {provider}");
    }
}

#[test]
fn rebuild_wiring_folds_only_after_unfolded_hook_equality_check() {
    let source = include_str!("context_rebuild.rs");
    let reserved = source.find("async fn rebuild_session_reserved(").unwrap();
    let source = &source[reserved..];
    let hook_equality = source.find("|| !source.matches(&session)").unwrap();
    let normalization = source.find("normalize_rebuild_input(&raw,").unwrap();
    let fold = source
        .find("let rebuild_input = fold_controls_for_rebuild(")
        .unwrap();
    let reconstruction = source.find("messages: &rebuild_input,").unwrap();
    let validation = source.find("validate_output(&messages,").unwrap();
    assert!(hook_equality < normalization && normalization < fold);
    assert!(fold < reconstruction && reconstruction < validation);
}
