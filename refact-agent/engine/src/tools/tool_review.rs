use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::global_context::GlobalContext;
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::tools::review_agents::config::{
    load_review_config, slot_model_id, stage_subchat_spec, ReviewConfig,
};
use crate::tools::review_agents::prompts::{build_adversarial_prompt, build_stage_prompt};
use crate::tools::review_agents::runner::{
    merge_metering, now_ms, stage_trace_chat_id, StageCtx, StageJob,
};
use crate::tools::review_agents::stages::{load_stage_catalog, select_stages, StagePhase, StageSpec};
use crate::tools::review_agents::{
    run_stage_jobs, ScheduleParams, StageExecutor, SubchatExecutor, HARVEST_GRACE,
};
use crate::tools::review_evidence::verify_evidence;
use crate::tools::review_merge::{merge_findings, rank_findings};
use crate::tools::review_scope::{build_review_scope, DiffAttribution, ReviewScope, ScopeRequest};
use crate::tools::review_types::{
    Dispute, ReviewDepth, ReviewFinding, ReviewOutcome, ReviewReport, ReviewSeverity, ScopeMode,
    StageRun, StageStatusKind,
};
use crate::tools::subagent_phases::resolve_gathered_file_path;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use crate::worktrees::scope::ExecutionScope;

pub struct ToolCodeReview {
    pub config_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReviewArgs {
    what_to_check: Option<String>,
    files: Vec<String>,
    base: Option<String>,
    plan: Option<String>,
    scope_mode: ScopeMode,
    stages: Option<Vec<String>>,
    depth: Option<ReviewDepth>,
    parallel_depth: Option<usize>,
    variants: Option<usize>,
    browser_scenario: Option<String>,
}

fn optional_string(args: &HashMap<String, Value>, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn string_list(args: &HashMap<String, Value>, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn positive_number(args: &HashMap<String, Value>, key: &str) -> Result<Option<u64>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            let number = value
                .as_u64()
                .ok_or_else(|| format!("argument `{key}` must be a positive integer"))?;
            if number == 0 {
                return Err(format!("argument `{key}` must be at least 1"));
            }
            Ok(Some(number))
        }
    }
}

fn parse_args(args: &HashMap<String, Value>) -> Result<ReviewArgs, String> {
    let files = string_list(args, "files");
    let base = optional_string(args, "base");
    let depth = match optional_string(args, "depth") {
        Some(value) => Some(
            ReviewDepth::parse(&value)
                .ok_or_else(|| format!("invalid depth '{value}', expected: normal, deep"))?,
        ),
        None => None,
    };
    let scope_mode = match optional_string(args, "scope_mode") {
        Some(value) => ScopeMode::parse(&value).ok_or_else(|| {
            format!("invalid scope_mode '{value}', expected: strict, adjacent, broad")
        })?,
        None if !files.is_empty() || base.is_some() => ScopeMode::Strict,
        None => ScopeMode::Broad,
    };
    let stages = {
        let requested = string_list(args, "stages");
        (!requested.is_empty()).then_some(requested)
    };
    let browser_requested = match args.get("browser") {
        None | Some(Value::Null) => false,
        Some(value) => value
            .as_bool()
            .ok_or_else(|| "argument `browser` must be a boolean".to_string())?,
    };
    let browser_scenario = optional_string(args, "browser_scenario");
    if browser_requested && browser_scenario.is_none() {
        return Err(
            "argument `browser_scenario` is required when `browser` is true: name the URL or route, the states to reach, and what must be true".to_string(),
        );
    }
    let variants = positive_number(args, "variants")?;
    if let Some(variants) = variants {
        if variants > 3 {
            return Err("argument `variants` must be between 1 and 3".to_string());
        }
    }
    Ok(ReviewArgs {
        what_to_check: optional_string(args, "what_to_check"),
        files,
        base,
        plan: optional_string(args, "plan"),
        scope_mode,
        stages,
        depth,
        parallel_depth: positive_number(args, "parallel_depth")?.map(|value| value as usize),
        variants: variants.map(|value| value as usize),
        browser_scenario,
    })
}

fn format_duration(duration_ms: u64) -> String {
    let seconds = duration_ms / 1000;
    if seconds < 60 {
        return format!("{seconds}s");
    }
    format!("{}m{:02}s", seconds / 60, seconds % 60)
}

fn markdown_cell(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('|', "\\|")
}

fn scratch_dir(gcx_project: Option<&Path>, review_id: &str) -> Option<PathBuf> {
    gcx_project.map(|root| root.join(".refact").join("review_scratch").join(review_id))
}

async fn write_scratch(dir: &Path, stage: &str, payload: &Value) -> bool {
    if tokio::fs::create_dir_all(dir).await.is_err() {
        return false;
    }
    let path = dir.join(format!("{stage}.json"));
    match serde_json::to_string_pretty(payload) {
        Ok(text) => tokio::fs::write(path, text).await.is_ok(),
        Err(_) => false,
    }
}

const BROWSER_STAGE: &str = "browser";

