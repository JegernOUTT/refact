use std::sync::Arc;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;
use async_trait::async_trait;
use axum::http::StatusCode;
use std::collections::HashMap;

use crate::subchat::{run_subchat_once_with_parent, resolve_subchat_params, resolve_subchat_model};
use crate::tools::code_review_candidates::{parse_candidates_with_reasons, ParsedCandidates};
use crate::tools::code_review_evidence::collect_evidence;
use crate::tools::code_review_rank::finalize_review_report;
use crate::tools::code_review_scope::{
    build_review_scope_with_max_files, validate_review_budget, ReviewScope,
};
use crate::tools::code_review_types::{ReviewFinding, ReviewReport, ReviewScopeSummary, ReviewSeverity};
use crate::tools::code_review_verify::{verification_status_label, verify_review_report};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use crate::tools::tool_helpers::{load_code_subagent_config, CodeSubagentConfig};
use crate::tools::subagent_phases::{
    gather_files_phase, get_last_assistant_content, resolve_gathered_file_path, GatherFilesParams,
    DEFAULT_MAX_FILES,
};
use crate::call_validation::{
    ChatMessage, ChatContent, ContextEnum, SubchatParameters, ContextFile, PostprocessSettings,
};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::caps::resolve_chat_model;
use crate::custom_error::ScratchError;
use crate::files_in_workspace::get_file_text_from_memory_or_disk;
use crate::global_context::{GlobalContext, try_load_caps_quickly_if_not_present};
use crate::postprocessing::pp_context_files::postprocess_context_files;
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::tokens::count_text_tokens_with_fallback;
use crate::worktrees::scope::ExecutionScope;

pub struct ToolCodeReview {
    pub config_path: String,
}

const CANDIDATE_RETRY_PROMPT: &str = r#"Your response did not end with a valid candidate envelope. Return the review again and end with exactly one fenced json block matching this schema:

```json
{"summary":"<2-4 sentences>","candidates":[{"file":"<path as given>","line1":1,"line2":10,"category":"correctness|consistency|security|tests|maintainability|performance","severity":"low|medium|high|critical","confidence":0.0,"claim":"<one falsifiable sentence>","rationale":"<short why>"}]}
```

Use an empty candidates array when no issue is plausible. Do not put another fenced json block before or after the envelope."#;

#[derive(Debug)]
struct CandidateStageOutput {
    report: ReviewReport,
    rationales: Vec<String>,
}

impl CandidateStageOutput {
    fn into_report(self) -> ReviewReport {
        let Self { report, rationales } = self;
        drop(rationales);
        report
    }
}

fn get_gather_files_params(
    config: &CodeSubagentConfig,
    focus: Option<String>,
    seed_files: Vec<String>,
) -> GatherFilesParams<'_> {
    GatherFilesParams {
        default_subagent_id: "code_review_gather_files",
        title: "Code Review: Gathering Files",
        default_system_prompt: config.gather_system_prompt.as_deref().unwrap_or(""),
        user_instruction:
            "Based on the conversation above, identify every file relevant to the review. Cast a wide net \u{2014} more related files is better.",
        focus,
        seed_files,
    }
}

async fn make_review_prompt(
    gcx: Arc<GlobalContext>,
    subchat_params: &SubchatParameters,
    scope: &ReviewScope,
    previous_messages: &[ChatMessage],
    config: &CodeSubagentConfig,
) -> Result<String, String> {
    let caps = try_load_caps_quickly_if_not_present(gcx.clone(), 0)
        .await
        .map_err(|x| x.message)?;
    let model_id = resolve_subchat_model(gcx.clone(), subchat_params).await?;
    let model_rec = resolve_chat_model(caps, &model_id)?;
    let tokenizer = crate::tokens::cached_tokenizer(gcx.clone(), &model_rec.base)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))
        .map_err(|x| x.message)?;

    validate_review_budget(subchat_params)?;

    let reviewer_prompt = config
        .reviewer_prompt
        .clone()
        .ok_or("reviewer_prompt not configured for code_review")?;

    let mut tokens_budget = scope.budgets.tokens_budget;
    let mut final_message = match scope.focus.as_deref() {
        Some(focus) => format!("{reviewer_prompt}\n\n# Focus \u{2014} what to check\n{focus}"),
        None => reviewer_prompt.clone(),
    };
    final_message.push_str(&format!(
        "\n\n# Candidate limit\nReturn at most {} candidates.",
        scope.budgets.max_candidates
    ));
    tokens_budget -= count_text_tokens_with_fallback(tokenizer.clone(), &final_message) as i64;

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
                tracing::warn!("code_review: failed to read file '{:?}'", p);
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
            subchat_params.subchat_tokens_for_rag + tokens_budget.max(0) as usize,
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
            "{final_message}\n\n# Conversation\n{context}\n\n# Files to Review\n{files_context}"
        ))
    } else {
        Ok(format!("{final_message}\n\n# Conversation\n{context}"))
    }
}

