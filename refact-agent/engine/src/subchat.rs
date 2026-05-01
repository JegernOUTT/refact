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
    run_llm_stream, StreamRunParams, ChoiceFinal, StreamCollector, normalize_tool_call,
};
use crate::chat::tools::{execute_tools, ExecuteToolsOptions};
use crate::chat::types::{TaskMeta, ThreadParams};
use crate::worktrees::types::WorktreeMeta;
use crate::chat::trajectories::save_trajectory_as;
use crate::chat::trajectory_ops::sanitize_messages_for_new_thread;
use crate::stats::event::{LlmCallEvent, canonicalize_mode_for_stats, split_model_provider};

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
    pub task_meta: Option<TaskMeta>,
    pub worktree: Option<WorktreeMeta>,
    pub model: String,
    pub mode: String,
    pub n_ctx: usize,
    pub max_new_tokens: usize,
    pub temperature: Option<f32>,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub parent_tool_call_id: Option<String>,
    pub parent_subchat_tx: Option<Arc<AMutex<mpsc::UnboundedSender<Value>>>>,
    pub abort_flag: Option<Arc<AtomicBool>>,
    pub subchat_depth: usize,
    pub buddy_meta: Option<crate::buddy::types::BuddyThreadMeta>,
}

fn should_stream_thinking_progress(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "deep_research"
            | "strategic_planning"
            | "strategic_planning_gather_files"
            | "code_review"
            | "code_review_gather_files"
    )
}

struct SubchatProgressCollector {
    sender: Option<mpsc::UnboundedSender<Value>>,
    tool_call_id: Option<String>,
    thinking_tail: String,
    reasoning_tail: String,
    content_tail: String,
    last_sent: String,
    last_sent_at: std::time::Instant,
}

impl SubchatProgressCollector {
    fn new(sender: Option<mpsc::UnboundedSender<Value>>, tool_call_id: Option<String>) -> Self {
        Self {
            sender,
            tool_call_id,
            thinking_tail: String::new(),
            reasoning_tail: String::new(),
            content_tail: String::new(),
            last_sent: String::new(),
            last_sent_at: std::time::Instant::now()
                .checked_sub(std::time::Duration::from_secs(60))
                .unwrap_or_else(std::time::Instant::now),
        }
    }

    fn append_tail(buf: &mut String, text: &str, max_chars: usize) {
        if text.is_empty() {
            return;
        }
        buf.push_str(text);
        if buf.len() > max_chars {
            let mut start = buf.len().saturating_sub(max_chars);
            while start < buf.len() && !buf.is_char_boundary(start) {
                start += 1;
            }
            buf.drain(..start);
        }
    }

    fn extract_thinking_preview(blocks: &[serde_json::Value]) -> Option<String> {
        for block in blocks.iter().rev() {
            let Some(obj) = block.as_object() else {
                continue;
            };
            for key in ["thinking", "text", "content"] {
                if let Some(s) = obj.get(key).and_then(|v| v.as_str()) {
                    if !s.trim().is_empty() {
                        return Some(s.to_string());
                    }
                }
            }
        }
        None
    }

    fn normalize_preview(text: &str) -> String {
        // Preserve newlines for markdown rendering, just normalize CRLF/CR.
        text.replace("\r\n", "\n").replace('\r', "\n")
    }

    fn maybe_send_update(&mut self) {
        let Some(sender) = self.sender.as_ref() else {
            return;
        };
        let Some(tool_call_id) = self.tool_call_id.as_ref() else {
            return;
        };

        let raw = if !self.thinking_tail.trim().is_empty() {
            &self.thinking_tail
        } else if !self.reasoning_tail.trim().is_empty() {
            &self.reasoning_tail
        } else {
            &self.content_tail
        };

        let mut progress = Self::normalize_preview(raw);
        if progress.is_empty() {
            return;
        }

        // UI renders markdown up to ~50k chars; keep progress within that.
        const MAX_CHARS: usize = 50_000;
        let truncated = crate::llm::safe_truncate(&progress, MAX_CHARS);
        if truncated.len() != progress.len() {
            progress = format!("{}…", truncated);
        }

        if self.last_sent == progress {
            return;
        }

        let now = std::time::Instant::now();
        if now.duration_since(self.last_sent_at) < std::time::Duration::from_millis(750)
            && !self.last_sent.is_empty()
        {
            return;
        }

        let msg = json!({
            "tool_call_id": tool_call_id,
            "subchat_id": progress,
        });
        let _ = sender.send(msg);

        self.last_sent = progress;
        self.last_sent_at = now;
    }
}

impl StreamCollector for SubchatProgressCollector {
    fn on_delta_ops(&mut self, _choice_idx: usize, ops: Vec<crate::chat::types::DeltaOp>) {
        for op in ops {
            match op {
                crate::chat::types::DeltaOp::AppendReasoning { text } => {
                    if self.thinking_tail.trim().is_empty() {
                        Self::append_tail(&mut self.reasoning_tail, &text, 50_000);
                    }
                }
                crate::chat::types::DeltaOp::AppendContent { text } => {
                    if self.thinking_tail.trim().is_empty() && self.reasoning_tail.trim().is_empty()
                    {
                        Self::append_tail(&mut self.content_tail, &text, 50_000);
                    }
                }
                crate::chat::types::DeltaOp::SetThinkingBlocks { blocks } => {
                    if let Some(preview) = Self::extract_thinking_preview(&blocks) {
                        self.thinking_tail = preview;
                    }
                }
                _ => {}
            }
        }

        self.maybe_send_update();
    }

