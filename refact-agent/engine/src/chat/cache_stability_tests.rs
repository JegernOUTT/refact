use std::collections::HashMap;
use std::sync::Arc;

use indexmap::IndexMap;
use refact_chat_api::{ClaudeCodeIdentity, FrozenRequestPrefix};
use refact_tool_api::{ToolDesc, ToolSource, ToolSourceType};
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ChatMeta, SamplingParameters};
use crate::caps::{BaseModelRecord, ChatModelRecord, CodeAssistantCaps};
use crate::chat::cache_diagnostics::compute_provider_request_hashes;
use crate::chat::cache_guard::{is_append_only_prefix, sanitize_body_for_cache_guard};
use crate::chat::prepare::{prepare_chat_passthrough, ChatPrepareOptions};
use crate::chat::trajectories::{
    load_trajectory_for_chat, save_trajectory_snapshot, TrajectorySnapshot,
};
use crate::chat::types::{ChatSession, ThreadParams};
use crate::global_context::GlobalContext;
use crate::llm::adapter::{AdapterSettings, LlmWireAdapter};
use crate::llm::adapters::anthropic::AnthropicAdapter;
use crate::llm::adapters::openai_chat::OpenAiChatAdapter;
use crate::llm::{CacheControl, CommonParams, LlmRequest, WireFormat};
use crate::scratchpad_abstract::HasTokenizerAndEot;
use crate::yaml_configs::customization_types::{ModeConfig, ProjectRegistry};

fn handoff_tool_desc() -> ToolDesc {
    ToolDesc {
        name: "handoff_to_mode".to_string(),
        display_name: "Handoff To Mode".to_string(),
        source: ToolSource {
            source_type: ToolSourceType::Builtin,
            config_path: String::new(),
        },
        experimental: false,
        allow_parallel: false,
        description: "Create a new chat in another mode using the current conversation context."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "target_mode": {
                    "type": "string",
                    "description": "Target mode ID to hand off to."
                }
            },
            "required": ["target_mode"]
        }),
        output_schema: None,
        annotations: None,
    }
}

fn mode_config(id: &str, title: &str, description: &str) -> ModeConfig {
    ModeConfig {
        schema_version: 1,
        id: id.to_string(),
        title: title.to_string(),
        description: description.to_string(),
        specific: false,
        prompt: String::new(),
        plan_template: String::new(),
        tools: Vec::new(),
        allow_integrations: false,
        allow_mcp: false,
        allow_subagents: false,
        model_defaults: Default::default(),
        tool_confirm: Default::default(),
        thread_defaults: Default::default(),
        ui: Default::default(),
        base: None,
        match_models: None,
        override_config: None,
    }
}

fn model_record(model_id: &str, wire_format: WireFormat, auth_token: &str) -> Arc<ChatModelRecord> {
    Arc::new(ChatModelRecord {
        base: BaseModelRecord {
            id: model_id.to_string(),
            name: model_id.to_string(),
            n_ctx: 8192,
            tokenizer: "fake".to_string(),
            wire_format,
            auth_token: auth_token.to_string(),
            ..Default::default()
        },
        supports_tools: true,
        supports_strict_tools: false,
        supports_temperature: true,
        ..Default::default()
    })
}

fn reasoning_model_record(model_id: &str) -> Arc<ChatModelRecord> {
    Arc::new(ChatModelRecord {
        base: BaseModelRecord {
            id: model_id.to_string(),
            name: model_id.to_string(),
            n_ctx: 8192,
            tokenizer: "fake".to_string(),
            ..Default::default()
        },
        supports_tools: true,
        supports_temperature: true,
        reasoning_effort_options: Some(vec!["low".to_string(), "medium".to_string()]),
        ..Default::default()
    })
}

fn install_project_registry(
    gcx: &Arc<GlobalContext>,
    workspace: &std::path::Path,
    modes: Vec<ModeConfig>,
) {
    let registry = ProjectRegistry {
        modes: modes
            .into_iter()
            .map(|mode| (mode.id.clone(), mode))
            .collect::<HashMap<_, _>>(),
        ..Default::default()
    };
    gcx.project_registry_cache
        .write()
        .unwrap()
        .insert(workspace.to_path_buf(), registry);
}

