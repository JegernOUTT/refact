pub use refact_agentic::mode_transition::{
    ConversationMetadata, FileReference, ParsedDecisions, TransitionContextBudget,
    assemble_new_chat as assemble_new_chat_pure, calculate_transition_context_budget,
    carried_goal_messages, carried_plan_messages, context_file_rendered_symbols,
    count_images_in_messages, current_base_goal_message, extract_conversation_metadata,
    extract_initial_plan_text, format_annotated_messages, format_budget_summary, format_file_list,
    format_memory_list, insert_goal_messages_before_plan, is_goal_delta_event,
    is_live_agents_snapshot, message_symbols, normalize_control_metadata,
    normalize_goal_message_content, parse_llm_response, push_context_file_with_budget,
    text_symbols, transfer_goal_ownership, GoalTransferResult, truncate_utf8,
};

use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use refact_context_api::PathsAccess;

use crate::call_validation::{ChatContent, ChatMessage};
use crate::global_context::GlobalContext;
use crate::subchat::{
    run_subchat, resolve_subchat_config_with_explicit_params, load_subchat_params,
    resolve_subchat_model, ExplicitSubchatSpec, TraceParent,
};
use crate::yaml_configs::customization_registry::get_subagent_config;

const SUBAGENT_ID: &str = "mode_transition";

#[derive(Clone, Debug)]
pub struct AgenticPathContext {
    cache_dir: PathBuf,
    config_dir: PathBuf,
    workspace_folders: Vec<PathBuf>,
}

impl AgenticPathContext {
    pub fn from_context<T: PathsAccess + ?Sized>(context: &T) -> Self {
        Self {
            cache_dir: context.cache_dir(),
            config_dir: context.config_dir(),
            workspace_folders: context.workspace_folders(),
        }
    }
}

impl PathsAccess for AgenticPathContext {
    fn cache_dir(&self) -> PathBuf {
        self.cache_dir.clone()
    }

    fn config_dir(&self) -> PathBuf {
        self.config_dir.clone()
    }

    fn workspace_folders(&self) -> Vec<PathBuf> {
        self.workspace_folders.clone()
    }
}

pub type ReconstructionHints = ParsedDecisions;

pub struct ReconstructionRequest<'a> {
    pub messages: &'a [ChatMessage],
    pub target_mode: &'a str,
    pub target_mode_description: &'a str,
    pub parent_chat_id: Option<&'a str>,
    pub model_override: Option<String>,
    pub chat_model: Option<String>,
    pub abort_flag: Option<Arc<AtomicBool>>,
    pub hints: Option<ReconstructionHints>,
    pub target_budget_symbols: Option<usize>,
    pub preserve_goal_messages: bool,
    pub live_agents_scope: Option<&'a str>,
}

pub struct ReconstructionOutcome {
    pub messages: Vec<ChatMessage>,
    pub decisions: ParsedDecisions,
    pub model: String,
    pub budget: TransitionContextBudget,
    pub reread_paths: Vec<String>,
}

fn check_cancelled(request: &ReconstructionRequest<'_>) -> Result<(), String> {
    if request
        .abort_flag
        .as_ref()
        .is_some_and(|f| f.load(Ordering::Acquire))
    {
        Err("Context reconstruction cancelled".into())
    } else {
        Ok(())
    }
}

struct MessageAudit<'a>(&'a [ChatMessage]);
impl refact_privacy::PrivacyAudited for MessageAudit<'_> {
    fn privacy_records(
        &self,
    ) -> Result<Vec<(usize, refact_privacy::FileRecord)>, refact_privacy::PrivacyAuditError> {
        refact_privacy::records_from_messages(self.0)
    }
}

const ANALYSIS_PREVIEW_CAP: usize = 16000;
const ANALYSIS_PREVIEW_FLOOR: usize = 250;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AnalysisRenderStep {
    preview_cap: usize,
    keep_recent: usize,
    control_cap: Option<usize>,
}

/// Degradation ladder for the analysis prompt: shorten previews first, then omit the
/// oldest non-mandatory messages, and only as a last resort truncate mandatory artifacts
/// (the backend carries plan/goal controls verbatim regardless of what the analyzer saw).
fn analysis_render_steps(non_mandatory: usize) -> impl Iterator<Item = AnalysisRenderStep> {
    let previews = std::iter::successors(Some(ANALYSIS_PREVIEW_CAP), |cap| {
        (*cap > ANALYSIS_PREVIEW_FLOOR).then(|| (cap / 2).max(ANALYSIS_PREVIEW_FLOOR))
    })
    .map(move |preview_cap| AnalysisRenderStep {
        preview_cap,
        keep_recent: non_mandatory,
        control_cap: None,
    });
    let omissions = std::iter::successors(Some(non_mandatory / 2), |keep| {
        (*keep > 0).then(|| keep / 2)
    })
    .map(|keep_recent| AnalysisRenderStep {
        preview_cap: ANALYSIS_PREVIEW_FLOOR,
        keep_recent,
        control_cap: None,
    });
    let controls = std::iter::successors(Some(ANALYSIS_PREVIEW_CAP), |cap| {
        (*cap > 0).then(|| cap / 2)
    })
    .map(|cap| AnalysisRenderStep {
        preview_cap: ANALYSIS_PREVIEW_FLOOR,
        keep_recent: 0,
        control_cap: Some(cap),
    });
    previews.chain(omissions).chain(controls)
}