    fn on_usage(&mut self, _usage: &ChatUsage) {}

    fn on_finish(&mut self, _choice_idx: usize, _finish_reason: Option<String>) {}
}

pub struct SubchatResult {
    pub messages: Vec<ChatMessage>,
    /// Reserved for provider-local usage metadata returned by nested agent calls.
    pub metering: serde_json::Map<String, serde_json::Value>,
    /// Set when `config.stateful == true`, allows caller to reference the saved trajectory.
    /// Intentionally public API - callers may use it for trajectory linking.
    #[allow(dead_code)]
    pub chat_id: Option<String>,
}

fn scale_subchat_budget(value: usize, new_n_ctx: usize, old_n_ctx: usize) -> usize {
    if value == 0 || old_n_ctx == 0 || new_n_ctx >= old_n_ctx {
        return value;
    }

    (((value as u128) * (new_n_ctx as u128)) / (old_n_ctx as u128)) as usize
}

fn normalize_subchat_params_for_model(
    tool_name: &str,
    params: &mut SubchatParameters,
    model_rec: &crate::caps::ChatModelRecord,
) {
    let requested_n_ctx = params.subchat_n_ctx;
    let requested_max_new_tokens = params.subchat_max_new_tokens;
    let requested_tokens_for_rag = params.subchat_tokens_for_rag;

    if model_rec.base.n_ctx > 0 && params.subchat_n_ctx > model_rec.base.n_ctx {
        params.subchat_n_ctx = model_rec.base.n_ctx;

        if requested_tokens_for_rag > 0 {
            params.subchat_max_new_tokens = scale_subchat_budget(
                requested_max_new_tokens,
                params.subchat_n_ctx,
                requested_n_ctx,
            )
            .max(1);
            params.subchat_tokens_for_rag = scale_subchat_budget(
                requested_tokens_for_rag,
                params.subchat_n_ctx,
                requested_n_ctx,
            );
        }

        info!(
            "normalized subchat '{}' budget for model '{}' from n_ctx={} to n_ctx={}, max_new_tokens={}, tokens_for_rag={}",
            tool_name,
            model_rec.base.id,
            requested_n_ctx,
            params.subchat_n_ctx,
            params.subchat_max_new_tokens,
            params.subchat_tokens_for_rag,
        );
    }

    if let Some(max_output_tokens) = model_rec.max_output_tokens.filter(|v| *v > 0) {
        if params.subchat_max_new_tokens > max_output_tokens {
            params.subchat_max_new_tokens = max_output_tokens;
        }
    }

    if params.subchat_n_ctx > 1 {
        params.subchat_max_new_tokens = params.subchat_max_new_tokens.min(params.subchat_n_ctx - 1);
    }

    let available_for_rag = params
        .subchat_n_ctx
        .saturating_sub(params.subchat_max_new_tokens)
        .saturating_sub(1);
    if params.subchat_tokens_for_rag > available_for_rag {
        params.subchat_tokens_for_rag = available_for_rag;
    }
}

