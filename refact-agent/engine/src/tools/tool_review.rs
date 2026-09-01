use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;
use async_trait::async_trait;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::exec::command_policy::chat_mode_for_exec;
use crate::global_context::GlobalContext;
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::tools::review_agents::config::{
    gather_spec, load_review_agents_config, slot_model_id, ReviewAgentsConfig,
};
use crate::tools::review_agents::{run_review_swarm, AgentCtx, SwarmResult};
use crate::tools::review_evidence::{apply_command_evidence, collect_mechanical_results};
use crate::tools::review_merge::{finalize_review_report, stable_finding_id, DEDUP_LOCATION_PREFIX};
use crate::tools::review_scope::{build_review_scope_with_max_files, ReviewScope};
use crate::tools::review_types::{
    evidence_kinds, AgentRunReport, AgentRunStatus, MechanicalResult, RankTier, ReviewDepth,
    ReviewFinding, ReviewPipelineMetadata, ReviewReport, ReviewScopeSummary, ReviewSeverity,
    ReviewStage, ReviewStageStatus, ScopeExpansion, ScopeMode, VerificationStatus,
};
use crate::tools::review_verify::verification_status_label;
use crate::tools::subagent_phases::{
    gather_files_phase_with_plan, resolve_gathered_file_path, GatherFilesParams, GatherPlan,
    GatherRunner, DEFAULT_GATHER_RETRY_PROMPT,
};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use crate::worktrees::scope::ExecutionScope;

pub struct ToolCodeReview {
    pub config_path: String,
}

fn review_stage(name: &str, status: ReviewStageStatus, reason: Option<&str>) -> ReviewStage {
    ReviewStage {
        name: name.to_string(),
        status,
        reason: reason.map(str::to_string),
    }
}

fn swarm_stage_names() -> [&'static str; 4] {
    ["static_agents", "llm_agents", "agentic_agents", "merge"]
}

