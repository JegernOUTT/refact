use std::sync::Arc;

use crate::call_validation::{ChatMessage, ContextFile, PostprocessSettings};
use crate::caps::resolve_chat_model;
use crate::files_in_workspace::get_file_text_from_memory_or_disk;
use crate::global_context::{try_load_caps_quickly_if_not_present, GlobalContext};
use crate::postprocessing::pp_context_files::postprocess_context_files;
use crate::subchat::{run_subchat_once_with_explicit_params, ExplicitSubchatSpec, TraceParent};
use crate::tokens::count_text_tokens_with_fallback;
use crate::tools::review_agents::{now_ms, AgentCtx, AgentOutcome};
use crate::tools::review_candidates::parse_candidates_with_reasons;
use crate::tools::review_evidence::collect_evidence;
use crate::tools::review_scope::ReviewScope;
use crate::tools::review_verify::{verify_findings, VerifierRunner};
use crate::tools::subagent_phases::get_last_assistant_content;

pub const CANDIDATE_RETRY_PROMPT: &str = r#"Your response did not end with a valid candidate envelope. Return the review again and end with exactly one fenced json block matching this schema:

```json
{"summary":"<2-4 sentences>","candidates":[{"file":"<path as given>","line1":1,"line2":10,"category":"correctness|consistency|security|tests|maintainability|performance|spec_compliance","severity":"low|medium|high|critical","confidence":0.0,"claim":"<one falsifiable sentence>","rationale":"<short why>"}]}
```

Use an empty candidates array when no issue is plausible. Do not put another fenced json block before or after the envelope."#;

const TOKENS_EXTRA_BUDGET_PERCENT: f32 = 0.06;
const DIFF_BUDGET_SHARE: f32 = 0.35;

fn budget_components(spec: &ExplicitSubchatSpec) -> (usize, usize) {
    let params = &spec.params;
    let extra = (params.subchat_n_ctx as f32 * TOKENS_EXTRA_BUDGET_PERCENT) as usize;
    let required = params.subchat_max_new_tokens + params.subchat_tokens_for_rag + extra;
    (extra, required)
}

fn validate_budget(spec: &ExplicitSubchatSpec) -> Result<i64, String> {
    let (extra, required) = budget_components(spec);
    if required < spec.params.subchat_n_ctx {
        return Ok(spec.params.subchat_n_ctx as i64 - required as i64);
    }
    Err(format!(
        "Bad subchat budget: max_new_tokens({}) + tokens_for_rag({}) + extra({}) = {} >= n_ctx({})",
        spec.params.subchat_max_new_tokens,
        spec.params.subchat_tokens_for_rag,
        extra,
        required,
        spec.params.subchat_n_ctx
    ))
}

fn truncate_to_tokens(
    tokenizer: Option<Arc<tokenizers::Tokenizer>>,
    text: &str,
    budget: i64,
) -> String {
    if budget <= 0 {
        return String::new();
    }
    if (count_text_tokens_with_fallback(tokenizer.clone(), text) as i64) <= budget {
        return text.to_string();
    }
    let mut kept: String = String::new();
    for line in text.lines() {
        let candidate = format!("{kept}{line}\n");
        if (count_text_tokens_with_fallback(tokenizer.clone(), &candidate) as i64) > budget {
            break;
        }
        kept = candidate;
    }
    kept.push_str("\n[diff truncated to fit the review budget]");
    kept
}