pub async fn resolve_subchat_params(
    gcx: Arc<ARwLock<GlobalContext>>,
    tool_name: &str,
) -> Result<SubchatParameters, String> {
    use crate::yaml_configs::customization_registry::get_subagent_config;

    let subagent_config = get_subagent_config(gcx.clone(), tool_name, None)
        .await
        .ok_or_else(|| {
            format!(
                "subchat params for '{}' not found in subagents registry",
                tool_name
            )
        })?;

    let subchat = &subagent_config.subchat;

    let model_type = match subchat.model_type.as_deref() {
        Some(mt) if mt.eq_ignore_ascii_case("light") => ChatModelType::Light,
        Some(mt) if mt.eq_ignore_ascii_case("thinking") => ChatModelType::Thinking,
        Some(mt) if mt.eq_ignore_ascii_case("default") => ChatModelType::Default,
        Some(mt) if mt.eq_ignore_ascii_case("buddy") => ChatModelType::Buddy,
        Some(mt) => {
            return Err(format!(
                "invalid model_type '{}' for '{}', expected: light, default, thinking, buddy",
                mt, tool_name
            ))
        }
        None => ChatModelType::Default,
    };

    let reasoning_effort = match subchat.reasoning_effort.as_deref() {
        Some(re) => match ReasoningEffort::from_str_opt(re) {
            Some(effort) => Some(effort),
            None => return Err(format!(
                "invalid reasoning_effort '{}' for '{}', expected: none, minimal, low, medium, high, xhigh, max",
                re, tool_name
            )),
        },
        None => None,
    };

    let mut params = SubchatParameters {
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

    let model = resolve_subchat_model_for_tool(gcx.clone(), tool_name, &params).await?;
    let caps = try_load_caps_quickly_if_not_present(gcx.clone(), 0)
        .await
        .map_err(|e| format!("failed to load caps: {:?}", e))?;
    let model_rec = resolve_chat_model(caps, &model)?;
    normalize_subchat_params_for_model(tool_name, &mut params, &model_rec);

    Ok(params)
}

pub async fn resolve_subchat_model(
    gcx: Arc<ARwLock<GlobalContext>>,
    params: &SubchatParameters,
) -> Result<String, String> {
    resolve_subchat_model_inner(gcx, params, None).await
}

async fn resolve_subchat_model_for_tool(
    gcx: Arc<ARwLock<GlobalContext>>,
    tool_name: &str,
    params: &SubchatParameters,
) -> Result<String, String> {
    resolve_subchat_model_inner(gcx, params, Some(tool_name)).await
}

async fn resolve_subchat_model_inner(
    gcx: Arc<ARwLock<GlobalContext>>,
    params: &SubchatParameters,
    tool_name: Option<&str>,
) -> Result<String, String> {
    let caps = try_load_caps_quickly_if_not_present(gcx.clone(), 0)
        .await
        .map_err(|e| format!("failed to load caps: {:?}", e))?;

    if !params.subchat_model.is_empty() {
        let model_rec = resolve_chat_model(caps, &params.subchat_model).map_err(|e| {
            subchat_explicit_model_error(tool_name, &params.subchat_model, "is not available", &e)
        })?;
        if let Some(reason) = llm_endpoint_unusable_reason(&model_rec.base.endpoint) {
            return Err(subchat_explicit_model_error(
                tool_name,
                &params.subchat_model,
                "is misconfigured",
                &reason,
            ));
        }
        return Ok(model_rec.base.id.clone());
    }

    let model_id = match params.subchat_model_type {
        ChatModelType::Light => &caps.defaults.chat_light_model,
        ChatModelType::Default => &caps.defaults.chat_default_model,
        ChatModelType::Thinking => &caps.defaults.chat_thinking_model,
        ChatModelType::Buddy => &caps.defaults.chat_buddy_model,
    };
    let model_label = subchat_model_type_label(params.subchat_model_type);

    if model_id.trim().is_empty() {
        return Err(format!(
            "{} is not set up. Go to Default model settings and configure {}.",
            subchat_model_requirement_label(tool_name, params.subchat_model_type),
            model_label
        ));
    }

    let model_rec = resolve_model(&caps.chat_models, model_id).map_err(|e| {
        format!(
            "{} '{}' is not available: {}. Go to Default model settings and configure {}.",
            subchat_model_requirement_label(tool_name, params.subchat_model_type),
            model_id,
            e,
            model_label
        )
    })?;

    if let Some(reason) = llm_endpoint_unusable_reason(&model_rec.base.endpoint) {
        return Err(format!(
            "{} '{}' is misconfigured: {}. Go to Default model settings and configure {}.",
            subchat_model_requirement_label(tool_name, params.subchat_model_type),
            model_id,
            reason,
            model_label
        ));
    }

    Ok(model_rec.base.id.clone())
}

fn subchat_explicit_model_error(
    tool_name: Option<&str>,
    model: &str,
    state: &str,
    reason: &str,
) -> String {
    match tool_name {
        Some(tool_name) => format!(
            "Subagent '{}' is pinned to model '{}', but it {}: {}. Go to Default model settings and configure this model or update the subagent config.",
            tool_name, model, state, reason
        ),
        None => format!(
            "Subchat model '{}' {}: {}. Go to Default model settings and configure this model or update the subagent config.",
            model, state, reason
        ),
    }
}

fn subchat_model_requirement_label(tool_name: Option<&str>, model_type: ChatModelType) -> String {
    let model_label = subchat_model_type_label(model_type);
    match tool_name {
        Some(tool_name) => format!(
            "{} required by subagent '{}' (model_type: {})",
            model_label,
            tool_name,
            subchat_model_type_config_value(model_type)
        ),
        None => model_label.to_string(),
    }
}

fn subchat_model_type_config_value(model_type: ChatModelType) -> &'static str {
    match model_type {
        ChatModelType::Light => "light",
        ChatModelType::Default => "default",
        ChatModelType::Thinking => "thinking",
        ChatModelType::Buddy => "buddy",
    }
}

fn subchat_model_type_label(model_type: ChatModelType) -> &'static str {
    match model_type {
        ChatModelType::Light => "Light model",
        ChatModelType::Default => "Default model",
        ChatModelType::Thinking => "Thinking model",
        ChatModelType::Buddy => "Buddy model",
    }
}