fn mechanical_failure_report(
    scope: ReviewScopeSummary,
    mechanical: MechanicalResult,
    depth: ReviewDepth,
) -> ReviewReport {
    let failed_names = mechanical
        .checks
        .iter()
        .filter(|check| check.exit_status != 0)
        .map(|check| check.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let mut stages = vec![review_stage(
        "mechanical",
        ReviewStageStatus::Failed,
        Some("one_or_more_checks_failed"),
    )];
    for name in swarm_stage_names() {
        stages.push(review_stage(
            name,
            ReviewStageStatus::Skipped,
            Some("mechanical_checks_failed"),
        ));
    }
    let mut report = ReviewReport {
        scope,
        findings: vec![],
        checks_performed: vec![],
        summary: format!("Mechanical checks failed ({failed_names}); review agents were skipped."),
        assumed_intent: None,
        pipeline: ReviewPipelineMetadata {
            stages,
            stopped_reason: Some("mechanical_checks_failed".to_string()),
            mechanical: Some(mechanical.clone()),
            depth: Some(depth.as_str().to_string()),
            agents: vec![],
        },
    };
    apply_command_evidence(&mut report, Some(&mechanical));
    report
}

fn apply_refutations(findings: &mut [ReviewFinding], refuted: &[String]) -> usize {
    if refuted.is_empty() {
        return 0;
    }
    let mut applied = 0;
    for finding in findings.iter_mut() {
        let id = stable_finding_id(finding);
        if refuted.contains(&id) {
            finding.verification_status = VerificationStatus::Downgraded;
            let marker = "a3_refuted".to_string();
            if !finding.checks_performed.contains(&marker) {
                finding.checks_performed.push(marker);
            }
            applied += 1;
        }
    }
    applied
}

fn count_out_of_scope_rejections(checks: &[String]) -> usize {
    checks
        .iter()
        .filter(|check| check.ends_with(":file_not_in_scope"))
        .count()
}

#[allow(clippy::too_many_arguments)]
async fn run_review_pipeline(
    gcx: Arc<GlobalContext>,
    ccx: Arc<AMutex<AtCommandsContext>>,
    scope: ReviewScope,
    cfg: Arc<ReviewAgentsConfig>,
    depth: ReviewDepth,
    scope_mode: ScopeMode,
    requested_files: usize,
    external_messages: Vec<ChatMessage>,
    tool_call_id: String,
    metering: &mut serde_json::Map<String, Value>,
) -> Result<ReviewReport, String> {
    let ctx = {
        let ccx_lock = ccx.lock().await;
        AgentCtx {
            tool_call_id: tool_call_id.clone(),
            subchat_tx: ccx_lock.subchat_tx.clone(),
            abort_flag: ccx_lock.abort_flag.clone(),
            depth: ccx_lock.subchat_depth,
            task_meta: ccx_lock.task_meta.clone(),
            worktree: ccx_lock.execution_scope_worktree(),
            chat_id: ccx_lock.chat_id.clone(),
            root_chat_id: ccx_lock.root_chat_id.clone(),
        }
    };

    let workspace_root = match ctx.worktree.as_ref() {
        Some(worktree) => Some(worktree.root.clone()),
        None => crate::files_correction::get_project_dirs(gcx.clone())
            .await
            .into_iter()
            .next(),
    };
    let chat_mode = chat_mode_for_exec(gcx.clone(), &ctx.chat_id).await;
    let mechanical =
        collect_mechanical_results(gcx.clone(), workspace_root, chat_mode, &ctx.chat_id).await;

    let filenames: Vec<String> = scope
        .files
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    let reviewed_files = filenames.len();
    let scope_summary = ReviewScopeSummary {
        files_reviewed: filenames,
        focus: scope.focus.clone(),
        diff_base: scope.diff_base.clone(),
        expansion: Some(ScopeExpansion {
            mode: scope_mode.as_str().to_string(),
            requested_files,
            reviewed_files,
            rejected_out_of_scope: 0,
        }),
    };

    if let Some(result) = mechanical.as_ref() {
        if !result.passed {
            return Ok(mechanical_failure_report(
                scope_summary,
                result.clone(),
                depth,
            ));
        }
    }

    let scope = Arc::new(scope);
    let external_messages = Arc::new(external_messages);
    let swarm: SwarmResult = run_review_swarm(
        gcx.clone(),
        ctx,
        cfg.clone(),
        scope.clone(),
        external_messages,
        depth,
    )
    .await;

    crate::tools::review_verify::merge_metering(metering, swarm.metering);

    let mut scope_summary = scope_summary;
    if let Some(expansion) = scope_summary.expansion.as_mut() {
        expansion.rejected_out_of_scope = count_out_of_scope_rejections(&swarm.checks);
    }

    let mut report = ReviewReport {
        scope: scope_summary,
        findings: swarm.findings,
        checks_performed: swarm.checks,
        summary: String::new(),
        assumed_intent: swarm.assumed_intent,
        pipeline: ReviewPipelineMetadata {
            stages: vec![
                match mechanical.as_ref() {
                    Some(_) => review_stage("mechanical", ReviewStageStatus::Completed, None),
                    None => review_stage(
                        "mechanical",
                        ReviewStageStatus::Skipped,
                        Some("review_commands_disabled"),
                    ),
                },
                review_stage("static_agents", ReviewStageStatus::Completed, None),
                review_stage("llm_agents", ReviewStageStatus::Completed, None),
                review_stage("agentic_agents", ReviewStageStatus::Completed, None),
                review_stage("merge", ReviewStageStatus::Completed, None),
            ],
            stopped_reason: None,
            mechanical: mechanical.clone(),
            depth: Some(depth.as_str().to_string()),
            agents: swarm.agents,
        },
    };

    let refuted_applied = apply_refutations(&mut report.findings, &swarm.refuted);
    if refuted_applied > 0 {
        report
            .checks_performed
            .push(format!("a3_refuted:{refuted_applied}"));
    }
    apply_command_evidence(&mut report, mechanical.as_ref());
    finalize_review_report(&mut report, &swarm.enrichment);
    Ok(report)
}

fn markdown_cell(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('|', "\\|")
}

fn severity_label(severity: &ReviewSeverity) -> &'static str {
    match severity {
        ReviewSeverity::Low => "low",
        ReviewSeverity::Medium => "medium",
        ReviewSeverity::High => "high",
        ReviewSeverity::Critical => "critical",
    }
}

fn finding_location(finding: &ReviewFinding) -> String {
    format!("{}:{}-{}", finding.file, finding.line1, finding.line2)
}