struct AnalysisEntry<'a> {
    id: &'a str,
    message: &'a ChatMessage,
    text: String,
    mandatory: bool,
}

fn is_control_message(message: &ChatMessage) -> bool {
    matches!(message.role.as_str(), "plan" | "goal")
        || message
            .extra
            .get("event")
            .and_then(|e| e.get("subkind"))
            .and_then(|s| s.as_str())
            .is_some_and(|s| matches!(s, "plan_delta" | "goal_delta" | "goal_pursuit"))
}

pub(crate) fn replace_live_agents_context(
    messages: &[ChatMessage],
    snapshot: Option<ChatMessage>,
) -> Vec<ChatMessage> {
    messages
        .iter()
        .filter(|message| !is_live_agents_snapshot(message))
        .cloned()
        .chain(snapshot)
        .collect()
}

/// The current request and plan/goal controls are mandatory; everything else gets a
/// bounded preview and may be omitted when the conversation is too long.
fn analysis_entries(metadata: &ConversationMetadata) -> Vec<AnalysisEntry<'_>> {
    let latest_user = metadata
        .annotated_messages
        .iter()
        .rposition(|(_, m)| m.role == "user");
    metadata
        .annotated_messages
        .iter()
        .enumerate()
        .map(|(index, (id, message))| AnalysisEntry {
            id,
            message,
            text: if is_live_agents_snapshot(message) {
                format!(
                    "{}\n{}",
                    message.content.content_text_only(),
                    message
                        .extra
                        .get("event")
                        .and_then(|event| event.get("payload"))
                        .map(|payload| payload.to_string())
                        .unwrap_or_default()
                )
            } else {
                refact_core::edit_results::project_edit_result(message)
                    .content
                    .content_text_only()
            },
            mandatory: Some(index) == latest_user
                || is_control_message(message)
                || is_live_agents_snapshot(message),
        })
        .collect()
}

fn render_annotated_messages(entries: &[AnalysisEntry<'_>], step: AnalysisRenderStep) -> String {
    let first_kept = if step.keep_recent == 0 {
        usize::MAX
    } else {
        entries
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.mandatory)
            .map(|(i, _)| i)
            .rev()
            .nth(step.keep_recent - 1)
            .unwrap_or(0)
    };
    let mut out = String::new();
    let mut omitted: Option<(usize, usize, usize)> = None;
    let flush = |out: &mut String, omitted: &mut Option<(usize, usize, usize)>| {
        if let Some((first, last, count)) = omitted.take() {
            if count == 1 {
                out.push_str(&format!("[{}] older message omitted\n", entries[first].id));
            } else {
                out.push_str(&format!(
                    "[{}..{}] {count} older messages omitted\n",
                    entries[first].id, entries[last].id
                ));
            }
        }
    };
    for (index, entry) in entries.iter().enumerate() {
        if !entry.mandatory && index < first_kept {
            omitted = Some(match omitted {
                Some((first, _, count)) => (first, index, count + 1),
                None => (index, index, 1),
            });
            continue;
        }
        flush(&mut out, &mut omitted);
        let text: std::borrow::Cow<str> = match (entry.mandatory, step.control_cap) {
            (true, None) => std::borrow::Cow::Borrowed(&entry.text),
            (true, Some(cap)) => truncate_utf8(&entry.text, cap).into(),
            (false, _) => truncate_utf8(&entry.text, step.preview_cap).into(),
        };
        out.push_str(&format!(
            "[{}] [{}]\n{text}\n",
            entry.id, entry.message.role
        ));
        for call in entry.message.tool_calls.iter().flatten() {
            out.push_str(&format!(
                "tool: {} {}\n",
                call.function.name,
                truncate_utf8(&call.function.arguments, step.preview_cap)
            ));
        }
    }
    flush(&mut out, &mut omitted);
    out
}

/// Consumes ladder steps until one fits, leaving the iterator positioned after it so a
/// provider rejection can resume trimming from the next step.
fn fit_analysis_prompt(
    steps: &mut impl Iterator<Item = AnalysisRenderStep>,
    input_limit: usize,
    render: impl Fn(AnalysisRenderStep) -> String,
    mut count_tokens: impl FnMut(&str) -> Result<usize, String>,
) -> Result<String, String> {
    for step in steps {
        let prompt = render(step);
        if count_tokens(&prompt)? <= input_limit {
            return Ok(prompt);
        }
    }
    Err("Mandatory reconstruction input and instructions exceed resolved analyzer context".into())
}

const ANALYSIS_DEFAULT_BYTES_PER_TOKEN: f64 = 2.0;
const ANALYSIS_CALIBRATION_MIN_TOKENS: usize = 50_000;
const ANALYSIS_CALIBRATION_MARGIN: f64 = 1.25;
const ANALYSIS_CONTEXT_REJECTION_RETRIES: usize = 3;

fn usage_input_tokens(message: &ChatMessage) -> Option<usize> {
    let usage = message
        .usage
        .as_ref()
        .filter(|_| message.role == "assistant")?;
    let input = usage
        .prompt_tokens
        .saturating_add(usage.cache_read_tokens.unwrap_or(0))
        .saturating_add(usage.cache_creation_tokens.unwrap_or(0));
    (input > 0).then_some(input)
}

fn message_bytes(message: &ChatMessage) -> usize {
    let calls = message
        .tool_calls
        .as_ref()
        .and_then(|calls| serde_json::to_string(calls).ok())
        .map_or(0, |calls| calls.len());
    message.content.content_text_only().len() + calls
}