fn apply_browser_scenario(
    catalog: &[StageSpec],
    scheduled: &mut Vec<StageSpec>,
    skipped: &mut Vec<(String, String)>,
    scenario: Option<&str>,
) {
    match scenario {
        None => {
            if let Some(index) = scheduled.iter().position(|spec| spec.id == BROWSER_STAGE) {
                scheduled.remove(index);
                skipped.push((
                    BROWSER_STAGE.to_string(),
                    "no browser_scenario given".to_string(),
                ));
            }
        }
        Some(_) => {
            if scheduled.iter().any(|spec| spec.id == BROWSER_STAGE) {
                return;
            }
            let Some(spec) = catalog.iter().find(|spec| spec.id == BROWSER_STAGE) else {
                return;
            };
            skipped.retain(|(id, _)| id != BROWSER_STAGE);
            scheduled.push(spec.clone());
        }
    }
}

struct StagePlan {
    jobs: Vec<StageJob>,
    rows: Vec<StageRun>,
}

#[allow(clippy::too_many_arguments)]
async fn plan_jobs(
    gcx: Arc<GlobalContext>,
    cfg: &ReviewConfig,
    review_id: &str,
    scope: &ReviewScope,
    specs: Vec<StageSpec>,
    variants: usize,
    browser_scenario: Option<&str>,
) -> StagePlan {
    let mut jobs = Vec::new();
    let mut rows = Vec::new();
    let slots = cfg.settings.slots_for_variants(variants);
    for spec in specs {
        let overrides = cfg.settings.stage_override(&spec.id);
        overrides.warn_if_deprecated(&spec.id);
        if overrides.enabled == Some(false) {
            rows.push(StageRun::not_run(
                &spec.id,
                "disabled in review_agents.yaml",
            ));
            continue;
        }
        let stage_slots = match overrides.model_slot {
            Some(slot) => vec![slot],
            None if spec.phase == StagePhase::PostMerge => vec![slots[0]],
            None => slots.clone(),
        };
        let spec = Arc::new(spec);
        for (index, slot) in stage_slots.iter().enumerate() {
            let model = match slot_model_id(gcx.clone(), *slot).await {
                Ok(model) => model,
                Err(error) => {
                    rows.push(StageRun::not_run(&spec.id, &error));
                    continue;
                }
            };
            let label = if stage_slots.len() > 1 {
                format!("{}@{}", spec.id, slot.label())
            } else {
                spec.id.clone()
            };
            if index > 0 && spec.writes_allowed {
                continue;
            }
            let trace_chat_id = stage_trace_chat_id(review_id, &label);
            jobs.push(StageJob {
                spec: spec.clone(),
                label,
                subchat: stage_subchat_spec(&cfg.base_params, &cfg.settings, model),
                max_steps: overrides.max_steps.unwrap_or(cfg.settings.max_steps).max(1),
                prompt: build_stage_prompt(
                    &spec,
                    scope,
                    (spec.id == BROWSER_STAGE)
                        .then_some(browser_scenario)
                        .flatten(),
                ),
                trace_chat_id,
                abort: Arc::new(AtomicBool::new(false)),
            });
        }
    }
    StagePlan { jobs, rows }
}

fn apply_verdicts(
    findings: &mut [ReviewFinding],
    stage: &str,
    verdicts: &[crate::tools::review_agents::contract::Verdict],
) -> usize {
    let mut disputed = 0;
    for verdict in verdicts {
        if verdict.supported {
            continue;
        }
        if let Some(finding) = findings.iter_mut().find(|finding| finding.id == verdict.id) {
            finding.disputed = Some(Dispute {
                stage: stage.to_string(),
                reason: match verdict.reason.trim() {
                    "" => "the adversarial pass could not support this claim".to_string(),
                    reason => reason.to_string(),
                },
            });
            disputed += 1;
        }
    }
    disputed
}

fn stage_line(run: &StageRun) -> String {
    let status = match run.status {
        StageStatusKind::Ok => format!("ok {}", format_duration(run.duration_ms)),
        StageStatusKind::TimedOut => format!("timed out {}", format_duration(run.duration_ms)),
        StageStatusKind::Failed => format!("failed {}", format_duration(run.duration_ms)),
        StageStatusKind::NotRun => "not run".to_string(),
    };
    let mut line = format!("{} {status}", run.name);
    if let Some(reason) = run.reason.as_deref() {
        line.push_str(&format!(" ({})", markdown_cell(reason)));
    }
    if run.status != StageStatusKind::Ok {
        if let Some(trace) = run.trace_chat_id.as_deref() {
            line.push_str(&format!(" [transcript {}]", markdown_cell(trace)));
        }
    }
    line
}

fn coverage_line(run: &StageRun) -> Option<String> {
    let coverage = &run.coverage;
    let mut parts = Vec::new();
    if !coverage.files_read.is_empty() {
        parts.push(format!("{} file(s) read", coverage.files_read.len()));
    }
    if !coverage.commands_run.is_empty() {
        let failed = coverage
            .commands_run
            .iter()
            .filter(|command| command.exit != 0)
            .count();
        parts.push(format!(
            "{} command(s) run, {failed} non-zero",
            coverage.commands_run.len()
        ));
    }
    if !coverage.tools_unavailable.is_empty() {
        parts.push(format!(
            "unavailable: {}",
            markdown_cell(&coverage.tools_unavailable.join(", "))
        ));
    }
    if let Some(stopped) = coverage.stopped_early.as_deref() {
        parts.push(format!("stopped early: {}", markdown_cell(stopped)));
    }
    if let Some(summary) = run.summary.as_deref() {
        parts.push(markdown_cell(summary));
    }
    (!parts.is_empty()).then(|| parts.join("; "))
}