async fn run_review_pipeline(
    gcx: Arc<GlobalContext>,
    ccx: Arc<AMutex<AtCommandsContext>>,
    scope: ReviewScope,
    subchat_params: SubchatParameters,
    external_messages: Vec<ChatMessage>,
    tool_call_id: String,
    config: &CodeSubagentConfig,
    metering: &mut serde_json::Map<String, serde_json::Value>,
) -> Result<CandidateStageOutput, String> {
    let (subchat_tx, abort_flag, parent_depth, parent_task_meta, parent_worktree) = {
        let ccx_lock = ccx.lock().await;
        (
            ccx_lock.subchat_tx.clone(),
            ccx_lock.abort_flag.clone(),
            ccx_lock.subchat_depth,
            ccx_lock.task_meta.clone(),
            ccx_lock.execution_scope_worktree(),
        )
    };

    let prompt = make_review_prompt(
        gcx.clone(),
        &subchat_params,
        &scope,
        &external_messages,
        config,
    )
    .await?;

    let history: Vec<ChatMessage> = vec![ChatMessage::new("user".to_string(), prompt)];

    let result = run_subchat_once_with_parent(
        gcx.clone(),
        "code_review",
        history,
        tool_call_id.clone(),
        subchat_tx.clone(),
        abort_flag.clone(),
        parent_depth,
        parent_task_meta.clone(),
        parent_worktree.clone(),
    )
    .await?;
    let review_text = get_last_assistant_content(&result.messages);

    let (parsed, fallback_text) = match parse_candidates_with_reasons(&review_text) {
        Ok(parsed) => {
            *metering = result.metering;
            (Some(parsed), review_text)
        }
        Err(first_error) => {
            tracing::info!(
                "code_review: candidate response malformed ({first_error}), requesting retry"
            );
            *metering = result.metering;
            let mut retry_messages = result.messages;
            retry_messages.push(ChatMessage::new(
                "user".to_string(),
                CANDIDATE_RETRY_PROMPT.to_string(),
            ));
            match run_subchat_once_with_parent(
                gcx.clone(),
                "code_review",
                retry_messages,
                tool_call_id.clone(),
                subchat_tx,
                abort_flag,
                parent_depth,
                parent_task_meta,
                parent_worktree,
            )
            .await
            {
                Ok(retry_result) => {
                    let retry_text = get_last_assistant_content(&retry_result.messages);
                    *metering = retry_result.metering;
                    match parse_candidates_with_reasons(&retry_text) {
                        Ok(parsed) => (Some(parsed), retry_text),
                        Err(second_error) => {
                            tracing::warn!(
                                "code_review: candidate response malformed after retry: {second_error}"
                            );
                            (None, retry_text)
                        }
                    }
                }
                Err(retry_error) => {
                    tracing::warn!(
                        "code_review: candidate format retry failed, using legacy response: {retry_error}"
                    );
                    (None, review_text)
                }
            }
        }
    };

    let filenames: Vec<String> = scope
        .files
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    let max_candidates = scope.budgets.max_candidates;
    let scope_summary = ReviewScopeSummary {
        files_reviewed: filenames,
        focus: scope.focus.clone(),
        diff_base: scope.diff_base.clone(),
    };

    let output = match parsed {
        Some(parsed) => candidate_stage_output(scope_summary, parsed, max_candidates),
        None => fallback_candidate_stage_output(scope_summary, fallback_text),
    };
    let output = apply_evidence_stage(gcx.clone(), &scope, output).await;
    let mut report = output.into_report();
    verify_review_report(gcx, ccx, tool_call_id, &mut report, metering).await;
    finalize_review_report(&mut report);
    Ok(CandidateStageOutput {
        report,
        rationales: vec![],
    })
}

async fn apply_evidence_stage(
    gcx: Arc<GlobalContext>,
    scope: &ReviewScope,
    mut output: CandidateStageOutput,
) -> CandidateStageOutput {
    let rejections = collect_evidence(gcx, &scope, &mut output.report.findings).await;
    if !rejections.is_empty() {
        let rejected_indices = rejections
            .iter()
            .map(|rejection| rejection.index)
            .collect::<std::collections::HashSet<_>>();
        output.rationales = output
            .rationales
            .into_iter()
            .enumerate()
            .filter_map(|(index, rationale)| {
                (!rejected_indices.contains(&(index + 1))).then_some(rationale)
            })
            .collect();
    }
    output
        .report
        .checks_performed
        .extend(rejections.iter().map(|rejection| rejection.check_name()));
    output
}