async fn gcx_with_models_and_modes(
    models: Vec<Arc<ChatModelRecord>>,
    modes: Vec<ModeConfig>,
) -> (Arc<GlobalContext>, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx().await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
    install_project_registry(&gcx, temp.path(), modes);

    let mut caps = CodeAssistantCaps::default();
    caps.chat_models = IndexMap::new();
    for model in models {
        caps.chat_models.insert(model.base.id.clone(), model);
    }
    caps.defaults.chat_default_model = caps
        .chat_models
        .keys()
        .next()
        .cloned()
        .unwrap_or_else(|| "test/cache-openai".to_string());
    {
        let app = AppState::from_gcx(gcx.clone()).await;
        let mut state = app.model.caps.write().await;
        state.caps = Some(Arc::new(caps));
        state.last_attempted_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }

    (gcx, temp)
}

fn frozen_prefix(system_prompt: &str, tools_canonical: Value) -> FrozenRequestPrefix {
    FrozenRequestPrefix {
        schema_version: 1,
        created_at: "2026-05-29T00:00:00Z".to_string(),
        system_prompt: Some(system_prompt.to_string()),
        tools_canonical: Some(tools_canonical),
    }
}

fn openai_settings(model_name: &str) -> AdapterSettings {
    AdapterSettings {
        api_key: "test-key".to_string(),
        auth_token: String::new(),
        endpoint: "https://api.openai.com/v1/chat/completions".to_string(),
        extra_headers: Default::default(),
        model_name: model_name.to_string(),
        supports_tools: true,
        supports_reasoning: true,
        reasoning_type: None,
        supports_temperature: true,
        supports_max_completion_tokens: false,
        eof_is_done: false,
        supports_web_search: false,
        supports_cache_control: false,
    }
}

fn claude_code_settings(model_name: &str) -> AdapterSettings {
    AdapterSettings {
        auth_token: "cc-oauth-token".to_string(),
        api_key: String::new(),
        endpoint: "https://api.anthropic.com/v1/messages".to_string(),
        extra_headers: Default::default(),
        model_name: model_name.to_string(),
        supports_tools: true,
        supports_reasoning: false,
        reasoning_type: None,
        supports_temperature: true,
        supports_max_completion_tokens: false,
        eof_is_done: false,
        supports_web_search: false,
        supports_cache_control: true,
    }
}

async fn prepare_request(
    gcx: Arc<GlobalContext>,
    model_id: &str,
    messages: Vec<ChatMessage>,
    tools: Vec<ToolDesc>,
    prefix: Option<FrozenRequestPrefix>,
    boost_reasoning: bool,
) -> crate::chat::prepare::PreparedChat {
    let app = AppState::from_gcx(gcx.clone()).await;
    let ccx = AtCommandsContext::new_from_app(
        app,
        8192,
        1,
        false,
        messages.clone(),
        "cache-stability-chat".to_string(),
        None,
        model_id.to_string(),
        None,
        None,
    )
    .await;
    let tokenizer = crate::tokens::cached_tokenizer(
        gcx.clone(),
        &BaseModelRecord {
            id: model_id.to_string(),
            name: model_id.to_string(),
            tokenizer: "fake".to_string(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let t = HasTokenizerAndEot::new(tokenizer);
    let thread = ThreadParams {
        model: model_id.to_string(),
        mode: "agent".to_string(),
        include_project_info: false,
        frozen_request_prefix: prefix.clone(),
        boost_reasoning: Some(boost_reasoning),
        ..Default::default()
    };
    let meta = ChatMeta {
        chat_id: "cache-stability-chat".to_string(),
        chat_mode: "agent".to_string(),
        chat_remote: false,
        current_config_file: String::new(),
        context_tokens_cap: Some(8192),
        include_project_info: false,
        request_attempt_id: "attempt".to_string(),
        worktree: None,
    };
    let mut sampling = SamplingParameters {
        max_new_tokens: 1024,
        boost_reasoning,
        ..Default::default()
    };
    let options = ChatPrepareOptions {
        prepend_system_prompt: false,
        allow_at_commands: false,
        allow_tool_prerun: false,
        supports_tools: true,
        cache_control: CacheControl::Ephemeral,
        frozen_request_prefix: prefix,
        ..Default::default()
    };

    let prepared = prepare_chat_passthrough(
        gcx,
        Arc::new(AMutex::new(ccx)),
        &t,
        messages,
        &thread,
        model_id,
        "agent",
        tools,
        &meta,
        &mut sampling,
        &options,
    )
    .await
    .unwrap();
    match prepared {
        crate::chat::prepare::PrepareOutcome::Ready(prepared) => prepared,
        crate::chat::prepare::PrepareOutcome::Pending {
            ..
        } => {
            panic!("ordinary cache-stability preparation must not defer")
        }
    }
}

fn openai_body(req: &LlmRequest, model_name: &str) -> Value {
    OpenAiChatAdapter
        .build_http(
            &refact_privacy::testing::cleared(req.clone()),
            &openai_settings(model_name),
        )
        .unwrap()
        .body
}

fn claude_body(req: &LlmRequest, model_name: &str) -> Value {
    AnthropicAdapter
        .build_http(
            &refact_privacy::testing::cleared(req.clone()),
            &claude_code_settings(model_name),
        )
        .unwrap()
        .body
}

fn normalize_claude_code_cache_body(mut body: Value) -> Value {
    body.as_object_mut().unwrap().remove("metadata");
    if let Some(system) = body.get_mut("system").and_then(Value::as_array_mut) {
        system.retain(|block| {
            !block
                .get("text")
                .and_then(Value::as_str)
                .is_some_and(|text| text.starts_with("x-anthropic-billing-header:"))
        });
    }
    body
}

fn user(text: &str) -> ChatMessage {
    ChatMessage::new("user".to_string(), text.to_string())
}

fn system(text: &str) -> ChatMessage {
    ChatMessage::new("system".to_string(), text.to_string())
}

fn assistant(text: &str) -> ChatMessage {
    ChatMessage::new("assistant".to_string(), text.to_string())
}

fn tool(text: &str) -> ChatMessage {
    ChatMessage {
        role: "tool".to_string(),
        content: ChatContent::SimpleText(text.to_string()),
        tool_call_id: "call_1".to_string(),
        ..Default::default()
    }
}

fn cache_tool() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "cache_probe",
            "description": "Stable cache probe",
            "parameters": {"type": "object"}
        }
    })
}