/// Extra line ranges folded into a survivor by deduplication, rendered next to the
/// primary location so a collapsed cluster still lists every place it was seen.
fn extra_line_references(finding: &ReviewFinding) -> Vec<String> {
    finding
        .checks_performed
        .iter()
        .filter_map(|check| check.strip_prefix(DEDUP_LOCATION_PREFIX))
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Evidence strength shown next to each finding. Nothing here is executed unless the
/// finding actually carries execution or command output.
fn evidence_kind_label(finding: &ReviewFinding) -> &'static str {
    let has = |kind: &str| finding.evidence.iter().any(|item| item.kind == kind);
    if has(evidence_kinds::EXECUTION_OUTPUT) || has(evidence_kinds::MUTATION_PROBE) {
        return "execution-output";
    }
    if has(evidence_kinds::COMMAND_OUTPUT) {
        return "command-output";
    }
    if has(evidence_kinds::CONSOLE_LOG) || has(evidence_kinds::SCREENSHOT) {
        return "browser-capture";
    }
    if has(evidence_kinds::DIFF_HUNK) {
        return "diff-hunk";
    }
    if has(evidence_kinds::EXCERPT) || has(evidence_kinds::SYMBOL) {
        return "source-excerpt";
    }
    if has(evidence_kinds::STATIC_FACT) || has(evidence_kinds::CHECK) {
        return "static-fact";
    }
    "no-evidence"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct TierCounts {
    pub execution_reproduced: usize,
    pub cross_model_corroborated: usize,
    pub model_verified: usize,
    pub needs_human_validation: usize,
    pub unreviewed: usize,
    pub downgraded: usize,
}

impl TierCounts {
    pub(crate) fn total(&self) -> usize {
        self.execution_reproduced
            + self.cross_model_corroborated
            + self.model_verified
            + self.needs_human_validation
            + self.unreviewed
            + self.downgraded
    }
}

/// Counts are derived from the exact finding list that gets rendered, so the header
/// can never disagree with the body.
pub(crate) fn tier_counts(findings: &[ReviewFinding]) -> TierCounts {
    let mut counts = TierCounts::default();
    for finding in findings {
        match finding.rank_tier {
            RankTier::ExecutionReproduced => counts.execution_reproduced += 1,
            RankTier::Corroborated => counts.cross_model_corroborated += 1,
            RankTier::Verified => counts.model_verified += 1,
            RankTier::NeedsHumanValidation => counts.needs_human_validation += 1,
            RankTier::Unverified => counts.unreviewed += 1,
            RankTier::Downgraded => counts.downgraded += 1,
        }
    }
    counts
}

fn review_verdict(report: &ReviewReport) -> String {
    if report.findings.is_empty() {
        return "No findings retained.".to_string();
    }
    let counts = tier_counts(&report.findings);
    format!(
        "Review retained {} finding(s): {} execution-reproduced, {} cross-model-corroborated, {} model-verified, {} needs-human-validation, {} unreviewed, {} downgraded.",
        counts.total(),
        counts.execution_reproduced,
        counts.cross_model_corroborated,
        counts.model_verified,
        counts.needs_human_validation,
        counts.unreviewed,
        counts.downgraded,
    )
}

fn agent_status_label(status: &AgentRunStatus) -> &'static str {
    match status {
        AgentRunStatus::Ran => "ran",
        AgentRunStatus::Skipped => "skipped",
        AgentRunStatus::Failed => "failed",
    }
}

pub(crate) fn agent_is_degraded(agent: &AgentRunReport) -> bool {
    if agent.status == AgentRunStatus::Failed {
        return true;
    }
    agent
        .reason
        .as_deref()
        .is_some_and(|reason| reason.contains("idle_timeout") || reason.contains("review_deadline"))
}

/// A degraded review must announce that it is degraded: timed-out and failed agents
/// go into the summary, not into a coverage table nobody reads.
fn render_degraded_agents(agents: &[AgentRunReport]) -> String {
    let degraded = agents
        .iter()
        .filter(|agent| agent_is_degraded(agent))
        .collect::<Vec<_>>();
    if degraded.is_empty() {
        return String::new();
    }
    let mut output = format!(
        "\n\n## Degraded coverage\n\nDEGRADED REVIEW: {} agent(s) did not complete; this report is partial and absence of findings is not evidence of absence.",
        degraded.len()
    );
    for agent in degraded {
        output.push_str(&format!(
            "\n- {} ({}): {}",
            markdown_cell(&agent.agent),
            agent_status_label(&agent.status),
            markdown_cell(agent.reason.as_deref().unwrap_or("no reason recorded")),
        ));
    }
    output
}

fn render_scope_expansion(scope: &ReviewScopeSummary) -> String {
    let Some(expansion) = scope.expansion.as_ref() else {
        return String::new();
    };
    let mut output = format!(
        "\n- Scope mode: {} (scope widened from {} requested to {} files)",
        expansion.mode, expansion.requested_files, expansion.reviewed_files
    );
    if expansion.rejected_out_of_scope > 0 {
        output.push_str(&format!(
            "\n- Rejected out of scope: {} finding(s)",
            expansion.rejected_out_of_scope
        ));
    }
    output
}