fn candidate_stage_output(
    scope: ReviewScopeSummary,
    mut parsed: ParsedCandidates,
    max_candidates: usize,
) -> CandidateStageOutput {
    parsed.candidates.truncate(max_candidates);
    let mut rationales = Vec::with_capacity(parsed.candidates.len());
    let findings = parsed
        .candidates
        .into_iter()
        .map(|candidate| {
            let (finding, rationale) = candidate.into_review_parts();
            rationales.push(rationale);
            finding
        })
        .collect();

    CandidateStageOutput {
        report: ReviewReport {
            scope,
            findings,
            checks_performed: vec![],
            summary: parsed.summary,
        },
        rationales,
    }
}

fn fallback_candidate_stage_output(
    scope: ReviewScopeSummary,
    reviewer_text: String,
) -> CandidateStageOutput {
    CandidateStageOutput {
        report: ReviewReport {
            scope,
            findings: vec![],
            checks_performed: vec!["candidates_parse_failed".to_string()],
            summary: reviewer_text,
        },
        rationales: vec![],
    }
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
    let verified = report
        .findings
        .iter()
        .filter(|finding| {
            finding.verification_status
                == crate::tools::code_review_types::VerificationStatus::Verified
        })
        .count();
    let needs_human = report
        .findings
        .iter()
        .filter(|finding| {
            finding.verification_status
                == crate::tools::code_review_types::VerificationStatus::NeedsHumanValidation
        })
        .count();
    format!(
        "Review retained {} finding(s): {verified} verified and {needs_human} needing human validation.",
        report.findings.len()
    )
}