pub(crate) async fn make_review_prompt(
    gcx: Arc<GlobalContext>,
    spec: &ExplicitSubchatSpec,
    scope: &ReviewScope,
    previous_messages: &[ChatMessage],
    instruction: &str,
) -> Result<String, String> {
    let caps = try_load_caps_quickly_if_not_present(gcx.clone(), 0)
        .await
        .map_err(|e| format!("failed to load caps: {:?}", e))?;
    let model_rec = resolve_chat_model(caps, &spec.model)?;
    let tokenizer = crate::tokens::cached_tokenizer(gcx.clone(), &model_rec.base)
        .await
        .map_err(|e| format!("tokenizer: {e}"))?;

    let mut tokens_budget = validate_budget(spec)?;

    let mut final_message = match scope.focus.as_deref() {
        Some(focus) => format!("{instruction}\n\n# Focus \u{2014} what to check\n{focus}"),
        None => instruction.to_string(),
    };
    final_message.push_str(&format!(
        "\n\n# Candidate limit\nReturn at most {} candidates.",
        scope.budgets.max_candidates
    ));
    tokens_budget -= count_text_tokens_with_fallback(tokenizer.clone(), &final_message) as i64;

    let diff_section = match scope.diff_patch.as_deref() {
        Some(patch) if !patch.trim().is_empty() => {
            let diff_budget = ((tokens_budget as f32) * DIFF_BUDGET_SHARE) as i64;
            let truncated = truncate_to_tokens(tokenizer.clone(), patch, diff_budget);
            if truncated.is_empty() {
                String::new()
            } else {
                let base = scope.diff_base.as_deref().unwrap_or("unknown");
                let section =
                    format!("\n\n# Diff under review (base {base})\n```diff\n{truncated}\n```");
                tokens_budget -=
                    count_text_tokens_with_fallback(tokenizer.clone(), &section) as i64;
                section
            }
        }
        _ => String::new(),
    };

    let mut context = String::new();
    let mut context_files = vec![];

    for p in scope.files.iter() {
        match get_file_text_from_memory_or_disk(gcx.clone(), p).await {
            Ok(text) => {
                let total_lines = text.lines().count();
                context_files.push(ContextFile {
                    file_name: p.to_string_lossy().to_string(),
                    file_content: String::new(),
                    line1: 1,
                    line2: total_lines.max(1),
                    file_rev: None,
                    symbols: vec![],
                    gradient_type: 4,
                    usefulness: 100.0,
                    skip_pp: false,
                });
            }
            Err(_) => {
                tracing::warn!("review: failed to read file '{:?}'", p);
            }
        }
    }

    for message in previous_messages.iter().rev() {
        let message_row = match message.role.as_str() {
            "system" => continue,
            "user" => format!(
                "👤:\n{}\n\n",
                &message.content.to_text_with_image_placeholders()
            ),
            "assistant" => format!(
                "🤖:\n{}\n\n",
                &message.content.to_text_with_image_placeholders()
            ),
            "tool" => format!(
                "📎:\n{}\n\n",
                &message.content.to_text_with_image_placeholders()
            ),
            _ => continue,
        };
        let left_tokens =
            tokens_budget - count_text_tokens_with_fallback(tokenizer.clone(), &message_row) as i64;
        if left_tokens >= 0 {
            tokens_budget = left_tokens;
            context.insert_str(0, &message_row);
        }
    }

    if !context_files.is_empty() {
        let mut pp_settings = PostprocessSettings::new();
        pp_settings.max_files_n = context_files.len();
        let mut files_context = String::new();
        let (pp_files, _notes) = postprocess_context_files(
            gcx.clone(),
            &mut context_files,
            tokenizer.clone(),
            spec.params.subchat_tokens_for_rag + tokens_budget.max(0) as usize,
            false,
            &pp_settings,
        )
        .await;

        for context_file in pp_files {
            files_context.push_str(&format!(
                "📎 {}:{}-{}\n```\n{}```\n\n",
                context_file.file_name,
                context_file.line1,
                context_file.line2,
                context_file.file_content
            ));
        }
        Ok(format!(
            "{final_message}{diff_section}\n\n# Conversation\n{context}\n\n# Files to Review\n{files_context}"
        ))
    } else {
        Ok(format!(
            "{final_message}{diff_section}\n\n# Conversation\n{context}"
        ))
    }
}

pub struct OneshotInstance {
    pub agent_id: String,
    pub slot_label: String,
    pub spec: ExplicitSubchatSpec,
    pub instruction: String,
}