fn render_review_markdown(report: &ReviewReport) -> String {
    let focus = report.scope.focus.as_deref().unwrap_or("not specified");
    let diff_base = report.scope.diff_base.as_deref().unwrap_or("not specified");
    let depth = report.pipeline.depth.as_deref().unwrap_or("normal");
    let verdict = if report.findings.is_empty() {
        report.summary.trim().to_string()
    } else {
        review_verdict(report)
    };
    let mut output = format!(
        "## Review summary\n\n- Depth: {}\n- Scope: {} files\n- Focus: {}\n- Diff base: {}",
        depth,
        report.scope.files_reviewed.len(),
        markdown_cell(focus),
        markdown_cell(diff_base),
    );
    output.push_str(&render_scope_expansion(&report.scope));
    output.push_str("\n\n");
    output.push_str(&verdict);
    if let Some(intent) = report.assumed_intent.as_deref() {
        output.push_str(&format!("\n\nAssumed intent: {}", markdown_cell(intent)));
    }
    if !report.findings.is_empty() && !report.summary.trim().is_empty() {
        output.push_str("\n\n");
        output.push_str(report.summary.trim());
    }

    output.push_str("\n\n## Findings");
    for tier in [
        RankTier::ExecutionReproduced,
        RankTier::Corroborated,
        RankTier::Verified,
        RankTier::NeedsHumanValidation,
        RankTier::Unverified,
        RankTier::Downgraded,
    ] {
        let findings = report
            .findings
            .iter()
            .filter(|finding| finding.rank_tier == tier)
            .collect::<Vec<_>>();
        if findings.is_empty() {
            continue;
        }
        output.push_str(&format!("\n\n### {} ({})", tier.label(), findings.len()));
        for finding in findings {
            output.push_str(&format!(
                "\n\n- [{}] {} — {} ({}, {}, evidence: {}, {:.2})",
                finding.id,
                markdown_cell(&finding_location(finding)),
                markdown_cell(&finding.claim),
                severity_label(&finding.severity),
                verification_status_label(&finding.verification_status),
                evidence_kind_label(finding),
                finding.confidence,
            ));
            let extra_lines = extra_line_references(finding);
            if !extra_lines.is_empty() {
                output.push_str(&format!(
                    "\n  - Also at: {}",
                    markdown_cell(&extra_lines.join(", "))
                ));
            }
            if !finding.sources.is_empty() {
                output.push_str(&format!(
                    "\n  - Sources: {}",
                    markdown_cell(&finding.sources.join(", "))
                ));
            }
            if let Some(impact) = finding.impact.as_deref() {
                output.push_str(&format!("\n  - Impact: {}", markdown_cell(impact)));
            }
            if let Some(remediation) = finding.remediation.as_deref() {
                output.push_str(&format!(
                    "\n  - Remediation: {}",
                    markdown_cell(remediation)
                ));
            }
        }
    }
    if report.findings.is_empty() {
        output.push_str("\n\nNo findings.");
    }

    output.push_str(&render_degraded_agents(&report.pipeline.agents));
    output.push_str(
        "\n\nPer-agent coverage, checks performed, and the machine-readable `ReviewReport` are attached to this tool result's metadata; they are not repeated here.",
    );
    output
}

fn gather_user_instruction(mode: ScopeMode) -> &'static str {
    match mode {
        ScopeMode::Strict => "Based on the conversation above, identify the files relevant to the review. Stay inside the requested paths and add only files a concrete dependency edge forces you to read.",
        ScopeMode::Adjacent => "Based on the conversation above, identify every file relevant to the review. Start from the requested paths and add their direct callers, callees, tests, and configuration.",
        ScopeMode::Broad => "Based on the conversation above, identify every file relevant to the review. Cast a wide net \u{2014} more related files is better; anything you name will be in scope for findings.",
    }
}

fn scope_max_files(mode: ScopeMode, configured: usize, requested: usize) -> usize {
    let configured = configured.max(1);
    match mode {
        ScopeMode::Strict => requested.max(1).min(configured),
        ScopeMode::Adjacent => configured.min(requested.saturating_mul(4).max(8)),
        ScopeMode::Broad => configured,
    }
}

