use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock, mpsc};
use serde_json::{json, Value};
use tracing::info;
use uuid::Uuid;

use crate::caps::{resolve_chat_model, resolve_model};
use crate::tools::tools_description::ToolDesc;
use crate::tools::tools_list::get_available_tools;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{
    ChatContent, ChatMeta, ChatToolCall, SamplingParameters, ChatMessage, ChatUsage,
    ReasoningEffort, ChatModelType, SubchatParameters, ContextFile,
};
use crate::global_context::{GlobalContext, try_load_caps_quickly_if_not_present};
use crate::scratchpad_abstract::HasTokenizerAndEot;
use crate::chat::prepare::{prepare_chat_passthrough, ChatPrepareOptions};
use crate::llm::params::CacheControl;
use crate::chat::stream_core::{
    run_llm_stream, StreamRunParams, NoopCollector, ChoiceFinal, normalize_tool_call,
};
use crate::chat::tools::{execute_tools, ExecuteToolsOptions};
use crate::chat::types::ThreadParams;
use crate::chat::trajectories::save_trajectory_as;
use crate::chat::trajectory_ops::sanitize_messages_for_new_thread;


fn get_context_files_from_messages(messages: &[ChatMessage]) -> Vec<String> {
    use std::collections::HashSet;
    let mut seen = HashSet::new();
    let mut paths = Vec::new();
    for msg in messages {
        if msg.role == "context_file" {
            match &msg.content {
                ChatContent::ContextFiles(files) => {
                    for file in files {
                        if seen.insert(file.file_name.clone()) {
                            paths.push(file.file_name.clone());
                        }
                    }
                }
                ChatContent::SimpleText(text) => {
                    if let Ok(files) = serde_json::from_str::<Vec<ContextFile>>(text) {
                        for file in files {
                            if seen.insert(file.file_name.clone()) {
                                paths.push(file.file_name.clone());
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    paths
}

#[derive(Clone, Debug)]
pub enum ToolsPolicy {
    All,
    None,
    Only(Vec<String>),
}

impl ToolsPolicy {
    pub fn from_option(opt: Option<Vec<String>>) -> Self {
        match opt {
            None => ToolsPolicy::All,
            Some(v) if v.is_empty() => ToolsPolicy::None,
            Some(v) => ToolsPolicy::Only(v),
        }
    }

    fn to_subset_for_llm(&self) -> Option<Vec<String>> {
        match self {
            ToolsPolicy::All => None,
            ToolsPolicy::None => Some(vec![]),
            ToolsPolicy::Only(v) => Some(v.clone()),
        }
    }

    fn allows_tool(&self, tool_name: &str) -> bool {
        match self {
            ToolsPolicy::All => true,
            ToolsPolicy::None => false,
            ToolsPolicy::Only(v) => v.iter().any(|t| t == tool_name),
        }
    }
}

#[derive(Clone)]
pub struct WrapUpConfig {
    pub depth: usize,
    pub tokens_cnt: usize,
    pub prompt: String,
}

#[derive(Clone)]
pub struct SubchatConfig {
    pub tool_name: String,
    pub stateful: bool,
    pub chat_id: Option<String>,
    pub title: Option<String>,
    pub parent_id: Option<String>,
    pub link_type: Option<String>,
    pub root_chat_id: Option<String>,
    pub tools: ToolsPolicy,
    pub max_steps: usize,
    pub prepend_system_prompt: bool,
    pub wrap_up: Option<WrapUpConfig>,
    pub model: String,
    pub mode: String,
    pub n_ctx: usize,
    pub max_new_tokens: usize,
    pub temperature: Option<f32>,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub parent_tool_call_id: Option<String>,
    pub parent_subchat_tx: Option<Arc<AMutex<mpsc::UnboundedSender<Value>>>>,
    pub abort_flag: Option<Arc<AtomicBool>>,
}

pub struct SubchatResult {
    pub messages: Vec<ChatMessage>,
    pub usage: ChatUsage,
    /// Aggregated metering data from all assistant messages (coins, tokens, etc.)
    pub metering: serde_json::Map<String, serde_json::Value>,
    /// Set when `config.stateful == true`, allows caller to reference the saved trajectory.
    /// Intentionally public API - callers may use it for trajectory linking.
    #[allow(dead_code)]
    pub chat_id: Option<String>,
}

pub async fn resolve_subchat_params(
    gcx: Arc<ARwLock<GlobalContext>>,
    tool_name: &str,
) -> Result<SubchatParameters, String> {
    use crate::yaml_configs::customization_registry::get_subagent_config;

    let subagent_config = get_subagent_config(gcx.clone(), tool_name, None).await
        .ok_or_else(|| {
            format!("subchat params for '{}' not found in subagents registry", tool_name)
        })?;

    let subchat = &subagent_config.subchat;

    let model_type = match subchat.model_type.as_deref() {
        Some(mt) if mt.eq_ignore_ascii_case("light") => ChatModelType::Light,
        Some(mt) if mt.eq_ignore_ascii_case("thinking") => ChatModelType::Thinking,
        Some(mt) if mt.eq_ignore_ascii_case("default") => ChatModelType::Default,
        Some(mt) => return Err(format!(
            "invalid model_type '{}' for '{}', expected: light, default, thinking",
            mt, tool_name
        )),
        None => ChatModelType::Default,
    };

    let reasoning_effort = match subchat.reasoning_effort.as_deref() {
        Some(re) if re.eq_ignore_ascii_case("low") => Some(ReasoningEffort::Low),
        Some(re) if re.eq_ignore_ascii_case("medium") => Some(ReasoningEffort::Medium),
        Some(re) if re.eq_ignore_ascii_case("high") => Some(ReasoningEffort::High),
        Some(re) => return Err(format!(
            "invalid reasoning_effort '{}' for '{}', expected: low, medium, high",
            re, tool_name
        )),
        None => None,
    };

    let params = SubchatParameters {
        subchat_model_type: model_type,
        subchat_model: subchat.model.clone().unwrap_or_default(),
        subchat_n_ctx: subchat.n_ctx.unwrap_or(0),
        subchat_max_new_tokens: subchat.max_new_tokens.unwrap_or(0),
        subchat_temperature: subchat.temperature,
        subchat_tokens_for_rag: subchat.tokens_for_rag.unwrap_or(0),
        subchat_reasoning_effort: reasoning_effort,
    };

    if params.subchat_n_ctx == 0 {
        return Err(format!(
            "subchat_n_ctx must be > 0 for tool '{}'",
            tool_name
        ));
    }
    if params.subchat_max_new_tokens == 0 {
        return Err(format!(
            "subchat_max_new_tokens must be > 0 for tool '{}'",
            tool_name
        ));
    }

    Ok(params)
}

pub async fn resolve_subchat_model(
    gcx: Arc<ARwLock<GlobalContext>>,
    params: &SubchatParameters,
) -> Result<String, String> {
    let caps = try_load_caps_quickly_if_not_present(gcx.clone(), 0)
        .await
        .map_err(|e| format!("failed to load caps: {:?}", e))?;

    if !params.subchat_model.is_empty() {
        resolve_chat_model(caps, &params.subchat_model)?;
        return Ok(params.subchat_model.clone());
    }

    let model_id = match params.subchat_model_type {
        ChatModelType::Light => &caps.defaults.chat_light_model,
        ChatModelType::Default => &caps.defaults.chat_default_model,
        ChatModelType::Thinking => &caps.defaults.chat_thinking_model,
    };

    if model_id.is_empty() {
        return Err(format!(
            "no model configured for {:?} in caps.defaults",
            params.subchat_model_type
        ));
    }

    let model_rec = resolve_model(&caps.chat_models, model_id)
        .map_err(|e| format!("model '{}' not found: {}", model_id, e))?;

    Ok(model_rec.base.id.clone())
}

pub async fn resolve_subchat_config(
    gcx: Arc<ARwLock<GlobalContext>>,
    tool_name: &str,
    stateful: bool,
    chat_id: Option<String>,
    title: Option<String>,
    parent_id: Option<String>,
    link_type: Option<String>,
    root_chat_id: Option<String>,
    tools: Option<Vec<String>>,
    max_steps: usize,
    prepend_system_prompt: bool,
    wrap_up: Option<WrapUpConfig>,
    mode: String,
) -> Result<SubchatConfig, String> {
    resolve_subchat_config_with_parent(
        gcx,
        tool_name,
        stateful,
        chat_id,
        title,
        parent_id,
        link_type,
        root_chat_id,
        tools,
        max_steps,
        prepend_system_prompt,
        wrap_up,
        mode,
        None,
        None,
        None,
        0,
    )
    .await
}

pub async fn resolve_subchat_config_with_parent(
    gcx: Arc<ARwLock<GlobalContext>>,
    tool_name: &str,
    stateful: bool,
    chat_id: Option<String>,
    title: Option<String>,
    parent_id: Option<String>,
    link_type: Option<String>,
    root_chat_id: Option<String>,
    tools: Option<Vec<String>>,
    max_steps: usize,
    prepend_system_prompt: bool,
    wrap_up: Option<WrapUpConfig>,
    mode: String,
    parent_tool_call_id: Option<String>,
    parent_subchat_tx: Option<Arc<AMutex<mpsc::UnboundedSender<Value>>>>,
    abort_flag: Option<Arc<AtomicBool>>,
    subchat_depth: usize,
) -> Result<SubchatConfig, String> {
    use crate::at_commands::at_commands::MAX_SUBCHAT_DEPTH;
    if max_steps == 0 {
        return Err("max_steps must be > 0".to_string());
    }
    if subchat_depth >= MAX_SUBCHAT_DEPTH {
        return Err(format!("subchat depth limit ({}) exceeded", MAX_SUBCHAT_DEPTH));
    }

    let params = resolve_subchat_params(gcx.clone(), tool_name).await?;
    let model = resolve_subchat_model(gcx.clone(), &params).await?;

    let caps = try_load_caps_quickly_if_not_present(gcx.clone(), 0)
        .await
        .map_err(|e| format!("failed to load caps: {:?}", e))?;

    let model_rec = resolve_chat_model(caps, &model)?;
    if params.subchat_n_ctx > model_rec.base.n_ctx && model_rec.base.n_ctx > 0 {
        return Err(format!(
            "subchat_n_ctx ({}) exceeds model '{}' n_ctx ({})",
            params.subchat_n_ctx, model, model_rec.base.n_ctx
        ));
    }

    Ok(SubchatConfig {
        tool_name: tool_name.to_string(),
        stateful,
        chat_id,
        title,
        parent_id,
        link_type,
        root_chat_id,
        tools: ToolsPolicy::from_option(tools),
        max_steps,
        prepend_system_prompt,
        wrap_up,
        model,
        mode,
        n_ctx: params.subchat_n_ctx,
        max_new_tokens: params.subchat_max_new_tokens,
        temperature: params.subchat_temperature,
        reasoning_effort: params.subchat_reasoning_effort,
        parent_tool_call_id,
        parent_subchat_tx,
        abort_flag,
    })
}

fn has_final_answer(messages: &[ChatMessage]) -> bool {
    messages
        .iter()
        .rev()
        .find(|m| m.role == "assistant")
        .map(|m| m.tool_calls.as_ref().map_or(true, |tc| tc.is_empty()))
        .unwrap_or(false)
}

pub async fn run_subchat(
    gcx: Arc<ARwLock<GlobalContext>>,
    messages: Vec<ChatMessage>,
    config: SubchatConfig,
) -> Result<SubchatResult, String> {
    info!(
        "run_subchat tool={} model={} stateful={}",
        config.tool_name, config.model, config.stateful
    );

    let chat_id = config
        .chat_id
        .clone()
        .unwrap_or_else(|| format!("subchat-{}", Uuid::new_v4()));

    let messages = sanitize_messages_for_new_thread(&messages);
    let ccx = Arc::new(AMutex::new(
        AtCommandsContext::new_with_abort(
            gcx.clone(),
            config.n_ctx,
            1,
            false,
            messages.clone(),
            chat_id.clone(),
            config.root_chat_id.clone(),
            config.model.clone(),
            None,
            config.abort_flag.clone(),
        )
        .await,
    ));

    if let Some(ref parent_tx) = config.parent_subchat_tx {
        ccx.lock().await.subchat_tx = parent_tx.clone();
    }

    let mut usage = ChatUsage::default();
    let mut current_messages = messages;

    if let Some(ref wrap_up) = config.wrap_up {
        current_messages = run_subchat_with_wrap_up(
            ccx.clone(),
            &config,
            current_messages,
            &config.tools,
            wrap_up,
            &mut usage,
        )
        .await?;
    } else {
        current_messages = run_subchat_loop(
            ccx.clone(),
            &config,
            current_messages,
            &config.tools,
            &mut usage,
        )
        .await?;
    }

    if config.stateful {
        let tool_use_str = match &config.tools {
            ToolsPolicy::All => "agent".to_string(),
            ToolsPolicy::None => "none".to_string(),
            ToolsPolicy::Only(v) => v.join(","),
        };

        let thread = ThreadParams {
            id: chat_id.clone(),
            title: config
                .title
                .clone()
                .unwrap_or_else(|| "Subchat".to_string()),
            model: config.model.clone(),
            mode: config.mode.clone(),
            tool_use: tool_use_str,
            parent_id: config.parent_id.clone(),
            link_type: config.link_type.clone(),
            ..Default::default()
        };

        save_trajectory_as(gcx.clone(), &thread, &current_messages).await;
    }

    let metering = aggregate_metering_from_messages(&current_messages);

    Ok(SubchatResult {
        messages: current_messages,
        usage,
        metering,
        chat_id: if config.stateful { Some(chat_id) } else { None },
    })
}

pub async fn run_subchat_once(
    gcx: Arc<ARwLock<GlobalContext>>,
    tool_name: &str,
    messages: Vec<ChatMessage>,
) -> Result<SubchatResult, String> {
    let config = resolve_subchat_config(
        gcx.clone(),
        tool_name,
        false,
        None,
        None,
        None,
        None,
        None,
        Some(vec![]),
        1,
        false,
        None,
        "agent".to_string(),
    )
    .await?;

    let chat_id = format!("subchat-{}", Uuid::new_v4());

    let messages = sanitize_messages_for_new_thread(&messages);
    let ccx = Arc::new(AMutex::new(
        AtCommandsContext::new(
            gcx.clone(),
            config.n_ctx,
            1,
            false,
            messages.clone(),
            chat_id.clone(),
            config.root_chat_id.clone(),
            config.model.clone(),
            None,
        )
        .await,
    ));

    let results = subchat_single_internal(
        ccx,
        &config.model,
        &config.mode,
        messages,
        Some(vec![]),
        false,
        config.temperature,
        config.max_new_tokens,
        config.reasoning_effort.clone(),
        false,
    )
    .await?;

    let mut usage = ChatUsage::default();
    update_usage_from_messages(&mut usage, &results);

    let final_messages = results.into_iter().next().unwrap_or_default();
    let metering = aggregate_metering_from_messages(&final_messages);

    Ok(SubchatResult {
        messages: final_messages,
        usage,
        metering,
        chat_id: None,
    })
}

fn is_aborted(abort_flag: &Option<Arc<AtomicBool>>) -> bool {
    abort_flag
        .as_ref()
        .map(|f| f.load(Ordering::SeqCst))
        .unwrap_or(false)
}

async fn run_subchat_loop(
    ccx: Arc<AMutex<AtCommandsContext>>,
    config: &SubchatConfig,
    mut messages: Vec<ChatMessage>,
    tools_policy: &ToolsPolicy,
    usage: &mut ChatUsage,
) -> Result<Vec<ChatMessage>, String> {
    for step in 0..config.max_steps {
        if is_aborted(&config.abort_flag) {
            return Err("Aborted".to_string());
        }

        let results = subchat_single_internal(
            ccx.clone(),
            &config.model,
            &config.mode,
            messages.clone(),
            tools_policy.to_subset_for_llm(),
            false,
            config.temperature,
            config.max_new_tokens,
            config.reasoning_effort.clone(),
            config.prepend_system_prompt && step == 0,
        )
        .await?;

        update_usage_from_messages(usage, &results);
        messages = results.into_iter().next().unwrap_or(messages);

        if has_final_answer(&messages) {
            break;
        }

        messages = execute_pending_tool_calls(
            ccx.clone(),
            &config.model,
            &config.mode,
            messages,
            tools_policy,
            step,
            config.max_steps,
            config.parent_tool_call_id.clone(),
        )
        .await?;

        if is_aborted(&config.abort_flag) {
            return Err("Aborted".to_string());
        }
    }

    Ok(messages)
}

async fn run_subchat_with_wrap_up(
    ccx: Arc<AMutex<AtCommandsContext>>,
    config: &SubchatConfig,
    mut messages: Vec<ChatMessage>,
    tools_policy: &ToolsPolicy,
    wrap_up: &WrapUpConfig,
    usage: &mut ChatUsage,
) -> Result<Vec<ChatMessage>, String> {
    let mut step_n = 0;

    loop {
        if is_aborted(&config.abort_flag) {
            return Err("Aborted".to_string());
        }

        if has_final_answer(&messages) {
            break;
        }

        let last_message = match messages.last() {
            Some(m) => m,
            None => break,
        };

        if last_message.role == "assistant"
            && last_message
                .tool_calls
                .as_ref()
                .map_or(false, |tc| !tc.is_empty())
        {
            if step_n >= wrap_up.depth {
                break;
            }
            if let Some(msg_usage) = &last_message.usage {
                if msg_usage.prompt_tokens + msg_usage.completion_tokens > wrap_up.tokens_cnt {
                    break;
                }
            }
        }

        let results = subchat_single_internal(
            ccx.clone(),
            &config.model,
            &config.mode,
            messages.clone(),
            tools_policy.to_subset_for_llm(),
            false,
            config.temperature,
            config.max_new_tokens,
            config.reasoning_effort.clone(),
            config.prepend_system_prompt && step_n == 0,
        )
        .await?;

        update_usage_from_messages(usage, &results);
        messages = results.into_iter().next().unwrap_or(messages);

        messages = execute_pending_tool_calls(
            ccx.clone(),
            &config.model,
            &config.mode,
            messages,
            tools_policy,
            step_n,
            config.max_steps,
            config.parent_tool_call_id.clone(),
        )
        .await?;

        step_n += 1;

        if is_aborted(&config.abort_flag) {
            return Err("Aborted".to_string());
        }
    }

    if is_aborted(&config.abort_flag) {
        return Err("Aborted".to_string());
    }

    messages = execute_pending_tool_calls(
        ccx.clone(),
        &config.model,
        &config.mode,
        messages,
        tools_policy,
        step_n,
        config.max_steps,
        config.parent_tool_call_id.clone(),
    )
    .await?;

    messages.push(ChatMessage::new("user".to_string(), wrap_up.prompt.clone()));

    let final_results = subchat_single_internal(
        ccx.clone(),
        &config.model,
        &config.mode,
        messages,
        Some(vec![]),
        false,
        config.temperature,
        config.max_new_tokens,
        config.reasoning_effort.clone(),
        false,
    )
    .await?;

    update_usage_from_messages(usage, &final_results);

    Ok(final_results.into_iter().next().unwrap_or_default())
}

fn truncate_args(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let boundary = s.char_indices()
        .take_while(|(i, _)| *i < max)
        .last()
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    format!("{}…", &s[..boundary])
}

async fn execute_pending_tool_calls(
    ccx: Arc<AMutex<AtCommandsContext>>,
    model_id: &str,
    mode_id: &str,
    mut messages: Vec<ChatMessage>,
    tools_policy: &ToolsPolicy,
    step_idx: usize,
    max_steps: usize,
    tx_toolid_mb: Option<String>,
) -> Result<Vec<ChatMessage>, String> {
    let (gcx, n_ctx) = {
        let ccx_locked = ccx.lock().await;
        (ccx_locked.global_context.clone(), ccx_locked.n_ctx)
    };
    let last = match messages.last() {
        Some(m) => m,
        None => return Ok(messages),
    };
    let tool_calls = match &last.tool_calls {
        Some(tc) if !tc.is_empty() => tc.clone(),
        _ => return Ok(messages),
    };

    let mut allowed: Vec<ChatToolCall> = vec![];
    let mut denied_msgs: Vec<ChatMessage> = vec![];

    for tc in tool_calls.iter() {
        if !tools_policy.allows_tool(&tc.function.name) {
            denied_msgs.push(ChatMessage {
                message_id: Uuid::new_v4().to_string(),
                role: "tool".to_string(),
                tool_call_id: tc.id.clone(),
                tool_failed: Some(true),
                content: ChatContent::SimpleText(format!(
                    "Tool '{}' not allowed in this subchat",
                    tc.function.name
                )),
                ..Default::default()
            });
        } else {
            allowed.push(tc.clone());
        }
    }

    let thread = ThreadParams {
        id: format!("subchat-{}", Uuid::new_v4()),
        model: model_id.to_string(),
        mode: mode_id.to_string(),
        context_tokens_cap: Some(n_ctx),
        ..Default::default()
    };

    if let Some(tx_toolid) = &tx_toolid_mb {
        let subchat_tx = ccx.lock().await.subchat_tx.clone();
        let context_files = get_context_files_from_messages(&messages);
        for tc in &allowed {
            let args_truncated = truncate_args(&tc.function.arguments, 200);
            let progress_msg = format!(
                "{}/{}: {}({})",
                step_idx + 1,
                max_steps,
                tc.function.name,
                args_truncated
            );
            let tool_msg = json!({
                "tool_call_id": tx_toolid,
                "subchat_id": progress_msg,
                "attached_files": context_files
            });
            let _ = subchat_tx.lock().await.send(tool_msg);
        }
    }

    let (mut tool_results, _) = execute_tools(
        gcx.clone(),
        &allowed,
        &messages,
        &thread,
        &thread.mode,
        Some(&thread.model),
        ExecuteToolsOptions::default(),
    )
    .await;

    for tc in &tool_calls {
        let answered = denied_msgs
            .iter()
            .chain(tool_results.iter())
            .any(|m| m.tool_call_id == tc.id);
        if !answered {
            tool_results.push(ChatMessage {
                message_id: Uuid::new_v4().to_string(),
                role: "tool".to_string(),
                tool_call_id: tc.id.clone(),
                tool_failed: Some(false),
                content: ChatContent::SimpleText("Tool executed with no output.".to_string()),
                ..Default::default()
            });
        }
    }

    messages.extend(denied_msgs);
    messages.extend(tool_results);

    if let Some(tx_toolid) = &tx_toolid_mb {
        let subchat_tx = ccx.lock().await.subchat_tx.clone();
        let context_files = get_context_files_from_messages(&messages);
        if !context_files.is_empty() {
            let tool_msg = json!({
                "tool_call_id": tx_toolid,
                "subchat_id": "/tool:files",
                "attached_files": context_files
            });
            let _ = subchat_tx.lock().await.send(tool_msg);
        }
    }

    Ok(messages)
}

async fn subchat_stream(
    ccx: Arc<AMutex<AtCommandsContext>>,
    model_id: &str,
    mode_id: &str,
    messages: Vec<ChatMessage>,
    tools: Vec<ToolDesc>,
    prepend_system_prompt: bool,
    temperature: Option<f32>,
    max_new_tokens: usize,
    reasoning_effort: Option<ReasoningEffort>,
    only_deterministic_messages: bool,
) -> Result<Vec<Vec<ChatMessage>>, String> {
    let (gcx, effective_n_ctx, abort_flag) = {
        let ccx_locked = ccx.lock().await;
        (
            ccx_locked.global_context.clone(),
            ccx_locked.n_ctx,
            ccx_locked.abort_flag.clone(),
        )
    };

    let caps = try_load_caps_quickly_if_not_present(gcx.clone(), 0)
        .await
        .map_err(|e| format!("no caps: {:?}", e))?;
    let model_rec = resolve_chat_model(caps, model_id)?;

    let tokenizer_arc = crate::tokens::cached_tokenizer(gcx.clone(), &model_rec.base).await?;
    let t = HasTokenizerAndEot::new(tokenizer_arc);

    let capped_n_ctx = if model_rec.base.n_ctx > 0 {
        effective_n_ctx.min(model_rec.base.n_ctx)
    } else {
        effective_n_ctx
    };

    let meta = ChatMeta {
        chat_id: Uuid::new_v4().to_string(),
        chat_mode: mode_id.to_string(),
        chat_remote: false,
        current_config_file: String::new(),
        context_tokens_cap: Some(capped_n_ctx),
        include_project_info: true,
        request_attempt_id: Uuid::new_v4().to_string(),
    };

    let mut parameters = SamplingParameters {
        max_new_tokens,
        temperature,
        n: Some(1),
        reasoning_effort,
        ..Default::default()
    };

    let options = ChatPrepareOptions {
        prepend_system_prompt,
        allow_at_commands: false,
        allow_tool_prerun: false,
        supports_tools: model_rec.supports_tools,
        cache_control: CacheControl::Ephemeral,
        ..Default::default()
    };

    if only_deterministic_messages {
        return Ok(vec![messages]);
    }

    let prepared = prepare_chat_passthrough(
        gcx.clone(),
        ccx.clone(),
        &t,
        messages.clone(),
        model_id,
        mode_id,
        tools,
        &meta,
        &mut parameters,
        &options,
    )
    .await?;

    let t1 = std::time::Instant::now();

    let params = StreamRunParams {
        llm_request: prepared.llm_request,
        model_rec: model_rec.base.clone(),
        abort_flag: Some(abort_flag),
        supports_tools: model_rec.supports_tools,
        supports_reasoning: model_rec.supports_reasoning.is_some(),
        reasoning_type: model_rec.supports_reasoning.clone(),
        supports_temperature: model_rec.supports_temperature,
    };

    let mut collector = NoopCollector;
    let results = run_llm_stream(gcx.clone(), params, &mut collector).await?;

    info!(
        "stream generation took {:?}ms",
        t1.elapsed().as_millis() as i32
    );

    convert_results_to_messages(results, messages)
}

fn convert_results_to_messages(
    results: Vec<ChoiceFinal>,
    original_messages: Vec<ChatMessage>,
) -> Result<Vec<Vec<ChatMessage>>, String> {
    if results.is_empty() {
        return Ok(vec![original_messages]);
    }

    let mut all_choices = vec![];
    for result in results {
        let tool_calls: Option<Vec<_>> = if result.tool_calls_raw.is_empty() {
            None
        } else {
            let parsed: Vec<_> = result
                .tool_calls_raw
                .iter()
                .filter_map(|tc| normalize_tool_call(tc))
                .collect();
            if parsed.is_empty() {
                None
            } else {
                Some(parsed)
            }
        };

        let msg = ChatMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            role: "assistant".to_string(),
            content: ChatContent::SimpleText(result.content),
            tool_calls,
            reasoning_content: if result.reasoning.is_empty() {
                None
            } else {
                Some(result.reasoning)
            },
            thinking_blocks: if result.thinking_blocks.is_empty() {
                None
            } else {
                Some(result.thinking_blocks)
            },
            citations: result.citations,
            finish_reason: result.finish_reason,
            usage: result.usage,
            extra: result.extra,
            ..Default::default()
        };

        let mut extended = original_messages.clone();
        extended.push(msg);
        all_choices.push(extended);
    }

    Ok(all_choices)
}

fn update_usage_from_messages(usage: &mut ChatUsage, messages: &[Vec<ChatMessage>]) {
    if let Some(message_0) = messages.first() {
        if let Some(last_message) = message_0.last() {
            if let Some(u) = last_message.usage.as_ref() {
                usage.total_tokens += u.total_tokens;
                usage.completion_tokens += u.completion_tokens;
                usage.prompt_tokens += u.prompt_tokens;
                if let Some(cache_creation) = u.cache_creation_tokens {
                    *usage.cache_creation_tokens.get_or_insert(0) += cache_creation;
                }
                if let Some(cache_read) = u.cache_read_tokens {
                    *usage.cache_read_tokens.get_or_insert(0) += cache_read;
                }
            }
        }
    }
}

fn aggregate_metering_from_messages(
    messages: &[ChatMessage],
) -> serde_json::Map<String, serde_json::Value> {
    let mut aggregated = serde_json::Map::new();

    for msg in messages.iter().filter(|m| m.role == "assistant") {
        for (key, value) in &msg.extra {
            if key.starts_with("metering_") {
                if let Some(num) = value.as_f64() {
                    let current = aggregated.get(key).and_then(|v| v.as_f64()).unwrap_or(0.0);
                    aggregated.insert(key.clone(), serde_json::json!(current + num));
                }
            }
        }
    }

    aggregated
}

async fn subchat_single_internal(
    ccx: Arc<AMutex<AtCommandsContext>>,
    model_id: &str,
    mode_id: &str,
    messages: Vec<ChatMessage>,
    tools_subset: Option<Vec<String>>,
    only_deterministic_messages: bool,
    temperature: Option<f32>,
    max_new_tokens: usize,
    reasoning_effort: Option<ReasoningEffort>,
    prepend_system_prompt: bool,
) -> Result<Vec<Vec<ChatMessage>>, String> {
    let gcx = {
        let ccx_locked = ccx.lock().await;
        ccx_locked.global_context.clone()
    };

    let tools_desclist: Vec<ToolDesc> = {
        let tools_turned_on_by_cmdline = get_available_tools(gcx.clone())
            .await
            .iter()
            .map(|tool| tool.tool_description())
            .collect::<Vec<_>>();

        match tools_subset {
            Some(ref subset) if subset.is_empty() => vec![],
            Some(ref subset) => tools_turned_on_by_cmdline
                .into_iter()
                .filter(|tool| subset.contains(&tool.name))
                .collect(),
            None => tools_turned_on_by_cmdline,
        }
    };

    let tools = tools_desclist
        .into_iter()
        .filter(|x| x.is_supported_by(model_id))
        .collect::<Vec<_>>();

    subchat_stream(
        ccx.clone(),
        model_id,
        mode_id,
        messages,
        tools,
        prepend_system_prompt,
        temperature,
        max_new_tokens,
        reasoning_effort,
        only_deterministic_messages,
    )
    .await
}