pub(crate) async fn run_oneshot_instance(
    gcx: Arc<GlobalContext>,
    ctx: AgentCtx,
    instance: OneshotInstance,
    scope: Arc<ReviewScope>,
    external_messages: Arc<Vec<ChatMessage>>,
    verifier: Option<Arc<VerifierRunner>>,
) -> AgentOutcome {
    let started = now_ms();
    let agent_label = format!("{}@{}", instance.agent_id, instance.slot_label);
    let attribution = format!("review_{}", instance.agent_id);
    let mut metering = serde_json::Map::new();

    let prompt = match make_review_prompt(
        gcx.clone(),
        &instance.spec,
        &scope,
        &external_messages,
        &instance.instruction,
    )
    .await
    {
        Ok(prompt) => prompt,
        Err(error) => {
            return AgentOutcome::failed(&agent_label, &error, started);
        }
    };

    let history = vec![ChatMessage::new("user".to_string(), prompt)];
    let result = match run_subchat_once_with_explicit_params(
        gcx.clone(),
        &attribution,
        &instance.spec,
        history,
        ctx.tool_call_id.clone(),
        ctx.subchat_tx.clone(),
        ctx.abort_flag.clone(),
        ctx.depth,
        ctx.task_meta.clone(),
        ctx.worktree.clone(),
        TraceParent::rooted(&ctx.chat_id, &ctx.root_chat_id),
    )
    .await
    {
        Ok(result) => result,
        Err(error) => {
            return AgentOutcome::failed(&agent_label, &error, started);
        }
    };

    let text = get_last_assistant_content(&result.messages);
    crate::tools::review_verify::merge_metering(&mut metering, result.metering.clone());

    let parsed = match parse_candidates_with_reasons(&text) {
        Ok(parsed) => Some(parsed),
        Err(first_error) => {
            tracing::info!(
                "{agent_label}: candidate response malformed ({first_error}), requesting retry"
            );
            let mut retry_messages = result.messages;
            retry_messages.push(ChatMessage::new(
                "user".to_string(),
                CANDIDATE_RETRY_PROMPT.to_string(),
            ));
            match run_subchat_once_with_explicit_params(
                gcx.clone(),
                &attribution,
                &instance.spec,
                retry_messages,
                ctx.tool_call_id.clone(),
                ctx.subchat_tx.clone(),
                ctx.abort_flag.clone(),
                ctx.depth,
                ctx.task_meta.clone(),
                ctx.worktree.clone(),
                TraceParent::rooted(&ctx.chat_id, &ctx.root_chat_id),
            )
            .await
            {
                Ok(retry) => {
                    let retry_text = get_last_assistant_content(&retry.messages);
                    crate::tools::review_verify::merge_metering(&mut metering, retry.metering);
                    parse_candidates_with_reasons(&retry_text).ok()
                }
                Err(retry_error) => {
                    tracing::warn!("{agent_label}: candidate retry failed: {retry_error}");
                    None
                }
            }
        }
    };

    let Some(parsed) = parsed else {
        let mut outcome = AgentOutcome::failed(&agent_label, "candidates_parse_failed", started);
        outcome.metering = metering;
        return outcome;
    };

    let mut findings = Vec::with_capacity(parsed.candidates.len());
    for candidate in parsed
        .candidates
        .into_iter()
        .take(scope.budgets.max_candidates)
    {
        let (mut finding, _rationale) = candidate.into_review_parts();
        finding.sources = vec![agent_label.clone()];
        findings.push(finding);
    }
    let candidates_count = findings.len();

    let rejections = collect_evidence(gcx.clone(), &scope, &mut findings).await;
    let mut checks: Vec<String> = rejections
        .iter()
        .map(|rejection| rejection.check_name())
        .collect();

    if let Some(verifier) = verifier {
        verify_findings(
            gcx.clone(),
            &ctx.verify_parent(),
            &verifier,
            &mut findings,
            &mut checks,
            &mut metering,
        )
        .await;
    }

    let mut outcome = AgentOutcome::ran(
        &agent_label,
        Some(instance.spec.model.clone()),
        candidates_count,
        findings,
        started,
    );
    outcome.checks = checks;
    outcome.metering = metering;
    if instance.agent_id == "l3_spec" && !parsed.summary.trim().is_empty() {
        outcome.assumed_intent = Some(parsed.summary.trim().to_string());
    }
    outcome
}