/// Provider-reported usage on the conversation's own messages is the only ground truth
/// available without a local tokenizer: bytes per token measured on the context the
/// model actually saw, discounted because the analyzer may run on a denser tokenizer.
fn usage_calibrated_bytes_per_token(messages: &[ChatMessage]) -> Option<f64> {
    let start = messages
        .iter()
        .rposition(|m| m.role == refact_core::active_context::COMPRESSION_REPORT_KEY)
        .map_or(0, |index| index + 1);
    let (index, input_tokens) =
        messages
            .iter()
            .enumerate()
            .skip(start)
            .rev()
            .find_map(|(index, message)| {
                usage_input_tokens(message)
                    .filter(|input| *input >= ANALYSIS_CALIBRATION_MIN_TOKENS)
                    .map(|input| (index, input))
            })?;
    let seen =
        crate::chat::linearize::apply_summarization_linearize(messages[..index].to_vec()).ok()?;
    let bytes = seen.iter().map(message_bytes).sum::<usize>();
    let bytes_per_token = bytes as f64 / input_tokens as f64 / ANALYSIS_CALIBRATION_MARGIN;
    (1.5..=6.0)
        .contains(&bytes_per_token)
        .then_some(bytes_per_token)
}

fn analysis_prompt_tokens(
    tokenizer: Option<Arc<tokenizers::Tokenizer>>,
    bytes_per_token: f64,
    prompt: &str,
) -> Result<usize, String> {
    if tokenizer.is_some() {
        return crate::tokens::count_text_tokens(tokenizer, prompt);
    }
    Ok((prompt.len() as f64 / bytes_per_token).ceil() as usize + 1)
}

fn is_context_rejection(error: &str) -> bool {
    refact_core::retry_policy::classify_user_error(error)
        == refact_core::retry_policy::UserErrorCategory::ContextTooLarge
}

async fn analyze_reconstruction(
    gcx: Arc<GlobalContext>,
    request: &ReconstructionRequest<'_>,
) -> Result<(ParsedDecisions, String, TransitionContextBudget), String> {
    check_cancelled(request)?;
    if request.messages.is_empty() {
        return Err("The provided chat is empty".into());
    }
    let subagent = get_subagent_config(gcx.clone(), SUBAGENT_ID, None)
        .await
        .ok_or("mode_transition config not found")?;
    let template = subagent
        .messages
        .user_template
        .as_ref()
        .ok_or("Missing reconstruction prompt")?;
    let params = load_subchat_params(gcx.clone(), SUBAGENT_ID).await?;
    let caps = crate::global_context::try_load_caps_quickly_if_not_present(gcx.clone(), 0)
        .await
        .map_err(|e| e.to_string())?;
    let candidates = analysis_model_candidates(
        request.model_override.as_deref(),
        request.chat_model.as_deref(),
        &caps.defaults,
    );
    let mut budget = calculate_transition_context_budget(request.messages);
    if let Some(total) = request.target_budget_symbols {
        budget.total_symbols = total;
        budget.files_symbols = total * 70 / 100;
        budget.messages_symbols = total - budget.files_symbols;
    }
    let metadata = extract_conversation_metadata(request.messages);
    let hints = request
        .hints
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let entries = analysis_entries(&metadata);
    let non_mandatory = entries.iter().filter(|e| !e.mandatory).count();
    let file_list = format_file_list(&metadata);
    let memory_list = format_memory_list(&metadata);
    let budget_summary = format_budget_summary(budget, request.messages);
    let render = |step: AnalysisRenderStep| {
        format!(
            "{}\nCaller preservation hints (source-grounded only): {}",
            template
                .replace("{target_mode}", request.target_mode)
                .replace("{target_mode_description}", request.target_mode_description)
                .replace(
                    "{annotated_message_list}",
                    &render_annotated_messages(&entries, step)
                )
                .replace("{file_list}", &file_list)
                .replace("{memory_list}", &memory_list)
                .replace("{budget_summary}", &budget_summary)
                .replace(
                    "{live_agents_policy}",
                    if request.live_agents_scope.is_some() {
                        "A fresh live-subagents event (source agents.compression_context) is carried verbatim by the backend. Reference its exact agent IDs and pending questions where they drive next actions; do not restate its payload."
                    } else {
                        "Any live-subagents event in source history is historical context, not a mandatory carried snapshot. Preserve relevant exact agent IDs, ownership and pending questions in the summary/pending tasks. Agents remain owned by their original chat; do not imply transfer."
                    },
                ),
            hints
        )
    };
    let bytes_per_token = usage_calibrated_bytes_per_token(request.messages)
        .unwrap_or(ANALYSIS_DEFAULT_BYTES_PER_TOKEN);
    let mut last_error = String::from("No analysis model is configured");
    for model in candidates {
        check_cancelled(request)?;
        let attempt = AnalysisAttempt {
            params: params.clone(),
            model: &model,
            non_mandatory,
            bytes_per_token,
            render: &render,
        };
        match analyze_with_model(gcx.clone(), request, attempt).await {
            Ok((decisions, resolved)) => return Ok((decisions, resolved, budget)),
            Err(error) => {
                tracing::warn!(
                    "reconstruction analysis with '{model}' failed: {}",
                    refact_chat_api::diagnostics::safe_provider_error_diagnostic(&error)
                );
                last_error = error;
            }
        }
    }
    Err(last_error)
}