fn render_review_markdown(report: &ReviewReport) -> Result<String, serde_json::Error> {
    let focus = report.scope.focus.as_deref().unwrap_or("not specified");
    let diff_base = report.scope.diff_base.as_deref().unwrap_or("not specified");
    let verdict = if report.findings.is_empty() {
        report.summary.trim().to_string()
    } else {
        review_verdict(report)
    };
    let mut output = format!(
        "## Review summary\n\n- Scope: {} files\n- Focus: {}\n- Diff base: {}\n\n{}",
        report.scope.files_reviewed.len(),
        markdown_cell(focus),
        markdown_cell(diff_base),
        verdict
    );
    if !report.findings.is_empty() && !report.summary.trim().is_empty() {
        output.push_str("\n\n");
        output.push_str(report.summary.trim());
    }

    output.push_str("\n\n## Findings");
    for severity in [
        ReviewSeverity::Critical,
        ReviewSeverity::High,
        ReviewSeverity::Medium,
        ReviewSeverity::Low,
    ] {
        let findings = report
            .findings
            .iter()
            .filter(|finding| finding.severity == severity)
            .collect::<Vec<_>>();
        if findings.is_empty() {
            continue;
        }
        output.push_str(&format!("\n\n### {}", severity_label(&severity)));
        for finding in findings {
            output.push_str(&format!(
                "\n\n- [{}] {} — {} ({}, {:.2})",
                finding.id,
                markdown_cell(&finding_location(finding)),
                markdown_cell(&finding.claim),
                verification_status_label(&finding.verification_status),
                finding.confidence,
            ));
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

#[async_trait]
impl Tool for ToolCodeReview {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "code_review".to_string(),
            display_name: "Code Review".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Perform a thorough code review. Optionally pass `what_to_check` (focus/scope) and `files` (initial guess of relevant paths \u{2014} the reviewer starts there and finds more). Finds all related files and checks for bugs, broken integration/wiring, missing tests, inconsistency, and AI-generated 'slop'. The Markdown result ends with a fenced JSON block containing the full machine-parseable ReviewReport.".to_string(),
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

        let config = load_code_subagent_config(gcx.clone(), "code_review", None).await?;

        let external_messages = {
            let ccx_lock = ccx.lock().await;
            ccx_lock.messages.clone()
        };

        let gather_params = get_gather_files_params(&config, what_to_check.clone(), seed_files);

        tracing::info!("code_review: phase 1 - gathering relevant files");
        let important_paths = gather_files_phase(
            gcx.clone(),
            ccx.clone(),
            external_messages.clone(),
            tool_call_id.clone(),
            &config,
            &gather_params,
        )
        .await?;
        let subchat_params = resolve_subchat_params(gcx.clone(), "code_review").await?;
        let scope = build_review_scope_with_max_files(
            gcx.clone(),
            important_paths,
            requested_seed_files,
            what_to_check,
            &subchat_params,
            config.max_files.unwrap_or(DEFAULT_MAX_FILES),
        )
        .await;

        tracing::info!(
            "code_review: phase 2 - performing review on {} files",
            scope.files.len()
        );

        let mut metering = serde_json::Map::new();
        let stage_output = run_review_pipeline(
            gcx,
            ccx.clone(),
            scope,
            subchat_params,
            external_messages,
            tool_call_id.clone(),
            &config,
            &mut metering,
        )
        .await?;
        let report = stage_output.into_report();
        let final_message = render_review_markdown(&report)
            .map_err(|error| format!("failed to serialize code review report: {error}"))?;

        let guardrails_prompt = config
            .guardrails_prompt
            .clone()
            .ok_or("guardrails_prompt not configured for code_review")?;

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
                    content: ChatContent::SimpleText(guardrails_prompt),
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
    use crate::tools::code_review_candidates::parse_candidates_with_reasons;
    use crate::tools::code_review_scope::ReviewBudgets;

    fn scope_summary() -> ReviewScopeSummary {
        ReviewScopeSummary {
            files_reviewed: vec!["src/lib.rs".to_string()],
            focus: None,
            diff_base: None,
        }
    }

    #[test]
    fn tool_code_review_parse_failure_falls_back_to_legacy_report_with_marker() {
        let reviewer_text = "Legacy prose review".to_string();
        let output = fallback_candidate_stage_output(scope_summary(), reviewer_text.clone());

        assert_eq!(output.report.summary, reviewer_text);
        assert!(output.report.findings.is_empty());
        assert_eq!(
            output.report.checks_performed,
            vec!["candidates_parse_failed"]
        );
        assert!(output.rationales.is_empty());
    }

    #[test]
    fn tool_code_review_candidate_output_quarantines_rationales() {
        let parsed = parse_candidates_with_reasons(
            r#"```json
{"summary":"One plausible issue.","candidates":[{"file":"src/lib.rs","line1":4,"line2":4,"category":"correctness","severity":"high","confidence":0.8,"claim":"The branch drops errors.","rationale":"The error arm returns success."}]}
```"#,
        )
        .unwrap();
        let output = candidate_stage_output(scope_summary(), parsed, 30);
        let serialized = serde_json::to_string(&output.report).unwrap();

        assert_eq!(output.rationales, vec!["The error arm returns success."]);
        assert_eq!(output.report.findings.len(), 1);
        assert!(!serialized.contains("rationale"));
        assert!(!serialized.contains("error arm returns success"));
    }

    #[test]
    fn tool_code_review_render_markdown_groups_findings_and_includes_details() {
        let report = ReviewReport {
            scope: scope_summary(),
            findings: vec![ReviewFinding {
                id: "rf-1234abcd".to_string(),
                category: "correctness".to_string(),
                severity: ReviewSeverity::High,
                confidence: 0.8,
                verification_status:
                    crate::tools::code_review_types::VerificationStatus::Unverified,
                file: "src/lib.rs".to_string(),
                line1: 4,
                line2: 6,
                claim: "The branch | drops errors.".to_string(),
                evidence: vec![],
                impact: Some("Errors are hidden.".to_string()),
                remediation: Some("Return the error.".to_string()),
                checks_performed: vec![],
            }],
            checks_performed: vec!["excerpt_ok".to_string()],
            summary: "One plausible issue.".to_string(),
        };

        let markdown = render_review_markdown(&report).unwrap();

        assert!(markdown.starts_with("## Review summary"));
        assert!(markdown.contains("## Findings\n\n### high"));
        assert!(markdown.contains(
            "[rf-1234abcd] src/lib.rs:4-6 — The branch \\| drops errors. (unverified, 0.80)"
        ));
        assert!(markdown.contains("Impact: Errors are hidden."));
        assert!(markdown.contains("Remediation: Return the error."));
        assert!(markdown.contains("## Checks performed\n\n- excerpt_ok"));
    }

    #[test]
    fn tool_code_review_render_markdown_clean_change_golden_and_json_roundtrip() {
        let mut report = ReviewReport {
            scope: scope_summary(),
            findings: vec![],
            checks_performed: vec!["verifier_rejected:2".to_string()],
            summary: "No surviving findings.".to_string(),
        };
        finalize_review_report(&mut report);

        let markdown = render_review_markdown(&report).unwrap();

        assert!(markdown.contains("- Scope: 1 files"));
        assert!(markdown.contains("No verified findings."));
        assert!(markdown.contains("## Checks performed\n\n- verifier_rejected:2"));
        assert!(markdown.contains("Reviewed 1 file. Checks performed: verifier_rejected:2."));
        let json_start = markdown.rfind("```json\n").unwrap() + "```json\n".len();
        let json_end = markdown.rfind("\n```").unwrap();
        let parsed: ReviewReport = serde_json::from_str(&markdown[json_start..json_end]).unwrap();
        assert_eq!(parsed, report);
    }

    #[test]
    fn tool_code_review_default_prompt_yaml_keeps_reviewer_config_key() {
        let config: crate::yaml_configs::customization_types::SubagentConfig =
            serde_yaml::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/crates/refact-yaml-configs/src/defaults/subagents/code_review.yaml"
            )))
            .unwrap();
        let reviewer = config.prompts.reviewer.unwrap();

        assert_eq!(config.schema_version, 3);
        assert!(reviewer.contains("Optimize for recall"));
        assert!(reviewer.contains("exactly one fenced json block"));
    }

    #[test]
    fn tool_code_review_verifier_prompt_is_blind_to_reviewer_rationale() {
        let canary = "PRIVATE_REVIEWER_RATIONALE_CANARY";
        let parsed = parse_candidates_with_reasons(&format!(
            r#"```json
{{"summary":"One plausible issue.","candidates":[{{"file":"src/lib.rs","line1":4,"line2":4,"category":"correctness","severity":"high","confidence":0.8,"claim":"The branch drops errors.","rationale":"{canary}"}}]}}
```"#,
        ))
        .unwrap();
        let mut output = candidate_stage_output(scope_summary(), parsed, 30);
        output.report.findings[0]
            .evidence
            .push(crate::tools::code_review_types::ReviewEvidence {
                kind: "excerpt".to_string(),
                path: Some("src/lib.rs".to_string()),
                line1: Some(4),
                line2: Some(4),
                content: "4: return Ok(())".to_string(),
            });
        let report = output.into_report();
        let batches = crate::tools::code_review_verify::build_verifier_batches(&report.findings);

        assert_eq!(batches.len(), 1);
        assert!(!batches[0].prompt.contains(canary));
        assert!(!batches[0].prompt.contains("rationale"));
        assert!(!batches[0].prompt.contains("confidence"));
        assert!(batches[0].prompt.contains("4: return Ok(())"));
    }

    #[test]
    fn tool_code_review_verifier_yaml_loads_with_expected_contract() {
        let config: crate::yaml_configs::customization_types::SubagentConfig =
            serde_yaml::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/crates/refact-yaml-configs/src/defaults/subagents/code_review_verifier.yaml"
            )))
            .unwrap();
        let prompt = config.messages.system_prompt.unwrap();

        assert_eq!(config.schema_version, 4);
        assert_eq!(config.id, "code_review_verifier");
        assert_eq!(config.subchat.n_ctx, Some(64_000));
        assert_eq!(config.subchat.max_new_tokens, Some(4_000));
        assert!(prompt.contains("skeptical fact-checker"));
        assert!(prompt.contains("needs_human_validation"));
        assert!(config.tools.is_empty());
    }

    #[tokio::test]
    async fn tool_code_review_evidence_stage_records_structural_rejections() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("src/lib.rs");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "fn present() {}\n").unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
        let parsed = parse_candidates_with_reasons(&format!(
            r#"```json
{{"summary":"Candidates.","candidates":[{{"file":"{}","line1":9,"line2":9,"category":"correctness","severity":"high","confidence":0.8,"claim":"The range is fabricated.","rationale":"Private rationale."}}]}}
```"#,
            file.to_string_lossy()
        ))
        .unwrap();
        let scope = ReviewScope {
            files: vec![file],
            seed_files: vec![],
            focus: None,
            diff_base: None,
            changed_files: vec![],
            diff_patch: None,
            budgets: ReviewBudgets {
                max_files: 1,
                tokens_budget: 10_000,
                max_candidates: 30,
            },
        };

        let output = apply_evidence_stage(
            gcx,
            &scope,
            candidate_stage_output(scope_summary(), parsed, 30),
        )
        .await;

        assert!(output.report.findings.is_empty());
        assert!(output.rationales.is_empty());
        assert_eq!(
            output.report.checks_performed,
            vec!["evidence_reject:1:range_out_of_bounds"]
        );
    }
}