#[async_trait]
impl Tool for ToolCodeReview {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "review".to_string(),
            display_name: "Code Review".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Perform a thorough multi-agent code review. Optionally pass `what_to_check` (focus/scope), `files` (initial guess of relevant paths \u{2014} the reviewer starts there and finds more), `depth` (normal = static analyzer agents with codebase-search enrichment, cross-model reviewer ensembles, repo-context and research agents; deep = + test-execution and browser agents), and `scope_mode` (how far outside `files` the reviewer may wander). The Markdown result reports findings by evidence tier; the full machine-parseable ReviewReport is attached to the tool result metadata.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "what_to_check": {
                        "type": "string",
                        "description": "Optional. What to review and what to look for (focus/scope). If omitted, the scope is inferred from the conversation."
                    },
                    "files": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Optional. Initial guess of relevant file paths. The reviewer starts from these and still searches for more related files."
                    },
                    "depth": {
                        "type": "string",
                        "enum": ["normal", "deep"],
                        "description": "Optional. Which agent families run. Defaults to the configured default depth (normal); deep adds test-execution and browser agents."
                    },
                    "scope_mode": {
                        "type": "string",
                        "enum": ["strict", "adjacent", "broad"],
                        "description": "Optional. How far outside `files` the reviewer may wander: strict stays inside the requested paths, adjacent adds direct callers/callees/tests, broad (default) casts a wide net. Every review reports how much the scope widened."
                    }
                },
                "required": [],
                "additionalProperties": false
            }),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let gcx = ccx.lock().await.app.gcx.clone();

        let what_to_check = args
            .get("what_to_check")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let depth_arg = match args.get("depth").and_then(Value::as_str) {
            Some(value) if !value.trim().is_empty() => Some(
                ReviewDepth::parse(value)
                    .ok_or_else(|| format!("invalid depth '{value}', expected: normal, deep"))?,
            ),
            _ => None,
        };
        let scope_mode = match args.get("scope_mode").and_then(Value::as_str) {
            Some(value) if !value.trim().is_empty() => {
                ScopeMode::parse(value).ok_or_else(|| {
                    format!("invalid scope_mode '{value}', expected: strict, adjacent, broad")
                })?
            }
            _ => ScopeMode::default(),
        };
        let seed_files: Vec<String> = args
            .get("files")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let execution_scope = ccx
            .lock()
            .await
            .execution_scope_worktree()
            .as_ref()
            .map(ExecutionScope::from_worktree);
        let mut requested_seed_files = Vec::new();
        for seed_file in &seed_files {
            if let Some(path) =
                resolve_gathered_file_path(gcx.clone(), execution_scope.as_ref(), seed_file).await
            {
                if !requested_seed_files.contains(&path) {
                    requested_seed_files.push(path);
                }
            }
        }

        let cfg = Arc::new(load_review_agents_config(gcx.clone()).await?);
        let depth = cfg.depth_or_default(depth_arg);

        let external_messages = {
            let ccx_lock = ccx.lock().await;
            ccx_lock.messages.clone()
        };

        let gather_section = &cfg.swarm.gather;
        let gather_system_prompt = gather_section
            .system_prompt
            .clone()
            .filter(|p| !p.trim().is_empty())
            .ok_or("gather system_prompt not configured for review_agents")?;
        let gather_model = slot_model_id(gcx.clone(), gather_section.model_slot).await?;
        let requested_files = requested_seed_files.len();
        let max_files =
            scope_max_files(scope_mode, gather_section.max_files.max(1), requested_files);
        let gather_params = GatherFilesParams {
            default_subagent_id: "review_gather",
            title: "Review: Gathering Files",
            default_system_prompt: "",
            user_instruction: gather_user_instruction(scope_mode),
            focus: what_to_check.clone(),
            seed_files,
        };
        let plan = GatherPlan {
            attribution_id: "review_gather".to_string(),
            system_prompt: gather_system_prompt,
            retry_prompt: gather_section
                .retry_prompt
                .clone()
                .unwrap_or_else(|| DEFAULT_GATHER_RETRY_PROMPT.to_string()),
            tools: gather_section.tools.clone(),
            max_steps: gather_section.max_steps.max(1),
            max_files,
            runner: GatherRunner::Explicit {
                spec: gather_spec(gather_section, gather_model),
            },
        };

        tracing::info!("review: phase 1 - gathering relevant files");
        let important_paths = gather_files_phase_with_plan(
            gcx.clone(),
            ccx.clone(),
            external_messages.clone(),
            tool_call_id.clone(),
            &gather_params,
            plan,
        )
        .await?;
        let scope = build_review_scope_with_max_files(
            gcx.clone(),
            important_paths,
            requested_seed_files,
            what_to_check,
            &cfg.base_params,
            max_files,
        )
        .await;

        tracing::info!(
            "review: phase 2 - running {} review at depth {} on {} files",
            "swarm",
            depth.as_str(),
            scope.files.len()
        );

        let mut metering = serde_json::Map::new();
        let report = run_review_pipeline(
            gcx.clone(),
            ccx.clone(),
            scope,
            cfg.clone(),
            depth,
            scope_mode,
            requested_files,
            external_messages,
            tool_call_id.clone(),
            &mut metering,
        )
        .await?;
        let final_message = render_review_markdown(&report);
        metering.insert(
            "review_report".to_string(),
            serde_json::to_value(&report)
                .map_err(|error| format!("failed to serialize code review report: {error}"))?,
        );
        let (review_refs, review_refs_truncated) = review_refs(
            &report,
            &crate::files_correction::get_project_dirs(gcx.clone()).await,
        );
        metering.insert("review_refs".to_string(), review_refs);
        metering.insert(
            "review_refs_truncated".to_string(),
            Value::Bool(review_refs_truncated),
        );

        Ok((
            false,
            vec![
                ContextEnum::ChatMessage(ChatMessage {
                    role: "tool".to_string(),
                    content: ChatContent::SimpleText(final_message),
                    tool_calls: None,
                    tool_call_id: tool_call_id.clone(),
                    usage: None,
                    preserve: Some(true),
                    extra: metering,
                    output_filter: Some(OutputFilter::no_limits()),
                    ..Default::default()
                }),
                ContextEnum::ChatMessage(ChatMessage {
                    role: "cd_instruction".to_string(),
                    content: ChatContent::SimpleText(cfg.guardrails_prompt.clone()),
                    ..Default::default()
                }),
            ],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

fn review_refs(report: &ReviewReport, roots: &[std::path::PathBuf]) -> (Value, bool) {
    let scope = report.scope.diff_base.clone();
    let mut references = Vec::new();
    for finding in &report.findings {
        for evidence in &finding.evidence {
            if references.len() == 32 {
                return (Value::Array(references), true);
            }
            let path = Path::new(evidence.path.as_deref().unwrap_or(&finding.file));
            let relative = if path.is_absolute() {
                roots.iter().find_map(|root| {
                    path.strip_prefix(root)
                        .ok()
                        .map(|path| path.to_string_lossy().replace('\\', "/"))
                })
            } else {
                (!path
                    .components()
                    .any(|component| matches!(component, std::path::Component::ParentDir)))
                .then(|| path.to_string_lossy().replace('\\', "/"))
            };
            let Some(relative) = relative else {
                continue;
            };
            references.push(json!({
                "id": finding.id,
                "severity": severity_label(&finding.severity),
                "path": relative,
                "line1": evidence.line1.unwrap_or(finding.line1),
                "line2": evidence.line2.unwrap_or(finding.line2),
                "evidence_kind": evidence.kind,
                "scope": scope,
            }));
        }
    }
    (Value::Array(references), false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::review_agents::config::ReviewSwarmConfig;
    use crate::tools::review_types::{MechanicalCheck, RankTier, ReviewEvidence};

    fn scope_summary() -> ReviewScopeSummary {
        ReviewScopeSummary {
            files_reviewed: vec!["src/lib.rs".to_string()],
            focus: None,
            diff_base: None,
            expansion: None,
        }
    }

    fn mechanical_result(passed: bool, exit_status: i32, output: &str) -> MechanicalResult {
        MechanicalResult {
            passed,
            checks: vec![MechanicalCheck {
                name: "cargo check".to_string(),
                command: vec!["cargo".to_string(), "check".to_string()],
                exit_status,
                output_excerpt: output.to_string(),
            }],
        }
    }

    fn sample_finding() -> ReviewFinding {
        ReviewFinding {
            id: "rf-1234abcd".to_string(),
            category: "correctness".to_string(),
            severity: ReviewSeverity::High,
            confidence: 0.8,
            verification_status: VerificationStatus::Unverified,
            rank_tier: RankTier::Unverified,
            sources: vec!["l1_diff@thinking".to_string()],
            file: "src/lib.rs".to_string(),
            line1: 4,
            line2: 6,
            claim: "The branch | drops errors.".to_string(),
            evidence: vec![ReviewEvidence {
                kind: "excerpt".to_string(),
                path: Some("src/lib.rs".to_string()),
                line1: Some(4),
                line2: Some(6),
                content: "deterministic excerpt".to_string(),
            }],
            impact: Some("Errors are hidden.".to_string()),
            remediation: Some("Return the error.".to_string()),
            checks_performed: vec![],
        }
    }

    #[test]
    fn tool_review_mechanical_failure_skips_all_swarm_stages() {
        let report = mechanical_failure_report(
            scope_summary(),
            mechanical_result(false, 101, "error[E0308]: mismatched types"),
            ReviewDepth::Deep,
        );

        assert_eq!(
            report.pipeline.stopped_reason.as_deref(),
            Some("mechanical_checks_failed")
        );
        assert_eq!(report.pipeline.depth.as_deref(), Some("deep"));
        assert_eq!(report.pipeline.stages.len(), 5);
        assert_eq!(report.pipeline.stages[0].status, ReviewStageStatus::Failed);
        assert!(report.pipeline.stages[1..]
            .iter()
            .all(|stage| stage.status == ReviewStageStatus::Skipped));
        let check = &report.pipeline.mechanical.as_ref().unwrap().checks[0];
        assert_eq!(check.exit_status, 101);
        assert_eq!(report.checks_performed, ["command:cargo_check:exit=101"]);
    }

    #[test]
    fn tool_review_refutations_downgrade_matching_findings() {
        let mut findings = vec![sample_finding()];
        let id = stable_finding_id(&findings[0]);

        let applied = apply_refutations(&mut findings, &[id]);

        assert_eq!(applied, 1);
        assert_eq!(
            findings[0].verification_status,
            VerificationStatus::Downgraded
        );
        assert!(findings[0]
            .checks_performed
            .contains(&"a3_refuted".to_string()));
        assert_eq!(apply_refutations(&mut findings, &[]), 0);
    }

    #[test]
    fn tool_review_render_markdown_groups_by_tier_and_points_at_metadata() {
        let mut finding = sample_finding();
        finding.rank_tier = RankTier::Corroborated;
        finding.sources = vec!["l1_diff@chat".to_string(), "s1_security".to_string()];
        let report = ReviewReport {
            scope: scope_summary(),
            findings: vec![finding],
            checks_performed: vec!["excerpt_ok".to_string()],
            summary: "One plausible issue.".to_string(),
            assumed_intent: Some("Fix the parser.".to_string()),
            pipeline: ReviewPipelineMetadata {
                stages: vec![],
                stopped_reason: None,
                mechanical: None,
                depth: Some("normal".to_string()),
                agents: vec![AgentRunReport {
                    agent: "l1_diff@chat".to_string(),
                    model: Some("some-model".to_string()),
                    status: AgentRunStatus::Ran,
                    reason: None,
                    candidates: 3,
                    survived: 1,
                    duration_ms: 900,
                    steps: Some(1),
                }],
            },
        };

        let markdown = render_review_markdown(&report);

        assert!(markdown.starts_with("## Review summary"));
        assert!(markdown.contains("- Depth: normal"));
        assert!(markdown.contains("Assumed intent: Fix the parser."));
        assert!(markdown.contains("## Findings\n\n### corroborated (1)"));
        assert!(markdown.contains(
            "[rf-1234abcd] src/lib.rs:4-6 — The branch \\| drops errors. (high, unreviewed, evidence: source-excerpt, 0.80)"
        ));
        assert!(markdown.contains("Sources: l1_diff@chat, s1_security"));
        assert!(markdown.contains("Impact: Errors are hidden."));
        assert!(markdown.contains("Remediation: Return the error."));
        // Per-agent coverage, checks performed and the machine-readable report moved out
        // of the model-visible markdown and into the tool result metadata.
        assert!(!markdown.contains("## Agent coverage"));
        assert!(!markdown.contains("## Checks performed"));
        assert!(!markdown.contains("```json"));
        assert!(markdown.contains(
            "the machine-readable `ReviewReport` are attached to this tool result's metadata"
        ));
    }

    #[test]
    fn tool_review_render_markdown_clean_report_golden_and_json_roundtrip() {
        let mut report = ReviewReport {
            scope: scope_summary(),
            findings: vec![],
            checks_performed: vec!["verifier_rejected:2".to_string()],
            summary: "No surviving findings.".to_string(),
            assumed_intent: None,
            pipeline: ReviewPipelineMetadata {
                depth: Some("normal".to_string()),
                ..Default::default()
            },
        };
        crate::tools::review_merge::finalize_review_report(
            &mut report,
            &crate::tools::review_merge::RiskEnrichment::default(),
        );

        let markdown = render_review_markdown(&report);

        assert!(markdown.contains("- Depth: normal"));
        assert!(markdown.contains("- Scope: 1 files"));
        assert!(markdown.contains("Reviewed 1 file. Checks performed: verifier_rejected:2."));
        assert!(markdown.contains("No findings."));
        assert!(!markdown.contains("```json"));

        // The machine-readable report is no longer appended to the markdown; it is
        // serialized into `metering["review_report"]`, so the roundtrip is asserted
        // against that same serialization.
        let metering_value = serde_json::to_value(&report).unwrap();
        let parsed: ReviewReport = serde_json::from_value(metering_value).unwrap();
        assert_eq!(parsed, report);
    }

    #[test]
    fn tool_review_default_yaml_carries_swarm_config_and_prompts() {
        let config: crate::yaml_configs::customization_types::SubagentConfig =
            serde_yaml::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/crates/refact-yaml-configs/src/defaults/subagents/review_agents.yaml"
            )))
            .unwrap();

        assert_eq!(config.schema_version, 5);
        assert_eq!(config.id, "review_agents");
        let reviewer = config.prompts.reviewer.as_deref().unwrap();
        assert!(reviewer.contains("Optimize for recall"));
        assert!(reviewer.contains("exactly one fenced json block"));
        assert!(config.prompts.guardrails.is_some());

        let swarm_value = config
            .extra
            .get("review_swarm")
            .expect("review_swarm section");
        let swarm: ReviewSwarmConfig = serde_yaml::from_value(swarm_value.clone()).unwrap();
        assert_eq!(swarm.default_depth, "normal");
        assert!(swarm.gather.system_prompt.is_some());
        assert!(swarm.verifier.prompt.is_some());
        let static_prompt = swarm.static_enrichment_prompt.as_deref().unwrap();
        assert!(static_prompt.contains("static-analysis triage investigator"));
        assert!(static_prompt.contains("refute false positives"));
        for agent in [
            &swarm.s1_security.agent,
            &swarm.s2_dead_code.agent,
            &swarm.s3_duplication.agent,
            &swarm.s4_test_integrity.agent,
            &swarm.s5_dependencies.agent,
        ] {
            assert!(agent.enabled);
            assert_eq!(
                agent.model_slot,
                crate::tools::review_agents::config::ModelSlot::Light
            );
            assert_eq!(agent.max_steps, 12);
        }
        assert!(swarm.l2_simplicity.prompt.is_some());
        assert!(swarm.l3_spec.prompt.is_some());
        assert!(swarm.a1_repo_context.prompt.is_some());
        assert!(swarm.a2_research.prompt.is_some());
        assert!(swarm.a3_execution.base.prompt.is_some());
        assert!(swarm.a4_browser.base.prompt.is_some());
        let verifier_prompt = swarm.verifier.prompt.as_deref().unwrap();
        assert!(verifier_prompt.contains("skeptical fact-checker"));
        assert!(verifier_prompt.contains("needs_human_validation"));
    }

    #[test]
    fn tool_review_verifier_prompt_is_blind_to_reviewer_rationale() {
        let canary = "PRIVATE_REVIEWER_RATIONALE_CANARY";
        let parsed = crate::tools::review_candidates::parse_candidates_with_reasons(&format!(
            r#"```json
{{"summary":"One plausible issue.","candidates":[{{"file":"src/lib.rs","line1":4,"line2":4,"category":"correctness","severity":"high","confidence":0.8,"claim":"The branch drops errors.","rationale":"{canary}"}}]}}
```"#,
        ))
        .unwrap();
        let mut findings: Vec<ReviewFinding> = parsed
            .candidates
            .into_iter()
            .map(|candidate| candidate.into_review_parts().0)
            .collect();
        findings[0].evidence.push(ReviewEvidence {
            kind: "excerpt".to_string(),
            path: Some("src/lib.rs".to_string()),
            line1: Some(4),
            line2: Some(4),
            content: "4: return Ok(())".to_string(),
        });
        let batches = crate::tools::review_verify::build_verifier_batches(&findings);

        assert_eq!(batches.len(), 1);
        assert!(!batches[0].prompt.contains(canary));
        assert!(!batches[0].prompt.contains("rationale"));
        assert!(!batches[0].prompt.contains("confidence"));
        assert!(batches[0].prompt.contains("4: return Ok(())"));
    }
}