fn llm_endpoint_unusable_reason(endpoint: &str) -> Option<String> {
    let endpoint = endpoint.trim();
    if endpoint.is_empty() {
        return Some("an empty LLM endpoint URL".to_string());
    }
    match url::Url::parse(endpoint) {
        Ok(url) if matches!(url.scheme(), "http" | "https") => None,
        Ok(url) => Some(format!(
            "an unsupported LLM endpoint URL scheme '{}': {}",
            url.scheme(),
            endpoint
        )),
        Err(e) => Some(format!("an invalid LLM endpoint URL '{}': {}", endpoint, e)),
    }
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
    task_meta: Option<TaskMeta>,
    worktree: Option<WorktreeMeta>,
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
        return Err(format!(
            "subchat depth limit ({}) exceeded",
            MAX_SUBCHAT_DEPTH
        ));
    }

    let params = resolve_subchat_params(gcx.clone(), tool_name).await?;
    let model = resolve_subchat_model_for_tool(gcx.clone(), tool_name, &params).await?;

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
        task_meta,
        worktree,
        model,
        mode,
        n_ctx: params.subchat_n_ctx,
        max_new_tokens: params.subchat_max_new_tokens,
        temperature: params.subchat_temperature,
        reasoning_effort: params.subchat_reasoning_effort,
        parent_tool_call_id,
        parent_subchat_tx,
        abort_flag,
        subchat_depth,
        buddy_meta: None,
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

fn stateful_thread_from_config(chat_id: &str, config: &SubchatConfig) -> ThreadParams {
    let tool_use = match &config.tools {
        ToolsPolicy::All => "agent".to_string(),
        ToolsPolicy::None => "none".to_string(),
        ToolsPolicy::Only(v) => v.join(","),
    };
    let task_meta = task_meta_for_stateful_subchat(config);

    ThreadParams {
        id: chat_id.to_string(),
        title: config
            .title
            .clone()
            .unwrap_or_else(|| "Subchat".to_string()),
        model: config.model.clone(),
        mode: config.mode.clone(),
        tool_use,
        task_meta,
        worktree: config.worktree.clone(),
        parent_id: config.parent_id.clone(),
        link_type: config.link_type.clone(),
        root_chat_id: config.root_chat_id.clone(),
        buddy_meta: config.buddy_meta.clone(),
        ..Default::default()
    }
}

fn is_subagentic_link_type(link_type: &str) -> bool {
    !matches!(link_type, "handoff" | "mode_transition" | "branch")
}

fn task_meta_for_stateful_subchat(config: &SubchatConfig) -> Option<TaskMeta> {
    let mut task_meta = config.task_meta.clone()?;
    if task_meta.role == "planner"
        && config
            .link_type
            .as_deref()
            .map(is_subagentic_link_type)
            .unwrap_or(false)
    {
        task_meta.role = "subchats".to_string();
    }
    Some(task_meta)
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
            config.task_meta.clone(),
            config.worktree.clone(),
            config.abort_flag.clone(),
        )
        .await,
    ));

    ccx.lock().await.subchat_depth = config.subchat_depth;

    if let Some(ref parent_tx) = config.parent_subchat_tx {
        ccx.lock().await.subchat_tx = parent_tx.clone();
    }

    let mut _usage = ChatUsage::default();
    let mut current_messages = messages;

    if let Some(ref wrap_up) = config.wrap_up {
        current_messages = run_subchat_with_wrap_up(
            ccx.clone(),
            &config,
            current_messages,
            &config.tools,
            wrap_up,
            &mut _usage,
        )
        .await?;
    } else {
        current_messages = run_subchat_loop(
            ccx.clone(),
            &config,
            current_messages,
            &config.tools,
            &mut _usage,
        )
        .await?;
    }

    if config.stateful {
        let thread = stateful_thread_from_config(&chat_id, &config);
        save_trajectory_as(gcx.clone(), &thread, &current_messages).await;
    }

    let metering = aggregate_metering_from_messages(&current_messages);

    Ok(SubchatResult {
        messages: current_messages,
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
            None,
        )
        .await,
    ));

    ccx.lock().await.subchat_depth = config.subchat_depth;

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
        None,
    )
    .await?;

    let final_messages = results.into_iter().next().unwrap_or_default();
    let metering = aggregate_metering_from_messages(&final_messages);

    Ok(SubchatResult {
        messages: final_messages,
        metering,
        chat_id: None,
    })
}

pub async fn run_subchat_once_with_parent(
    gcx: Arc<ARwLock<GlobalContext>>,
    tool_name: &str,
    messages: Vec<ChatMessage>,
    parent_tool_call_id: String,
    parent_subchat_tx: Arc<AMutex<mpsc::UnboundedSender<Value>>>,
    parent_abort_flag: Arc<AtomicBool>,
    parent_depth: usize,
    parent_task_meta: Option<TaskMeta>,
    parent_worktree: Option<WorktreeMeta>,
) -> Result<SubchatResult, String> {
    let config = resolve_subchat_config_with_parent(
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
        parent_task_meta,
        parent_worktree,
        Some(parent_tool_call_id),
        Some(parent_subchat_tx),
        Some(parent_abort_flag),
        parent_depth + 1,
    )
    .await?;

    run_subchat(gcx, messages, config).await
}

#[cfg(test)]
mod progress_collector_tests {
    use super::SubchatProgressCollector;
    use serde_json::json;

    #[test]
    fn test_extract_thinking_preview_skips_non_objects() {
        let blocks = vec![json!({"thinking": "hello"}), json!(123)];
        let preview = SubchatProgressCollector::extract_thinking_preview(&blocks);
        assert_eq!(preview.as_deref(), Some("hello"));
    }