/// An explicitly requested model is used as-is; otherwise the chat's own model is tried
/// first, then the default chat slots.
fn analysis_model_candidates(
    explicit: Option<&str>,
    chat_model: Option<&str>,
    defaults: &crate::caps::DefaultModels,
) -> Vec<String> {
    if let Some(explicit) = explicit.map(str::trim).filter(|model| !model.is_empty()) {
        return vec![explicit.to_string()];
    }
    let mut candidates: Vec<String> = Vec::new();
    for model in [
        chat_model,
        Some(defaults.chat_default_model.as_str()),
        Some(defaults.chat_model_2.as_str()),
    ]
    .into_iter()
    .flatten()
    .map(str::trim)
    {
        if !model.is_empty() && !candidates.iter().any(|known| known == model) {
            candidates.push(model.to_string());
        }
    }
    candidates
}

struct AnalysisAttempt<'a, R: Fn(AnalysisRenderStep) -> String> {
    params: crate::call_validation::SubchatParameters,
    model: &'a str,
    non_mandatory: usize,
    bytes_per_token: f64,
    render: &'a R,
}

async fn analyze_with_model<R: Fn(AnalysisRenderStep) -> String>(
    gcx: Arc<GlobalContext>,
    request: &ReconstructionRequest<'_>,
    mut attempt: AnalysisAttempt<'_, R>,
) -> Result<(ParsedDecisions, String), String> {
    attempt.params.subchat_model = attempt.model.to_string();
    let model = resolve_subchat_model(gcx.clone(), &attempt.params).await?;
    let spec = ExplicitSubchatSpec {
        params: attempt.params,
        model: model.clone(),
        autonomous_no_confirm: false,
    };
    let mut config = resolve_subchat_config_with_explicit_params(
        gcx.clone(),
        SUBAGENT_ID,
        &spec,
        false,
        None,
        None,
        None,
        None,
        None,
        Some(vec![]),
        1,
        false,
        "agent".into(),
        None,
        None,
        None,
        None,
        request.abort_flag.clone(),
        0,
    )
    .await?;
    config.trace_parent = TraceParent::from_parts(request.parent_chat_id, None);
    let caps = crate::global_context::try_load_caps_quickly_if_not_present(gcx.clone(), 0)
        .await
        .map_err(|e| e.to_string())?;
    let model_rec = crate::caps::resolve_chat_model(caps, &model)?;
    let tokenizer = crate::tokens::cached_tokenizer(gcx.clone(), &model_rec.base).await?;
    let input_limit = config
        .n_ctx
        .saturating_sub(config.max_new_tokens)
        .saturating_sub(1024);
    let mut steps = analysis_render_steps(attempt.non_mandatory);
    let mut rejections_left = ANALYSIS_CONTEXT_REJECTION_RETRIES;
    let result = loop {
        let prompt = fit_analysis_prompt(&mut steps, input_limit, attempt.render, |prompt| {
            analysis_prompt_tokens(tokenizer.clone(), attempt.bytes_per_token, prompt)
        })?;
        let mut analysis = ChatMessage::new("user".into(), prompt);
        crate::privacy::records::carry_records_into(&mut analysis, request.messages)
            .map_err(|e| e.to_string())?;
        crate::privacy::destinations::clear_for_model(
            &gcx,
            MessageAudit(std::slice::from_ref(&analysis)),
            &model_rec.base,
        )
        .map_err(|e| e.to_string())?;
        check_cancelled(request)?;
        match run_subchat(gcx.clone(), vec![analysis], config.clone()).await {
            Ok(result) => break result,
            Err(error) if rejections_left > 0 && is_context_rejection(&error) => {
                rejections_left -= 1;
                tracing::warn!(
                    "reconstruction analysis rejected as too long, trimming further: {}",
                    refact_chat_api::diagnostics::safe_provider_error_diagnostic(&error)
                );
            }
            Err(error) => return Err(error),
        }
    };
    check_cancelled(request)?;
    let response = result
        .messages
        .last()
        .filter(|m| m.role == "assistant" && m.tool_calls.as_ref().is_none_or(|c| c.is_empty()))
        .ok_or("No reconstruction analysis answer generated")?;
    let decisions = parse_llm_response(&response.content.content_text_only());
    refact_agentic::mode_transition::validate_decisions(&decisions, request.messages)?;
    Ok((decisions, model))
}