#[tokio::test]
async fn cache_stability_reasoning_toggle_preserves_prefix_and_strips_stale_thinking() {
    let model_id = "test/reasoning-openai";
    let frozen_tools = json!([cache_tool()]);
    let prefix = frozen_prefix("FROZEN REASONING SYSTEM", frozen_tools);
    let (gcx, _workspace) = gcx_with_models_and_modes(
        vec![reasoning_model_record(model_id)],
        vec![mode_config("agent", "Agent", "Do agent work")],
    )
    .await;
    let messages = vec![
        system("dynamic reasoning system"),
        user("solve"),
        ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText("answer".to_string()),
            reasoning_content: Some("stale reasoning".to_string()),
            thinking_blocks: Some(vec![json!({
                "type": "thinking",
                "thinking": "stale thought",
                "signature": "sig"
            })]),
            ..Default::default()
        },
        user("continue"),
    ];

    let reasoning_on = prepare_request(
        gcx.clone(),
        model_id,
        messages.clone(),
        vec![handoff_tool_desc()],
        Some(prefix.clone()),
        true,
    )
    .await;
    let reasoning_off = prepare_request(
        gcx,
        model_id,
        messages,
        vec![handoff_tool_desc()],
        Some(prefix),
        false,
    )
    .await;
    let on_hashes = compute_provider_request_hashes(&openai_body(
        &reasoning_on.llm_request,
        "reasoning-openai",
    ));
    let off_body = openai_body(&reasoning_off.llm_request, "reasoning-openai");
    let off_hashes = compute_provider_request_hashes(&off_body);

    assert_eq!(on_hashes.tools_sha256, off_hashes.tools_sha256);
    assert_eq!(on_hashes.system_sha256, off_hashes.system_sha256);
    assert!(reasoning_off
        .llm_request
        .messages
        .iter()
        .all(|message| message.thinking_blocks.is_none() && message.reasoning_content.is_none()));
    assert!(!off_body.to_string().contains("stale thought"));
    assert!(!off_body.to_string().contains("stale reasoning"));
}