    #[test]
    fn test_append_tail_unicode_no_panic() {
        let mut s = String::new();
        // Force truncation in the middle of multibyte chars.
        for _ in 0..50 {
            SubchatProgressCollector::append_tail(&mut s, "✅", 10);
        }
        assert!(s.is_char_boundary(s.len()));
    }
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
            if should_stream_thinking_progress(&config.tool_name) {
                config.parent_tool_call_id.as_deref()
            } else {
                None
            },
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
            if should_stream_thinking_progress(&config.tool_name) {
                config.parent_tool_call_id.as_deref()
            } else {
                None
            },
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
        if should_stream_thinking_progress(&config.tool_name) {
            config.parent_tool_call_id.as_deref()
        } else {
            None
        },
    )
    .await?;

    update_usage_from_messages(usage, &final_results);

    Ok(final_results.into_iter().next().unwrap_or_default())
}

fn truncate_args(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let boundary = s
        .char_indices()
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
    let (gcx, n_ctx, task_meta, worktree) = {
        let ccx_locked = ccx.lock().await;
        (
            ccx_locked.global_context.clone(),
            ccx_locked.n_ctx,
            ccx_locked.task_meta.clone(),
            ccx_locked.execution_scope_worktree(),
        )
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
        task_meta,
        worktree,
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
    progress_tool_call_id: Option<&str>,
) -> Result<Vec<Vec<ChatMessage>>, String> {
    let (gcx, effective_n_ctx, abort_flag, task_meta, worktree) = {
        let ccx_locked = ccx.lock().await;
        (
            ccx_locked.global_context.clone(),
            ccx_locked.n_ctx,
            ccx_locked.abort_flag.clone(),
            ccx_locked.task_meta.clone(),
            ccx_locked.execution_scope_worktree(),
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
        worktree: worktree.clone(),
    };

    let thread = ThreadParams {
        id: meta.chat_id.clone(),
        model: model_id.to_string(),
        mode: mode_id.to_string(),
        context_tokens_cap: Some(capped_n_ctx),
        task_meta,
        worktree,
        ..Default::default()
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

    let messages_count = messages.len();
    let tools_count = tools.len();
    let mode_for_stats = canonicalize_mode_for_stats(mode_id);

    let (
        stats_chat_id,
        stats_root_chat_id,
        stats_task_id,
        stats_task_role,
        stats_agent_id,
        stats_card_id,
    ) = {
        let ccx_locked = ccx.lock().await;
        let tm = ccx_locked.task_meta.as_ref();
        (
            ccx_locked.chat_id.clone(),
            ccx_locked.root_chat_id.clone(),
            tm.map(|t| t.task_id.clone()),
            tm.map(|t| t.role.clone()),
            tm.and_then(|t| t.agent_id.clone()),
            tm.and_then(|t| t.card_id.clone()),
        )
    };

    let prepared = prepare_chat_passthrough(
        gcx.clone(),
        ccx.clone(),
        &t,
        messages.clone(),
        &thread,
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
        chat_id: None,
        abort_flag: Some(abort_flag),
        supports_tools: model_rec.supports_tools,
        supports_reasoning: model_rec.has_reasoning_support(),
        reasoning_type: model_rec.reasoning_type_string(),
        supports_temperature: model_rec.supports_temperature,
    };

    let progress_sender: Option<mpsc::UnboundedSender<Value>> = if progress_tool_call_id.is_some() {
        let subchat_tx_arc = ccx.lock().await.subchat_tx.clone();
        let x = Some(subchat_tx_arc.lock().await.clone());
        x
    } else {
        None
    };

    let mut collector = SubchatProgressCollector::new(
        progress_sender,
        progress_tool_call_id.map(|s| s.to_string()),
    );

    let call_ts_start = chrono::Utc::now().to_rfc3339();
    let call_start = std::time::Instant::now();

    let results = run_llm_stream(gcx.clone(), params, &mut collector).await;

    let duration_ms = call_start.elapsed().as_millis() as u64;
    let call_ts_end = chrono::Utc::now().to_rfc3339();

    let (provider, model_short) = split_model_provider(model_id);

    match &results {
        Err(e) => {
            let event = LlmCallEvent {
                id: uuid::Uuid::new_v4().to_string(),
                ts_start: call_ts_start,
                ts_end: call_ts_end,
                duration_ms,
                chat_id: stats_chat_id,
                root_chat_id: Some(stats_root_chat_id),
                mode: mode_for_stats.clone(),
                task_id: stats_task_id,
                task_role: stats_task_role,
                agent_id: stats_agent_id,
                card_id: stats_card_id,
                model_id: model_id.to_string(),
                provider,
                model: model_short,
                messages_count,
                tools_count,
                max_tokens: max_new_tokens,
                temperature,
                success: false,
                error_message: Some(e.chars().take(200).collect()),
                finish_reason: None,
                attempt_n: 1,
                retry_reason: None,
                prompt_tokens: 0,
                completion_tokens: 0,
                cache_read_tokens: None,
                cache_creation_tokens: None,
                total_tokens: 0,
                cost_usd: None,
            };
            if let Some(sender) = &gcx.read().await.llm_stats_sender {
                if sender.try_send(event).is_err() {
                    tracing::warn!("stats: channel full, dropping LLM call event");
                }
            }
        }
        Ok(ref results_ok) => {
            let usage = results_ok.first().and_then(|r| r.usage.as_ref());
            let event = LlmCallEvent {
                id: uuid::Uuid::new_v4().to_string(),
                ts_start: call_ts_start,
                ts_end: call_ts_end,
                duration_ms,
                chat_id: stats_chat_id,
                root_chat_id: Some(stats_root_chat_id),
                mode: mode_for_stats.clone(),
                task_id: stats_task_id,
                task_role: stats_task_role,
                agent_id: stats_agent_id,
                card_id: stats_card_id,
                model_id: model_id.to_string(),
                provider,
                model: model_short,
                messages_count,
                tools_count,
                max_tokens: max_new_tokens,
                temperature,
                success: true,
                error_message: None,
                finish_reason: results_ok.first().and_then(|r| r.finish_reason.clone()),
                attempt_n: 1,
                retry_reason: None,
                prompt_tokens: usage.map(|u| u.prompt_tokens).unwrap_or(0),
                completion_tokens: usage.map(|u| u.completion_tokens).unwrap_or(0),
                cache_read_tokens: usage.and_then(|u| u.cache_read_tokens),
                cache_creation_tokens: usage.and_then(|u| u.cache_creation_tokens),
                total_tokens: usage.map(|u| u.total_tokens).unwrap_or(0),
                cost_usd: usage
                    .and_then(|u| u.metering_usd.as_ref())
                    .map(|m| m.total_usd),
            };
            if let Some(sender) = &gcx.read().await.llm_stats_sender {
                if sender.try_send(event).is_err() {
                    tracing::warn!("stats: channel full, dropping LLM call event");
                }
            }
        }
    }

    let results = results?;

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
    _messages: &[ChatMessage],
) -> serde_json::Map<String, serde_json::Value> {
    serde_json::Map::new()
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
    progress_tool_call_id: Option<&str>,
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

    let tools = tools_desclist;

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
        progress_tool_call_id,
    )
    .await
}
#[cfg(test)]
mod subchat_tests {
    use super::{
        resolve_subchat_model, resolve_subchat_params, stateful_thread_from_config, SubchatConfig,
        ToolsPolicy,
    };
    use crate::call_validation::{ChatModelType, ReasoningEffort, SubchatParameters};
    use crate::chat::types::TaskMeta;
    use crate::caps::{BaseModelRecord, ChatModelRecord, CodeAssistantCaps};
    use crate::global_context::tests::make_test_gcx;
    use crate::worktrees::types::WorktreeMeta;
    use crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all;
    use std::fs;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn chat_model_record(id: &str, n_ctx: usize, endpoint: &str) -> Arc<ChatModelRecord> {
        Arc::new(ChatModelRecord {
            base: BaseModelRecord {
                id: id.to_string(),
                name: id.to_string(),
                n_ctx,
                endpoint: endpoint.to_string(),
                ..Default::default()
            },
            max_output_tokens: Some(128_000),
            ..Default::default()
        })
    }

    fn sample_worktree() -> (tempfile::TempDir, WorktreeMeta) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("worktree");
        let source = temp.path().join("source");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&source).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let source = fs::canonicalize(source).unwrap();
        (
            temp,
            WorktreeMeta {
                id: "wt-subchat".to_string(),
                kind: "task_agent".to_string(),
                root,
                source_workspace_root: source.clone(),
                repo_root: source,
                branch: Some("feature".to_string()),
                base_branch: Some("main".to_string()),
                base_commit: Some("base".to_string()),
                task_id: Some("task-1".to_string()),
                card_id: Some("card-1".to_string()),
                agent_id: Some("agent-1".to_string()),
                enforce: true,
            },
        )
    }

    async fn install_caps(
        gcx: Arc<tokio::sync::RwLock<crate::global_context::GlobalContext>>,
        caps: CodeAssistantCaps,
    ) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .saturating_add(60);
        let mut gcx_locked = gcx.write().await;
        gcx_locked.caps = Some(Arc::new(caps));
        gcx_locked.caps_last_attempted_ts = now;
    }

    #[test]
    fn subchat_worktree_stateful_thread_from_config_carries_scope() {
        let (_temp, worktree) = sample_worktree();
        let task_meta = TaskMeta {
            task_id: "task-1".to_string(),
            role: "agents".to_string(),
            agent_id: Some("agent-1".to_string()),
            card_id: Some("card-1".to_string()),
        };
        let config = SubchatConfig {
            tool_name: "subagent".to_string(),
            stateful: true,
            chat_id: None,
            title: Some("Subchat".to_string()),
            parent_id: Some("parent".to_string()),
            link_type: Some("subagent".to_string()),
            root_chat_id: Some("root".to_string()),
            tools: ToolsPolicy::Only(vec!["cat".to_string()]),
            max_steps: 3,
            prepend_system_prompt: false,
            wrap_up: None,
            task_meta: Some(task_meta.clone()),
            worktree: Some(worktree.clone()),
            model: "model".to_string(),
            mode: "agent".to_string(),
            n_ctx: 4096,
            max_new_tokens: 512,
            temperature: None,
            reasoning_effort: Some(ReasoningEffort::Low),
            parent_tool_call_id: None,
            parent_subchat_tx: None,
            abort_flag: None,
            subchat_depth: 1,
            buddy_meta: None,
        };

        let thread = stateful_thread_from_config("subchat-1", &config);

        assert_eq!(thread.id, "subchat-1");
        assert_eq!(thread.task_meta, Some(task_meta));
        assert_eq!(thread.worktree, Some(worktree));
        assert_eq!(thread.tool_use, "cat");
        assert_eq!(thread.parent_id.as_deref(), Some("parent"));
        assert_eq!(thread.root_chat_id.as_deref(), Some("root"));
    }

    #[test]
    fn stateful_subchat_from_planner_uses_hidden_task_role() {
        let task_meta = TaskMeta {
            task_id: "task-1".to_string(),
            role: "planner".to_string(),
            agent_id: None,
            card_id: None,
        };
        let config = SubchatConfig {
            tool_name: "strategic_planning_gather_files".to_string(),
            stateful: true,
            chat_id: None,
            title: Some("Strategic Planning: Gathering Files".to_string()),
            parent_id: Some("planner-task-1-1".to_string()),
            link_type: Some("gather_files".to_string()),
            root_chat_id: Some("planner-task-1-1".to_string()),
            tools: ToolsPolicy::Only(vec!["cat".to_string(), "tree".to_string()]),
            max_steps: 3,
            prepend_system_prompt: false,
            wrap_up: None,
            task_meta: Some(task_meta),
            worktree: None,
            model: "model".to_string(),
            mode: "agent".to_string(),
            n_ctx: 4096,
            max_new_tokens: 512,
            temperature: None,
            reasoning_effort: None,
            parent_tool_call_id: None,
            parent_subchat_tx: None,
            abort_flag: None,
            subchat_depth: 1,
            buddy_meta: None,
        };

        let thread = stateful_thread_from_config("subchat-1", &config);

        assert_eq!(
            thread.task_meta.as_ref().map(|m| m.role.as_str()),
            Some("subchats")
        );
        assert_eq!(
            thread.task_meta.as_ref().map(|m| m.task_id.as_str()),
            Some("task-1")
        );
        assert_eq!(thread.parent_id.as_deref(), Some("planner-task-1-1"));
        assert_eq!(thread.link_type.as_deref(), Some("gather_files"));
        assert_eq!(thread.root_chat_id.as_deref(), Some("planner-task-1-1"));
    }

    #[test]
    fn subchat_worktree_config_fields_carry_parent_scope() {
        let (_temp, worktree) = sample_worktree();
        let task_meta = TaskMeta {
            task_id: "task-1".to_string(),
            role: "agents".to_string(),
            agent_id: Some("agent-1".to_string()),
            card_id: Some("card-1".to_string()),
        };
        let config = SubchatConfig {
            tool_name: "subagent".to_string(),
            stateful: false,
            chat_id: None,
            title: None,
            parent_id: None,
            link_type: None,
            root_chat_id: None,
            tools: ToolsPolicy::All,
            max_steps: 1,
            prepend_system_prompt: false,
            wrap_up: None,
            task_meta: Some(task_meta.clone()),
            worktree: Some(worktree.clone()),
            model: "model".to_string(),
            mode: "agent".to_string(),
            n_ctx: 4096,
            max_new_tokens: 512,
            temperature: None,
            reasoning_effort: None,
            parent_tool_call_id: None,
            parent_subchat_tx: None,
            abort_flag: None,
            subchat_depth: 1,
            buddy_meta: None,
        };

        assert_eq!(config.task_meta, Some(task_meta));
        assert_eq!(config.worktree, Some(worktree));
    }

    #[tokio::test]
    async fn test_resolve_subchat_params_normalizes_code_review_for_smaller_model() {
        let gcx = make_test_gcx().await;
        let config_dir = gcx.read().await.config_dir.clone();
        global_configs_try_create_all(&config_dir).await.unwrap();

        let thinking_model_id = "claude_code/claude-opus-4-6".to_string();
        let mut caps = CodeAssistantCaps::default();
        caps.chat_models.insert(
            thinking_model_id.clone(),
            chat_model_record(
                &thinking_model_id,
                200_000,
                "https://api.anthropic.com/v1/messages",
            ),
        );
        caps.defaults.chat_default_model = thinking_model_id.clone();
        caps.defaults.chat_light_model = thinking_model_id.clone();
        caps.defaults.chat_thinking_model = thinking_model_id;

        install_caps(gcx.clone(), caps).await;

        let params = resolve_subchat_params(gcx, "code_review").await.unwrap();
        let extra_budget = (params.subchat_n_ctx as f32 * 0.06) as usize;

        assert_eq!(params.subchat_n_ctx, 200_000);
        assert!(
            params.subchat_max_new_tokens + params.subchat_tokens_for_rag + extra_budget
                < params.subchat_n_ctx,
            "normalized code_review budget must fit the clamped model context window"
        );
    }

    #[tokio::test]
    async fn test_resolve_subchat_model_errors_when_light_model_missing() {
        let gcx = make_test_gcx().await;
        let default_model_id = "openai/gpt-4o".to_string();

        let mut caps = CodeAssistantCaps::default();
        caps.chat_models.insert(
            default_model_id.clone(),
            chat_model_record(
                &default_model_id,
                128_000,
                "https://api.openai.com/v1/chat/completions",
            ),
        );
        caps.defaults.chat_default_model = default_model_id;

        install_caps(gcx.clone(), caps).await;

        let params = SubchatParameters {
            subchat_model_type: ChatModelType::Light,
            subchat_model: String::new(),
            subchat_n_ctx: 128_000,
            subchat_max_new_tokens: 8_192,
            subchat_temperature: None,
            subchat_tokens_for_rag: 0,
            subchat_reasoning_effort: None,
        };

        let err = resolve_subchat_model(gcx, &params).await.unwrap_err();
        assert!(err.contains("Light model is not set up"));
        assert!(err.contains("Default model settings"));
    }

    #[tokio::test]
    async fn test_resolve_subchat_model_errors_when_endpoint_empty() {
        let gcx = make_test_gcx().await;
        let default_model_id = "openai/gpt-4o".to_string();
        let thinking_model_id = "broken/o1".to_string();

        let mut caps = CodeAssistantCaps::default();
        caps.chat_models.insert(
            default_model_id.clone(),
            chat_model_record(
                &default_model_id,
                128_000,
                "https://api.openai.com/v1/chat/completions",
            ),
        );
        caps.chat_models.insert(
            thinking_model_id.clone(),
            chat_model_record(&thinking_model_id, 128_000, ""),
        );
        caps.defaults.chat_default_model = default_model_id;
        caps.defaults.chat_light_model = "openai/gpt-4o-mini".to_string();
        caps.defaults.chat_thinking_model = thinking_model_id.clone();

        install_caps(gcx.clone(), caps).await;

        let params = SubchatParameters {
            subchat_model_type: ChatModelType::Thinking,
            subchat_model: String::new(),
            subchat_n_ctx: 128_000,
            subchat_max_new_tokens: 8_192,
            subchat_temperature: None,
            subchat_tokens_for_rag: 0,
            subchat_reasoning_effort: None,
        };

        let err = resolve_subchat_model(gcx, &params).await.unwrap_err();
        assert!(err.contains("Thinking model 'broken/o1' is misconfigured"));
        assert!(err.contains("an empty LLM endpoint URL"));
        assert!(err.contains("Default model settings"));
    }

    #[tokio::test]
    async fn test_resolve_subchat_model_errors_when_endpoint_relative() {
        let gcx = make_test_gcx().await;
        let default_model_id = "openai/gpt-4o".to_string();
        let thinking_model_id = "openai/o1".to_string();

        let mut caps = CodeAssistantCaps::default();
        caps.chat_models.insert(
            default_model_id.clone(),
            chat_model_record(
                &default_model_id,
                128_000,
                "https://api.openai.com/v1/chat/completions",
            ),
        );
        caps.chat_models.insert(
            thinking_model_id.clone(),
            chat_model_record(&thinking_model_id, 128_000, "/v1/chat/completions"),
        );
        caps.defaults.chat_default_model = default_model_id;
        caps.defaults.chat_thinking_model = thinking_model_id.clone();

        install_caps(gcx.clone(), caps).await;

        let params = SubchatParameters {
            subchat_model_type: ChatModelType::Thinking,
            subchat_model: String::new(),
            subchat_n_ctx: 128_000,
            subchat_max_new_tokens: 8_192,
            subchat_temperature: None,
            subchat_tokens_for_rag: 0,
            subchat_reasoning_effort: None,
        };

        let err = resolve_subchat_model(gcx, &params).await.unwrap_err();
        assert!(err.contains("Thinking model 'openai/o1' is misconfigured"));
        assert!(err.contains("an invalid LLM endpoint URL"));
        assert!(err.contains("Default model settings"));
    }

    #[tokio::test]
    async fn test_resolve_subchat_params_names_gather_files_light_model_requirement() {
        let gcx = make_test_gcx().await;
        let config_dir = gcx.read().await.config_dir.clone();
        global_configs_try_create_all(&config_dir).await.unwrap();

        let thinking_model_id = "openai/gpt-5".to_string();
        let mut caps = CodeAssistantCaps::default();
        caps.chat_models.insert(
            thinking_model_id.clone(),
            chat_model_record(
                &thinking_model_id,
                128_000,
                "https://api.openai.com/v1/chat/completions",
            ),
        );
        caps.defaults.chat_default_model = thinking_model_id.clone();
        caps.defaults.chat_thinking_model = thinking_model_id;

        install_caps(gcx.clone(), caps).await;

        let err = resolve_subchat_params(gcx, "code_review_gather_files")
            .await
            .unwrap_err();

        assert!(err.contains("Light model required by subagent 'code_review_gather_files'"));
        assert!(err.contains("model_type: light"));
        assert!(err.contains("Default model settings"));
    }
}