pub async fn reconstruct_context(
    gcx: Arc<GlobalContext>,
    request: ReconstructionRequest<'_>,
) -> Result<ReconstructionOutcome, String> {
    check_cancelled(&request)?;
    let live_before = match request.live_agents_scope {
        Some(scope) => {
            Some(crate::agents::compression_context::collect_live_agents(gcx.clone(), scope).await?)
        }
        None => None,
    };
    let source = match &live_before {
        Some(snapshot) => replace_live_agents_context(request.messages, snapshot.to_message()),
        None => request.messages.to_vec(),
    };
    let request = ReconstructionRequest {
        messages: &source,
        target_mode: request.target_mode,
        target_mode_description: request.target_mode_description,
        parent_chat_id: request.parent_chat_id,
        model_override: request.model_override,
        chat_model: request.chat_model,
        abort_flag: request.abort_flag,
        hints: request.hints,
        target_budget_symbols: request.target_budget_symbols,
        preserve_goal_messages: request.preserve_goal_messages,
        live_agents_scope: request.live_agents_scope,
    };
    let (mut decisions, model, budget) = analyze_reconstruction(gcx.clone(), &request).await?;
    let live_agents = match (request.live_agents_scope, live_before) {
        (Some(scope), Some(before)) => {
            let after =
                crate::agents::compression_context::collect_live_agents(gcx.clone(), scope).await?;
            after.merged_with_earlier(before).to_message()
        }
        _ => None,
    };
    let workspace_dirs = gcx.workspace_folders();
    let metadata = extract_conversation_metadata(request.messages);
    let allowed: std::collections::HashSet<_> = metadata
        .context_files
        .iter()
        .chain(&metadata.edited_files)
        .map(|f| f.path.clone())
        .chain(metadata.memory_paths.iter().cloned())
        .collect();
    let caps = crate::global_context::try_load_caps_quickly_if_not_present(gcx.clone(), 0)
        .await
        .map_err(|e| e.to_string())?;
    let model_rec = crate::caps::resolve_chat_model(caps, &model)?;
    let mut reread_records = std::collections::HashMap::new();
    let mut preloaded = std::collections::HashMap::new();
    // Only canonical, conversation-allowlisted, runtime-cleared paths may be read by assembly.
    for paths in [
        &mut decisions.files_to_open,
        &mut decisions.memories_to_include,
    ] {
        let mut safe = Vec::new();
        for path in paths.iter() {
            check_cancelled(&request)?;
            if !allowed.contains(path) {
                continue;
            }
            let full = PathBuf::from(path);
            let full = if full.is_absolute() {
                full
            } else if let Some(root) = workspace_dirs.first() {
                root.join(full)
            } else {
                continue;
            };
            let Ok(canonical) = full.canonicalize() else {
                continue;
            };
            let in_workspace = workspace_dirs.iter().any(|root| {
                root.canonicalize()
                    .is_ok_and(|root| canonical.starts_with(root))
            });
            let in_refact = canonical
                .components()
                .any(|c| matches!(c, std::path::Component::Normal(name) if name == ".refact"));
            if !in_workspace && !in_refact {
                continue;
            }
            crate::files_in_workspace::check_file_privacy_for_model_context(
                gcx.clone(),
                &canonical,
            )
            .await?;
            let record = crate::privacy::records::declared_file_record(&gcx, &canonical)?;
            let mut audit = ChatMessage::default();
            crate::privacy::records::attach_record(&mut audit, record.clone());
            crate::privacy::destinations::clear_for_model(
                &gcx,
                MessageAudit(std::slice::from_ref(&audit)),
                &model_rec.base,
            )
            .map_err(|e| e.to_string())?;
            // Read the same canonical path that was cleared, once; pure assembly must
            // never resolve the original alias again after the privacy check.
            if tokio::fs::metadata(&canonical)
                .await
                .map_err(|e| e.to_string())?
                .len()
                > 1024 * 1024
            {
                continue;
            }
            let content = tokio::fs::read_to_string(&canonical)
                .await
                .map_err(|e| e.to_string())?;
            preloaded.insert(path.clone(), content);
            reread_records.insert(path.clone(), record);
            safe.push(path.clone());
        }
        *paths = safe;
    }
    let mut messages = refact_agentic::mode_transition::assemble_reconstruction_with_mandatory(
        request.messages,
        &decisions,
        request.target_mode,
        &workspace_dirs,
        budget,
        request.preserve_goal_messages,
        &preloaded,
        live_agents.as_slice(),
    )
    .await?;
    let reread_paths = messages
        .iter()
        .filter_map(|m| match &m.content {
            ChatContent::ContextFiles(f) => Some(f.iter().map(|f| f.file_name.clone())),
            _ => None,
        })
        .flatten()
        .collect();
    let carried =
        crate::privacy::records::records_to_carry(request.messages).map_err(|e| e.to_string())?;
    attach_reconstruction_records(&mut messages, &carried, &reread_records);
    if let Some(summary) = messages
        .iter_mut()
        .find(|message| message.extra.contains_key("reconstruction_summary"))
    {
        summary.extra.insert("reconstruction_summary".into(), serde_json::json!({
            "generated_at": chrono::Utc::now().to_rfc3339(),
            "source_first_message_id": request.messages.first().map(|message| &message.message_id),
            "source_last_message_id": request.messages.last().map(|message| &message.message_id),
            "source_message_count": request.messages.len(),
        }));
    }
    check_cancelled(&request)?;
    Ok(ReconstructionOutcome {
        messages,
        decisions,
        model,
        budget,
        reread_paths,
    })
}

/// Fresh derived `user` messages carry the source provenance union; fresh context files
/// carry the records of their own re-read files; cloned control messages keep their own.
fn attach_reconstruction_records(
    messages: &mut [ChatMessage],
    carried: &[refact_privacy::FileRecord],
    reread_records: &std::collections::HashMap<String, refact_privacy::FileRecord>,
) {
    let mut carried_once = false;
    for message in messages.iter_mut() {
        if !message.message_id.is_empty() {
            continue;
        }
        message.message_id = uuid::Uuid::new_v4().to_string();
        let records = match &message.content {
            ChatContent::ContextFiles(files) => files
                .iter()
                .filter_map(|file| reread_records.get(&file.file_name).cloned())
                .collect::<Vec<_>>(),
            _ if message.role == "user" && !carried_once => {
                carried_once = true;
                message
                    .extra
                    .insert("reconstruction_summary".into(), serde_json::json!({}));
                carried.to_vec()
            }
            _ => Vec::new(),
        };
        crate::privacy::records::merge_records(message, records);
    }
    crate::privacy::records::compact_reconstruction_privacy(messages);
}