fn render_finding(finding: &ReviewFinding) -> String {
    let mut line = format!(
        "\n- [{}] {}:{}-{} — {}",
        finding.id,
        markdown_cell(&finding.file),
        finding.line_start,
        finding.line_end,
        markdown_cell(&finding.claim),
    );
    let mut tags = vec![format!("stage: {}", finding.reported_by.join("+"))];
    if let Some(reproduction) = finding.reproduction.as_deref() {
        tags.push(format!("repro: {}", markdown_cell(reproduction)));
    }
    if !finding.evidence_present {
        tags.push("evidence not found in file".to_string());
    }
    if !finding.introduced_by_diff {
        tags.push("pre-existing".to_string());
    }
    if finding.out_of_scope {
        tags.push("out of scope".to_string());
    }
    if let Some(dispute) = finding.disputed.as_ref() {
        tags.push(format!("disputed: {}", markdown_cell(&dispute.reason)));
    }
    line.push_str(&format!("\n  - {}", tags.join(" · ")));
    if let Some(fix) = finding.fix.as_deref() {
        line.push_str(&format!("\n  - fix: {}", markdown_cell(fix)));
    }
    if !finding.locations.is_empty() {
        let extra = finding
            .locations
            .iter()
            .map(|location| {
                format!(
                    "{}:{}-{}",
                    location.file, location.line_start, location.line_end
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        line.push_str(&format!("\n  - also at: {}", markdown_cell(&extra)));
    }
    line
}

fn incomplete_stage_lines(report: &ReviewReport) -> String {
    report
        .incomplete_stages()
        .into_iter()
        .map(|run| {
            format!(
                "\n- {}: {} ({})",
                run.name,
                run.status.as_str(),
                markdown_cell(run.reason.as_deref().unwrap_or("no reason reported"))
            )
        })
        .collect::<Vec<_>>()
        .join("")
}

pub fn render_review_markdown(report: &ReviewReport) -> String {
    let scope = &report.scope;
    let badge = match report.outcome {
        ReviewOutcome::Reviewed => "",
        ReviewOutcome::Partial => " ⚠️ PARTIAL",
        ReviewOutcome::Inconclusive => " ⚠️ INCONCLUSIVE",
    };
    let mut header = format!(
        "## Review{badge} · {} file(s) requested → {} reviewed ({}",
        scope.requested_files, scope.reviewed_files, scope.mode
    );
    if let Some(expansion) = scope.expansion.as_deref() {
        header.push_str(&format!(", {}", markdown_cell(expansion)));
    }
    header.push(')');
    match report.diff.base.as_deref() {
        Some(base) => header.push_str(&format!(
            " · base {}..{} ({} changed file(s), {} hunk(s))",
            markdown_cell(base),
            markdown_cell(report.diff.head.as_deref().unwrap_or("HEAD")),
            report.diff.changed_files,
            report.diff.hunks
        )),
        None => header.push_str(" · no git diff available"),
    }
    header.push_str(&format!(
        " · depth {} · {}",
        report.depth,
        format_duration(report.duration_ms)
    ));

    let mut output = header;
    output.push_str("\n\nStages: ");
    output.push_str(
        &report
            .stages
            .iter()
            .map(stage_line)
            .collect::<Vec<_>>()
            .join(" · "),
    );

    let facts = report.facts();
    let hypotheses = report.hypotheses();
    output.push_str(&format!(
        "\n\nFindings: {} supported ({} with a reproduction) · {} hypotheses · {} duplicate(s) merged · {} pre-existing · {} out of scope",
        facts.len(),
        report.reproduced(),
        hypotheses.len(),
        report.duplicates_merged,
        report.pre_existing(),
        scope.out_of_scope_findings,
    ));

    for severity in [
        ReviewSeverity::Blocker,
        ReviewSeverity::High,
        ReviewSeverity::Medium,
        ReviewSeverity::Low,
        ReviewSeverity::Note,
    ] {
        let bucket: Vec<&ReviewFinding> = facts
            .iter()
            .copied()
            .filter(|finding| finding.severity == severity)
            .collect();
        if bucket.is_empty() {
            continue;
        }
        output.push_str(&format!("\n\n### {} ({})", severity.as_str(), bucket.len()));
        for finding in bucket {
            output.push_str(&render_finding(finding));
        }
    }
    if facts.is_empty() {
        let completed = report.completed_stages();
        let incomplete = report.stages.len() - completed;
        match report.outcome {
            ReviewOutcome::Reviewed => output.push_str("\n\n### No supported findings"),
            ReviewOutcome::Inconclusive => {
                output.push_str(&format!(
                    "\n\n### Nothing was checked — this is NOT a pass\n\nEvery stage of this review ended without producing a result, so no part of the change was actually reviewed. Do not read the empty findings list as approval. Incomplete stage(s) ({incomplete}):{}",
                    incomplete_stage_lines(report)
                ));
            }
            ReviewOutcome::Partial => {
                output.push_str(&format!(
                    "\n\n### No supported findings from the {completed} stage(s) that completed\n\n{incomplete} stage(s) did not complete, so their part of the change was not reviewed:{}",
                    incomplete_stage_lines(report)
                ));
            }
        }
    } else if report.outcome != ReviewOutcome::Reviewed {
        let incomplete = report.stages.len() - report.completed_stages();
        output.push_str(&format!(
            "\n\n### Stages that did not complete ({incomplete}){}",
            incomplete_stage_lines(report)
        ));
    }

    if !hypotheses.is_empty() {
        output.push_str(&format!(
            "\n\n### Hypotheses ({}) — no reproduction and no verified evidence; you decide",
            hypotheses.len()
        ));
        for finding in hypotheses {
            output.push_str(&render_finding(finding));
        }
    }

    let coverage: Vec<String> = report
        .stages
        .iter()
        .filter_map(|run| coverage_line(run).map(|line| format!("\n- {}: {line}", run.name)))
        .collect();
    if !coverage.is_empty() {
        output.push_str("\n\n### Stage coverage");
        for line in coverage {
            output.push_str(&line);
        }
    }
    if !scope.dropped_files.is_empty() {
        output.push_str(&format!(
            "\n\n{} file(s) were not reviewed (max_files cap); the full list is in the machine-readable report.",
            scope.dropped_files.len()
        ));
    }
    if let Some(dir) = report.scratch_dir.as_deref() {
        output.push_str(&format!("\n\nRaw per-stage output: {dir}"));
    }
    output.push_str(
        "\n\nThe machine-readable ReviewReport is attached to this tool result's metadata.",
    );
    output
}

fn review_refs(report: &ReviewReport, roots: &[PathBuf]) -> (Value, bool) {
    let mut references = Vec::new();
    for finding in &report.findings {
        if references.len() == 32 {
            return (Value::Array(references), true);
        }
        let path = Path::new(&finding.file);
        let relative = if path.is_absolute() {
            roots.iter().find_map(|root| {
                path.strip_prefix(root)
                    .ok()
                    .map(|path| path.to_string_lossy().replace('\\', "/"))
            })
        } else {
            Some(path.to_string_lossy().replace('\\', "/"))
        };
        let Some(relative) = relative else {
            continue;
        };
        references.push(json!({
            "id": finding.id,
            "severity": finding.severity.as_str(),
            "path": relative,
            "line1": finding.line_start,
            "line2": finding.line_end,
            "stage": finding.stage,
            "hypothesis": finding.is_hypothesis(),
        }));
    }
    (Value::Array(references), false)
}

#[allow(clippy::too_many_arguments)]
async fn run_review(
    gcx: Arc<GlobalContext>,
    ctx: StageCtx,
    cfg: ReviewConfig,
    args: ReviewArgs,
    metering: &mut serde_json::Map<String, Value>,
) -> Result<ReviewReport, String> {
    let started = now_ms();
    let execution_scope = ctx.worktree.as_ref().map(ExecutionScope::from_worktree);
    let mut requested = Vec::new();
    for file in &args.files {
        if let Some(path) =
            resolve_gathered_file_path(gcx.clone(), execution_scope.as_ref(), file).await
        {
            if !requested.contains(&path) {
                requested.push(path);
            }
        }
    }
    let requested_count = requested.len();

    let scope = build_review_scope(
        gcx.clone(),
        ScopeRequest {
            requested,
            mode: args.scope_mode,
            base: args.base.clone(),
            focus: args.what_to_check.clone(),
            plan: args.plan.clone(),
            max_files: cfg.settings.max_files,
        },
    )
    .await;

    let depth = cfg.settings.depth_or_default(args.depth);
    let catalog = load_stage_catalog(gcx.clone()).await;
    let mut selection = select_stages(
        catalog.clone(),
        depth,
        args.stages.as_deref(),
        &scope.file_strings(),
    )?;
    apply_browser_scenario(
        &catalog,
        &mut selection.scheduled,
        &mut selection.skipped,
        args.browser_scenario.as_deref(),
    );

    let mut stage_rows: Vec<StageRun> = selection
        .skipped
        .iter()
        .map(|(id, reason)| StageRun::not_run(id, reason))
        .collect();

    let (post_merge, parallel): (Vec<StageSpec>, Vec<StageSpec>) = selection
        .scheduled
        .into_iter()
        .partition(|spec| spec.phase == StagePhase::PostMerge);

    let variants = args.variants.unwrap_or(cfg.settings.variants).clamp(1, 3);
    let plan = plan_jobs(
        gcx.clone(),
        &cfg,
        &ctx.review_id,
        &scope,
        parallel,
        variants,
        args.browser_scenario.as_deref(),
    )
    .await;
    stage_rows.extend(plan.rows);

    let schedule = ScheduleParams {
        parallel_depth: args
            .parallel_depth
            .unwrap_or(cfg.settings.parallel_depth)
            .max(1),
        idle_timeout: Duration::from_secs(cfg.settings.idle_timeout_secs),
        grace: HARVEST_GRACE,
    };

    let scratch = scratch_dir(
        crate::files_correction::get_project_dirs(gcx.clone())
            .await
            .first()
            .map(PathBuf::as_path),
        &ctx.review_id,
    );
    let executor: Arc<dyn StageExecutor> = Arc::new(SubchatExecutor {
        gcx: gcx.clone(),
        ctx: ctx.clone(),
    });

    let mut findings: Vec<ReviewFinding> = Vec::new();
    let mut scratch_written = false;
    for product in run_stage_jobs(executor.clone(), plan.jobs, schedule).await {
        merge_metering(metering, product.metering);
        if let (Some(dir), Some(raw)) = (scratch.as_ref(), product.raw.as_ref()) {
            scratch_written |= write_scratch(
                dir,
                &product.run.name,
                &json!({"run": product.run, "raw": raw}),
            )
            .await;
        }
        findings.extend(product.findings);
        stage_rows.push(product.run);
    }

    verify_evidence(gcx.clone(), &scope, &mut findings).await;
    let attribution = DiffAttribution::new(&scope);
    for finding in &mut findings {
        finding.introduced_by_diff = attribution
            .introduced(&finding.file, finding.line_start, finding.line_end)
            .await;
        finding.out_of_scope = !scope.in_scope(&finding.file);
    }
    let (mut findings, duplicates_merged) = merge_findings(findings);
    rank_findings(&mut findings);

    for spec in post_merge {
        if findings.is_empty() {
            stage_rows.push(StageRun::not_run(&spec.id, "no findings to check"));
            continue;
        }
        let overrides = cfg.settings.stage_override(&spec.id);
        overrides.warn_if_deprecated(&spec.id);
        if overrides.enabled == Some(false) {
            stage_rows.push(StageRun::not_run(
                &spec.id,
                "disabled in review_agents.yaml",
            ));
            continue;
        }
        let slot = overrides.model_slot.unwrap_or(cfg.settings.model_slot);
        let model = match slot_model_id(gcx.clone(), slot).await {
            Ok(model) => model,
            Err(error) => {
                stage_rows.push(StageRun::not_run(&spec.id, &error));
                continue;
            }
        };
        let prompt = build_adversarial_prompt(&spec, &scope, &findings);
        let label = spec.id.clone();
        let job = StageJob {
            spec: Arc::new(spec),
            trace_chat_id: stage_trace_chat_id(&ctx.review_id, &label),
            label,
            subchat: stage_subchat_spec(&cfg.base_params, &cfg.settings, model),
            max_steps: overrides.max_steps.unwrap_or(cfg.settings.max_steps).max(1),
            prompt,
            abort: Arc::new(AtomicBool::new(false)),
        };
        for mut product in run_stage_jobs(executor.clone(), vec![job], schedule).await {
            merge_metering(metering, std::mem::take(&mut product.metering));
            if let (Some(dir), Some(raw)) = (scratch.as_ref(), product.raw.as_ref()) {
                scratch_written |= write_scratch(
                    dir,
                    &product.run.name,
                    &json!({"run": product.run, "raw": raw}),
                )
                .await;
            }
            let disputed = apply_verdicts(&mut findings, &product.run.name, &product.verdicts);
            let mut run = product.run;
            run.findings = disputed;
            stage_rows.push(run);
        }
    }

    let mut scope_summary = scope.summary();
    scope_summary.requested_files = requested_count;
    scope_summary.out_of_scope_findings = findings
        .iter()
        .filter(|finding| finding.out_of_scope)
        .count();

    let mut report = ReviewReport {
        depth: depth.as_str().to_string(),
        outcome: ReviewOutcome::Inconclusive,
        scope: scope_summary,
        diff: scope.diff_summary(),
        stages: stage_rows,
        findings,
        duration_ms: now_ms().saturating_sub(started),
        duplicates_merged,
        scratch_dir: scratch
            .filter(|_| scratch_written)
            .map(|dir| dir.to_string_lossy().to_string()),
    };
    report.outcome = report.derive_outcome();
    Ok(report)
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
            description: "Run a multi-stage agentic code review of the current change. Each stage is an agent that investigates with tools and reports findings with quoted evidence and, where it could get one, an executed reproduction. Findings the pipeline could verify are reported as facts; the rest are reported as hypotheses for you to triage. Every stage reports what it read, ran, and could not do. The machine-readable ReviewReport is attached to the tool result metadata.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "what_to_check": {
                        "type": "string",
                        "description": "Optional. Focus text: what to review and what to look for."
                    },
                    "files": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Optional. Files to review. When given, scope_mode defaults to strict."
                    },
                    "base": {
                        "type": "string",
                        "description": "Optional. Git ref to diff against; the review uses the merge-base of this ref and HEAD. Defaults to the upstream or main branch."
                    },
                    "plan": {
                        "type": "string",
                        "description": "Optional. Plan or acceptance criteria the change must satisfy; drives the spec stage."
                    },
                    "scope_mode": {
                        "type": "string",
                        "enum": ["strict", "adjacent", "broad"],
                        "description": "Optional. strict reports only on the requested and changed files, adjacent adds one dependency hop, broad allows anything. Defaults to strict when files or base are given, otherwise broad."
                    },
                    "stages": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Optional. Explicit stage list, overrides depth. Unknown names are rejected with the catalog."
                    },
                    "browser": {
                        "type": "boolean",
                        "description": "Optional. Run the browser stage. Requires browser_scenario; the browser stage never runs without one."
                    },
                    "browser_scenario": {
                        "type": "string",
                        "description": "What the browser stage must drive: the URL or route, the states to reach, the interactions to perform, and what must be true afterwards. Supplying it enables the browser stage at any depth."
                    },
                    "depth": {
                        "type": "string",
                        "enum": ["normal", "deep"],
                        "description": "Optional. normal runs the cheap stages; deep adds tests, execution, browser and the adversarial pass."
                    },
                    "parallel_depth": {
                        "type": "integer",
                        "description": "Optional. Stage agents running concurrently. Defaults to 4; use 1-2 on a laptop."
                    },
                    "variants": {
                        "type": "integer",
                        "description": "Optional. Model variants per stage, 1-3. Defaults to 1."
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
        let args = parse_args(args)?;
        let (gcx, ctx) = {
            let ccx_lock = ccx.lock().await;
            let gcx = ccx_lock.app.gcx.clone();
            let ctx = StageCtx {
                review_id: format!("rv-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]),
                tool_call_id: tool_call_id.clone(),
                subchat_tx: ccx_lock.subchat_tx.clone(),
                abort_flag: ccx_lock.abort_flag.clone(),
                depth: ccx_lock.subchat_depth,
                task_meta: ccx_lock.task_meta.clone(),
                worktree: ccx_lock.execution_scope_worktree(),
                chat_id: ccx_lock.chat_id.clone(),
                root_chat_id: ccx_lock.root_chat_id.clone(),
                activity: None,
            };
            (gcx, ctx)
        };

        let cfg = load_review_config(gcx.clone()).await?;
        let guardrails = cfg.guardrails_prompt.clone();
        let mut metering = serde_json::Map::new();
        let report = run_review(gcx.clone(), ctx, cfg, args, &mut metering).await?;
        let markdown = render_review_markdown(&report);

        metering.insert(
            "review_report".to_string(),
            serde_json::to_value(&report)
                .map_err(|error| format!("failed to serialize review report: {error}"))?,
        );
        let (refs, truncated) = review_refs(
            &report,
            &crate::files_correction::get_project_dirs(gcx.clone()).await,
        );
        metering.insert("review_refs".to_string(), refs);
        metering.insert("review_refs_truncated".to_string(), Value::Bool(truncated));

        Ok((
            false,
            vec![
                ContextEnum::ChatMessage(ChatMessage {
                    role: "tool".to_string(),
                    content: ChatContent::SimpleText(markdown),
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
                    content: ChatContent::SimpleText(guardrails),
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
    use crate::tools::review_agents::contract::Verdict;
    use crate::tools::review_types::{
        CommandRun, FindingLocation, ReviewDiffSummary, ReviewScopeSummary, StageCoverage,
    };

    fn args_map(pairs: Vec<(&str, Value)>) -> HashMap<String, Value> {
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect()
    }

    fn finding(id: &str, severity: ReviewSeverity) -> ReviewFinding {
        ReviewFinding {
            id: id.to_string(),
            stage: "diff".to_string(),
            model: Some("model-a".to_string()),
            title: "Error dropped".to_string(),
            severity,
            file: "src/lib.rs".to_string(),
            line_start: 10,
            line_end: 14,
            claim: "The error arm returns success.".to_string(),
            evidence: "return Ok(());".to_string(),
            evidence_present: true,
            reproduction: Some("cargo test -p thing".to_string()),
            fix: Some("Propagate the error.".to_string()),
            introduced_by_diff: true,
            out_of_scope: false,
            reported_by: vec!["diff".to_string()],
            locations: vec![],
            disputed: None,
        }
    }

    fn report(findings: Vec<ReviewFinding>, stages: Vec<StageRun>) -> ReviewReport {
        let mut report = ReviewReport {
            depth: "normal".to_string(),
            outcome: ReviewOutcome::Inconclusive,
            scope: ReviewScopeSummary {
                mode: "strict".to_string(),
                requested_files: 12,
                reviewed_files: 14,
                files: vec!["src/lib.rs".to_string()],
                focus: None,
                expansion: Some("+2 dependency edges".to_string()),
                out_of_scope_findings: 1,
                dropped_files: vec![],
            },
            diff: ReviewDiffSummary {
                base: Some("1a2b3c".to_string()),
                head: Some("HEAD".to_string()),
                changed_files: 7,
                hunks: 31,
            },
            stages,
            findings,
            duration_ms: 252_000,
            duplicates_merged: 3,
            scratch_dir: Some(".refact/review_scratch/rv-1".to_string()),
        };
        report.outcome = report.derive_outcome();
        report
    }

    #[test]
    fn tool_review_args_default_scope_mode_to_strict_when_files_are_given() {
        let with_files = parse_args(&args_map(vec![("files", json!(["src/lib.rs"]))])).unwrap();
        let with_base = parse_args(&args_map(vec![("base", json!("main"))])).unwrap();
        let bare = parse_args(&args_map(vec![])).unwrap();

        assert_eq!(with_files.scope_mode, ScopeMode::Strict);
        assert_eq!(with_files.files, ["src/lib.rs"]);
        assert_eq!(with_base.scope_mode, ScopeMode::Strict);
        assert_eq!(bare.scope_mode, ScopeMode::Broad);
        assert_eq!(bare.stages, None);
        assert_eq!(bare.variants, None);
    }

    #[test]
    fn tool_review_args_reject_out_of_range_values() {
        assert!(parse_args(&args_map(vec![("depth", json!("max"))]))
            .unwrap_err()
            .contains("invalid depth"));
        assert!(
            parse_args(&args_map(vec![("scope_mode", json!("everything"))]))
                .unwrap_err()
                .contains("invalid scope_mode")
        );
        assert!(parse_args(&args_map(vec![("variants", json!(4))]))
            .unwrap_err()
            .contains("between 1 and 3"));
        assert!(parse_args(&args_map(vec![("parallel_depth", json!(0))]))
            .unwrap_err()
            .contains("at least 1"));
        assert_eq!(
            parse_args(&args_map(vec![
                ("stages", json!(["diff", "security"])),
                ("parallel_depth", json!(2))
            ]))
            .unwrap()
            .stages,
            Some(vec!["diff".to_string(), "security".to_string()])
        );
    }

    #[test]
    fn tool_review_browser_flag_requires_a_scenario() {
        let error = parse_args(&args_map(vec![("browser", json!(true))])).unwrap_err();
        assert!(error.contains("`browser_scenario` is required"));

        let with_scenario = parse_args(&args_map(vec![
            ("browser", json!(true)),
            (
                "browser_scenario",
                json!("open /settings, expect no console errors"),
            ),
        ]))
        .unwrap();
        assert_eq!(
            with_scenario.browser_scenario.as_deref(),
            Some("open /settings, expect no console errors")
        );

        let scenario_only =
            parse_args(&args_map(vec![("browser_scenario", json!("open /"))])).unwrap();
        assert_eq!(scenario_only.browser_scenario.as_deref(), Some("open /"));
        assert!(parse_args(&args_map(vec![]))
            .unwrap()
            .browser_scenario
            .is_none());
        assert!(parse_args(&args_map(vec![("browser", json!("yes"))]))
            .unwrap_err()
            .contains("must be a boolean"));
    }

    #[test]
    fn tool_review_browser_stage_runs_only_with_a_scenario() {
        let catalog = crate::tools::review_agents::stages::embedded_catalog();
        let browser = catalog
            .iter()
            .find(|spec| spec.id == "browser")
            .unwrap()
            .clone();

        let mut scheduled = vec![browser.clone()];
        let mut skipped = Vec::new();
        apply_browser_scenario(&catalog, &mut scheduled, &mut skipped, None);
        assert!(scheduled.is_empty());
        assert_eq!(
            skipped,
            vec![(
                "browser".to_string(),
                "no browser_scenario given".to_string()
            )]
        );

        let mut scheduled = Vec::new();
        let mut skipped = vec![(
            "browser".to_string(),
            "no matching files in scope".to_string(),
        )];
        apply_browser_scenario(&catalog, &mut scheduled, &mut skipped, Some("open /"));
        assert_eq!(scheduled.len(), 1);
        assert_eq!(scheduled[0].id, "browser");
        assert!(skipped.is_empty());

        let mut scheduled = vec![browser];
        let mut skipped = Vec::new();
        apply_browser_scenario(&catalog, &mut scheduled, &mut skipped, Some("open /"));
        assert_eq!(scheduled.len(), 1);
    }

    #[test]
    fn tool_review_render_reports_scope_stages_and_fact_vs_hypothesis() {
        let mut hypothesis = finding("rf-2", ReviewSeverity::Medium);
        hypothesis.reproduction = None;
        hypothesis.evidence_present = false;
        hypothesis.claim = "Maybe the cache is stale.".to_string();
        let mut merged = finding("rf-3", ReviewSeverity::High);
        merged.reported_by = vec!["diff".to_string(), "impact".to_string()];
        merged.locations = vec![FindingLocation {
            file: "src/other.rs".to_string(),
            line_start: 4,
            line_end: 6,
        }];
        let mut timed_out =
            StageRun::timed_out("dependencies", None, 360_000, "no activity for 360s");
        timed_out.coverage = StageCoverage {
            files_read: vec!["a.rs".to_string(), "b.rs".to_string()],
            commands_run: vec![CommandRun {
                cmd: "cargo check".to_string(),
                exit: 101,
            }],
            tools_unavailable: vec![],
            stopped_early: Some("idle".to_string()),
        };

        let markdown = render_review_markdown(&report(
            vec![finding("rf-1", ReviewSeverity::Blocker), merged, hypothesis],
            vec![
                StageRun::ok("mechanical", Some("m".to_string()), 192_000),
                timed_out,
                StageRun::not_run("execution", "depth normal"),
            ],
        ));

        assert!(markdown.starts_with(
            "## Review ⚠️ PARTIAL · 12 file(s) requested → 14 reviewed (strict, +2 dependency edges) · base 1a2b3c..HEAD (7 changed file(s), 31 hunk(s)) · depth normal · 4m12s"
        ));
        assert!(markdown.contains("Stages: mechanical ok 3m12s · dependencies timed out 6m00s (no activity for 360s) · execution not run (depth normal)"));
        assert!(markdown.contains("Findings: 2 supported (2 with a reproduction) · 1 hypotheses · 3 duplicate(s) merged · 0 pre-existing · 1 out of scope"));
        assert!(markdown.contains("### blocker (1)"));
        assert!(markdown.contains("### high (1)"));
        assert!(markdown.contains("repro: cargo test -p thing"));
        assert!(markdown.contains("stage: diff+impact"));
        assert!(markdown.contains("also at: src/other.rs:4-6"));
        assert!(markdown.contains("### Hypotheses (1)"));
        assert!(markdown.contains("evidence not found in file"));
        assert!(markdown.contains("dependencies: 2 file(s) read; 1 command(s) run, 1 non-zero"));
        assert!(markdown.contains("Raw per-stage output: .refact/review_scratch/rv-1"));
        assert!(!markdown.contains("confidence"));
    }

    #[test]
    fn tool_review_render_states_when_nothing_was_supported() {
        let markdown = render_review_markdown(&report(
            vec![],
            vec![StageRun::ok("diff", Some("m".to_string()), 1000)],
        ));

        assert!(markdown.starts_with("## Review · 12 file(s) requested"));
        assert!(markdown.contains("### No supported findings"));
        assert!(!markdown.contains("### Hypotheses"));
        assert!(!markdown.contains("INCONCLUSIVE"));
    }

    #[test]
    fn tool_review_render_marks_an_all_stages_dead_review_inconclusive_not_clean() {
        let markdown = render_review_markdown(&report(
            vec![],
            vec![
                StageRun::timed_out("diff", None, 360_000, "no activity for 120s")
                    .with_trace_chat_id(Some("subchat-rv-1-diff".to_string())),
                StageRun::timed_out("security", None, 360_000, "no activity for 120s")
                    .with_trace_chat_id(Some("subchat-rv-1-security".to_string())),
                StageRun::failed("spec", None, 12, "output_contract"),
            ],
        ));

        assert!(markdown.starts_with("## Review ⚠️ INCONCLUSIVE · 12 file(s) requested"));
        assert!(
            !markdown.contains("### No supported findings"),
            "an inconclusive review must never render the clean-bill header: {markdown}"
        );
        assert!(markdown.contains("### Nothing was checked — this is NOT a pass"));
        assert!(markdown.contains("Do not read the empty findings list as approval"));
        assert!(markdown.contains("Incomplete stage(s) (3):"));
        assert!(markdown.contains("- diff: timed out (no activity for 120s)"));
        assert!(markdown.contains("- spec: failed (output_contract)"));
        assert!(markdown.contains("[transcript subchat-rv-1-diff]"));
    }

    #[test]
    fn tool_review_render_partial_says_how_many_stages_completed() {
        let markdown = render_review_markdown(&report(
            vec![],
            vec![
                StageRun::ok("diff", Some("m".to_string()), 1000),
                StageRun::timed_out("security", None, 360_000, "no activity for 360s"),
            ],
        ));

        assert!(markdown.starts_with("## Review ⚠️ PARTIAL · 12 file(s) requested"));
        assert!(!markdown.contains("### No supported findings\n"));
        assert!(markdown.contains("### No supported findings from the 1 stage(s) that completed"));
        assert!(markdown.contains("1 stage(s) did not complete"));
        assert!(markdown.contains("- security: timed out (no activity for 360s)"));
    }

    #[test]
    fn tool_review_render_lists_files_that_were_never_reviewed() {
        let mut with_dropped = report(
            vec![],
            vec![StageRun::ok("diff", Some("m".to_string()), 1000)],
        );
        with_dropped.scope.dropped_files = vec!["src/a.rs".to_string(), "src/b.rs".to_string()];

        let markdown = render_review_markdown(&with_dropped);

        assert!(markdown.contains("2 file(s) were not reviewed"));
        assert!(!markdown.contains("\n- src/a.rs"));
        assert!(!markdown.contains("\n- src/b.rs"));
    }

    #[test]
    fn tool_review_render_omits_the_scratch_path_when_nothing_was_written() {
        let mut nothing_written = report(
            vec![],
            vec![StageRun::timed_out(
                "diff",
                None,
                1000,
                "no activity for 360s",
            )],
        );
        nothing_written.scratch_dir = None;

        let markdown = render_review_markdown(&nothing_written);

        assert!(!markdown.contains("Raw per-stage output"));
    }

    #[test]
    fn tool_review_adversarial_verdicts_only_dispute_matching_ids() {
        let mut findings = vec![
            finding("rf-1", ReviewSeverity::High),
            finding("rf-2", ReviewSeverity::Low),
        ];
        let verdicts = vec![
            Verdict {
                id: "rf-1".to_string(),
                supported: false,
                reason: "guarded at src/lib.rs:4".to_string(),
            },
            Verdict {
                id: "rf-2".to_string(),
                supported: true,
                reason: String::new(),
            },
            Verdict {
                id: "rf-missing".to_string(),
                supported: false,
                reason: "unknown".to_string(),
            },
        ];

        let disputed = apply_verdicts(&mut findings, "adversarial", &verdicts);

        assert_eq!(disputed, 1);
        assert_eq!(
            findings[0].disputed.as_ref().unwrap().reason,
            "guarded at src/lib.rs:4"
        );
        assert!(findings[0].is_hypothesis());
        assert!(findings[1].disputed.is_none());
        assert!(!findings[1].is_hypothesis());
    }

    #[test]
    fn tool_review_refs_are_relative_and_flag_hypotheses() {
        let mut hypothesis = finding("rf-2", ReviewSeverity::Low);
        hypothesis.reproduction = None;
        hypothesis.evidence_present = false;
        hypothesis.file = crate::test_paths::abs_str("repo/src/other.rs");
        let report = report(
            vec![finding("rf-1", ReviewSeverity::High), hypothesis],
            vec![],
        );

        let (refs, truncated) = review_refs(&report, &[crate::test_paths::abs("repo")]);

        assert!(!truncated);
        let refs = refs.as_array().unwrap();
        assert_eq!(refs[0]["path"], "src/lib.rs");
        assert_eq!(refs[0]["hypothesis"], false);
        assert_eq!(refs[1]["path"], "src/other.rs");
        assert_eq!(refs[1]["hypothesis"], true);
        assert_eq!(refs[1]["severity"], "low");
    }

    #[test]
    fn tool_review_duration_formatting() {
        assert_eq!(format_duration(999), "0s");
        assert_eq!(format_duration(59_000), "59s");
        assert_eq!(format_duration(192_000), "3m12s");
        assert_eq!(format_duration(3_600_000), "60m00s");
    }
}