#[tokio::test]
async fn cache_stability_claude_code_identity_survives_reload_and_reuses_bytes() {
    let identity = ClaudeCodeIdentity {
        device_id: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
        session_id: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee".to_string(),
    };
    let temp = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx().await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
    let mut thread = ThreadParams {
        title: "Claude Code identity".to_string(),
        model: "claude_code/claude-opus-4".to_string(),
        mode: "agent".to_string(),
        claude_code_identity: Some(identity.clone()),
        ..Default::default()
    };
    thread.tool_use = "agent".to_string();
    let snapshot = TrajectorySnapshot::from_thread_parts(
        "cc-cache-identity".to_string(),
        &thread,
        vec![user("hello")],
        "2026-05-29T00:00:00Z".to_string(),
        1,
    );
    save_trajectory_snapshot(gcx.clone(), snapshot)
        .await
        .unwrap();
    let loaded = load_trajectory_for_chat(gcx, "cc-cache-identity")
        .await
        .unwrap();
    assert_eq!(loaded.thread.claude_code_identity, Some(identity.clone()));

    let req_a = LlmRequest::new(
        "claude_code/claude-opus-4".to_string(),
        vec![system("system"), user("hello")],
    )
    .with_params(CommonParams {
        max_tokens: 1024,
        n: Some(1),
        ..Default::default()
    })
    .with_claude_code_identity(loaded.thread.claude_code_identity.clone());
    let req_b = req_a.clone();
    let http_a = AnthropicAdapter
        .build_http(
            &refact_privacy::testing::cleared(req_a),
            &claude_code_settings("claude-opus-4"),
        )
        .unwrap();
    let http_b = AnthropicAdapter
        .build_http(
            &refact_privacy::testing::cleared(req_b),
            &claude_code_settings("claude-opus-4"),
        )
        .unwrap();

    assert_eq!(
        http_a.headers.get("x-claude-code-session-id"),
        http_b.headers.get("x-claude-code-session-id")
    );
    assert_eq!(http_a.body["metadata"], http_b.body["metadata"]);
    let user_id = http_a.body["metadata"]["user_id"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(user_id).unwrap();
    assert_eq!(parsed["device_id"], identity.device_id);
    assert_eq!(parsed["session_id"], identity.session_id);
}

/// Crossing a plan checkpoint is a projection decision only: every
/// `Never`-exempt delta stays on the wire and the already-cached prefix bytes
/// are untouched.
#[test]
fn cache_stability_plan_checkpoint_appends_without_changing_the_prior_wire_prefix() {
    let approved = "# Approved plan\n\nNon-goal: never rewrite the approved base.";
    let delta = |seq: u64| {
        crate::chat::internal_roles::plan_delta(
            "tool.update_plan",
            json!({"seq": seq}),
            format!("- update {seq}"),
        )
    };
    let mut messages = vec![
        system("stable system"),
        user("approve the plan"),
        crate::chat::internal_roles::plan("task_planner", 1, approved, None),
    ];
    messages.extend((1..=24u64).map(delta));
    messages.push(assistant("worked through the first batch"));
    messages.push(user("keep going"));
    let before = messages.clone();
    messages.extend((25..=60u64).map(delta));

    let limits = refact_chat_api::role_projection::ProjectionLimits {
        checkpoint_after_deltas: 24,
        max_chars: 4_096,
    };
    let mut session = ChatSession::new("plan-checkpoint-wire".to_string());
    session.messages = messages.clone();
    let projection = crate::chat::plan_role::project_current_plan(&session, limits).unwrap();

    assert_eq!(projection.checkpoint.unwrap().folded_deltas, 36);
    assert!(projection.text.starts_with(approved));
    assert_eq!(
        session.messages[2].content.content_text_only(),
        approved,
        "the approved base message bytes must never be rewritten"
    );

    for anthropic in [false, true] {
        let body = |messages: Vec<ChatMessage>| {
            let request = LlmRequest::new("test/cache-openai".to_string(), messages).with_params(
                CommonParams {
                    max_tokens: 1024,
                    ..Default::default()
                },
            );
            sanitize_body_for_cache_guard(&if anthropic {
                normalize_claude_code_cache_body(claude_body(&request, "claude-opus-4"))
            } else {
                openai_body(&request, "cache-openai")
            })
        };
        let old = body(before.clone());
        let new = body(messages.clone());

        assert!(is_append_only_prefix(&old, &new), "anthropic={anthropic}");
        assert_eq!(old["messages"][2], new["messages"][2]);
        let serialized = new.to_string();
        for seq in 1..=60u64 {
            assert!(
                serialized.contains(&format!("plan-update seq=\\\"{seq}\\\"")),
                "delta {seq} dropped from the wire (anthropic={anthropic})"
            );
        }
    }
}

#[tokio::test]
async fn completed_prefix_fork_keeps_parent_wire_bytes_and_thinking_signatures_per_provider() {
    let model = "test/cache-openai";
    let (gcx, workspace) = gcx_with_models_and_modes(
        vec![model_record(model, WireFormat::OpenaiChatCompletions, "")],
        vec![mode_config("agent", "Agent", "Agent")],
    )
    .await;
    let root = workspace.path().canonicalize().unwrap();
    let thinking = |text: &str, signature: &str| {
        Some(vec![
            json!({"type":"thinking","thinking":text,"signature":signature}),
        ])
    };
    let calling: ChatMessage = serde_json::from_value(json!({
        "message_id":"parent-call","role":"assistant","content":"looking",
        "thinking_blocks":[{"type":"thinking","thinking":"decide to read","signature":"sig-call"}],
        "tool_calls":[{"id":"call_1","type":"function","function":{"name":"cat","arguments":"{\"path\":\"a.rs\"}"}}]
    }))
    .unwrap();
    let mut answer = assistant("final parent answer");
    answer.message_id = "parent-answer".into();
    answer.thinking_blocks = thinking("conclude", "sig-answer");
    let pending: ChatMessage = serde_json::from_value(json!({
        "message_id":"parent-pending","role":"assistant","content":"unfinished",
        "tool_calls":[{"id":"call_2","type":"function","function":{"name":"shell","arguments":"{}"}}]
    }))
    .unwrap();
    let parent = vec![
        system("parent policy"),
        user("please read a.rs"),
        calling,
        tool("file body"),
        answer,
        pending,
    ];
    let base_model = BaseModelRecord {
        id: model.into(),
        ..Default::default()
    };
    let scope = crate::agents::launch_spec::LaunchScope {
        caller: crate::coordination::scope::WorkCaller {
            source_root: root.clone(),
            checkout_root: root,
            principal: "parent".into(),
            root_chat_id: None,
            model: Some(base_model.clone()),
        },
        child_model: base_model,
        limits: crate::file_activity::settings(),
    };
    let spec = crate::agents::launch_spec::LaunchSpec {
        history_mode: Some(crate::agents::launch_spec::HistoryForkMode::ForkCompletedPrefix),
        ..Default::default()
    };
    let resolved =
        crate::agents::launch_spec::resolve_scoped_launch_spec(gcx.clone(), &spec, &parent, &scope)
            .await;
    assert!(resolved.skipped.is_empty(), "{:?}", resolved.skipped);
    assert_eq!(resolved.prepended_messages.len(), 4);
    let config = crate::subchat::SubchatConfig {
        history_source_model: Some(model.into()),
        model: model.into(),
        ..crate::subchat::test_support::minimal_config("subchat-fork", "parent")
    };
    let child = crate::subchat::prepare_child_history(&resolved.prepended_messages, &config);
    assert_eq!(child.len(), 4);
    assert_eq!(child[1].thinking_blocks, parent[2].thinking_blocks);
    assert_eq!(child[3].thinking_blocks, parent[4].thinking_blocks);
    assert!(child
        .iter()
        .all(|message| message.message_id != "parent-pending"));
    let request = |messages: Vec<ChatMessage>| {
        LlmRequest::new(model.into(), messages).with_params(CommonParams {
            max_tokens: 1024,
            ..Default::default()
        })
    };
    let parent_segment = parent[1..5].to_vec();
    let openai_parent = openai_body(&request(parent_segment.clone()), "cache-openai");
    let openai_child = openai_body(&request(child.clone()), "cache-openai");
    assert_eq!(openai_parent["messages"], openai_child["messages"]);
    let reasoning_body = |req: &LlmRequest| {
        let mut settings = claude_code_settings("claude-opus-4");
        settings.supports_reasoning = true;
        normalize_claude_code_cache_body(
            AnthropicAdapter
                .build_http(&refact_privacy::testing::cleared(req.clone()), &settings)
                .unwrap()
                .body,
        )
    };
    let claude_parent = reasoning_body(&request(parent_segment));
    let claude_child = reasoning_body(&request(child.clone()));
    assert_eq!(claude_parent["messages"], claude_child["messages"]);
    let serialized = claude_child["messages"].to_string();
    assert!(serialized.contains("sig-call") && serialized.contains("sig-answer"));
    assert!(!serialized.contains("call_2"));
    let mut child_system = vec![system("child runtime policy")];
    child_system.extend(child.clone());
    let first_turn =
        sanitize_body_for_cache_guard(&openai_body(&request(child_system.clone()), "cache-openai"));
    child_system.push(user("continue in the child"));
    let second_turn =
        sanitize_body_for_cache_guard(&openai_body(&request(child_system), "cache-openai"));
    assert!(is_append_only_prefix(&first_turn, &second_turn));
}
