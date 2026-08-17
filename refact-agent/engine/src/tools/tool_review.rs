use std::collections::HashMap;
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
use crate::tools::review_merge::{finalize_review_report, stable_finding_id};
use crate::tools::review_scope::{build_review_scope_with_max_files, ReviewScope};
use crate::tools::review_types::{
    AgentRunReport, AgentRunStatus, MechanicalResult, RankTier, ReviewDepth, ReviewFinding,
    ReviewPipelineMetadata, ReviewReport, ReviewScopeSummary, ReviewSeverity, ReviewStage,
    ReviewStageStatus, VerificationStatus,
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

async fn run_review_pipeline(
    gcx: Arc<GlobalContext>,
    ccx: Arc<AMutex<AtCommandsContext>>,
    scope: ReviewScope,
    cfg: Arc<ReviewAgentsConfig>,
    depth: ReviewDepth,
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
    let scope_summary = ReviewScopeSummary {
        files_reviewed: filenames,
        focus: scope.focus.clone(),
        diff_base: scope.diff_base.clone(),
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

fn review_verdict(report: &ReviewReport) -> String {
    if report.findings.is_empty() {
        return "No verified findings.".to_string();
    }
    let reproduced = report
        .findings
        .iter()
        .filter(|finding| finding.rank_tier == RankTier::ExecutionReproduced)
        .count();
    let corroborated = report
        .findings
        .iter()
        .filter(|finding| finding.rank_tier == RankTier::Corroborated)
        .count();
    let verified = report
        .findings
        .iter()
        .filter(|finding| finding.rank_tier == RankTier::Verified)
        .count();
    format!(
        "Review retained {} finding(s): {reproduced} execution-reproduced, {corroborated} corroborated, {verified} verified.",
        report.findings.len()
    )
}

fn agent_status_label(status: &AgentRunStatus) -> &'static str {
    match status {
        AgentRunStatus::Ran => "ran",
        AgentRunStatus::Skipped => "skipped",
        AgentRunStatus::Failed => "failed",
    }
}

fn render_agent_coverage(agents: &[AgentRunReport]) -> String {
    if agents.is_empty() {
        return String::new();
    }
    let mut output = String::from(
        "\n\n## Agent coverage\n\n| agent | model | status | reason | candidates | survived | steps | ms |\n|---|---|---|---|---|---|---|---|",
    );
    for row in agents {
        output.push_str(&format!(
            "\n| {} | {} | {} | {} | {} | {} | {} | {} |",
            markdown_cell(&row.agent),
            markdown_cell(row.model.as_deref().unwrap_or("—")),
            agent_status_label(&row.status),
            markdown_cell(row.reason.as_deref().unwrap_or("—")),
            row.candidates,
            row.survived,
            row.steps.map(|s| s.to_string()).unwrap_or("—".to_string()),
            row.duration_ms,
        ));
    }
    output
}

fn render_review_markdown(report: &ReviewReport) -> Result<String, serde_json::Error> {
    let focus = report.scope.focus.as_deref().unwrap_or("not specified");
    let diff_base = report.scope.diff_base.as_deref().unwrap_or("not specified");
    let depth = report.pipeline.depth.as_deref().unwrap_or("normal");
    let verdict = if report.findings.is_empty() {
        report.summary.trim().to_string()
    } else {
        review_verdict(report)
    };
    let mut output = format!(
        "## Review summary\n\n- Depth: {}\n- Scope: {} files\n- Focus: {}\n- Diff base: {}\n\n{}",
        depth,
        report.scope.files_reviewed.len(),
        markdown_cell(focus),
        markdown_cell(diff_base),
        verdict
    );
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
        output.push_str(&format!("\n\n### {}", tier.label()));
        for finding in findings {
            output.push_str(&format!(
                "\n\n- [{}] {} — {} ({}, {}, {:.2})",
                finding.id,
                markdown_cell(&finding_location(finding)),
                markdown_cell(&finding.claim),
                severity_label(&finding.severity),
                verification_status_label(&finding.verification_status),
                finding.confidence,
            ));
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

    output.push_str(&render_agent_coverage(&report.pipeline.agents));

    output.push_str("\n\n## Checks performed");
    if report.checks_performed.is_empty() {
        output.push_str("\n\n- None recorded");
    } else {
        for check in &report.checks_performed {
            output.push_str(&format!("\n\n- {}", markdown_cell(check)));
        }
    }

    output.push_str("\n\n```json\n");
    output.push_str(&serde_json::to_string_pretty(report)?);
    output.push_str("\n```");
    Ok(output)
}

fn gather_user_instruction() -> &'static str {
    "Based on the conversation above, identify every file relevant to the review. Cast a wide net \u{2014} more related files is better."
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
            description: "Perform a thorough multi-agent code review. Optionally pass `what_to_check` (focus/scope), `files` (initial guess of relevant paths \u{2014} the reviewer starts there and finds more), and `depth` (normal = static analyzer agents with codebase-search enrichment, cross-model reviewer ensembles, repo-context and research agents; deep = + test-execution and browser agents). The Markdown result ends with a fenced JSON block containing the full machine-parseable ReviewReport.".to_string(),
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
        let gather_params = GatherFilesParams {
            default_subagent_id: "review_gather",
            title: "Review: Gathering Files",
            default_system_prompt: "",
            user_instruction: gather_user_instruction(),
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
            max_files: gather_section.max_files.max(1),
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
            gather_section.max_files.max(1),
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
            gcx,
            ccx.clone(),
            scope,
            cfg.clone(),
            depth,
            external_messages,
            tool_call_id.clone(),
            &mut metering,
        )
        .await?;
        let final_message = render_review_markdown(&report)
            .map_err(|error| format!("failed to serialize code review report: {error}"))?;

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
    fn tool_review_render_markdown_groups_by_tier_and_includes_coverage() {
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

        let markdown = render_review_markdown(&report).unwrap();

        assert!(markdown.starts_with("## Review summary"));
        assert!(markdown.contains("- Depth: normal"));
        assert!(markdown.contains("Assumed intent: Fix the parser."));
        assert!(markdown.contains("## Findings\n\n### corroborated"));
        assert!(markdown.contains(
            "[rf-1234abcd] src/lib.rs:4-6 — The branch \\| drops errors. (high, unverified, 0.80)"
        ));
        assert!(markdown.contains("Sources: l1_diff@chat, s1_security"));
        assert!(markdown.contains("## Agent coverage"));
        assert!(markdown.contains("| l1_diff@chat | some-model | ran | — | 3 | 1 | 1 | 900 |"));
        assert!(markdown.contains("Impact: Errors are hidden."));
        assert!(markdown.contains("Remediation: Return the error."));
        assert!(markdown.contains("## Checks performed\n\n- excerpt_ok"));
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

        let markdown = render_review_markdown(&report).unwrap();

        assert!(markdown.contains("- Depth: normal"));
        assert!(markdown.contains("- Scope: 1 files"));
        assert!(markdown.contains("Reviewed 1 file. Checks performed: verifier_rejected:2."));
        let json_start = markdown.rfind("```json\n").unwrap() + "```json\n".len();
        let json_end = markdown.rfind("\n```").unwrap();
        let parsed: ReviewReport = serde_json::from_str(&markdown[json_start..json_end]).unwrap();
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
