use std::sync::Arc;

use crate::call_validation::{ChatContent, ChatMessage};
use crate::subchat::{
    resolve_subchat_config_with_explicit_params, run_subchat,
    run_subchat_once_with_explicit_params, ExplicitSubchatSpec, TraceParent,
};
use crate::tools::review_agents::oneshot::CANDIDATE_RETRY_PROMPT;
use crate::tools::review_agents::{now_ms, AgentCtx, AgentOutcome};
use crate::tools::review_candidates::parse_candidates_with_reasons;
use crate::tools::review_evidence::collect_evidence;
use crate::tools::review_scope::ReviewScope;
use crate::tools::review_verify::{merge_metering, verify_findings, VerifierRunner};
use crate::tools::subagent_phases::get_last_assistant_content;

const DIFF_CHAR_CAP: usize = 20000;
const MAX_LISTED_FILES: usize = 60;

pub fn build_agent_task_prompt(scope: &ReviewScope, extra: Option<&str>) -> String {
    let mut prompt = String::new();
    if let Some(focus) = scope.focus.as_deref() {
        prompt.push_str(&format!("# Focus — what to check\n{focus}\n\n"));
    }
    if let Some(extra) = extra {
        prompt.push_str(extra);
        prompt.push_str("\n\n");
    }
    prompt.push_str("# Files in review scope\n");
    for path in scope.files.iter().take(MAX_LISTED_FILES) {
        prompt.push_str(&format!("- {}\n", path.to_string_lossy()));
    }
    if scope.files.len() > MAX_LISTED_FILES {
        prompt.push_str(&format!(
            "… and {} more\n",
            scope.files.len() - MAX_LISTED_FILES
        ));
    }
    if !scope.changed_files.is_empty() {
        prompt.push_str("\n# Changed files\n");
        for path in scope.changed_files.iter().take(MAX_LISTED_FILES) {
            prompt.push_str(&format!("- {}\n", path.to_string_lossy()));
        }
    }
    if let Some(patch) = scope.diff_patch.as_deref() {
        let base = scope.diff_base.as_deref().unwrap_or("unknown");
        let mut body = patch.to_string();
        if body.len() > DIFF_CHAR_CAP {
            let mut end = DIFF_CHAR_CAP;
            while end > 0 && !body.is_char_boundary(end) {
                end -= 1;
            }
            body.truncate(end);
            body.push_str("\n[diff truncated]");
        }
        prompt.push_str(&format!(
            "\n# Diff under review (base {base})\n```diff\n{body}\n```\n"
        ));
    }
    prompt.push_str(&format!(
        "\n# Output contract\nWhen your investigation is complete, end your FINAL answer with exactly one fenced json block:\n\n```json\n{{\"summary\":\"<2-4 sentences>\",\"candidates\":[{{\"file\":\"<path>\",\"line1\":1,\"line2\":10,\"category\":\"correctness|consistency|security|tests|maintainability|performance|spec_compliance\",\"severity\":\"low|medium|high|critical\",\"confidence\":0.0,\"claim\":\"<one falsifiable sentence>\",\"rationale\":\"<short why>\"}}]}}\n```\n\nReturn at most {} candidates. Use an empty candidates array when no issue is plausible.\n",
        scope.budgets.max_candidates
    ));
    prompt
}

pub struct AgenticInstance {
    pub agent_id: String,
    pub slot_label: String,
    pub spec: ExplicitSubchatSpec,
    pub system_prompt: String,
    pub task_prompt: String,
    pub tools: Vec<String>,
    pub max_steps: usize,
    pub title: String,
    pub verify: bool,
}

pub(crate) async fn run_agentic_instance(
    gcx: Arc<crate::global_context::GlobalContext>,
    ctx: AgentCtx,
    instance: AgenticInstance,
    scope: Arc<ReviewScope>,
    verifier: Option<Arc<VerifierRunner>>,
) -> AgentOutcome {
    let started = now_ms();
    let agent_label = format!("{}@{}", instance.agent_id, instance.slot_label);
    let attribution = format!("review_{}", instance.agent_id);
    let mut metering = serde_json::Map::new();

    let config = match resolve_subchat_config_with_explicit_params(
        gcx.clone(),
        &attribution,
        &instance.spec,
        true,
        None,
        Some(instance.title.clone()),
        Some(ctx.chat_id.clone()),
        Some("review_agent".to_string()),
        Some(ctx.root_chat_id.clone()),
        Some(instance.tools.clone()),
        instance.max_steps,
        false,
        "agent".to_string(),
        ctx.task_meta.clone(),
        ctx.worktree.clone(),
        Some(ctx.tool_call_id.clone()),
        Some(ctx.subchat_tx.clone()),
        Some(ctx.abort_flag.clone()),
        ctx.depth + 1,
    )
    .await
    {
        Ok(config) => config,
        Err(error) => return AgentOutcome::failed(&agent_label, &error, started),
    };

    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: ChatContent::SimpleText(instance.system_prompt.clone()),
            ..Default::default()
        },
        ChatMessage::new("user".to_string(), instance.task_prompt.clone()),
    ];

    let result = match run_subchat(gcx.clone(), messages, config).await {
        Ok(result) => result,
        Err(error) => return AgentOutcome::failed(&agent_label, &error, started),
    };
    merge_metering(&mut metering, result.metering.clone());
    let steps = result
        .messages
        .iter()
        .filter(|m| m.role == "assistant")
        .count();

    let text = get_last_assistant_content(&result.messages);
    let parsed = match parse_candidates_with_reasons(&text) {
        Ok(parsed) => Some(parsed),
        Err(first_error) => {
            tracing::info!(
                "{agent_label}: final answer missing candidate envelope ({first_error}), requesting retry"
            );
            let mut retry_messages = result.messages.clone();
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
                    merge_metering(&mut metering, retry.metering);
                    parse_candidates_with_reasons(&retry_text).ok()
                }
                Err(retry_error) => {
                    tracing::warn!("{agent_label}: envelope retry failed: {retry_error}");
                    None
                }
            }
        }
    };

    let Some(parsed) = parsed else {
        let mut outcome = AgentOutcome::failed(&agent_label, "candidates_parse_failed", started);
        outcome.metering = metering;
        outcome.run.steps = Some(steps);
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

    if instance.verify {
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
    outcome.run.steps = Some(steps);
    outcome.raw_final_answer = Some(text);
    outcome
}
