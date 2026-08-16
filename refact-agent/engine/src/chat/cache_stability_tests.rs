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
use crate::chat::summarization::{is_segment_summary, summarize_oldest_segment_with_static_summary};
use crate::chat::trajectories::{
    ensure_frozen_prefix, load_trajectory_for_chat, save_trajectory_snapshot, TrajectorySnapshot,
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

    prepare_chat_passthrough(
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
    .unwrap()
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

fn direct_openai_body(messages: Vec<ChatMessage>, tools: Vec<Value>) -> Value {
    let req = LlmRequest::new("test/cache-openai".to_string(), messages)
        .with_params(CommonParams {
            max_tokens: 1024,
            n: Some(1),
            ..Default::default()
        })
        .with_tools(tools, None);
    openai_body(&req, "cache-openai")
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
async fn cache_stability_switch_mode_enrichment_drift_reuses_frozen_tools() {
    let model_id = "test/cache-openai";
    let (gcx, workspace) = gcx_with_models_and_modes(
        vec![model_record(
            model_id,
            WireFormat::OpenaiChatCompletions,
            "",
        )],
        vec![
            mode_config("agent", "Agent", "Do agent work"),
            mode_config("explore", "Explore", "Look around"),
        ],
    )
    .await;
    let tools = vec![handoff_tool_desc()];
    let first = prepare_request(
        gcx.clone(),
        model_id,
        vec![system("FROZEN SYSTEM"), user("hello")],
        tools.clone(),
        None,
        false,
    )
    .await;
    let first_body = openai_body(&first.llm_request, "cache-openai");
    let first_hashes = compute_provider_request_hashes(&first_body);
    let prefix = frozen_prefix(
        "FROZEN SYSTEM",
        Value::Array(first.llm_request.tools.clone().unwrap()),
    );

    install_project_registry(
        &gcx,
        workspace.path(),
        vec![
            mode_config("agent", "Agent", "Mutated agent mode"),
            mode_config("mutated", "Mutated", "This must not enter frozen tools"),
        ],
    );
    let second = prepare_request(
        gcx,
        model_id,
        vec![system("FROZEN SYSTEM"), user("hello again")],
        tools,
        Some(prefix),
        false,
    )
    .await;
    let second_body = openai_body(&second.llm_request, "cache-openai");
    let second_hashes = compute_provider_request_hashes(&second_body);

    assert_eq!(first_hashes.tools_sha256, second_hashes.tools_sha256);
    assert_eq!(first_hashes.system_sha256, second_hashes.system_sha256);
    assert!(!second_body["tools"].to_string().contains("mutated"));
}

#[tokio::test]
async fn cache_stability_system_prompt_drift_reuses_frozen_system() {
    let model_id = "test/cache-openai";
    let frozen_tools = json!([cache_tool()]);
    let prefix = frozen_prefix("FROZEN SYSTEM", frozen_tools);
    let (gcx, _workspace) = gcx_with_models_and_modes(
        vec![model_record(
            model_id,
            WireFormat::OpenaiChatCompletions,
            "",
        )],
        vec![mode_config("agent", "Agent", "Do agent work")],
    )
    .await;

    let first = prepare_request(
        gcx.clone(),
        model_id,
        vec![system("dynamic workspace tree A"), user("hello")],
        vec![handoff_tool_desc()],
        Some(prefix.clone()),
        false,
    )
    .await;
    let second = prepare_request(
        gcx,
        model_id,
        vec![system("dynamic workspace tree B"), user("hello again")],
        vec![handoff_tool_desc()],
        Some(prefix),
        false,
    )
    .await;
    let first_hashes =
        compute_provider_request_hashes(&openai_body(&first.llm_request, "cache-openai"));
    let second_hashes =
        compute_provider_request_hashes(&openai_body(&second.llm_request, "cache-openai"));

    assert_eq!(first_hashes.system_sha256, second_hashes.system_sha256);
    assert_eq!(
        first.llm_request.messages[0].content.content_text_only(),
        "FROZEN SYSTEM"
    );
    assert_eq!(
        second.llm_request.messages[0].content.content_text_only(),
        "FROZEN SYSTEM"
    );
}

#[tokio::test]
async fn cache_stability_model_switch_away_and_back_never_refreezes_prefix() {
    let cc_model = "claude_code/claude-opus-4";
    let other_model = "test/cache-openai";
    let frozen_tools = json!([cache_tool()]);
    let prefix = frozen_prefix("FROZEN SYSTEM", frozen_tools);
    let (gcx, _workspace) = gcx_with_models_and_modes(
        vec![
            model_record(cc_model, WireFormat::AnthropicMessages, "cc-oauth-token"),
            model_record(other_model, WireFormat::OpenaiChatCompletions, ""),
        ],
        vec![mode_config("agent", "Agent", "Do agent work")],
    )
    .await;

    let before = prepare_request(
        gcx.clone(),
        cc_model,
        vec![system("dynamic before"), user("hello")],
        vec![handoff_tool_desc()],
        Some(prefix.clone()),
        false,
    )
    .await;
    let before_body =
        normalize_claude_code_cache_body(claude_body(&before.llm_request, "claude-opus-4"));
    let before_hashes = compute_provider_request_hashes(&before_body);

    let mut session = ChatSession::new("model-switch-cache".to_string());
    session.thread.frozen_request_prefix = Some(prefix.clone());
    session.thread.model = other_model.to_string();
    assert!(
        ensure_frozen_prefix(&mut session, Some("REFREEZE".to_string()), Some(json!([]))).is_none()
    );
    assert_eq!(session.thread.frozen_request_prefix, Some(prefix.clone()));
    session.thread.model = cc_model.to_string();

    let after = prepare_request(
        gcx,
        cc_model,
        vec![system("dynamic after"), user("hello after switch")],
        vec![handoff_tool_desc()],
        session.thread.frozen_request_prefix.clone(),
        false,
    )
    .await;
    let after_body =
        normalize_claude_code_cache_body(claude_body(&after.llm_request, "claude-opus-4"));
    let after_hashes = compute_provider_request_hashes(&after_body);

    assert_eq!(before_hashes.tools_sha256, after_hashes.tools_sha256);
    assert_eq!(before_body["system"], after_body["system"]);
    assert_eq!(before_hashes.system_sha256, after_hashes.system_sha256);
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

#[test]
fn cache_stability_compression_preserves_user_bytes_and_resets_append_only_baseline() {
    let mut session = ChatSession::new("compression-cache".to_string());
    session.messages = vec![
        system("stable system"),
        user("first exact bytes 🐸"),
        assistant("assistant run"),
        tool("tool result"),
        user("second exact bytes\nwith newline"),
        assistant("second assistant run"),
        user("third exact bytes"),
    ];
    let before_users: Vec<String> = session
        .messages
        .iter()
        .filter(|message| message.role == "user")
        .map(|message| serde_json::to_string(message).unwrap())
        .collect();

    assert!(summarize_oldest_segment_with_static_summary(
        &mut session.messages,
        "compressed assistant segment",
        "stub-summarizer",
    ));
    session.cache_guard_force_next = true;

    let after_users: Vec<String> = session
        .messages
        .iter()
        .filter(|message| message.role == "user")
        .map(|message| serde_json::to_string(message).unwrap())
        .collect();
    assert_eq!(after_users, before_users);
    let summaries: Vec<&ChatMessage> = session
        .messages
        .iter()
        .filter(|message| is_segment_summary(message))
        .collect();
    assert_eq!(summaries.len(), 1);
    assert!(summaries.iter().all(|message| message.role == "assistant"));

    let compressed_body = sanitize_body_for_cache_guard(&direct_openai_body(
        session.messages.clone(),
        vec![cache_tool()],
    ));
    let forced_baseline = if session.cache_guard_force_next {
        compressed_body.clone()
    } else {
        panic!("compression must force the next cache guard snapshot")
    };
    session.cache_guard_force_next = false;
    session.messages.push(user("post compression append"));
    let next_body =
        sanitize_body_for_cache_guard(&direct_openai_body(session.messages, vec![cache_tool()]));

    assert!(is_append_only_prefix(&forced_baseline, &next_body));
}

#[test]
fn cache_stability_append_only_event_at_tail_preserves_prior_prefix() {
    let mut messages = vec![system("stable system"), user("hello"), assistant("answer")];
    let prev_body =
        sanitize_body_for_cache_guard(&direct_openai_body(messages.clone(), vec![cache_tool()]));
    let prev_hashes = compute_provider_request_hashes(&prev_body);

    messages.push(crate::chat::internal_roles::event(
        crate::chat::internal_roles::EventSubkind::SystemNotice,
        "cache_stability.test",
        json!({"kind": "tail"}),
        "tail event".to_string(),
    ));
    let next_body =
        sanitize_body_for_cache_guard(&direct_openai_body(messages, vec![cache_tool()]));
    let next_hashes = compute_provider_request_hashes(&next_body);

    assert_eq!(prev_hashes.tools_sha256, next_hashes.tools_sha256);
    assert_eq!(prev_hashes.system_sha256, next_hashes.system_sha256);
    assert!(is_append_only_prefix(&prev_body, &next_body));
}