pub async fn analyze_mode_transition(
    gcx: Arc<GlobalContext>,
    messages: &[ChatMessage],
    target_mode: &str,
    target_mode_description: &str,
    parent_chat_id: Option<&str>,
) -> Result<ParsedDecisions, String> {
    analyze_reconstruction(
        gcx,
        &ReconstructionRequest {
            messages,
            target_mode,
            target_mode_description,
            parent_chat_id,
            model_override: None,
            chat_model: None,
            abort_flag: None,
            hints: None,
            target_budget_symbols: None,
            preserve_goal_messages: false,
            live_agents_scope: None,
        },
    )
    .await
    .map(|(decisions, _, _)| decisions)
}

pub async fn assemble_new_chat<T: PathsAccess + ?Sized>(
    context: &T,
    original_messages: &[ChatMessage],
    decisions: &ParsedDecisions,
    target_mode: &str,
) -> Result<Vec<ChatMessage>, String> {
    let workspace_dirs = context.workspace_folders();
    assemble_new_chat_pure(original_messages, decisions, target_mode, &workspace_dirs).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_validation::{ChatContent, ChatMessage, ContextFile};

    #[test]
    fn reconstruction_cancellation_is_checked_before_work() {
        let flag = Arc::new(AtomicBool::new(true));
        let request = ReconstructionRequest {
            messages: &[],
            target_mode: "agent",
            target_mode_description: "",
            parent_chat_id: None,
            model_override: None,
            chat_model: None,
            abort_flag: Some(flag.clone()),
            hints: None,
            target_budget_symbols: None,
            preserve_goal_messages: true,
            live_agents_scope: None,
        };
        assert!(check_cancelled(&request).is_err());
        flag.store(false, Ordering::Release);
        assert!(check_cancelled(&request).is_ok());
    }

    #[test]
    fn replacing_empty_agent_context_removes_stale_snapshot_but_keeps_other_events() {
        let old = crate::chat::internal_roles::event(
            crate::chat::internal_roles::EventSubkind::SystemNotice,
            refact_agentic::mode_transition::LIVE_AGENTS_EVENT_SOURCE,
            serde_json::json!({"agents": [{"agent_id": "old-agent"}]}),
            "old snapshot",
        );
        let other = crate::chat::internal_roles::event(
            crate::chat::internal_roles::EventSubkind::SystemNotice,
            "other.source",
            serde_json::json!({}),
            "keep me",
        );
        let history = vec![text_message("user", "request"), old, other];
        let replaced = replace_live_agents_context(&history, None);
        assert_eq!(replaced.len(), 2);
        assert_eq!(
            serde_json::to_value(&replaced[1]).unwrap(),
            serde_json::to_value(&history[2]).unwrap()
        );
        assert!(!replaced.iter().any(is_live_agents_snapshot));
    }

    fn text_message(role: &str, text: &str) -> ChatMessage {
        ChatMessage::new(role.to_string(), text.to_string())
    }

    fn tool_call_message(name: &str, arguments: &str) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            tool_calls: Some(vec![crate::call_validation::ChatToolCall {
                id: "call-1".to_string(),
                index: None,
                function: crate::call_validation::ChatToolFunction {
                    name: name.to_string(),
                    arguments: arguments.to_string(),
                },
                tool_type: "function".to_string(),
                extra_content: None,
                started_at_ms: None,
                completed_at_ms: None,
            }]),
            ..Default::default()
        }
    }

    fn sample_conversation() -> Vec<ChatMessage> {
        vec![
            text_message("user", "first request"),
            tool_call_message("t_cat", "{\"paths\":\"src/main.rs\"}"),
            text_message("tool", "first tool output"),
            text_message("plan", "PLAN BODY"),
            text_message("assistant", "second answer"),
            text_message("tool", "second tool output"),
            text_message("user", "latest request"),
            text_message("assistant", "third answer"),
        ]
    }

    #[test]
    fn render_annotated_messages_omits_oldest_but_keeps_controls_and_latest_user() {
        let messages = sample_conversation();
        let metadata = extract_conversation_metadata(&messages);
        let entries = analysis_entries(&metadata);
        let rendered = render_annotated_messages(
            &entries,
            AnalysisRenderStep {
                preview_cap: ANALYSIS_PREVIEW_FLOOR,
                keep_recent: 2,
                control_cap: None,
            },
        );
        assert!(rendered.starts_with("[MSG_ID:0..MSG_ID:2] 3 older messages omitted\n"));
        assert!(rendered.contains("[MSG_ID:3] [plan]\nPLAN BODY\n[MSG_ID:4] older message omitted\n[MSG_ID:5] [tool]\nsecond tool output\n"));
        assert!(rendered.contains("[MSG_ID:6] [user]\nlatest request\n"));
        assert!(rendered.contains("[MSG_ID:7] [assistant]\nthird answer\n"));
        assert!(!rendered.contains("first request"));
        assert!(!rendered.contains("tool: t_cat"));
    }

    #[test]
    fn reconstruction_analysis_renders_edit_excerpt_not_raw_diff() {
        let mut diff = text_message("diff", "RAW_RECONSTRUCTION_DIFF_SENTINEL");
        diff.extra.insert(
            "edit_result".into(),
            serde_json::json!({
                "version": 1, "model_content": "FINAL excerpt: safe replacement"
            }),
        );
        let messages = vec![text_message("user", "request"), diff];
        let metadata = extract_conversation_metadata(&messages);
        let entries = analysis_entries(&metadata);
        let rendered = render_annotated_messages(
            &entries,
            AnalysisRenderStep {
                preview_cap: ANALYSIS_PREVIEW_CAP,
                keep_recent: entries.len(),
                control_cap: None,
            },
        );
        assert!(rendered.contains("FINAL excerpt: safe replacement"));
        assert!(!rendered.contains("RAW_RECONSTRUCTION_DIFF_SENTINEL"));
        assert_eq!(
            messages[1].content.content_text_only(),
            "RAW_RECONSTRUCTION_DIFF_SENTINEL"
        );
    }

    #[test]
    fn fit_analysis_prompt_resumes_after_the_fitting_step_and_fails_after_the_ladder() {
        let render = |step: AnalysisRenderStep| {
            format!(
                "{}|{}|{:?}",
                step.preview_cap, step.keep_recent, step.control_cap
            )
        };
        let count = |prompt: &str| Ok(prompt.split('|').next().unwrap().parse::<usize>().unwrap());
        let mut steps = analysis_render_steps(8);
        assert_eq!(
            fit_analysis_prompt(&mut steps, 5000, &render, count).unwrap(),
            "4000|8|None"
        );
        assert_eq!(
            fit_analysis_prompt(&mut steps, 5000, &render, count).unwrap(),
            "2000|8|None"
        );
        let error = fit_analysis_prompt(&mut steps, 10, &render, |_| Ok(11)).unwrap_err();
        assert!(error.contains("exceed resolved analyzer context"));
        assert_eq!(
            fit_analysis_prompt(&mut analysis_render_steps(8), 10, &render, |_| Err(
                "boom".to_string()
            ))
            .unwrap_err(),
            "boom"
        );
    }

    fn metered_assistant(input_tokens: usize) -> ChatMessage {
        let mut message = text_message("assistant", "ok");
        message.usage = Some(crate::call_validation::ChatUsage {
            prompt_tokens: input_tokens / 4,
            cache_read_tokens: Some(input_tokens / 2),
            cache_creation_tokens: Some(input_tokens - input_tokens / 4 - input_tokens / 2),
            ..Default::default()
        });
        message
    }

    #[test]
    fn long_conversation_fits_analyzer_context_without_local_tokenizer() {
        let mut messages = Vec::new();
        for index in 0..1500 {
            messages.push(tool_call_message(
                "t_cat",
                &format!("{{\"paths\":\"src/file_{index}.rs\"}}"),
            ));
            messages.push(text_message(
                "tool",
                &format!("output {index} ").repeat(150),
            ));
            if index == 20 {
                messages.push(text_message("plan", &"PLAN LINE\n".repeat(7000)));
            }
            if index == 300 {
                messages.push(text_message("goal", "GOAL BODY"));
            }
        }
        let visible_bytes = messages.iter().map(message_bytes).sum::<usize>();
        messages.push(metered_assistant(visible_bytes / 3));
        messages.push(text_message("user", "latest request"));
        let calibrated = usage_calibrated_bytes_per_token(&messages).unwrap();
        assert!(
            (calibrated - 3.0 / ANALYSIS_CALIBRATION_MARGIN).abs() < 0.01,
            "{calibrated}"
        );
        let metadata = extract_conversation_metadata(&messages);
        let entries = analysis_entries(&metadata);
        let non_mandatory = entries.iter().filter(|e| !e.mandatory).count();
        let input_limit = 200_000 - 32_000 - 1024;
        let mut kept = Vec::new();
        for bytes_per_token in [calibrated, ANALYSIS_DEFAULT_BYTES_PER_TOKEN] {
            let renders = std::cell::Cell::new(0);
            let prompt = fit_analysis_prompt(
                &mut analysis_render_steps(non_mandatory),
                input_limit,
                |step| {
                    renders.set(renders.get() + 1);
                    render_annotated_messages(&entries, step)
                },
                |prompt| analysis_prompt_tokens(None, bytes_per_token, prompt),
            )
            .unwrap();
            assert!(renders.get() <= 12, "took {} renders", renders.get());
            assert!(analysis_prompt_tokens(None, bytes_per_token, &prompt).unwrap() <= input_limit);
            assert!(prompt.contains(&"PLAN LINE\n".repeat(7000)));
            assert!(prompt.contains("[goal]\nGOAL BODY\n"));
            assert!(prompt.ends_with("[user]\nlatest request\n"));
            assert!(prompt.starts_with("[MSG_ID:0..MSG_ID:"));
            assert!(prompt.contains("older messages omitted\n"));
            assert!(prompt.contains("[tool]\noutput 1499 "));
            kept.push(prompt.matches("\n[MSG_ID:").count());
        }
        assert!(
            kept[0] >= kept[1],
            "calibrated density never keeps less: {kept:?}"
        );
    }

    fn guarded_record(path: &str, zone: &str) -> refact_privacy::FileRecord {
        refact_privacy::FileRecord {
            path: path.to_string(),
            zone: zone.to_string(),
            attribution: refact_privacy::Attribution::Declared,
        }
    }

    fn message_records(message: &ChatMessage) -> Vec<refact_privacy::FileRecord> {
        message
            .extra
            .get("privacy")
            .map(|value| {
                serde_json::from_value::<refact_privacy::PrivacyRecord>(value.clone())
                    .unwrap()
                    .files
            })
            .unwrap_or_default()
    }

    fn context_file_message(file_name: &str) -> ChatMessage {
        ChatMessage {
            role: "context_file".to_string(),
            content: ChatContent::ContextFiles(vec![ContextFile {
                file_name: file_name.to_string(),
                file_content: "fn main() {}\n".to_string(),
                line1: 1,
                line2: 1,
                ..Default::default()
            }]),
            ..Default::default()
        }
    }

    fn cloned_control(
        id: &str,
        role: &str,
        records: Vec<refact_privacy::FileRecord>,
    ) -> ChatMessage {
        let mut message = ChatMessage {
            message_id: id.to_string(),
            role: role.to_string(),
            content: ChatContent::SimpleText(format!("{role} body")),
            ..Default::default()
        };
        crate::privacy::records::merge_records(&mut message, records);
        message
    }

    #[test]
    fn attach_reconstruction_records_stamps_union_once_per_reconstruction() {
        let secret = guarded_record("secret.rs", "secrets");
        let another = guarded_record("another.rs", "secrets");
        let other = guarded_record("other.rs", "secrets");
        let reread = guarded_record("src/a.rs", "secrets");
        let carried = vec![secret.clone(), another.clone()];
        let reread_records =
            std::collections::HashMap::from([("src/a.rs".to_string(), reread.clone())]);
        let mut messages = vec![
            context_file_message("src/a.rs"),
            ChatMessage::new("user".to_string(), "narrative".to_string()),
            ChatMessage::new("user".to_string(), "continuation".to_string()),
            cloned_control("evt-1", "event", vec![secret.clone()]),
            cloned_control("plan-1", "plan", vec![other.clone()]),
        ];

        attach_reconstruction_records(&mut messages, &carried, &reread_records);

        assert_eq!(message_records(&messages[0]), vec![reread.clone()]);
        assert_eq!(message_records(&messages[1]), carried);
        assert!(message_records(&messages[2]).is_empty());
        assert!(messages[3].extra.get("privacy").is_none());
        assert_eq!(message_records(&messages[4]), vec![other.clone()]);
        assert!(messages.iter().all(|m| !m.message_id.is_empty()));
        assert_eq!(
            messages
                .iter()
                .filter(|m| message_records(m) == carried)
                .count(),
            1
        );
        let attached = refact_privacy::records_from_messages(&messages)
            .unwrap()
            .into_iter()
            .map(|(_, record)| record)
            .collect::<std::collections::HashSet<_>>();
        for record in [secret, another, reread, other] {
            assert!(attached.contains(&record), "{record:?} must survive");
        }
    }

    #[test]
    fn reconstruction_guarded_record_still_refuses_untrusted_provider() {
        let policy = refact_privacy::PrivacyPolicy {
            blocked: Vec::new(),
            zones: vec![
                refact_privacy::Zone {
                    name: "secrets".to_string(),
                    patterns: vec!["secret.rs".to_string()],
                    send_to: vec!["trusted".to_string()],
                    on_shell_read: refact_privacy::ShellBehavior::Withhold,
                },
                refact_privacy::Zone {
                    name: "normal".to_string(),
                    patterns: vec!["*".to_string()],
                    send_to: vec!["*".to_string()],
                    on_shell_read: refact_privacy::ShellBehavior::Withhold,
                },
            ],
            subagents: refact_privacy::SubagentPolicy::default(),
            ..Default::default()
        };
        let compiled = policy.compile().unwrap();
        let source = cloned_control(
            "src-1",
            "user",
            vec![guarded_record("secret.rs", "secrets")],
        );
        let carried = crate::privacy::records::records_to_carry(&[source]).unwrap();
        let mut messages = vec![
            context_file_message("src/a.rs"),
            ChatMessage::new("user".to_string(), "narrative".to_string()),
            ChatMessage::new("user".to_string(), "continuation".to_string()),
            cloned_control("evt-1", "event", carried.clone()),
        ];

        attach_reconstruction_records(&mut messages, &carried, &std::collections::HashMap::new());

        let destination = |id: &str| refact_privacy::Destination {
            id: refact_privacy::DestinationId(id.to_string()),
            kind: refact_privacy::DestinationKind::Provider,
            display_name: id.to_string(),
        };
        assert!(
            refact_privacy::clear(
                MessageAudit(&messages),
                &destination("untrusted"),
                &compiled
            )
            .is_err(),
            "compaction must not hide guarded records from the gate"
        );
        assert!(
            refact_privacy::clear(MessageAudit(&messages), &destination("trusted"), &compiled)
                .is_ok()
        );
    }

    struct TestPaths {
        workspace_folders: Vec<PathBuf>,
    }

    impl TestPaths {
        fn new(workspace_folders: Vec<PathBuf>) -> Self {
            Self {
                workspace_folders,
            }
        }
    }

    impl PathsAccess for TestPaths {
        fn cache_dir(&self) -> PathBuf {
            PathBuf::new()
        }

        fn config_dir(&self) -> PathBuf {
            PathBuf::new()
        }

        fn workspace_folders(&self) -> Vec<PathBuf> {
            self.workspace_folders.clone()
        }
    }
}
