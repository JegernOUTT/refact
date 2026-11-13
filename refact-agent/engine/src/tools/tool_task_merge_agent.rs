use std::collections::HashMap;
#[cfg(test)]
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;

use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::chat::verifier::{load_expected_card_state, outcome_is_infrastructure};
use crate::tasks::types::VerificationOutcome;
use crate::global_context::GlobalContext;
use crate::tasks::storage;
use crate::tools::task_tool_helpers::{require_bound_planner_task, stop_live_card_sessions};
use crate::worktrees::service::WorktreeService;
use crate::worktrees::types::{
    MergeWorktreeRequest, MergeWorktreeResponse, WorktreeMergeStrategy, WorktreeRecordView,
    WorktreeReference,
};

pub(crate) async fn merge_under_available_lease<F, Fut>(
    app: &crate::app_state::AppState,
    service: &WorktreeService,
    worktree_id: &str,
    request: MergeWorktreeRequest,
    post_merge_check: Option<crate::chat::post_merge_check::PostMergeCheckRequest>,
    check: F,
) -> Result<
    (
        MergeWorktreeResponse,
        Option<crate::coordination::merge_gate::MergeVerification>,
        bool,
    ),
    String,
>
where
    F: FnOnce(Arc<GlobalContext>, crate::chat::post_merge_check::PostMergeCheckRequest) -> Fut,
    Fut: std::future::Future<
        Output = Result<crate::chat::post_merge_check::PostMergeCheckResult, String>,
    >,
{
    let target = match request.target_branch.clone() {
        Some(target) => target,
        None => service
            .get_worktree(worktree_id)
            .await?
            .meta
            .base_branch
            .ok_or("Merge target branch is required")?,
    };
    let request = MergeWorktreeRequest {
        target_branch: Some(target.clone()),
        ..request
    };
    let service = crate::worktrees::service::fenced_service(
        service.clone(),
        app.runtime.tool_jobs.scopes.clone(),
    )
    .fence_merge_worktree(worktree_id, Some(&target))
    .await?
    .acquire_merge_permit()
    .await?;
    let service = service.for_merge_target(&target).await?;
    let Some(leases) =
        crate::coordination::leases::merge_leases_for(app, service.source_workspace_root()).await?
    else {
        service.checkpoint_merge_phase().await?;
        let delete_requested = request.delete_after_merge;
        let mut response = service
            .merge_worktree(
                worktree_id,
                MergeWorktreeRequest {
                    delete_after_merge: false,
                    ..request
                },
            )
            .await?;
        let verification = if response.merged && !response.source_kept {
            crate::coordination::merge_gate::MERGE_PHASE_SCOPE
                .scope(
                    service.clone(),
                    verify_merged_checkout(
                        app.gcx.clone(),
                        check,
                        post_merge_check,
                        service.merge_checkout_root().to_path_buf(),
                        response.merge_commit.clone(),
                    ),
                )
                .await
        } else {
            None
        };
        let applied = response.merged;
        let late = |response: &mut MergeWorktreeResponse,
                    result: Result<(), String>,
                    context: &str|
         -> Result<(), String> {
            match result {
                Ok(()) => Ok(()),
                Err(error) if applied => {
                    response
                        .warnings
                        .push(format!("The merge was applied, but {context}: {error}"));
                    Ok(())
                }
                Err(error) => Err(error),
            }
        };
        let after_merge = service.checkpoint_merge_phase().await;
        let authority_ok = after_merge.is_ok();
        late(
            &mut response,
            after_merge,
            "merge authority was lost afterwards",
        )?;
        if authority_ok
            && delete_requested
            && !crate::coordination::merge_gate::source_kept(&response)
            && (response.merged || response.status == "nothing_to_merge")
            && verification
                .as_ref()
                .map_or(true, |result| result.passed || result.auto_reverted)
        {
            match service.delete_worktree(worktree_id, true, true).await {
                Ok(cleanup) => {
                    response.cleanup = Some(crate::worktrees::types::WorktreeRemovalResult {
                        worktree_deleted: cleanup.deleted,
                        branch_deleted: cleanup.branch_deleted,
                        registry_deleted: cleanup.deleted,
                        stale_path: cleanup.stale_path,
                        warnings: cleanup.warnings,
                    })
                }
                Err(error) => late(
                    &mut response,
                    Err(error),
                    "the source worktree could not be cleaned up and was kept",
                )?,
            }
        }
        let target_cleanup = service.cleanup_merge_target().await;
        late(
            &mut response,
            target_cleanup,
            "cleaning up the merge target failed",
        )?;
        return Ok((response, verification, false));
    };
    let settings = crate::file_activity::settings();
    let gate = crate::coordination::merge_gate::MergeGate::new(leases, &settings);
    let gcx = app.gcx.clone();
    let outcome = crate::coordination::merge_gate::merge_under_lease(
        &gate,
        &service,
        worktree_id,
        request,
        None,
        move |checkout, merge_commit| {
            verify_merged_checkout(gcx, check, post_merge_check, checkout, merge_commit)
        },
    )
    .await
    .map_err(|error| match error {
        crate::coordination::merge_gate::MergeGateError::Busy(_) => {
            crate::coordination::error::WorkError::new("merge_busy", error.to_string(), true)
                .to_string()
        }
        other => other.to_string(),
    })?;
    let post_merge = outcome.post_merge.clone();
    let mut response = outcome.response;
    if let Some(cleanup) = outcome.cleanup {
        response.cleanup = Some(crate::worktrees::types::WorktreeRemovalResult {
            worktree_deleted: cleanup.deleted,
            branch_deleted: cleanup.branch_deleted,
            registry_deleted: cleanup.deleted,
            stale_path: cleanup.stale_path,
            warnings: cleanup.warnings,
        });
    }
    Ok((response, post_merge, true))
}

async fn verify_merged_checkout<F, Fut>(
    gcx: Arc<GlobalContext>,
    check: F,
    request: Option<crate::chat::post_merge_check::PostMergeCheckRequest>,
    checkout: std::path::PathBuf,
    merge_commit: Option<String>,
) -> Option<crate::coordination::merge_gate::MergeVerification>
where
    F: FnOnce(Arc<GlobalContext>, crate::chat::post_merge_check::PostMergeCheckRequest) -> Fut,
    Fut: std::future::Future<
        Output = Result<crate::chat::post_merge_check::PostMergeCheckResult, String>,
    >,
{
    let Some(mut request) = request else {
        return None;
    };
    let Some(merge_commit) = merge_commit.filter(|commit| !commit.trim().is_empty()) else {
        return Some(post_merge_infrastructure_failure(
            "merge produced no commit".to_string(),
        ));
    };
    request.workspace_root = checkout;
    request.expected_merge_commit = merge_commit.clone();
    Some(post_merge_verification(
        check(gcx, request).await,
        merge_commit,
    ))
}

fn post_merge_infrastructure_failure(
    error: String,
) -> crate::coordination::merge_gate::MergeVerification {
    crate::coordination::merge_gate::MergeVerification {
        stage: crate::coordination::merge_gate::VerificationStage::PostMerge,
        command: "post_merge_check".to_string(),
        passed: false,
        exit_code: None,
        output_tail: error,
        auto_reverted: false,
        merge_commit: None,
        revert_commit: None,
        fix_card_id: None,
        skipped_reason: None,
        revert_skipped_reason: Some("post-merge verification could not run".to_string()),
        outcome: VerificationOutcome::InfrastructureFailed,
    }
}

fn post_merge_verification(
    result: Result<crate::chat::post_merge_check::PostMergeCheckResult, String>,
    merge_commit: String,
) -> crate::coordination::merge_gate::MergeVerification {
    use crate::coordination::merge_gate::{MergeVerification, VerificationStage};

    match result {
        Ok(result) => MergeVerification {
            stage: VerificationStage::PostMerge,
            command: result.command.unwrap_or_default(),
            passed: result.checked && result.outcome == VerificationOutcome::Passed,
            exit_code: result.exit_code,
            output_tail: result.output_tail,
            auto_reverted: result.auto_reverted,
            merge_commit: result.merge_commit.or(Some(merge_commit)),
            revert_commit: result.revert_commit,
            fix_card_id: result.fix_card_id,
            skipped_reason: result.skipped_reason,
            revert_skipped_reason: result.revert_skipped_reason,
            outcome: result.outcome,
        },
        Err(error) => {
            let mut verification = post_merge_infrastructure_failure(error);
            verification.merge_commit = Some(merge_commit);
            verification
        }
    }
}

fn strategy_from_str(strategy: &str) -> Result<WorktreeMergeStrategy, String> {
    match strategy {
        "merge" => Ok(WorktreeMergeStrategy::Merge),
        "squash" => Ok(WorktreeMergeStrategy::Squash),
        _ => Err(format!(
            "Invalid strategy '{}', must be 'merge' or 'squash'",
            strategy
        )),
    }
}

fn strategy_arg(args: &HashMap<String, Value>) -> Result<&str, String> {
    match args.get("strategy") {
        None => Ok("squash"),
        Some(Value::String(strategy)) => Ok(strategy.as_str()),
        Some(_) => Err("'strategy' must be a string".to_string()),
    }
}

fn cleanup_summary(response: &MergeWorktreeResponse) -> String {
    match response.cleanup.as_ref() {
        Some(cleanup) => format!(
            "worktree_deleted={}, branch_deleted={}, registry_deleted={}, affected_references={}",
            cleanup.worktree_deleted,
            cleanup.branch_deleted,
            cleanup.registry_deleted,
            response.affected_reference_count
        ),
        None => format!(
            "no cleanup requested, affected_references={}",
            response.affected_reference_count
        ),
    }
}

fn append_post_merge_check_message(
    message: &mut String,
    result: &crate::chat::post_merge_check::PostMergeCheckResult,
) {
    use crate::tasks::types::VerificationOutcome;

    if !result.checked {
        if let Some(reason) = result.skipped_reason.as_deref() {
            message.push_str(&format!("\n\n**Post-merge check:** skipped ({})", reason));
        }
        return;
    }
    let command = result.command.as_deref().unwrap_or("unknown");
    if result.outcome == VerificationOutcome::Passed {
        message.push_str(&format!("\n\n**Post-merge check:** `{}` passed", command));
        return;
    }
    if result.outcome != VerificationOutcome::CommandFailed {
        let label = match result.outcome {
            VerificationOutcome::Rejected => "could not be run as a command",
            VerificationOutcome::PolicyDenied => "denied by command policy",
            VerificationOutcome::InfrastructureFailed => {
                "could not run due to infrastructure failure"
            }
            VerificationOutcome::NoCommands => "found no verification commands",
            VerificationOutcome::Unknown => "returned an unknown result",
            VerificationOutcome::Passed | VerificationOutcome::CommandFailed => unreachable!(),
        };
        message.push_str(&format!(
            "\n\n**Post-merge check:** `{}` {}\n\n{}",
            command,
            label,
            crate::chat::post_merge_check::first_error_line(&result.output_tail)
        ));
        return;
    }
    message.push_str(&format!(
        "\n\n## Post-merge regression detected\n\n**Verification:** `{}` failed{}\n**Auto-reverted:** {}\n**Merge commit:** {}",
        command,
        result.exit_code.map(|code| format!(" with exit code {}", code)).unwrap_or_default(),
        if result.auto_reverted {
            "true".to_string()
        } else {
            format!("false ({})", result.revert_skipped_reason.as_deref().unwrap_or("not attempted"))
        },
        result.merge_commit.as_deref().unwrap_or("unknown"),
    ));
    if let Some(revert_commit) = result.revert_commit.as_deref() {
        message.push_str(&format!("\n**Revert commit:** {}", revert_commit));
    }
    if let Some(fix_card_id) = result.fix_card_id.as_deref() {
        message.push_str(&format!("\n**Fix card:** {}", fix_card_id));
    }
    message.push_str(&format!(
        "\n\n{}",
        crate::chat::post_merge_check::first_error_line(&result.output_tail)
    ));
}

fn append_merge_warnings(message: &mut String, warnings: &[String]) {
    if warnings.is_empty() {
        return;
    }
    message.push_str("\n\n## Warnings");
    for warning in warnings {
        message.push_str(&format!("\n- {}", warning));
    }
}

fn response_warnings(response: &MergeWorktreeResponse) -> Vec<String> {
    let mut warnings = response.warnings.clone();
    if let Some(cleanup) = response.cleanup.as_ref() {
        warnings.extend(cleanup.warnings.clone());
    }
    warnings
}

fn dirty_baseline_summary(baseline: &crate::worktrees::types::DirtyBaseline) -> String {
    let mut files = baseline
        .files
        .iter()
        .map(|file| format!("- {}", file))
        .collect::<Vec<_>>();
    if baseline.files_truncated {
        files.push("- ...".to_string());
    }
    format!(
        "\n\n## Uncommitted Target Files\n{}\n\n(staged: {}, unstaged: {}, untracked: {})",
        files.join("\n"),
        baseline.staged_count,
        baseline.unstaged_count,
        baseline.untracked_count
    )
}

fn not_merged_next_step(status: &str) -> &'static str {
    match status {
        "dirty_baseline" => "commit or stash the uncommitted target changes listed above, then call merge_agent again.",
        _ => "inspect the warnings above and the target checkout, resolve the reported problem, then call merge_agent again.",
    }
}

fn merge_response_message(
    card_id: &str,
    response: &MergeWorktreeResponse,
    post_merge_check: Option<&crate::chat::post_merge_check::PostMergeCheckResult>,
) -> String {
    if let Some(conflict) = response.conflict.as_ref() {
        let mut message = format!(
            "# Merge Conflicts Detected\n\n**Card:** {}\n**Branch:** {} → {}\n**Strategy:** {}\n**Aborted:** {}\n\n## Conflicting Files\n{}\n\n{}",
            card_id,
            response.source_branch,
            response.target_branch,
            response.strategy,
            conflict.aborted,
            if conflict.files.is_empty() {
                "None detected".to_string()
            } else {
                conflict
                    .files
                    .iter()
                    .map(|file| format!("- {}", file))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            conflict.instructions
        );
        append_merge_warnings(&mut message, &response_warnings(response));
        return message;
    }
    if response.status == "nothing_to_merge" {
        let mut message = format!(
            "# Nothing to Merge\n\n**Card:** {}\n**Branch:** {} → {}\n**Commits ahead of base:** 0\n\nCleanup: {}.",
            card_id,
            response.source_branch,
            response.target_branch,
            cleanup_summary(response)
        );
        append_merge_warnings(&mut message, &response_warnings(response));
        return message;
    }
    if !response.merged {
        let mut message = format!(
            "# Merge Not Performed\n\n**Card:** {}\n**Status:** {}\n**Branch:** {} → {}\n**Strategy:** {}\n\nNothing was merged; the target branch is unchanged and the agent worktree and branch were kept.{}\n\n**Next step:** {}",
            card_id,
            response.status,
            response.source_branch,
            response.target_branch,
            response.strategy,
            response
                .dirty_baseline
                .as_ref()
                .map(dirty_baseline_summary)
                .unwrap_or_default(),
            not_merged_next_step(&response.status)
        );
        append_merge_warnings(&mut message, &response_warnings(response));
        return message;
    }
    let mut message = format!(
        "# Agent Work Merged\n\n**Card:** {}\n**Strategy:** {}\n**Branch:** {} → {}\n**Cleanup:** {}\n\nThe agent's work has been successfully merged back to the target branch.",
        card_id,
        response.strategy,
        response.source_branch,
        response.target_branch,
        cleanup_summary(response)
    );
    if let Some(result) = post_merge_check {
        append_post_merge_check_message(&mut message, result);
    }
    append_merge_warnings(&mut message, &response_warnings(response));
    message
}

fn parse_force(args: &HashMap<String, Value>) -> Result<bool, String> {
    parse_schema_bool_arg(args, "force", false)
}

fn parse_auto_revert(args: &HashMap<String, Value>) -> Result<bool, String> {
    parse_schema_bool_arg(args, "auto_revert", false)
}

fn parse_schema_bool_arg(
    args: &HashMap<String, Value>,
    name: &str,
    default: bool,
) -> Result<bool, String> {
    match args.get(name) {
        None => Ok(default),
        Some(value) => refact_tool_api::coerce_bool(value)
            .ok_or_else(|| format!("'{}' must be a boolean", name)),
    }
}

fn parse_auto_revert_timeout(args: &HashMap<String, Value>) -> Result<Duration, String> {
    let Some(value) = args.get("auto_revert_timeout_secs") else {
        return Ok(Duration::from_secs(
            crate::chat::post_merge_check::DEFAULT_POST_MERGE_CHECK_TIMEOUT_SECS,
        ));
    };
    let secs = match value {
        Value::Number(number) => number
            .as_u64()
            .ok_or_else(|| "auto_revert_timeout_secs must be a positive integer".to_string())?,
        _ => return Err("auto_revert_timeout_secs must be a positive integer".to_string()),
    };
    if secs == 0 {
        return Err("auto_revert_timeout_secs must be greater than zero".to_string());
    }
    Ok(Duration::from_secs(secs))
}

fn required_string_arg<'a>(
    args: &'a HashMap<String, Value>,
    name: &str,
) -> Result<&'a str, String> {
    match args.get(name) {
        Some(Value::String(value)) if value.trim().is_empty() => Err(format!("Missing '{}'", name)),
        Some(Value::String(value)) if value.trim() == value.as_str() => Ok(value.as_str()),
        Some(Value::String(_)) => Err(format!(
            "'{}' must not have leading or trailing whitespace",
            name
        )),
        Some(Value::Null) | None => Err(format!("Missing '{}'", name)),
        Some(_) => Err(format!("'{}' must be a string", name)),
    }
}

fn ensure_legacy_agent_worktree_checkout(
    workspace_root: &std::path::Path,
    agent_worktree: &str,
    agent_branch: &str,
) -> Result<(), String> {
    let workspace_root = workspace_root
        .canonicalize()
        .map_err(|e| format!("Failed to canonicalize workspace root: {}", e))?;
    let agent_worktree_path = std::path::Path::new(agent_worktree);
    let agent_worktree_root = agent_worktree_path
        .canonicalize()
        .map_err(|e| format!("Failed to canonicalize agent worktree: {}", e))?;
    if agent_worktree_root == workspace_root {
        return Err("Agent worktree path points at the main workspace; refusing to auto-commit main workspace changes".to_string());
    }
    let status = crate::worktrees::git::status_for_path(&agent_worktree_root);
    if !status.is_git_worktree {
        return Err(format!(
            "Agent worktree path '{}' is not a git worktree",
            agent_worktree
        ));
    }
    let agent_top_level =
        crate::worktrees::git::run_git(&agent_worktree_root, &["rev-parse", "--show-toplevel"])?;
    let agent_top_level = std::path::Path::new(agent_top_level.trim())
        .canonicalize()
        .map_err(|e| format!("Failed to canonicalize agent worktree root: {}", e))?;
    if agent_top_level != agent_worktree_root {
        return Err(format!(
            "Agent worktree path '{}' is not the git worktree root; refusing to auto-commit it",
            agent_worktree
        ));
    }
    let workspace_common_dir =
        crate::worktrees::git::run_git(&workspace_root, &["rev-parse", "--git-common-dir"])?;
    let agent_common_dir =
        crate::worktrees::git::run_git(&agent_worktree_root, &["rev-parse", "--git-common-dir"])?;
    let canonical_git_dir =
        |root: &std::path::Path, common_dir: &str| -> Result<std::path::PathBuf, String> {
            let path = std::path::Path::new(common_dir.trim());
            let path = if path.is_absolute() {
                path.to_path_buf()
            } else {
                root.join(path)
            };
            path.canonicalize()
                .map_err(|e| format!("Failed to canonicalize git common dir: {}", e))
        };
    if canonical_git_dir(&workspace_root, &workspace_common_dir)?
        != canonical_git_dir(&agent_worktree_root, &agent_common_dir)?
    {
        return Err(
            "Agent worktree belongs to a different repository; refusing to auto-commit it"
                .to_string(),
        );
    }
    crate::worktrees::git::ensure_checkout_on_branch(
        &agent_worktree_root,
        "Agent worktree",
        agent_branch,
    )
    .map_err(|error| {
        format!(
            "Agent checkout '{}' expected branch '{}': {}",
            agent_worktree_root.display(),
            agent_branch,
            error
        )
    })
}

fn reference_matches_card(
    task_id: &str,
    card: &crate::tasks::types::BoardCard,
    reference: &WorktreeReference,
) -> bool {
    if reference.task_id.as_deref() == Some(task_id)
        && reference.card_id.as_deref() == Some(card.id.as_str())
    {
        return true;
    }
    if let Some(agent_chat_id) = card.agent_chat_id.as_deref() {
        if reference.chat_id.as_deref() == Some(agent_chat_id) {
            return true;
        }
    }
    card.assignee
        .as_deref()
        .map(|assignee| {
            reference.task_id.as_deref() == Some(task_id)
                && reference.agent_id.as_deref() == Some(assignee)
        })
        .unwrap_or(false)
}

fn registered_record_matches_card(
    view: &WorktreeRecordView,
    task_id: &str,
    card: &crate::tasks::types::BoardCard,
) -> bool {
    if view.meta.task_id.as_deref() == Some(task_id)
        && view.meta.card_id.as_deref() == Some(card.id.as_str())
    {
        return true;
    }
    view.references
        .iter()
        .any(|reference| reference_matches_card(task_id, card, reference))
}

async fn worktree_id_is_valid_for_card(
    service: &WorktreeService,
    worktree_id: &str,
    task_id: &str,
    card: &crate::tasks::types::BoardCard,
) -> Result<WorktreeRecordView, String> {
    let view = service.get_worktree(worktree_id).await?;
    if !registered_record_matches_card(&view, task_id, card) {
        return Err(format!(
            "Registered worktree '{}' does not match task {} card {}",
            worktree_id, task_id, card.id
        ));
    }
    service
        .validate_registered_worktree_checkout(worktree_id)
        .await?;
    Ok(view)
}

fn render_concern_list(concerns: &[String]) -> String {
    if concerns.is_empty() {
        "- verifier did not pass and recorded no concerns".to_string()
    } else {
        concerns
            .iter()
            .map(|concern| format!("- {}", concern))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn verifier_failed_warning(card_id: &str, concerns: &[String]) -> String {
    format!(
        "**Verification FAILED** for card {}: the verifier recorded genuine verification failures, but the work was merged anyway because merges are never blocked by verification. Review and fix these failures (for example with a follow-up card). Concerns:\n{}",
        card_id,
        render_concern_list(concerns)
    )
}

fn verifier_unverified_warning(card_id: &str, reason: &str, concerns: &[String]) -> String {
    format!(
        "Card {} was not verified ({}); the work was merged anyway because merges are never blocked by verification. Review the merged code. Concerns:\n{}",
        card_id,
        reason,
        render_concern_list(concerns)
    )
}

fn append_verifier_warning(message: &mut String, warning: Option<&str>) {
    if let Some(warning) = warning {
        message.push_str("\n\n## Verifier Warning\n\n");
        message.push_str(warning);
    }
}

fn concern_is_infrastructure_only(concern: &str, infrastructure_commands: &[&str]) -> bool {
    concern.starts_with("Verifier failed to launch; human review recommended:")
        || concern.starts_with("Verifier review subchat unavailable; human review recommended:")
        || infrastructure_commands
            .iter()
            .any(|command| concern.strip_prefix("Verification command failed: ") == Some(*command))
}

pub(crate) fn ensure_verifier_allows_merge(
    card: &crate::tasks::types::BoardCard,
    _force: bool,
) -> Result<Option<String>, String> {
    use crate::tasks::types::{VerificationOutcome, VerifierReportClassification};

    let Some(report) = card.verifier_report.as_ref() else {
        return Ok(Some(verifier_unverified_warning(
            &card.id,
            "no verifier report was recorded",
            &["Verification evidence is unavailable".to_string()],
        )));
    };
    let deterministic_failure = report.classification
        == VerifierReportClassification::VerificationFailed
        || report
            .command_results
            .iter()
            .any(|result| result.outcome == VerificationOutcome::CommandFailed);
    if deterministic_failure {
        return Ok(Some(verifier_failed_warning(&card.id, &report.concerns)));
    }
    let consistent_pass = report.passed
        && report.classification == VerifierReportClassification::Passed
        && !report.command_results.is_empty()
        && report
            .command_results
            .iter()
            .all(|result| result.passed && result.outcome == VerificationOutcome::Passed);
    if consistent_pass {
        return Ok(None);
    }
    if matches!(
        report.classification,
        VerifierReportClassification::HumanReview | VerifierReportClassification::Unavailable
    ) {
        return Ok(Some(verifier_unverified_warning(
            &card.id,
            "the verifier requested human review or verification evidence was unavailable",
            &report.concerns,
        )));
    }
    let infrastructure_commands = report
        .command_results
        .iter()
        .filter(|result| outcome_is_infrastructure(&result.outcome))
        .map(|result| result.command.as_str())
        .collect::<Vec<_>>();
    let genuine_failure = report.concerns.is_empty()
        || report
            .concerns
            .iter()
            .any(|concern| !concern_is_infrastructure_only(concern, &infrastructure_commands));
    if genuine_failure {
        return Ok(Some(verifier_failed_warning(&card.id, &report.concerns)));
    }
    Ok(Some(verifier_unverified_warning(
        &card.id,
        "verification could not run because of infrastructure problems",
        &report.concerns,
    )))
}

fn ensure_card_done_allows_merge(card: &crate::tasks::types::BoardCard) -> Option<String> {
    let report_present = card
        .final_report
        .as_deref()
        .is_some_and(|report| !report.trim().is_empty());
    if card.column == "done" && report_present {
        return None;
    }
    let reason = if card.column != "done" {
        format!("it was in column '{}', not 'done'", card.column)
    } else {
        "it has no final report".to_string()
    };
    Some(format!(
        "⚠️ Merged card {} although {}; the normal completion checks were skipped because merges are never blocked. Verify the merged code carefully.",
        card.id, reason
    ))
}

fn append_completion_warning(message: &mut String, warning: Option<&str>) {
    if let Some(warning) = warning {
        message.push_str("\n\n");
        message.push_str(warning);
    }
}

fn terminal_card_column(column: &str) -> bool {
    matches!(column, "done" | "failed" | "regressed")
}

fn force_merge_card_matches_current(
    card: &crate::tasks::types::BoardCard,
    expected_assignee: Option<&str>,
    expected_agent_chat_id: Option<&str>,
) -> bool {
    card.assignee.as_deref() == expected_assignee
        && card.agent_chat_id.as_deref() == expected_agent_chat_id
}

fn force_merge_post_check_passed_or_absent(
    result: &Result<Option<crate::chat::post_merge_check::PostMergeCheckResult>, String>,
) -> bool {
    use crate::tasks::types::VerificationOutcome;

    match result {
        Ok(None) => true,
        Ok(Some(check)) => check.checked && check.outcome == VerificationOutcome::Passed,
        Err(_) => false,
    }
}

fn retained_post_merge_verification_failure(
    result: &Result<Option<crate::chat::post_merge_check::PostMergeCheckResult>, String>,
) -> Option<String> {
    use crate::tasks::types::VerificationOutcome;

    let check = result.as_ref().ok()?.as_ref()?;
    if check.auto_reverted || (check.checked && check.outcome == VerificationOutcome::Passed) {
        return None;
    }
    let outcome = match check.outcome {
        VerificationOutcome::CommandFailed => "verification command failed",
        VerificationOutcome::Rejected => "verification command was rejected",
        VerificationOutcome::PolicyDenied => "verification was denied by policy",
        VerificationOutcome::InfrastructureFailed => "verification infrastructure failed",
        VerificationOutcome::NoCommands => "no verification commands were available",
        VerificationOutcome::Unknown => "verification returned an unknown outcome",
        VerificationOutcome::Passed => "verification was not explicitly checked",
    };
    Some(format!(
        "Unfinished or unverified merge remains applied, but post-merge verification did not pass: {}. Stale agent ownership was cleared; the merged work is retained and the card was not marked done.",
        outcome
    ))
}

fn retain_force_merge_without_finalizing(
    card: &mut crate::tasks::types::BoardCard,
    message: &str,
) -> bool {
    card.assignee = None;
    card.agent_chat_id = None;
    card.status_updates.push(crate::tasks::types::StatusUpdate {
        timestamp: chrono::Utc::now().to_rfc3339(),
        message: message.to_string(),
    });
    true
}

pub(crate) const MERGE_AGENT_STATUS_PREFIX: &str = "Merged by planner";

#[derive(Clone, Debug)]
pub(crate) struct AppliedMerge {
    pub(crate) since: chrono::DateTime<chrono::Utc>,
    pub(crate) commit: Option<String>,
}

impl AppliedMerge {
    fn commit_tag(&self) -> Option<String> {
        self.commit
            .as_deref()
            .map(str::trim)
            .filter(|commit| !commit.is_empty())
            .map(|commit| format!(" at {commit}."))
    }
}

fn has_merge_marker(card: &crate::tasks::types::BoardCard, merge: &AppliedMerge) -> bool {
    let tag = merge.commit_tag();
    card.status_updates.iter().any(|update| {
        update.message.starts_with(MERGE_AGENT_STATUS_PREFIX)
            && match &tag {
                Some(tag) => update.message.ends_with(tag.as_str()),
                None => chrono::DateTime::parse_from_rfc3339(&update.timestamp)
                    .is_ok_and(|time| time >= merge.since),
            }
    })
}

fn push_merge_marker_once(
    card: &mut crate::tasks::types::BoardCard,
    merge: &AppliedMerge,
    strategy: &str,
    target: &str,
) -> bool {
    if has_merge_marker(card, merge) {
        return false;
    }
    let tag = merge.commit_tag().unwrap_or_else(|| ".".to_string());
    card.status_updates.push(crate::tasks::types::StatusUpdate {
        timestamp: chrono::Utc::now().to_rfc3339(),
        message: format!(
            "{MERGE_AGENT_STATUS_PREFIX} via merge_agent ({strategy}) into {target}{tag}"
        ),
    });
    true
}

async fn record_merge_marker(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    card_id: &str,
    merge: AppliedMerge,
    strategy: &str,
    target: &str,
) -> Result<(), String> {
    let card_id = card_id.to_string();
    let strategy = strategy.to_string();
    let target = target.to_string();
    storage::update_board_atomic(gcx, task_id, move |board| {
        Ok(board
            .get_card_mut(&card_id)
            .is_some_and(|card| push_merge_marker_once(card, &merge, &strategy, &target)))
    })
    .await
    .map(|_| ())
}

fn finalize_force_merge_card_if_current(
    card: &mut crate::tasks::types::BoardCard,
    expected_assignee: Option<&str>,
    expected_agent_chat_id: Option<&str>,
    merged: Option<&AppliedMerge>,
) -> bool {
    if !force_merge_card_matches_current(card, expected_assignee, expected_agent_chat_id) {
        return false;
    }
    let mut changed = card.assignee.is_some() || card.agent_chat_id.is_some();
    if !terminal_card_column(&card.column) {
        let now = chrono::Utc::now().to_rfc3339();
        let previous_column = card.column.clone();
        card.column = "done".to_string();
        card.completed_at = Some(now.clone());
        let message = if merged.is_some_and(|merge| has_merge_marker(card, merge)) {
            format!(
                "Marked done by planner for manual review from column '{}' after merge_agent merged it without the normal agent_finish path.",
                previous_column
            )
        } else if merged.is_some() {
            format!(
                "{MERGE_AGENT_STATUS_PREFIX} without the normal agent_finish path; marked done for manual review from column '{}'.",
                previous_column
            )
        } else {
            format!(
                "Finalized by planner via merge_agent with nothing to merge, without the normal agent_finish path; marked done for manual review from column '{}'.",
                previous_column
            )
        };
        card.status_updates.push(crate::tasks::types::StatusUpdate {
            timestamp: now,
            message,
        });
        changed = true;
    }
    card.assignee = None;
    card.agent_chat_id = None;
    changed
}

async fn finalize_force_merge_card_after_merge(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    card_id: &str,
    expected_assignee: Option<String>,
    expected_agent_chat_id: Option<String>,
    merged: Option<AppliedMerge>,
) -> Result<(), String> {
    let card_id_owned = card_id.to_string();
    let (_, changed) = storage::update_board_atomic(gcx.clone(), task_id, move |board| {
        let Some(card) = board.get_card_mut(&card_id_owned) else {
            return Ok(false);
        };
        Ok(finalize_force_merge_card_if_current(
            card,
            expected_assignee.as_deref(),
            expected_agent_chat_id.as_deref(),
            merged.as_ref(),
        ))
    })
    .await?;
    if changed {
        storage::update_task_stats(gcx, task_id).await?;
    }
    Ok(())
}

async fn clear_board_mirrors_after_registered_merge(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    card_id: &str,
    response: &MergeWorktreeResponse,
    finalize_card: bool,
    finalize_force_merge: bool,
    force_merge_auto_reverted: bool,
    retained_verification_failure: Option<String>,
    post_merge_check_error: Option<String>,
    expected_assignee: Option<String>,
    expected_agent_chat_id: Option<String>,
    merge_started_at: chrono::DateTime<chrono::Utc>,
) -> Result<(), String> {
    let merged = response.merged.then(|| AppliedMerge {
        since: merge_started_at,
        commit: response.merge_commit.clone(),
    });
    let clear_ownership = finalize_card
        && (response.merged || response.status == "nothing_to_merge")
        && (finalize_force_merge
            || force_merge_auto_reverted
            || retained_verification_failure.is_some()
            || post_merge_check_error.is_some());
    let Some(cleanup) = response.cleanup.as_ref() else {
        if clear_ownership {
            if force_merge_auto_reverted
                || retained_verification_failure.is_some()
                || post_merge_check_error.is_some()
            {
                let card_id_owned = card_id.to_string();
                let retained_verification_failure = retained_verification_failure.clone();
                let post_merge_check_error = post_merge_check_error.clone();
                let (_, changed) =
                    storage::update_board_atomic(gcx.clone(), task_id, move |board| {
                        let Some(card) = board.get_card_mut(&card_id_owned) else {
                            return Ok(false);
                        };
                        if !force_merge_card_matches_current(
                            card,
                            expected_assignee.as_deref(),
                            expected_agent_chat_id.as_deref(),
                        ) {
                            return Ok(false);
                        }
                        let message = retained_verification_failure
                            .clone()
                            .or_else(|| post_merge_check_error.as_deref().map(|error| format!("Unfinished or unverified merge remains applied, but post-merge verification failed to run: {}. Stale agent ownership was cleared; merge success was not inferred from verification.", error)))
                            .unwrap_or_else(|| "Unfinished or unverified merge was auto-reverted after post-merge verification failed; the original card was not marked done and stale agent ownership was cleared.".to_string());
                        Ok(retain_force_merge_without_finalizing(card, &message))
                    })
                    .await?;
                if changed {
                    storage::update_task_stats(gcx, task_id).await?;
                }
            } else {
                finalize_force_merge_card_after_merge(
                    gcx,
                    task_id,
                    card_id,
                    expected_assignee,
                    expected_agent_chat_id,
                    merged,
                )
                .await?;
            }
        }
        return Ok(());
    };
    if !cleanup.worktree_deleted
        && !cleanup.branch_deleted
        && !cleanup.registry_deleted
        && !clear_ownership
    {
        return Ok(());
    }
    let card_id_owned = card_id.to_string();
    let clear_worktree = cleanup.worktree_deleted || cleanup.registry_deleted;
    let clear_branch = cleanup.branch_deleted;
    let retained_verification_failure_for_update = retained_verification_failure.clone();
    let post_merge_check_error_for_update = post_merge_check_error.clone();
    let (_, stats_changed) = storage::update_board_atomic(gcx.clone(), task_id, move |board| {
        if let Some(card) = board.get_card_mut(&card_id_owned) {
            if !force_merge_card_matches_current(
                card,
                expected_assignee.as_deref(),
                expected_agent_chat_id.as_deref(),
            ) {
                return Ok(false);
            }
            if clear_branch {
                card.agent_branch = None;
            }
            if clear_worktree {
                card.agent_worktree = None;
                card.agent_worktree_name = None;
            }
            if clear_ownership {
                if force_merge_auto_reverted
                    || retained_verification_failure_for_update.is_some()
                    || post_merge_check_error_for_update.is_some()
                {
                    let message = retained_verification_failure_for_update
                        .clone()
                        .or_else(|| post_merge_check_error_for_update.as_deref().map(|error| format!("Unfinished or unverified merge remains applied, but post-merge verification failed to run: {}. Stale worktree, branch, and agent ownership references were cleared; merge success was not inferred from verification.", error)))
                        .unwrap_or_else(|| "Unfinished or unverified merge was auto-reverted after post-merge verification failed; the original card was not marked done and stale worktree, branch, and agent ownership references were cleared.".to_string());
                    return Ok(retain_force_merge_without_finalizing(card, &message));
                }
                return Ok(finalize_force_merge_card_if_current(
                    card,
                    expected_assignee.as_deref(),
                    expected_agent_chat_id.as_deref(),
                    merged.as_ref(),
                ));
            }
        }
        Ok(false)
    })
    .await?;
    if stats_changed {
        storage::update_task_stats(gcx, task_id).await?;
    }
    Ok(())
}

#[cfg(test)]
tokio::task_local! {
    pub(crate) static FAIL_BOARD_AFTER_MERGE: bool;
}

fn registered_board_fault() -> Result<(), String> {
    #[cfg(test)]
    if FAIL_BOARD_AFTER_MERGE
        .try_with(|fail| *fail)
        .unwrap_or(false)
    {
        return Err("injected board update failure after merge".into());
    }
    Ok(())
}

async fn merge_registered_task_worktree(
    gcx: Arc<GlobalContext>,
    workspace_root: &std::path::Path,
    task_id: &str,
    card_id: &str,
    strategy: &str,
    tool_call_id: &str,
    commit_message_override: Option<String>,
    auto_revert: bool,
    auto_revert_timeout: Duration,
) -> Result<Option<(bool, Vec<ContextEnum>)>, String> {
    let board = storage::load_board(gcx.clone(), task_id).await?;
    let Some(card) = board.get_card(card_id) else {
        return Err(format!("Card {} not found", card_id));
    };
    let verifier_warning = ensure_verifier_allows_merge(card, false)?;
    let completion_warning = ensure_card_done_allows_merge(card);
    let finalize_card = verifier_warning.is_some() || completion_warning.is_some();
    let expected_assignee = card.assignee.clone();
    let expected_agent_chat_id = card.agent_chat_id.clone();
    let Some(worktree_id) = card.agent_worktree_name.clone() else {
        return Ok(None);
    };
    let task_meta = storage::load_task_meta(gcx.clone(), task_id).await?;
    let cache_dir = gcx.cache_dir.clone();
    let service = WorktreeService::new_async(cache_dir, workspace_root.to_path_buf()).await?;
    let registered = worktree_id_is_valid_for_card(&service, &worktree_id, task_id, card).await?;
    let card_pair_present = card.base_branch.is_some() || card.base_commit.is_some();
    let registered_pair_present =
        registered.meta.base_branch.is_some() || registered.meta.base_commit.is_some();
    let (base_branch, base_commit) = if card_pair_present {
        (card.base_branch.clone(), card.base_commit.clone())
    } else if registered_pair_present {
        (
            registered.meta.base_branch.clone(),
            registered.meta.base_commit.clone(),
        )
    } else {
        (task_meta.base_branch, task_meta.base_commit)
    };
    let target_branch = base_branch.clone().ok_or("Task has no base branch set")?;
    let diff = crate::worktrees::git::diff_for_path(
        &registered.meta.root,
        base_commit.as_deref(),
        base_branch.as_deref(),
        1024 * 1024,
    )
    .ok();
    let changed_files = diff
        .as_ref()
        .map(|diff| {
            diff.files
                .iter()
                .map(|file| file.path.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let commit_message = match commit_message_override {
        Some(message) if !message.trim().is_empty() => Some(message),
        _ if changed_files.is_empty() => None,
        _ => {
            let diff_text = diff
                .as_ref()
                .map(|diff| diff.patch.clone())
                .unwrap_or_default();
            match crate::agentic::generate_commit_message::generate_commit_message_by_diff(
                gcx.clone(),
                &diff_text,
                &Some(card.title.clone()),
                None,
            )
            .await
            {
                Ok(message) if !message.trim().is_empty() => Some(message),
                _ => Some(format!("Card {}: {}", card_id, card.title)),
            }
        }
    };
    let post_merge_expected_state = if auto_revert {
        Some(load_expected_card_state(gcx.clone(), task_id, card_id).await?)
    } else {
        None
    };
    let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
    let auto_revert = auto_revert && post_merge_expected_state.is_some();
    let merge_started_at = chrono::Utc::now();
    let lease_check_request = if auto_revert {
        post_merge_expected_state
            .clone()
            .map(
                |expected_card_state| crate::chat::post_merge_check::PostMergeCheckRequest {
                    task_id: task_id.to_string(),
                    card_id: card_id.to_string(),
                    workspace_root: workspace_root.to_path_buf(),
                    enabled: true,
                    timeout: auto_revert_timeout,
                    expected_merge_commit: String::new(),
                    expected_card_state: Some(expected_card_state),
                },
            )
    } else {
        None
    };
    let (response, post_merge_verification, lease_used) = merge_under_available_lease(
        &app,
        &service,
        &worktree_id,
        MergeWorktreeRequest {
            strategy: strategy_from_str(strategy)?,
            delete_after_merge: true,
            include_uncommitted: true,
            target_branch: Some(target_branch),
            commit_message,
            generate_commit_message: false,
            acknowledge_dirty_baseline: true,
        },
        lease_check_request,
        |gcx, check_request| async move {
            crate::chat::post_merge_check::post_merge_check(gcx, check_request).await
        },
    )
    .await?;
    let mut response = response;
    let reverted_under_lease = post_merge_verification
        .as_ref()
        .is_some_and(|verification| verification.auto_reverted);
    if response.merged && !reverted_under_lease {
        if let Err(error) = record_merge_marker(
            gcx.clone(),
            task_id,
            card_id,
            AppliedMerge {
                since: merge_started_at,
                commit: response.merge_commit.clone(),
            },
            strategy,
            &response.target_branch,
        )
        .await
        {
            response.warnings.push(format!(
                "The merge was applied, but recording the merge on the card failed: {}",
                error
            ));
        }
    }
    let post_merge_check_result = match post_merge_verification {
        Some(verification) => Ok(Some(crate::chat::post_merge_check::PostMergeCheckResult {
            checked: verification.outcome != VerificationOutcome::Unknown,
            auto_reverted: verification.auto_reverted,
            command: Some(verification.command),
            exit_code: verification.exit_code,
            output_tail: verification.output_tail,
            merge_commit: verification.merge_commit,
            revert_commit: verification.revert_commit,
            fix_card_id: verification.fix_card_id,
            skipped_reason: verification.skipped_reason,
            revert_skipped_reason: verification.revert_skipped_reason,
            outcome: verification.outcome,
        })),
        None if response.source_kept && auto_revert => Err(
            "Post-merge verification was skipped: the merge was kept after its rollback failed; source retained"
                .to_string(),
        ),
        None if lease_used => Ok(None),
        None => match (response.merge_commit.as_ref(), post_merge_expected_state) {
            (Some(merge_commit), Some(expected_card_state)) if response.merged && auto_revert => {
                crate::chat::post_merge_check::post_merge_check(
                    gcx.clone(),
                    crate::chat::post_merge_check::PostMergeCheckRequest {
                        task_id: task_id.to_string(),
                        card_id: card_id.to_string(),
                        workspace_root: workspace_root.to_path_buf(),
                        enabled: true,
                        timeout: auto_revert_timeout,
                        expected_merge_commit: merge_commit.clone(),
                        expected_card_state: Some(expected_card_state),
                    },
                )
                .await
                .map(Some)
            }
            _ => Ok(None),
        },
    };
    let force_merge_auto_reverted = post_merge_check_result
        .as_ref()
        .ok()
        .and_then(|result| result.as_ref())
        .map(|result| result.auto_reverted)
        .unwrap_or(false);
    let finalize_force_merge = force_merge_post_check_passed_or_absent(&post_merge_check_result);
    let retained_verification_failure =
        retained_post_merge_verification_failure(&post_merge_check_result);
    let post_merge_check_error = post_merge_check_result.as_ref().err().cloned();
    let board_result = registered_board_fault().and(
        clear_board_mirrors_after_registered_merge(
            gcx.clone(),
            task_id,
            card_id,
            &response,
            finalize_card,
            finalize_force_merge,
            force_merge_auto_reverted,
            retained_verification_failure,
            post_merge_check_error,
            expected_assignee,
            expected_agent_chat_id,
            merge_started_at,
        )
        .await,
    );
    if let Err(error) = board_result {
        if !response.merged {
            return Err(error);
        }
        response.warnings.push(format!(
            "The merge was applied, but updating the card after the merge failed: {}",
            error
        ));
    }
    let post_merge_check = match post_merge_check_result {
        Ok(check) => check,
        Err(error) if response.merged => {
            response.warnings.push(format!(
                "The merge was applied, but post-merge verification could not run: {}",
                error
            ));
            None
        }
        Err(error) => return Err(error),
    };
    if response.merged && !force_merge_auto_reverted && !changed_files.is_empty() {
        let _ = crate::chat::task_agent_monitor::append_card_target_files(
            crate::app_state::AppState::from_gcx(gcx.clone()).await,
            task_id,
            card_id,
            changed_files,
        )
        .await;
    }
    let mut message = merge_response_message(card_id, &response, post_merge_check.as_ref());
    if response.merged {
        append_verifier_warning(&mut message, verifier_warning.as_deref());
        append_completion_warning(&mut message, completion_warning.as_deref());
    }
    Ok(Some((
        false,
        vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(message),
            tool_calls: None,
            tool_call_id: tool_call_id.to_string(),
            ..Default::default()
        })],
    )))
}

struct MergeRequest {
    gcx: Arc<GlobalContext>,
    chat_facade: Arc<dyn refact_runtime_api::ChatSessionFacade>,
    task_id: String,
    card_id: String,
    strategy: String,
    auto_revert: bool,
    auto_revert_timeout: Duration,
    tool_call_id: String,
}

struct MergeState {
    request: MergeRequest,
    workspace_root: std::path::PathBuf,
    card: crate::tasks::types::BoardCard,
    base_branch: String,
    comparison_base: String,
    verifier_warning: Option<String>,
    completion_warning: Option<String>,
    expected_assignee: Option<String>,
    expected_agent_chat_id: Option<String>,
    merge_service: Option<WorktreeService>,
}

impl MergeState {
    fn finalizes_card(&self) -> bool {
        self.verifier_warning.is_some() || self.completion_warning.is_some()
    }
}

async fn resolve_merge_request(
    ccx: Arc<AMutex<AtCommandsContext>>,
    args: &HashMap<String, Value>,
    tool_call_id: &str,
) -> Result<MergeRequest, String> {
    let ccx_lock = ccx.lock().await;
    let is_planner = ccx_lock
        .task_meta
        .as_ref()
        .map(|meta| meta.role == "planner")
        .unwrap_or(false);
    if !is_planner {
        return Err("merge_agent can only be called by the task planner. \
                 Switch to the planner chat to merge agent work."
            .to_string());
    }
    let gcx = ccx_lock.app.gcx.clone();
    let chat_facade = ccx_lock.app.chat.facade.clone();
    drop(ccx_lock);

    let task_id = require_bound_planner_task(&ccx, args).await?;
    let card_id = required_string_arg(args, "card_id")?.to_string();
    let strategy = strategy_arg(args)?.to_string();
    parse_force(args)?;
    let auto_revert = parse_auto_revert(args)?;
    let auto_revert_timeout = parse_auto_revert_timeout(args)?;
    if strategy != "merge" && strategy != "squash" {
        return Err(format!(
            "Invalid strategy '{}', must be 'merge' or 'squash'",
            strategy
        ));
    }
    Ok(MergeRequest {
        gcx,
        chat_facade,
        task_id,
        card_id,
        strategy,
        auto_revert,
        auto_revert_timeout,
        tool_call_id: tool_call_id.to_string(),
    })
}

async fn resolve_merge_state<F, Fut>(
    request: MergeRequest,
    git_probe: F,
) -> Result<MergeState, String>
where
    F: FnOnce(std::path::PathBuf) -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let board = storage::load_board(request.gcx.clone(), &request.task_id).await?;
    let card = board
        .get_card(&request.card_id)
        .ok_or(format!("Card {} not found", request.card_id))?;
    let verifier_warning = ensure_verifier_allows_merge(card, false)?;
    let completion_warning = ensure_card_done_allows_merge(card);
    if card.column == "doing" {
        let reports = stop_live_card_sessions(
            &request.gcx,
            request.chat_facade.as_ref(),
            &request.task_id,
            &request.card_id,
        )
        .await
        .map_err(|error| format!("agent still running; cancel it first: {}", error))?;
        for report in reports {
            report.result.map_err(|error| {
                format!(
                    "agent still running; cancel it first ({}): {}",
                    report.chat_id, error
                )
            })?;
        }
    }
    let project_dirs = crate::files_correction::get_project_dirs(request.gcx.clone()).await;
    let workspace_root = project_dirs.first().ok_or("No workspace folder found")?;
    if !git_probe(workspace_root.clone()).await {
        return Err("Workspace is not a git repository".to_string());
    }

    let expected_assignee = card.assignee.clone();
    let expected_agent_chat_id = card.agent_chat_id.clone();
    let task_meta = storage::load_task_meta(request.gcx.clone(), &request.task_id).await?;
    let base_branch = card
        .base_branch
        .as_ref()
        .or(task_meta.base_branch.as_ref())
        .ok_or("Task has no base branch set")?
        .clone();
    let comparison_base = card
        .base_commit
        .as_deref()
        .unwrap_or(&base_branch)
        .to_string();

    Ok(MergeState {
        request,
        workspace_root: workspace_root.to_path_buf(),
        card: card.clone(),
        base_branch,
        comparison_base,
        verifier_warning,
        completion_warning,
        expected_assignee,
        expected_agent_chat_id,
        merge_service: None,
    })
}

async fn execute_merge_state(state: MergeState) -> Result<(bool, Vec<ContextEnum>), String> {
    let request = &state.request;
    if state.card.agent_worktree_name.is_some() {
        match Box::pin(merge_registered_task_worktree(
            request.gcx.clone(),
            &state.workspace_root,
            &request.task_id,
            &request.card_id,
            &request.strategy,
            &request.tool_call_id,
            None,
            request.auto_revert,
            request.auto_revert_timeout,
        ))
        .await
        {
            Ok(Some(result)) => return Ok(result),
            Ok(None) => {}
            Err(error) => return Err(error),
        }
    }
    Box::pin(merge_legacy_task_worktree(state)).await
}

pub struct ToolTaskMergeAgent;

pub(super) async fn merge_agent_with_git_probe<F, Fut>(
    ccx: Arc<AMutex<AtCommandsContext>>,
    tool_call_id: &str,
    args: &HashMap<String, Value>,
    git_probe: F,
) -> Result<(bool, Vec<ContextEnum>), String>
where
    F: FnOnce(std::path::PathBuf) -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let request = resolve_merge_request(ccx, args, tool_call_id).await?;
    let state = resolve_merge_state(request, git_probe).await?;
    run_merge_detached(state).await
}

async fn run_merge_detached(state: MergeState) -> Result<(bool, Vec<ContextEnum>), String> {
    let actor = crate::file_activity::current_actor().unwrap_or_default();
    let operation = crate::file_activity::TOOL_ACTOR.scope(actor, async move {
        #[cfg(test)]
        if let Some(gate) = merge_task_gate::take(&state.request.card_id) {
            gate.wait().await;
        }
        Box::pin(execute_merge_state(state)).await
    });
    #[cfg(test)]
    let operation = {
        let fail_board = FAIL_BOARD_AFTER_MERGE
            .try_with(|fail| *fail)
            .unwrap_or(false);
        FAIL_BOARD_AFTER_MERGE.scope(fail_board, operation)
    };
    let handle = tokio::spawn(operation);
    handle
        .await
        .map_err(|error| format!("merge_agent task failed: {error}; inspect the target checkout and card before retrying"))?
}

#[cfg(test)]
pub(crate) mod merge_task_gate {
    use std::sync::Mutex;

    pub(crate) struct Gate {
        pub(crate) entered: tokio::sync::oneshot::Sender<()>,
        pub(crate) release: tokio::sync::oneshot::Receiver<()>,
    }

    impl Gate {
        pub(super) async fn wait(self) {
            let _ = self.entered.send(());
            let _ = self.release.await;
        }
    }

    static GATES: Mutex<Vec<(String, Gate)>> = Mutex::new(Vec::new());

    pub(crate) fn install(card_id: &str, gate: Gate) {
        GATES.lock().unwrap().push((card_id.to_string(), gate));
    }

    pub(super) fn take(card_id: &str) -> Option<Gate> {
        let mut gates = GATES.lock().unwrap();
        let index = gates.iter().position(|(card, _)| card == card_id)?;
        Some(gates.remove(index).1)
    }
}

impl ToolTaskMergeAgent {
    pub fn new() -> Self {
        Self
    }
}

async fn run_git_in(cwd: &std::path::Path, args: &[&str]) -> Result<String, String> {
    if let Some(service) = crate::coordination::merge_gate::verification_service() {
        service.checkpoint_merge_phase().await?;
    }
    let output = tokio::process::Command::new("git")
        .kill_on_drop(true)
        .args(args)
        .current_dir(cwd)
        .output()
        .await
        .map_err(|e| format!("Failed to run git: {}", e))?;

    if let Some(service) = crate::coordination::merge_gate::verification_service() {
        service.checkpoint_merge_phase().await?;
    }
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

#[async_trait]
impl Tool for ToolTaskMergeAgent {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "merge_agent".to_string(),
            display_name: "Task Merge Agent".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Merge an agent's work back to the main branch and always cleanup the worktree after a successful merge or no-op merged state. The agent must have completed work on a card with an associated git branch and worktree.".to_string(),
            input_schema: json_schema_from_params(&[("card_id", "string", "Card ID whose agent branch to merge"), ("strategy", "string", "Merge strategy: 'squash' (default) or 'merge'"), ("force", "boolean", "Deprecated and ignored: merges are never blocked by verification or completion state; failures are reported as warnings"), ("auto_revert", "boolean", "Opt in to post-merge verification and automatic git revert on deterministic regression"), ("auto_revert_timeout_secs", "integer", "Timeout for post-merge verification and revert commands, default 300 seconds")], &["card_id"]),
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
        merge_agent_with_git_probe(ccx, tool_call_id, args, |root| async move {
            tokio::process::Command::new("git")
                .args(["rev-parse", "--is-inside-work-tree"])
                .current_dir(root)
                .output()
                .await
                .map(|output| output.status.success())
                .unwrap_or(false)
        })
        .await
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

enum LegacyPreparationOutcome {
    Complete((bool, Vec<ContextEnum>)),
    Ready(LegacyMergePreparation),
}

struct LegacyMergePreparation {
    state: MergeState,
    agent_branch: String,
    agent_worktree: String,
    changed_files: Vec<String>,
}

struct LegacyMergeCommit {
    preparation: LegacyMergePreparation,
    commit_msg: String,
    expected_card_state: Option<crate::chat::verifier::ExpectedCardState>,
}

struct LegacyMergeGitResult {
    commit: LegacyMergeCommit,
    merge_started_at: chrono::DateTime<chrono::Utc>,
    authority_lost: bool,
    source_kept: bool,
    squash_nothing_to_commit: bool,
    merge_commit: Option<String>,
    conflict_response: Option<String>,
    agent_worktree_dirty: bool,
    worktree_removed: bool,
    branch_deleted: bool,
    warnings: Vec<String>,
}

async fn merge_legacy_task_worktree(
    mut state: MergeState,
) -> Result<(bool, Vec<ContextEnum>), String> {
    ensure_legacy_agent_worktree_checkout(
        &state.workspace_root,
        state
            .card
            .agent_worktree
            .as_deref()
            .ok_or("Card has no agent worktree")?,
        state
            .card
            .agent_branch
            .as_deref()
            .ok_or("Card has no agent branch")?,
    )?;
    let app = crate::app_state::AppState::from_gcx(state.request.gcx.clone()).await;
    let service = crate::worktrees::service::fenced_service(
        WorktreeService::new_async(app.paths.cache_dir.clone(), state.workspace_root.clone())
            .await?,
        app.runtime.tool_jobs.scopes.clone(),
    );
    // Mirror the accepted board attachment for the merge-only fence. This does
    // not grant the stronger authority required to bind a task-agent session.
    let root = tokio::fs::canonicalize(
        state
            .card
            .agent_worktree
            .as_deref()
            .ok_or("Card has no agent worktree")?,
    )
    .await
    .map_err(|error| format!("Legacy checkout identity unavailable: {error}"))?;
    let root = dunce::simplified(&root).to_path_buf();
    let meta = crate::worktrees::types::WorktreeMeta {
        id: state
            .card
            .agent_worktree_name
            .clone()
            .or_else(|| {
                root.file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_string)
            })
            .ok_or("Legacy checkout identity missing")?,
        kind: "task_agent".into(),
        root,
        source_workspace_root: service.source_workspace_root().to_path_buf(),
        repo_root: service.source_workspace_root().to_path_buf(),
        branch: state.card.agent_branch.clone(),
        base_branch: Some(state.base_branch.clone()),
        base_commit: state.card.base_commit.clone(),
        task_id: Some(state.request.task_id.clone()),
        card_id: Some(state.request.card_id.clone()),
        agent_id: state.card.assignee.clone(),
        enforce: true,
    };
    let service = service
        .fence_legacy_merge_worktree(&meta, &state.base_branch)
        .await?
        .acquire_merge_permit()
        .await?
        .for_merge_target(&state.base_branch)
        .await?;
    state.workspace_root = service.merge_checkout_root().to_path_buf();
    let (service, renewal) = if let Some(leases) =
        crate::coordination::leases::merge_leases_for(&app, service.source_workspace_root()).await?
    {
        let gate = crate::coordination::merge_gate::MergeGate::new(
            leases.clone(),
            &crate::file_activity::settings(),
        );
        let key = crate::coordination::merge_gate::checkout_key(&state.workspace_root);
        let slot = gate.acquire(&key).await.map_err(|e| e.to_string())?;
        let service = service.with_merge_authority(leases.phase_validator(slot.holder().clone()));
        (
            service,
            Some(crate::coordination::leases::LeaseRenewal::spawn(
                leases,
                slot.holder().clone(),
            )),
        )
    } else {
        (service, None)
    };
    state.merge_service = Some(service.clone());
    let landed = std::sync::atomic::AtomicBool::new(false);
    #[cfg(test)]
    let target_root = state.workspace_root.clone();
    let result = crate::coordination::merge_gate::MERGE_PHASE_SCOPE
        .scope(
            service.clone(),
            Box::pin(async {
                service.materialize_merge_target().await?;
                let preparation = match Box::pin(prepare_legacy_merge(state)).await? {
                    LegacyPreparationOutcome::Complete(result) => return Ok(result),
                    LegacyPreparationOutcome::Ready(preparation) => preparation,
                };
                let commit = Box::pin(prepare_legacy_merge_commit(preparation)).await?;
                let git_result = Box::pin(run_legacy_git_merge(commit)).await?;
                let applied =
                    git_result.conflict_response.is_none() && !git_result.squash_nothing_to_commit;
                landed.store(applied, std::sync::atomic::Ordering::SeqCst);
                let result = Box::pin(finish_legacy_merge(git_result)).await;
                let cleanup = match service.checkpoint_merge_phase().await {
                    Ok(()) if result.is_ok() => service.cleanup_merge_target().await,
                    Ok(()) => Ok(()),
                    Err(error) => Err(error),
                };
                if let Err(error) = cleanup {
                    if !applied {
                        return Err(error);
                    }
                    tracing::warn!("legacy merge applied; merge target cleanup failed: {error}");
                }
                result
            }),
        )
        .await;
    let Some(mut renewal) = renewal else {
        return result;
    };
    let released = renewal.shutdown().await.map_err(|error| error.to_string());
    #[cfg(test)]
    let released = released.and_then(|held| {
        legacy_fault::hit(&target_root, legacy_fault::Point::LeaseRelease).map(|_| held)
    });
    let Err(error) = released else {
        return result;
    };
    if !landed.load(std::sync::atomic::Ordering::SeqCst) {
        return result.and(Err(error));
    }
    tracing::warn!("legacy merge applied; releasing the merge lease failed: {error}");
    result.map(|(stop, mut messages)| {
        if let Some(ContextEnum::ChatMessage(message)) = messages.last_mut() {
            if let ChatContent::SimpleText(text) = &mut message.content {
                text.push_str(&format!(
                    "\n\n**Warning:** The merge was applied, but releasing the merge lease failed: {error}"
                ));
            }
        }
        (stop, messages)
    })
}

async fn prepare_legacy_merge(state: MergeState) -> Result<LegacyPreparationOutcome, String> {
    let agent_branch = state.card.agent_branch.clone().ok_or(format!(
        "Card {} has no agent branch",
        state.request.card_id
    ))?;
    let agent_worktree = state.card.agent_worktree.clone().ok_or(format!(
        "Card {} has no agent worktree",
        state.request.card_id
    ))?;
    ensure_legacy_agent_worktree_checkout(&state.workspace_root, &agent_worktree, &agent_branch)?;

    let merge_in_progress = run_git_in(
        &state.workspace_root,
        &["rev-parse", "-q", "--verify", "MERGE_HEAD"],
    )
    .await
    .is_ok();
    if merge_in_progress {
        let status = run_git_in(&state.workspace_root, &["status", "--porcelain"])
            .await
            .unwrap_or_default();
        let conflict_files: Vec<String> = status
            .lines()
            .filter(|line| is_merge_conflict_line(line))
            .filter_map(|line| line.get(3..).map(|file| file.to_string()))
            .collect();
        let conflict_msg = format!(
            r#"# Merge Already In Progress

A previous merge is still in progress with unresolved conflicts.

## Conflicting Files
{}

## How to Resolve

### Option 1: Resolve conflicts
1. Use `cat <file>` to see conflict markers (`<<<<<<<`, `=======`, `>>>>>>>`)
2. Use `update_textdoc` to resolve each conflict
3. Stage and commit: `git add -A && git commit -m "Resolved conflicts"`

### Option 2: Abort and retry
```
git merge --abort
```
Then call `merge_agent` again."#,
            if conflict_files.is_empty() {
                "None detected (check `git status`)".to_string()
            } else {
                conflict_files
                    .iter()
                    .map(|file| format!("- {}", file))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        );
        return Ok(LegacyPreparationOutcome::Complete((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(conflict_msg),
                tool_calls: None,
                tool_call_id: state.request.tool_call_id.clone(),
                ..Default::default()
            })],
        )));
    }

    crate::worktrees::git::validate_merge_checkout(&state.workspace_root, &state.base_branch)?;
    state
        .merge_service
        .as_ref()
        .ok_or("Legacy merge permit missing")?
        .checkpoint_merge_phase()
        .await?;
    let commit_root = std::path::PathBuf::from(&agent_worktree);
    let commit_message = format!("Card {}: {}", state.request.card_id, state.card.title);
    state
        .merge_service
        .as_ref()
        .ok_or("Legacy merge permit missing")?
        .run_merge_git_phase(move || {
            crate::worktrees::git::commit_all(&commit_root, &commit_message)
        })
        .await?;
    let changed_files = crate::chat::task_agent_monitor::git_diff_name_only(
        std::path::Path::new(&agent_worktree),
        &state.comparison_base,
        &agent_branch,
    );
    let commits_ahead_result = run_git_in(
        &state.workspace_root,
        &[
            "rev-list",
            "--count",
            &format!("{}..{}", state.comparison_base, agent_branch),
        ],
    )
    .await;
    let commits_ahead = match commits_ahead_result {
        Ok(output) => output
            .trim()
            .parse::<u32>()
            .map_err(|error| format!("Failed to parse commits ahead count: {}", error))?,
        Err(error) => {
            return Err(format!(
                "Failed to count commits ahead (base: {}, agent: {}): {}. \
                Check that both branches exist and are valid.",
                state.comparison_base, agent_branch, error
            ));
        }
    };

    if commits_ahead == 0 {
        let result = Box::pin(finish_legacy_nothing_to_merge(
            &state,
            &agent_branch,
            &agent_worktree,
        ))
        .await?;
        return Ok(LegacyPreparationOutcome::Complete(result));
    }

    Ok(LegacyPreparationOutcome::Ready(LegacyMergePreparation {
        state,
        agent_branch,
        agent_worktree,
        changed_files,
    }))
}

fn is_merge_conflict_line(line: &str) -> bool {
    let bytes = line.as_bytes();
    bytes.len() >= 2
        && (bytes[0] == b'U'
            || bytes[1] == b'U'
            || (bytes[0] == b'A' && bytes[1] == b'A')
            || (bytes[0] == b'D' && bytes[1] == b'D'))
}

async fn finish_legacy_nothing_to_merge(
    state: &MergeState,
    agent_branch: &str,
    agent_worktree: &str,
) -> Result<(bool, Vec<ContextEnum>), String> {
    let request = &state.request;
    state
        .merge_service
        .as_ref()
        .ok_or("Legacy merge permit missing")?
        .checkpoint_merge_phase()
        .await?;
    let worktree_status = tokio::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(agent_worktree)
        .output()
        .await
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_default();
    let main_status = run_git_in(&state.workspace_root, &["status", "--porcelain"])
        .await
        .unwrap_or_default();
    let main_dirty = !main_status.trim().is_empty();
    let worktree_dirty = !worktree_status.is_empty();
    let diagnostic = if main_dirty && !worktree_dirty {
        "Main workspace has uncommitted changes but agent worktree is clean. Agent likely edited files in the wrong directory."
    } else if worktree_dirty {
        "Agent worktree has uncommitted changes. The worktree and branch are preserved; inspect the worktree or use restart_agent(mode=resume)."
    } else {
        "Both main workspace and agent worktree are clean. Agent may not have made any changes."
    };
    let mut cleanup_status = Vec::new();
    let mut worktree_removed = false;
    let mut branch_deleted = false;
    if worktree_dirty {
        cleanup_status.push(
            "cleanup skipped; dirty agent worktree preserved for inspection or restart_agent(mode=resume)",
        );
    } else if agent_branch != state.base_branch {
        (worktree_removed, branch_deleted) = state
            .merge_service
            .as_ref()
            .ok_or("Legacy merge permit missing")?
            .cleanup_legacy_merge_source()
            .await?;
        if worktree_removed {
            cleanup_status.push("worktree removed");
        }
        if branch_deleted {
            cleanup_status.push("branch deleted");
        }
    }

    let finalize_card = state.finalizes_card();
    if worktree_removed || branch_deleted || finalize_card {
        let card_id_owned = request.card_id.clone();
        let clear_worktree = worktree_removed;
        let clear_branch = branch_deleted;
        let expected_assignee = state.expected_assignee.clone();
        let expected_agent_chat_id = state.expected_agent_chat_id.clone();
        let (_, stats_changed) =
            storage::update_board_atomic(request.gcx.clone(), &request.task_id, move |board| {
                let Some(card) = board.get_card_mut(&card_id_owned) else {
                    return Ok(false);
                };
                if !force_merge_card_matches_current(
                    card,
                    expected_assignee.as_deref(),
                    expected_agent_chat_id.as_deref(),
                ) {
                    return Ok(false);
                }
                if clear_branch {
                    card.agent_branch = None;
                }
                if clear_worktree {
                    card.agent_worktree = None;
                    card.agent_worktree_name = None;
                }
                Ok(finalize_card
                    && finalize_force_merge_card_if_current(
                        card,
                        expected_assignee.as_deref(),
                        expected_agent_chat_id.as_deref(),
                        None,
                    ))
            })
            .await?;
        if stats_changed {
            storage::update_task_stats(request.gcx.clone(), &request.task_id).await?;
        }
    }
    let cleanup_msg = if cleanup_status.is_empty() {
        "No cleanup performed.".to_string()
    } else {
        format!("Cleanup: {}.", cleanup_status.join(", "))
    };
    let mut message = format!(
        "# Nothing to Merge\n\n**Card:** {}\n**Branch:** {}\n**Commits ahead of base:** 0\n\n**Diagnostic:** {}\n\n{}",
        request.card_id, agent_branch, diagnostic, cleanup_msg
    );
    append_verifier_warning(&mut message, state.verifier_warning.as_deref());
    Ok((
        false,
        vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(message),
            tool_calls: None,
            tool_call_id: request.tool_call_id.clone(),
            ..Default::default()
        })],
    ))
}

async fn prepare_legacy_merge_commit(
    preparation: LegacyMergePreparation,
) -> Result<LegacyMergeCommit, String> {
    let diff = run_git_in(
        &preparation.state.workspace_root,
        &[
            "diff",
            &format!(
                "{}...{}",
                preparation.state.comparison_base, preparation.agent_branch
            ),
        ],
    )
    .await
    .unwrap_or_default();
    let commit_msg = match Box::pin(
        crate::agentic::generate_commit_message::generate_commit_message_by_diff(
            preparation.state.request.gcx.clone(),
            &diff,
            &Some(preparation.state.card.title.clone()),
            None,
        ),
    )
    .await
    {
        Ok(message) if !message.trim().is_empty() => message,
        _ => format!(
            "Card {}: {}",
            preparation.state.request.card_id, preparation.state.card.title
        ),
    };
    let expected_card_state = if preparation.state.request.auto_revert {
        Some(
            load_expected_card_state(
                preparation.state.request.gcx.clone(),
                &preparation.state.request.task_id,
                &preparation.state.request.card_id,
            )
            .await?,
        )
    } else {
        None
    };
    Ok(LegacyMergeCommit {
        preparation,
        commit_msg,
        expected_card_state,
    })
}

fn legacy_conflict_response(
    state: &MergeState,
    agent_branch: &str,
    files: Vec<String>,
    aborted: bool,
    warnings: Vec<String>,
) -> String {
    let response = MergeWorktreeResponse {
        id: String::new(),
        status: "conflict".to_string(),
        merged: false,
        strategy: state.request.strategy.clone(),
        source_branch: agent_branch.to_string(),
        target_branch: state.base_branch.clone(),
        committed_uncommitted: None,
        merge_commit: None,
        cleanup: None,
        conflict: Some(crate::worktrees::types::WorktreeConflictState {
            files,
            aborted,
            merge_in_progress: crate::worktrees::git::has_merge_in_progress(&state.workspace_root),
            instructions: if aborted {
                "The merge was aborted and cleaned up; no conflict markers were left behind and uncommitted target changes were restored. Merge the source branch changes against the target branch yourself, then retry."
            } else {
                "Conflict cleanup failed. Inspect the target workspace and warnings before retrying; uncommitted target changes that could not be restored remain in git stash."
            }
            .to_string(),
        }),
        dirty_baseline: None,
        affected_references: Vec::new(),
        affected_reference_count: 0,
        warnings,
        source_kept: false,
    };
    merge_response_message(&state.request.card_id, &response, None)
}

#[cfg(test)]
pub(crate) mod legacy_fault {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum Point {
        AfterStash,
        AfterMerge,
        AfterCommit,
        CommitWindow,
        WriteTree,
        LeaseRelease,
    }

    pub(crate) enum Action {
        Fail,
        Block {
            entered: std::sync::mpsc::Sender<()>,
            release: std::sync::mpsc::Receiver<()>,
        },
    }

    static FAULTS: Mutex<Vec<(PathBuf, Point, Action)>> = Mutex::new(Vec::new());

    fn key(root: &Path) -> PathBuf {
        dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf())
    }

    pub(crate) fn install(root: &Path, point: Point, action: Action) {
        FAULTS.lock().unwrap().push((key(root), point, action));
    }

    pub(super) fn hit(root: &Path, point: Point) -> Result<(), String> {
        let root = key(root);
        let action = {
            let mut faults = FAULTS.lock().unwrap();
            let Some(index) = faults
                .iter()
                .position(|(path, at, _)| *path == root && *at == point)
            else {
                return Ok(());
            };
            faults.remove(index).2
        };
        match action {
            Action::Fail => Err(format!("injected merge authority loss at {point:?}")),
            Action::Block {
                entered,
                release,
            } => {
                let _ = entered.send(());
                let _ = release.recv();
                Ok(())
            }
        }
    }
}

fn legacy_fault_point(_root: &std::path::Path, _point: LegacyFaultPoint) -> Result<(), String> {
    #[cfg(test)]
    {
        let point = match _point {
            LegacyFaultPoint::AfterStash => legacy_fault::Point::AfterStash,
            LegacyFaultPoint::AfterMerge => legacy_fault::Point::AfterMerge,
            LegacyFaultPoint::AfterCommit => legacy_fault::Point::AfterCommit,
            LegacyFaultPoint::CommitWindow => legacy_fault::Point::CommitWindow,
            LegacyFaultPoint::WriteTree => legacy_fault::Point::WriteTree,
        };
        legacy_fault::hit(_root, point)?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum LegacyFaultPoint {
    AfterStash,
    AfterMerge,
    AfterCommit,
    CommitWindow,
    WriteTree,
}

fn legacy_post_commit_warning(error: &str) -> String {
    format!(
        "The merge commit already landed, so it was kept; merge authority was lost afterwards: {}",
        error
    )
}

struct LegacySectionInput {
    root: std::path::PathBuf,
    base_branch: String,
    agent_branch: String,
    agent_worktree: String,
    squash: bool,
    commit_msg: String,
}

enum LegacySectionOutcome {
    Merged {
        squash_nothing_to_commit: bool,
        merge_commit: Option<String>,
        authority_lost: bool,
        source_kept: bool,
        warnings: Vec<String>,
    },
    Conflict {
        files: Vec<String>,
        aborted: bool,
        warnings: Vec<String>,
    },
    IgnoredCollision {
        files: Vec<String>,
        restored: bool,
        warnings: Vec<String>,
    },
    Failed(String),
}

struct LegacyRecoveryGuard {
    root: std::path::PathBuf,
    pre_merge_head: Option<String>,
    own_commit: Option<String>,
    stash: Option<crate::worktrees::git::DirtyWorktreeStash>,
    merge_started: bool,
}

impl LegacyRecoveryGuard {
    fn needs_recovery(&self) -> bool {
        self.stash.is_some()
    }

    fn recover(&mut self) -> (bool, Vec<String>) {
        let (ok, warnings, _) = self.recover_detailed();
        (ok, warnings)
    }

    fn recover_detailed(
        &mut self,
    ) -> (
        bool,
        Vec<String>,
        Option<crate::worktrees::git::RestoreStashedChangesResult>,
    ) {
        let stash = self.stash.take();
        if stash.is_none() && !self.merge_started {
            return (true, Vec::new(), None);
        }
        let result = crate::worktrees::git::rollback_merge_owned(
            &self.root,
            self.pre_merge_head.as_deref(),
            self.own_commit.as_deref(),
            stash.as_ref(),
        );
        let mut warnings = result.warnings.clone();
        if !result.stash_restored && result.foreign_head.is_none() {
            if let Some(stash) = stash.as_ref() {
                warnings.push(format!(
                    "Your uncommitted target edits are preserved in {}",
                    stash.describe()
                ));
            }
        }
        (result.critical_ok, warnings, Some(result))
    }

    fn recovery_error(&mut self, error: String) -> String {
        let had_stash = self.stash.is_some();
        if !had_stash && !self.merge_started {
            return error;
        }
        let stash_desc = self.stash.as_ref().map(|stash| stash.describe());
        let (restored, warnings, result) = self.recover_detailed();
        let (rolled_back, stash_restored, foreign_head) =
            result.as_ref().map_or((restored, true, None), |result| {
                (
                    result.rolled_back && result.foreign_head.is_none(),
                    result.stash_restored,
                    result.foreign_head.clone(),
                )
            });
        let rendered = render_inline_warnings(&warnings);
        if let Some(head) = foreign_head {
            let restore = match (had_stash, stash_restored) {
                (false, _) => String::new(),
                (true, true) => "; your uncommitted target edits were restored".to_string(),
                (true, false) => "; restoring your uncommitted target edits FAILED".to_string(),
            };
            return format!(
                "{error}; the target HEAD moved to {head} during the merge; nothing was rolled back{restore}{rendered}"
            );
        }
        match (had_stash, rolled_back && restored, stash_restored) {
            (true, true, _) => format!(
                "{error}; the target was rolled back to its pre-merge HEAD and your uncommitted target edits were restored{rendered}"
            ),
            (true, false, true) => format!(
                "{error}; the target was rolled back with warnings; your uncommitted target edits were restored{rendered}"
            ),
            (true, false, false) => format!(
                "{error}; restoring your uncommitted target edits FAILED — they are preserved in {}{rendered}",
                stash_desc.unwrap_or_default()
            ),
            (false, true, _) => format!(
                "{error}; the target was rolled back to its pre-merge HEAD{rendered}"
            ),
            (false, false, _) => format!(
                "{error}; rolling the target back failed; inspect it before retrying{rendered}"
            ),
        }
    }
}

impl Drop for LegacyRecoveryGuard {
    fn drop(&mut self) {
        if self.needs_recovery() {
            let (restored, warnings) = self.recover();
            tracing::warn!(
                "legacy merge section ended abnormally; target rollback restored={} warnings={:?}",
                restored,
                warnings
            );
        }
    }
}

fn ignored_overwrite_paths(error: &str) -> Option<Vec<String>> {
    let marker = "would be overwritten by merge";
    let start = error.find(marker)?;
    let files = error[start..]
        .lines()
        .skip(1)
        .take_while(|line| line.starts_with('\t') || line.starts_with("    "))
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    Some(files)
}

fn legacy_retained_error(context: &str, error: &str, input: &LegacySectionInput) -> String {
    format!(
        "{}: {}; integration state retained at '{}' with source worktree '{}' intact; inspect the target index and worktree before retrying",
        context,
        error,
        input.root.display(),
        input.agent_worktree
    )
}

fn run_legacy_merge_section(
    input: LegacySectionInput,
    checkpoint: &dyn Fn() -> Result<(), String>,
) -> LegacySectionOutcome {
    use crate::worktrees::git;

    let root = input.root.clone();
    let mut guard = LegacyRecoveryGuard {
        root: root.clone(),
        pre_merge_head: None,
        own_commit: None,
        stash: None,
        merge_started: false,
    };
    match git::stash_dirty_worktree_retaining(&root, "Merge target", Some(&input.base_branch)) {
        Ok(stash) => guard.stash = stash,
        Err((error, stash)) => {
            guard.stash = stash;
            return LegacySectionOutcome::Failed(guard.recovery_error(error));
        }
    }
    let had_stash = guard.stash.is_some();
    let forward = |guard: &mut LegacyRecoveryGuard, point: Option<LegacyFaultPoint>| {
        point
            .map_or(Ok(()), |point| legacy_fault_point(&root, point))
            .and_then(|_| checkpoint())
            .map_err(|error| guard.recovery_error(error))
    };
    if let Err(error) = forward(&mut guard, Some(LegacyFaultPoint::AfterStash)) {
        return LegacySectionOutcome::Failed(error);
    }
    if let Err(error) = git::checkout_branch(&root, &input.base_branch) {
        return LegacySectionOutcome::Failed(
            guard.recovery_error(format!("Failed to checkout base branch: {}", error)),
        );
    }
    match git::head_rev(&root) {
        Ok(head) => guard.pre_merge_head = Some(head),
        Err(error) => {
            return LegacySectionOutcome::Failed(
                guard.recovery_error(format!("Failed to read pre-merge target HEAD: {}", error)),
            )
        }
    }
    let collisions = match git::ignored_paths_colliding_with_incoming(&root, &input.agent_branch) {
        Ok(collisions) => collisions,
        Err(error) => {
            return LegacySectionOutcome::Failed(guard.recovery_error(format!(
                "Failed to check ignored target files against incoming paths: {}",
                error
            )))
        }
    };
    if !collisions.is_empty() {
        let (restored, warnings) = guard.recover();
        return LegacySectionOutcome::IgnoredCollision {
            files: collisions,
            restored,
            warnings,
        };
    }
    if let Err(error) = forward(&mut guard, None) {
        return LegacySectionOutcome::Failed(error);
    }
    guard.merge_started = true;
    let pre_merge_head = guard.pre_merge_head.clone().unwrap_or_default();
    let mut post_commit_warnings = Vec::new();
    let source_oid = match git::run_git(
        &root,
        &[
            "rev-parse",
            "--verify",
            &format!("{}^{{commit}}", input.agent_branch),
        ],
    ) {
        Ok(oid) => oid.trim().to_string(),
        Err(error) => {
            return LegacySectionOutcome::Failed(
                guard.recovery_error(format!("Failed to resolve the source branch: {}", error)),
            )
        }
    };
    let merge_result = if input.squash {
        git::run_git(
            &root,
            &[
                "merge",
                "--squash",
                "--no-overwrite-ignore",
                &input.agent_branch,
            ],
        )
    } else {
        let merged = refact_worktrees::locking::with_git_phase(None, || {
            git::run_git_with_refact_author(
                &root,
                &[
                    "merge",
                    "--no-ff",
                    "--no-overwrite-ignore",
                    "--no-gpg-sign",
                    &input.agent_branch,
                    "-m",
                    &input.commit_msg,
                ],
            )
            .and_then(|out| legacy_fault_point(&root, LegacyFaultPoint::CommitWindow).map(|_| out))
        });
        match merged {
            Err(error)
                if git::find_landed_commit(
                    &root,
                    &pre_merge_head,
                    git::Landing::Merge {
                        source: &source_oid,
                    },
                )
                .is_some() =>
            {
                post_commit_warnings.push(legacy_post_commit_warning(&error));
                Ok(String::new())
            }
            merged => merged,
        }
    };
    if let Err(error) = merge_result {
        let conflict_files = git::conflict_files_for_path(&root);
        if !conflict_files.is_empty() {
            let (aborted, warnings) = if had_stash {
                guard.recover()
            } else {
                refact_worktrees::locking::with_git_phase(None, || {
                    let mut warnings = git::cleanup_failed_merge(&root);
                    let mut ok = warnings.is_empty();
                    if let Err(error) =
                        git::ensure_clean_worktree(&root, "Merge target after conflict cleanup")
                    {
                        ok = false;
                        warnings.push(format!("Failed to clean up merge conflicts: {}", error));
                    }
                    (ok, warnings)
                })
            };
            return LegacySectionOutcome::Conflict {
                files: conflict_files,
                aborted,
                warnings,
            };
        }
        if let Some(files) = ignored_overwrite_paths(&error) {
            let (restored, warnings) = if had_stash {
                guard.recover()
            } else {
                (true, Vec::new())
            };
            return LegacySectionOutcome::IgnoredCollision {
                files,
                restored,
                warnings,
            };
        }
        if had_stash {
            return LegacySectionOutcome::Failed(guard.recovery_error(format!(
                "Merge failed: {}; the merge was rolled back with source worktree '{}' intact",
                error, input.agent_worktree
            )));
        }
        return LegacySectionOutcome::Failed(legacy_retained_error("Merge failed", &error, &input));
    }
    let retained_or_recovered = |guard: &mut LegacyRecoveryGuard, context: &str, error: String| {
        if had_stash {
            guard.recovery_error(format!(
                "{}: {}; the merge was rolled back with source worktree '{}' intact",
                context, error, input.agent_worktree
            ))
        } else {
            legacy_retained_error(context, &error, &input)
        }
    };
    if let Err(error) =
        legacy_fault_point(&root, LegacyFaultPoint::AfterMerge).and_then(|_| checkpoint())
    {
        if input.squash {
            return LegacySectionOutcome::Failed(retained_or_recovered(
                &mut guard,
                "Merge authority lost after the merge",
                error,
            ));
        }
        post_commit_warnings.push(legacy_post_commit_warning(&error));
    }
    let mut squash_nothing_to_commit = false;
    let mut squash_tree = String::new();
    if input.squash {
        let captured = refact_worktrees::locking::with_git_phase(None, || {
            legacy_fault_point(&root, LegacyFaultPoint::WriteTree)
                .and_then(|_| git::run_git(&root, &["write-tree"]))
        })
        .map(|tree| tree.trim().to_string())
        .and_then(|tree| {
            if tree.is_empty() {
                Err("git write-tree returned no tree".to_string())
            } else {
                Ok(tree)
            }
        });
        squash_tree = match captured {
            Ok(tree) => tree,
            Err(error) => {
                return LegacySectionOutcome::Failed(retained_or_recovered(
                    &mut guard,
                    "Failed to capture the squash tree before committing",
                    error,
                ))
            }
        };
        let committed = refact_worktrees::locking::with_git_phase(None, || {
            git::run_git_with_refact_author(
                &root,
                &["commit", "--no-gpg-sign", "-m", &input.commit_msg],
            )
            .and_then(|out| legacy_fault_point(&root, LegacyFaultPoint::CommitWindow).map(|_| out))
        });
        if let Err(error) = committed {
            if git::find_landed_commit(
                &root,
                &pre_merge_head,
                git::Landing::Squash {
                    tree: &squash_tree,
                },
            )
            .is_some()
            {
                post_commit_warnings.push(legacy_post_commit_warning(&error));
            } else if error.contains("nothing to commit") {
                squash_nothing_to_commit = true;
            } else {
                return LegacySectionOutcome::Failed(retained_or_recovered(
                    &mut guard,
                    "Failed to commit squash merge",
                    error,
                ));
            }
        }
    }
    if post_commit_warnings.is_empty() {
        let after_commit =
            legacy_fault_point(&root, LegacyFaultPoint::AfterCommit).and_then(|_| checkpoint());
        if let Err(error) = after_commit {
            if squash_nothing_to_commit {
                return LegacySectionOutcome::Failed(retained_or_recovered(
                    &mut guard,
                    "Merge authority lost after the squash",
                    error,
                ));
            }
            post_commit_warnings.push(legacy_post_commit_warning(&error));
        }
    }
    let mut warnings = Vec::new();
    let merge_commit = if squash_nothing_to_commit {
        None
    } else {
        let landing = if input.squash {
            git::Landing::Squash {
                tree: &squash_tree,
            }
        } else {
            git::Landing::Merge {
                source: &source_oid,
            }
        };
        let verified_head = git::landed_commit(&root, &pre_merge_head, landing);
        guard.own_commit = verified_head.clone();
        let found = verified_head
            .or_else(|| git::landed_commit_below_head(&root, &pre_merge_head, landing));
        if found.is_none() {
            let head = refact_worktrees::locking::with_git_phase(None, || git::head_rev(&root))
                .unwrap_or_else(|e| format!("<unreadable: {e}>"));
            warnings.push(format!(
                "The target HEAD is {} and this merge's own commit could not be identified on it; inspect the target history before relying on this merge",
                head
            ));
        }
        found
    };
    let authority_lost = !post_commit_warnings.is_empty();
    warnings.splice(0..0, post_commit_warnings);
    let Some(stash) = guard.stash.take() else {
        return LegacySectionOutcome::Merged {
            squash_nothing_to_commit,
            merge_commit,
            authority_lost,
            source_kept: false,
            warnings,
        };
    };
    let applied = refact_worktrees::locking::with_git_phase(None, || {
        git::apply_and_drop_stash(&root, &stash)
    });
    match applied {
        Ok(stash_warnings) => {
            warnings.push(
                "Uncommitted target changes were stashed before the merge and restored afterwards."
                    .to_string(),
            );
            warnings.extend(stash_warnings);
            LegacySectionOutcome::Merged {
                squash_nothing_to_commit,
                merge_commit,
                authority_lost,
                source_kept: false,
                warnings,
            }
        }
        Err(error) => {
            let conflict_files = git::conflict_files_for_path(&root);
            let stash_location = stash.describe();
            guard.stash = Some(stash);
            let (aborted, recover_warnings, result) = guard.recover_detailed();
            let (rolled_back, stash_restored, foreign_head) =
                result.as_ref().map_or((true, true, None), |result| {
                    (
                        result.rolled_back && result.foreign_head.is_none(),
                        result.stash_restored,
                        result.foreign_head.clone(),
                    )
                });
            let kept = !squash_nothing_to_commit
                && match merge_commit.as_deref() {
                    Some(commit) => git::commit_retained(&root, commit),
                    None => !rolled_back,
                };
            if kept {
                warnings.push(git::kept_after_reapply_failure(
                    &error,
                    stash_restored,
                    foreign_head.as_deref(),
                    &stash_location,
                ));
                warnings.extend(recover_warnings);
                warnings.push(crate::worktrees::service::SOURCE_KEPT_WARNING.to_string());
                return LegacySectionOutcome::Merged {
                    squash_nothing_to_commit,
                    merge_commit,
                    authority_lost,
                    source_kept: true,
                    warnings,
                };
            }
            let aborted = aborted && rolled_back && stash_restored;
            warnings.push(git::reapply_failure_summary(
                &error,
                rolled_back,
                stash_restored,
                foreign_head.as_deref(),
                &stash_location,
            ));
            warnings.extend(recover_warnings);
            let files = if conflict_files.is_empty() {
                git::conflict_files_for_path(&root)
            } else {
                conflict_files
            };
            LegacySectionOutcome::Conflict {
                files,
                aborted,
                warnings,
            }
        }
    }
}

async fn run_legacy_git_merge(commit: LegacyMergeCommit) -> Result<LegacyMergeGitResult, String> {
    let state = &commit.preparation.state;
    let service = state
        .merge_service
        .as_ref()
        .ok_or("Legacy merge permit missing")?;
    service.checkpoint_merge_phase().await?;
    crate::worktrees::git::validate_merge_checkout(&state.workspace_root, &state.base_branch)?;
    let input = LegacySectionInput {
        root: state.workspace_root.clone(),
        base_branch: state.base_branch.clone(),
        agent_branch: commit.preparation.agent_branch.clone(),
        agent_worktree: commit.preparation.agent_worktree.clone(),
        squash: state.request.strategy == "squash",
        commit_msg: commit.commit_msg.clone(),
    };
    let merge_started_at = chrono::Utc::now();
    let handle = service
        .spawn_merge_git_section(move |checkpoint| run_legacy_merge_section(input, checkpoint));
    let outcome = handle
        .await
        .map_err(|error| format!("Legacy merge worker failed: {error}; inspect the target checkout and `git stash list` before retrying"))??;
    match outcome {
        LegacySectionOutcome::Merged {
            squash_nothing_to_commit,
            merge_commit,
            authority_lost,
            source_kept,
            warnings,
        } => Ok(LegacyMergeGitResult {
            commit,
            merge_started_at,
            authority_lost,
            source_kept,
            squash_nothing_to_commit,
            merge_commit,
            conflict_response: None,
            agent_worktree_dirty: false,
            worktree_removed: false,
            branch_deleted: false,
            warnings,
        }),
        LegacySectionOutcome::Conflict {
            files,
            aborted,
            warnings,
        } => {
            let message = legacy_conflict_response(
                state,
                &commit.preparation.agent_branch,
                files,
                aborted,
                warnings,
            );
            Ok(LegacyMergeGitResult::conflict(commit, message))
        }
        LegacySectionOutcome::IgnoredCollision {
            files,
            restored,
            warnings,
        } => {
            let message =
                legacy_ignored_collision_message(state, &commit, &files, restored, &warnings);
            Ok(LegacyMergeGitResult::conflict(commit, message))
        }
        LegacySectionOutcome::Failed(error) => Err(error),
    }
}

fn legacy_ignored_collision_message(
    state: &MergeState,
    commit: &LegacyMergeCommit,
    files: &[String],
    restored: bool,
    warnings: &[String],
) -> String {
    let mut message = format!(
        "# Merge Not Performed\n\n**Card:** {}\n**Status:** ignored_file_collision\n**Branch:** {} → {}\n**Strategy:** {}\n\nNothing was merged: the incoming branch adds tracked files at paths occupied by ignored files in the target checkout, and merging would overwrite them. The target branch is unchanged{} and the agent worktree and branch were kept.\n\n## Colliding Ignored Files\n{}\n\n**Next step:** move or delete the ignored files listed above (or rename the incoming paths), then call merge_agent again.",
        state.request.card_id,
        commit.preparation.agent_branch,
        state.base_branch,
        state.request.strategy,
        if restored {
            ", your uncommitted target edits were restored,"
        } else {
            ""
        },
        files
            .iter()
            .map(|file| format!("- {}", file))
            .collect::<Vec<_>>()
            .join("\n")
    );
    append_merge_warnings(&mut message, warnings);
    message
}

fn render_inline_warnings(warnings: &[String]) -> String {
    if warnings.is_empty() {
        String::new()
    } else {
        format!("; warnings: {}", warnings.join("; "))
    }
}

impl LegacyMergeGitResult {
    fn conflict(commit: LegacyMergeCommit, message: String) -> Self {
        Self {
            commit,
            merge_started_at: chrono::Utc::now(),
            authority_lost: false,
            source_kept: false,
            squash_nothing_to_commit: true,
            merge_commit: None,
            conflict_response: Some(message),
            agent_worktree_dirty: false,
            worktree_removed: false,
            branch_deleted: false,
            warnings: Vec::new(),
        }
    }
}

async fn cleanup_legacy_merge(result: &mut LegacyMergeGitResult) -> Result<(), String> {
    let state = &result.commit.preparation.state;
    let agent_branch = &result.commit.preparation.agent_branch;
    let agent_worktree = &result.commit.preparation.agent_worktree;
    result.agent_worktree_dirty = if agent_branch != &state.base_branch {
        match tokio::process::Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(agent_worktree)
            .output()
            .await
        {
            Ok(output) if output.status.success() => {
                !String::from_utf8_lossy(&output.stdout).trim().is_empty()
            }
            _ => true,
        }
    } else {
        false
    };
    if agent_branch != &state.base_branch && !result.agent_worktree_dirty {
        let (worktree_removed, branch_deleted) = state
            .merge_service
            .as_ref()
            .ok_or("Legacy merge permit missing")?
            .cleanup_legacy_merge_source()
            .await?;
        result.worktree_removed = worktree_removed;
        result.branch_deleted = branch_deleted;
    }
    Ok(())
}

async fn finish_legacy_merge(
    mut result: LegacyMergeGitResult,
) -> Result<(bool, Vec<ContextEnum>), String> {
    if let Some(conflict_msg) = result.conflict_response.as_deref() {
        return Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(conflict_msg.to_string()),
                tool_calls: None,
                tool_call_id: result.commit.preparation.state.request.tool_call_id.clone(),
                ..Default::default()
            })],
        ));
    }
    let state = &result.commit.preparation.state;
    let request = &state.request;
    let applied = !result.squash_nothing_to_commit;
    if applied && !request.auto_revert {
        record_legacy_marker(&mut result).await;
    }
    let state = &result.commit.preparation.state;
    let checkpoint = match state.merge_service.as_ref() {
        _ if result.authority_lost => {
            Err("merge authority was already lost inside the merge section".to_string())
        }
        Some(service) => service.checkpoint_merge_phase().await,
        None => Err("Legacy merge permit missing".to_string()),
    };
    let authority_ok = checkpoint.is_ok();
    if let Err(error) = checkpoint {
        if !applied {
            return Err(error);
        }
        result.warnings.push(format!(
            "The merge was applied, but merge authority was lost afterwards: {}",
            error
        ));
    }
    let state = &result.commit.preparation.state;
    let request = &state.request;
    let post_merge_check_result = if request.auto_revert && authority_ok && !result.source_kept {
        match result.merge_commit.clone() {
            Some(merge_commit) => {
                let expected_card_state = result
                    .commit
                    .expected_card_state
                    .clone()
                    .expect("post-merge state is captured whenever auto-revert is enabled");
                Box::pin(crate::chat::post_merge_check::post_merge_check(
                    request.gcx.clone(),
                    crate::chat::post_merge_check::PostMergeCheckRequest {
                        task_id: request.task_id.clone(),
                        card_id: request.card_id.clone(),
                        workspace_root: state.workspace_root.clone(),
                        enabled: true,
                        timeout: request.auto_revert_timeout,
                        expected_merge_commit: merge_commit,
                        expected_card_state: Some(expected_card_state),
                    },
                ))
                .await
                .map(Some)
            }
            None if result.squash_nothing_to_commit => Ok(None),
            None => Err(
                "Post-merge verification could not run: merge commit unavailable; source retained"
                    .to_string(),
            ),
        }
    } else if request.auto_revert && result.source_kept {
        Err(
            "Post-merge verification was skipped: the merge was kept after its rollback failed; source retained"
                .to_string(),
        )
    } else if request.auto_revert {
        Err(
            "Post-merge verification could not run: merge authority was lost; source retained"
                .to_string(),
        )
    } else {
        Ok(None)
    };
    let merge_auto_reverted = post_merge_check_result
        .as_ref()
        .ok()
        .and_then(|result| result.as_ref())
        .map(|result| result.auto_reverted)
        .unwrap_or(false);
    if applied && !merge_auto_reverted {
        record_legacy_marker(&mut result).await;
    }
    if authority_ok
        && !result.source_kept
        && (force_merge_post_check_passed_or_absent(&post_merge_check_result)
            || merge_auto_reverted)
    {
        if let Err(error) = cleanup_legacy_merge(&mut result).await {
            if !applied {
                return Err(error);
            }
            result.warnings.push(format!(
                "The merge was applied, but the agent worktree and branch could not be cleaned up and were kept: {}",
                error
            ));
        }
    }
    let state = &result.commit.preparation.state;
    let request = &state.request;
    let post_merge_check_error = post_merge_check_result.as_ref().err().cloned();
    let retained_verification_failure =
        retained_post_merge_verification_failure(&post_merge_check_result);
    if !merge_auto_reverted && !result.squash_nothing_to_commit {
        let _ = crate::chat::task_agent_monitor::append_card_target_files(
            crate::app_state::AppState::from_gcx(request.gcx.clone()).await,
            &request.task_id,
            &request.card_id,
            result.commit.preparation.changed_files.clone(),
        )
        .await;
    }
    let board = registered_board_fault().and(
        Box::pin(update_legacy_merge_board(
            &result,
            &post_merge_check_result,
            merge_auto_reverted,
            retained_verification_failure,
            post_merge_check_error,
        ))
        .await,
    );
    if let Err(error) = board {
        if !applied {
            return Err(error);
        }
        result.warnings.push(format!(
            "The merge was applied, but updating the card after the merge failed: {}",
            error
        ));
    }
    let post_merge_check = match post_merge_check_result {
        Ok(check) => check,
        Err(error) if applied => {
            result.warnings.push(format!(
                "The merge was applied, but post-merge verification could not run: {}",
                error
            ));
            None
        }
        Err(error) => return Err(error),
    };
    let state = &result.commit.preparation.state;
    let request = &state.request;
    let cleanup_info = legacy_merge_cleanup_message(
        &result.commit.preparation.agent_branch,
        &state.base_branch,
        result.agent_worktree_dirty,
        result.worktree_removed,
        result.branch_deleted,
    );
    let result_message =
        legacy_merge_result_message(&result, &cleanup_info, post_merge_check.as_ref());
    Ok((
        false,
        vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(result_message),
            tool_calls: None,
            tool_call_id: request.tool_call_id.clone(),
            ..Default::default()
        })],
    ))
}

async fn record_legacy_marker(result: &mut LegacyMergeGitResult) {
    let state = &result.commit.preparation.state;
    let request = &state.request;
    if let Err(error) = record_merge_marker(
        request.gcx.clone(),
        &request.task_id,
        &request.card_id,
        AppliedMerge {
            since: result.merge_started_at,
            commit: result.merge_commit.clone(),
        },
        &request.strategy,
        &state.base_branch,
    )
    .await
    {
        result.warnings.push(format!(
            "The merge was applied, but recording the merge on the card failed: {}",
            error
        ));
    }
}

async fn update_legacy_merge_board(
    result: &LegacyMergeGitResult,
    post_merge_check_result: &Result<
        Option<crate::chat::post_merge_check::PostMergeCheckResult>,
        String,
    >,
    merge_auto_reverted: bool,
    retained_verification_failure: Option<String>,
    post_merge_check_error: Option<String>,
) -> Result<(), String> {
    let state = &result.commit.preparation.state;
    let request = &state.request;
    let finalize_card = state.finalizes_card();
    if result.worktree_removed || result.branch_deleted || finalize_card {
        let card_id_owned = request.card_id.clone();
        let clear_worktree = result.worktree_removed;
        let clear_branch = result.branch_deleted;
        let expected_assignee = state.expected_assignee.clone();
        let expected_agent_chat_id = state.expected_agent_chat_id.clone();
        let retained_verification_failure = retained_verification_failure.clone();
        let post_merge_check_error = post_merge_check_error.clone();
        let finalize_force = force_merge_post_check_passed_or_absent(post_merge_check_result);
        let merged = (!result.squash_nothing_to_commit).then(|| AppliedMerge {
            since: result.merge_started_at,
            commit: result.merge_commit.clone(),
        });
        let (_, stats_changed) = storage::update_board_atomic(
            request.gcx.clone(),
            &request.task_id,
            move |board| {
                let Some(card) = board.get_card_mut(&card_id_owned) else {
                    return Ok(false);
                };
                if !force_merge_card_matches_current(card, expected_assignee.as_deref(), expected_agent_chat_id.as_deref()) {
                    return Ok(false);
                }
                if clear_branch { card.agent_branch = None; }
                if clear_worktree { card.agent_worktree = None; card.agent_worktree_name = None; }
                if finalize_card && merge_auto_reverted {
                    card.assignee = None;
                    card.agent_chat_id = None;
                    card.status_updates.push(crate::tasks::types::StatusUpdate {
                        timestamp: chrono::Utc::now().to_rfc3339(),
                        message: "Unfinished or unverified merge was auto-reverted after post-merge verification failed; the original card was not marked done and stale worktree, branch, and agent ownership references were cleared.".to_string(),
                    });
                    return Ok(true);
                }
                if finalize_card {
                    if let Some(message) = retained_verification_failure.as_deref() {
                        return Ok(retain_force_merge_without_finalizing(card, message));
                    }
                    if let Some(error) = post_merge_check_error.as_deref() {
                        card.assignee = None;
                        card.agent_chat_id = None;
                        card.status_updates.push(crate::tasks::types::StatusUpdate {
                            timestamp: chrono::Utc::now().to_rfc3339(),
                            message: format!("Unfinished or unverified merge remains applied, but post-merge verification failed to run: {}. Stale worktree, branch, and agent ownership references were cleared; merge success was not inferred from verification.", error),
                        });
                        return Ok(true);
                    }
                }
                Ok(finalize_card && finalize_force && finalize_force_merge_card_if_current(
                    card,
                    expected_assignee.as_deref(),
                    expected_agent_chat_id.as_deref(),
                    merged.as_ref(),
                ))
            },
        ).await?;
        if stats_changed {
            storage::update_task_stats(request.gcx.clone(), &request.task_id).await?;
        }
    }
    Ok(())
}

fn legacy_merge_cleanup_message(
    agent_branch: &str,
    base_branch: &str,
    agent_worktree_dirty: bool,
    worktree_removed: bool,
    branch_deleted: bool,
) -> String {
    if agent_branch == base_branch {
        return "Cleanup skipped because agent branch matches base branch.".to_string();
    }
    if agent_worktree_dirty {
        return "Worktree retained (uncommitted changes detected; use restart_agent or inspect manually)".to_string();
    }
    match (worktree_removed, branch_deleted) {
        (true, true) => "Worktree and branch cleaned up.".to_string(),
        (true, false) => "Worktree removed, branch deletion failed.".to_string(),
        (false, true) => "Worktree removal failed, branch deleted.".to_string(),
        (false, false) => {
            "Worktree and branch retained; cleanup was deferred or could not complete.".to_string()
        }
    }
}

fn legacy_merge_result_message(
    result: &LegacyMergeGitResult,
    cleanup_info: &str,
    post_merge_check: Option<&crate::chat::post_merge_check::PostMergeCheckResult>,
) -> String {
    let state = &result.commit.preparation.state;
    let mut message = if result.squash_nothing_to_commit {
        format!(
            "# Nothing to Merge\n\n**Card:** {}\n**Branch:** {} → {}\n\n**Cleanup:** {}",
            state.request.card_id,
            result.commit.preparation.agent_branch,
            state.base_branch,
            cleanup_info
        )
    } else {
        format!(
            "# Agent Work Merged\n\n**Card:** {}\n**Strategy:** {}\n**Branch:** {} → {}\n**Cleanup:** {}\n\nThe agent's work has been successfully merged back to the main branch.",
            state.request.card_id,
            state.request.strategy,
            result.commit.preparation.agent_branch,
            state.base_branch,
            cleanup_info
        )
    };
    append_verifier_warning(&mut message, state.verifier_warning.as_deref());
    append_completion_warning(&mut message, state.completion_warning.as_deref());
    if let Some(post_merge_check) = post_merge_check {
        append_post_merge_check_message(&mut message, post_merge_check);
    }
    append_merge_warnings(&mut message, &result.warnings);
    message
}

#[cfg(test)]
#[path = "tool_task_merge_agent_lease_tests.rs"]
mod lease_tests;

#[cfg(test)]
mod worktree_merge_tool_tests {
    use super::*;
    use crate::app_state::AppState;
    use crate::chat::types::TaskMeta as ThreadTaskMeta;
    use crate::tasks::types::{BoardCard, TaskBoard, TaskMeta, TaskStatus, VerifierReport};
    use crate::worktrees::types::CreateWorktreeRequest;
    use serde_json::json;
    use std::path::Path;

    fn run_git(cwd: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap_or_else(|e| panic!("failed to run git {:?}: {}", args, e));
        if !output.status.success() {
            panic!(
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    fn init_repo(root: &Path) {
        run_git(root, &["init"]);
        run_git(root, &["checkout", "-b", "main"]);
        run_git(root, &["config", "core.autocrlf", "false"]);
        run_git(root, &["config", "user.email", "test@example.com"]);
        run_git(root, &["config", "user.name", "Test User"]);
        std::fs::write(root.join("file.txt"), "hello\n").unwrap();
        std::fs::write(root.join(".gitignore"), ".refact/\n").unwrap();
        run_git(root, &["add", "file.txt", ".gitignore"]);
        run_git(root, &["commit", "-m", "initial"]);
    }

    fn commit_file(root: &Path, file: &str, content: &str, message: &str) {
        std::fs::write(root.join(file), content).unwrap();
        run_git(root, &["add", file]);
        run_git(root, &["commit", "-m", message]);
    }

    fn branch_exists(root: &Path, branch: &str) -> bool {
        Command::new("git")
            .args(["rev-parse", "--verify", branch])
            .current_dir(root)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    fn create_legacy_agent_worktree(source: &Path, worktree: &Path, branch: &str) {
        run_git(source, &["branch", branch]);
        run_git(
            source,
            &["worktree", "add", worktree.to_str().unwrap(), branch],
        );
    }

    fn tool_text(result: &(bool, Vec<ContextEnum>)) -> String {
        let ContextEnum::ChatMessage(message) = &result.1[0] else {
            panic!("expected chat message")
        };
        let ChatContent::SimpleText(text) = &message.content else {
            panic!("expected simple text")
        };
        text.clone()
    }

    async fn set_workspace(gcx: Arc<GlobalContext>, root: &Path) {
        let root = root.canonicalize().unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root];
    }

    async fn planner_ccx(gcx: Arc<GlobalContext>) -> Arc<AMutex<AtCommandsContext>> {
        Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                AppState::from_gcx(gcx).await,
                4096,
                20,
                false,
                vec![],
                "planner-chat".to_string(),
                None,
                "model".to_string(),
                Some(ThreadTaskMeta {
                    task_id: "task-1".to_string(),
                    role: "planner".to_string(),
                    agent_id: None,
                    card_id: None,
                    planner_chat_id: None,
                }),
                None,
            )
            .await,
        ))
    }

    fn test_card_with_branch(worktree_id: Option<&str>, branch: &str, root: &Path) -> BoardCard {
        BoardCard {
            id: "T-1".to_string(),
            title: "Card T-1".to_string(),
            column: "done".to_string(),
            priority: "P1".to_string(),
            depends_on: vec![],
            instructions: String::new(),
            assignee: Some("agent-1".to_string()),
            agent_chat_id: Some("agent-chat-1".to_string()),
            status_updates: vec![],
            comments: vec![],
            final_report: Some("done".to_string()),
            final_report_structured: None,
            verifier_report: Some(VerifierReport {
                notes: Vec::new(),
                passed: true,
                concerns: Vec::new(),
                recommendation: "merge".into(),
                classification: crate::tasks::types::VerifierReportClassification::Passed,
                command_results: vec![crate::tasks::types::VerificationResult {
                    command: "fixture verification".into(),
                    exit_code: Some(0),
                    passed: true,
                    outcome: VerificationOutcome::Passed,
                    ..Default::default()
                }],
            }),
            created_at: chrono::Utc::now().to_rfc3339(),
            started_at: None,
            last_heartbeat_at: None,
            paused_at: None,
            paused_by: None,
            completed_at: Some(chrono::Utc::now().to_rfc3339()),
            agent_branch: Some(branch.to_string()),
            agent_worktree: Some(root.to_string_lossy().to_string()),
            agent_worktree_name: worktree_id.map(str::to_string),
            base_branch: None,
            base_commit: None,
            ab_variants: None,
            team_members: vec![],
            target_files: vec![],
            scope_guard_mode: Default::default(),
            attention: None,
            deliverable: None,
            verification: Vec::new(),
        }
    }

    fn test_card(worktree_id: &str, branch: &str, root: &Path) -> BoardCard {
        test_card_with_branch(Some(worktree_id), branch, root)
    }

    fn launch_failure_verifier_report() -> VerifierReport {
        VerifierReport {
            notes: Vec::new(),
            passed: false,
            concerns: vec![
                "Verifier failed to launch; human review recommended: no verifier model"
                    .to_string(),
            ],
            recommendation: "human-review".to_string(),
            ..Default::default()
        }
    }

    fn merged_response_with_warning() -> MergeWorktreeResponse {
        MergeWorktreeResponse {
            id: "wt-1".to_string(),
            status: "merged".to_string(),
            merged: true,
            strategy: "squash".to_string(),
            source_branch: "refact/task/T-1".to_string(),
            target_branch: "main".to_string(),
            committed_uncommitted: None,
            merge_commit: Some("abc123".to_string()),
            cleanup: None,
            conflict: None,
            dirty_baseline: None,
            affected_references: Vec::new(),
            affected_reference_count: 0,
            warnings: vec!["stash cleanup needs inspection".to_string()],
            source_kept: false,
        }
    }

    fn merged_response_with_cleanup_warning() -> MergeWorktreeResponse {
        MergeWorktreeResponse {
            cleanup: Some(crate::worktrees::types::WorktreeRemovalResult {
                worktree_deleted: true,
                branch_deleted: false,
                registry_deleted: true,
                stale_path: false,
                warnings: vec!["branch deletion failed".to_string()],
            }),
            warnings: Vec::new(),
            ..merged_response_with_warning()
        }
    }

    #[test]
    fn post_merge_message_only_labels_command_failures_as_regressions() {
        use crate::chat::post_merge_check::PostMergeCheckResult;
        use crate::tasks::types::VerificationOutcome;

        for (outcome, expected) in [
            (
                VerificationOutcome::Rejected,
                "could not be run as a command",
            ),
            (
                VerificationOutcome::PolicyDenied,
                "denied by command policy",
            ),
            (
                VerificationOutcome::InfrastructureFailed,
                "infrastructure failure",
            ),
            (VerificationOutcome::NoCommands, "no verification commands"),
            (VerificationOutcome::Unknown, "unknown result"),
        ] {
            let result = PostMergeCheckResult {
                checked: true,
                auto_reverted: false,
                command: Some("cargo test".to_string()),
                exit_code: None,
                output_tail: "details".to_string(),
                merge_commit: None,
                revert_commit: None,
                fix_card_id: None,
                skipped_reason: None,
                revert_skipped_reason: None,
                outcome,
            };
            let mut message = String::new();
            append_post_merge_check_message(&mut message, &result);
            assert!(message.contains(expected), "{message}");
            assert!(!message.contains("regression"), "{message}");
            assert!(!message.contains("Fix card"), "{message}");
        }
    }

    #[test]
    fn merge_agent_bool_args_accept_string_spellings_and_reject_garbage() {
        let args = HashMap::from([("force".to_string(), Value::String("false".to_string()))]);

        assert_eq!(parse_force(&args), Ok(false));

        let args = HashMap::from([("auto_revert".to_string(), Value::String("True".to_string()))]);

        assert_eq!(parse_auto_revert(&args), Ok(true));

        let args = HashMap::from([(
            "auto_revert".to_string(),
            Value::String("sometimes".to_string()),
        )]);

        assert!(parse_auto_revert(&args)
            .unwrap_err()
            .contains("must be a boolean"));
    }

    fn merged_with(card: &BoardCard) -> String {
        ensure_verifier_allows_merge(card, false)
            .expect("verifier state never blocks a merge")
            .expect("non-passing verification must produce a warning")
    }

    #[test]
    fn merge_agent_merges_failed_verifier_with_prominent_warning() {
        let mut card = test_card("wt", "agent", Path::new("/tmp/worktree"));
        card.verifier_report = Some(VerifierReport {
            notes: Vec::new(),
            passed: false,
            concerns: vec!["cargo test failed".to_string()],
            recommendation: "fix-needed".to_string(),
            ..Default::default()
        });

        let warning = merged_with(&card);

        assert!(warning.contains("Verification FAILED"), "{warning}");
        assert!(warning.contains("merged anyway"), "{warning}");
        assert!(warning.contains("cargo test failed"), "{warning}");
        assert_eq!(
            ensure_verifier_allows_merge(&card, true).unwrap(),
            Some(warning)
        );
    }

    #[test]
    fn merge_agent_merges_deterministic_failures_with_failed_warning() {
        use crate::tasks::types::{
            VerificationOutcome, VerificationResult, VerifierReportClassification,
        };

        let mut card = test_card("wt", "agent", Path::new("/tmp/worktree"));
        card.verifier_report = Some(VerifierReport {
            notes: Vec::new(),
            passed: true,
            classification: VerifierReportClassification::Passed,
            command_results: vec![VerificationResult {
                command: "cargo test".to_string(),
                passed: false,
                outcome: VerificationOutcome::CommandFailed,
                ..Default::default()
            }],
            ..Default::default()
        });
        assert!(merged_with(&card).contains("Verification FAILED"));

        card.verifier_report = Some(infrastructure_human_review_report(
            VerificationOutcome::CommandFailed,
        ));
        let warning = merged_with(&card);
        assert!(warning.contains("Verification FAILED"), "{warning}");
        assert!(warning.contains("Verification command failed: cargo test"));

        card.verifier_report.as_mut().unwrap().classification =
            VerifierReportClassification::VerificationFailed;
        assert!(merged_with(&card).contains("merged anyway"));
    }

    #[test]
    fn merge_agent_merges_unverified_reports_with_warning() {
        use crate::tasks::types::{
            VerificationOutcome, VerificationResult, VerifierReportClassification,
        };

        let mut card = test_card("wt", "agent", Path::new("/tmp/worktree"));
        card.verifier_report = Some(launch_failure_verifier_report());
        let warning = merged_with(&card);
        assert!(warning.contains("not verified"), "{warning}");
        assert!(warning.contains("Verifier failed to launch"), "{warning}");
        assert!(!warning.contains("Verification FAILED"), "{warning}");

        let mut mixed = launch_failure_verifier_report();
        mixed
            .concerns
            .push("Implementation does not satisfy the requested behavior".to_string());
        card.verifier_report = Some(mixed);
        assert!(merged_with(&card).contains("Verification FAILED"));

        card.verifier_report = Some(VerifierReport {
            notes: Vec::new(),
            passed: false,
            recommendation: "human-review".to_string(),
            classification: VerifierReportClassification::HumanReview,
            command_results: vec![VerificationResult {
                command: "cargo test".to_string(),
                passed: false,
                outcome: VerificationOutcome::InfrastructureFailed,
                ..Default::default()
            }],
            ..Default::default()
        });
        assert!(merged_with(&card).contains("human review"));

        card.verifier_report = Some(infrastructure_human_review_report(
            VerificationOutcome::InfrastructureFailed,
        ));
        assert!(merged_with(&card).contains("merged anyway"));

        card.verifier_report = Some(VerifierReport {
            notes: Vec::new(),
            passed: true,
            classification: VerifierReportClassification::Unknown,
            command_results: vec![VerificationResult {
                command: "cargo test".to_string(),
                passed: true,
                ..Default::default()
            }],
            ..Default::default()
        });
        assert!(merged_with(&card).contains("merged anyway"));
    }

    fn infrastructure_human_review_report(
        outcome: crate::tasks::types::VerificationOutcome,
    ) -> VerifierReport {
        use crate::tasks::types::{VerificationResult, VerifierReportClassification};

        VerifierReport {
            notes: Vec::new(),
            passed: false,
            concerns: vec!["Verification command failed: cargo test".to_string()],
            recommendation: "human-review".to_string(),
            classification: VerifierReportClassification::HumanReview,
            command_results: vec![VerificationResult {
                command: "cargo test".to_string(),
                passed: false,
                outcome,
                ..Default::default()
            }],
        }
    }

    #[test]
    fn merge_agent_missing_report_warns_and_consistent_pass_is_silent() {
        use crate::tasks::types::{
            VerificationOutcome, VerificationResult, VerifierReportClassification,
        };

        let mut card = test_card("wt", "agent", Path::new("/tmp/worktree"));
        card.verifier_report = None;
        assert!(merged_with(&card).contains("no verifier report"));
        card.verifier_report = Some(VerifierReport {
            notes: Vec::new(),
            passed: true,
            concerns: Vec::new(),
            recommendation: "merge".to_string(),
            classification: VerifierReportClassification::Passed,
            command_results: vec![VerificationResult {
                command: "cargo test".to_string(),
                passed: true,
                outcome: VerificationOutcome::Passed,
                ..Default::default()
            }],
        });

        assert!(ensure_verifier_allows_merge(&card, false)
            .unwrap()
            .is_none());
        assert!(ensure_verifier_allows_merge(&card, true).unwrap().is_none());
    }

    #[test]
    fn merge_agent_force_schema_is_deprecated() {
        let schema = ToolTaskMergeAgent::new().tool_description().input_schema;
        let description = schema["properties"]["force"]["description"]
            .as_str()
            .unwrap();
        assert!(description.contains("never blocked"), "{description}");
    }

    #[test]
    fn dirty_baseline_response_renders_merge_not_performed() {
        let response = MergeWorktreeResponse {
            status: "dirty_baseline".to_string(),
            merged: false,
            merge_commit: None,
            dirty_baseline: Some(crate::worktrees::types::DirtyBaseline {
                branch: Some("main".to_string()),
                files: vec!["notes.txt".to_string()],
                files_truncated: false,
                staged_count: 0,
                unstaged_count: 0,
                untracked_count: 1,
            }),
            warnings: vec!["Target workspace has 1 uncommitted file(s)".to_string()],
            ..merged_response_with_warning()
        };

        let message = merge_response_message("T-1", &response, None);

        assert!(message.contains("Merge Not Performed"), "{message}");
        assert!(message.contains("dirty_baseline"), "{message}");
        assert!(message.contains("notes.txt"), "{message}");
        assert!(message.contains("Next step"), "{message}");
        assert!(message.contains("1 uncommitted file"), "{message}");
        assert!(!message.contains("successfully merged"), "{message}");
        assert!(!message.contains("Agent Work Merged"), "{message}");

        let other = MergeWorktreeResponse {
            status: "blocked".to_string(),
            dirty_baseline: None,
            ..response
        };
        let message = merge_response_message("T-1", &other, None);
        assert!(message.contains("Merge Not Performed"), "{message}");
        assert!(message.contains("blocked"), "{message}");
        assert!(!message.contains("successfully merged"), "{message}");

        let merged = merge_response_message("T-1", &merged_response_with_warning(), None);
        assert!(merged.contains("successfully merged"), "{merged}");
    }

    #[test]
    fn merge_agent_auto_revert_schema_and_args_are_opt_in() {
        let schema = ToolTaskMergeAgent::new().tool_description().input_schema;
        assert!(schema["properties"].get("auto_revert").is_some());
        assert!(schema["properties"]
            .get("auto_revert_timeout_secs")
            .is_some());
        let empty = HashMap::new();
        assert!(!parse_auto_revert(&empty).unwrap());
        assert_eq!(
            parse_auto_revert_timeout(&empty).unwrap(),
            Duration::from_secs(
                crate::chat::post_merge_check::DEFAULT_POST_MERGE_CHECK_TIMEOUT_SECS
            )
        );
        let args = HashMap::from([
            ("auto_revert".to_string(), Value::Bool(true)),
            (
                "auto_revert_timeout_secs".to_string(),
                Value::Number(120.into()),
            ),
        ]);
        assert!(parse_auto_revert(&args).unwrap());
        assert_eq!(
            parse_auto_revert_timeout(&args).unwrap(),
            Duration::from_secs(120)
        );
    }

    #[tokio::test]
    async fn merge_agent_rejects_mismatched_task_id() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let ccx = planner_ccx(gcx).await;
        let args = HashMap::from([
            ("card_id".to_string(), json!("T-1")),
            ("task_id".to_string(), json!("task-2")),
        ]);

        let err = ToolTaskMergeAgent::new()
            .tool_execute(ccx, &"call".to_string(), &args)
            .await
            .unwrap_err();

        assert_eq!(
            err,
            "task_id override is not allowed from this planner chat"
        );
    }

    #[tokio::test]
    async fn merge_agent_merges_failing_verifier_report_with_warning() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_workspace(gcx.clone(), &source).await;
        let cache_dir = gcx.cache_dir.clone();
        let service = WorktreeService::new(cache_dir, source.canonicalize().unwrap()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/task/task-1/card/T-1/agent".to_string()),
                kind: Some("task_agent".to_string()),
                task_id: Some("task-1".to_string()),
                card_id: Some("T-1".to_string()),
                agent_id: Some("agent-1".to_string()),
                chat_id: Some("agent-chat-1".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let branch = created.worktree.meta.branch.clone().unwrap();
        let root = created.worktree.meta.root.clone();
        commit_file(
            &root,
            "file.txt",
            "merged after human review\n",
            "agent change",
        );
        let mut card = test_card(&created.worktree.meta.id, &branch, &root);
        card.verifier_report = Some(infrastructure_human_review_report(
            VerificationOutcome::CommandFailed,
        ));
        write_task(gcx.clone(), &source, card).await;

        let result = merge_registered_task_worktree(
            gcx.clone(),
            &source.canonicalize().unwrap(),
            "task-1",
            "T-1",
            "squash",
            "tool-call",
            Some("task merge".to_string()),
            false,
            Duration::from_secs(5),
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(source.join("file.txt")).unwrap(),
            "merged after human review\n"
        );
        let ContextEnum::ChatMessage(message) = &result.1[0] else {
            panic!("expected chat message")
        };
        let ChatContent::SimpleText(text) = &message.content else {
            panic!("expected simple text")
        };
        assert!(text.contains("Agent Work Merged"), "{text}");
        assert!(text.contains("Verification FAILED"), "{text}");
        assert!(text.contains("merged anyway"), "{text}");
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let card = board.get_card("T-1").unwrap();
        assert!(card.assignee.is_none());
        assert!(card.agent_chat_id.is_none());
    }

    #[tokio::test]
    async fn merge_agent_refuses_legacy_worktree_pointing_at_main_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        std::fs::write(source.join("main-dirty.txt"), "main dirty\n").unwrap();

        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_workspace(gcx.clone(), &source).await;
        write_task(
            gcx.clone(),
            &source,
            test_card_with_branch(None, "main", &source),
        )
        .await;
        let ccx = planner_ccx(gcx.clone()).await;
        let args = HashMap::from([("card_id".to_string(), json!("T-1"))]);

        let err = ToolTaskMergeAgent::new()
            .tool_execute(ccx, &"call".to_string(), &args)
            .await
            .unwrap_err();

        assert!(err.contains("main workspace"), "{err}");
        assert_eq!(
            run_git(&source, &["rev-list", "--count", "HEAD"]).trim(),
            "1"
        );
        assert_eq!(
            std::fs::read_to_string(source.join("main-dirty.txt")).unwrap(),
            "main dirty\n"
        );
    }

    #[tokio::test]
    async fn merge_agent_refuses_legacy_worktree_pointing_inside_main_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(source.join("src")).unwrap();
        init_repo(&source);
        std::fs::write(source.join("src/main-dirty.txt"), "main dirty\n").unwrap();

        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_workspace(gcx.clone(), &source).await;
        write_task(
            gcx.clone(),
            &source,
            test_card_with_branch(None, "main", &source.join("src")),
        )
        .await;
        let ccx = planner_ccx(gcx.clone()).await;
        let args = HashMap::from([("card_id".to_string(), json!("T-1"))]);

        let err = ToolTaskMergeAgent::new()
            .tool_execute(ccx, &"call".to_string(), &args)
            .await
            .unwrap_err();

        assert!(err.contains("not the git worktree root"), "{err}");
        assert_eq!(
            run_git(&source, &["rev-list", "--count", "HEAD"]).trim(),
            "1"
        );
        assert_eq!(
            std::fs::read_to_string(source.join("src/main-dirty.txt")).unwrap(),
            "main dirty\n"
        );
    }

    #[test]
    fn merge_agent_legacy_validation_rejects_detached_and_wrong_branch() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let agent_worktree = temp.path().join("agent-worktree-validate");
        let branch = "refact/task/task-1/card/T-1/agent-validate";
        create_legacy_agent_worktree(&source, &agent_worktree, branch);

        run_git(&agent_worktree, &["checkout", "-b", "other-agent-branch"]);
        let err = ensure_legacy_agent_worktree_checkout(
            &source,
            agent_worktree.to_str().unwrap(),
            branch,
        )
        .unwrap_err();
        assert!(err.contains("different branch"), "{err}");

        let head = run_git(&agent_worktree, &["rev-parse", "HEAD"]);
        run_git(&agent_worktree, &["checkout", "--detach", head.trim()]);
        let err = ensure_legacy_agent_worktree_checkout(
            &source,
            agent_worktree.to_str().unwrap(),
            branch,
        )
        .unwrap_err();
        assert!(err.contains("detached HEAD"), "{err}");
    }

    #[tokio::test]
    async fn merge_agent_with_commits_ahead_and_dirty_worktree_auto_commits_and_cleans_up() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let agent_worktree = temp.path().join("agent-worktree-committed-dirty");
        let branch = "refact/task/task-1/card/T-1/agent-committed-dirty";
        create_legacy_agent_worktree(&source, &agent_worktree, branch);
        run_git(
            &agent_worktree,
            &["commit", "--allow-empty", "-m", "agent empty change"],
        );
        std::fs::write(agent_worktree.join("dirty.txt"), "uncommitted\n").unwrap();

        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_workspace(gcx.clone(), &source).await;
        write_task(
            gcx.clone(),
            &source,
            test_card_with_branch(None, branch, &agent_worktree),
        )
        .await;
        let ccx = planner_ccx(gcx.clone()).await;
        let args = HashMap::from([("card_id".to_string(), json!("T-1"))]);

        let result = ToolTaskMergeAgent::new()
            .tool_execute(ccx, &"call".to_string(), &args)
            .await
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(source.join("dirty.txt")).unwrap(),
            "uncommitted\n"
        );
        assert!(!agent_worktree.exists());
        assert!(!branch_exists(&source, branch));
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let card = board.get_card("T-1").unwrap();
        assert!(card.agent_branch.is_none());
        assert!(card.agent_worktree.is_none());
        assert!(card.agent_worktree_name.is_none());
        assert!(tool_text(&result).contains("cleaned up"));
    }

    #[tokio::test]
    async fn merge_agent_legacy_dirty_target_is_stashed_and_restored() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let agent_worktree = temp.path().join("agent-worktree-main-dirty");
        let branch = "refact/task/task-1/card/T-1/agent-main-dirty";
        create_legacy_agent_worktree(&source, &agent_worktree, branch);
        commit_file(
            &agent_worktree,
            "file.txt",
            "agent change\n",
            "agent change",
        );
        std::fs::write(source.join("main-dirty.txt"), "main dirty\n").unwrap();

        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_workspace(gcx.clone(), &source).await;
        write_task(
            gcx.clone(),
            &source,
            test_card_with_branch(None, branch, &agent_worktree),
        )
        .await;
        let ccx = planner_ccx(gcx.clone()).await;
        let args = HashMap::from([("card_id".to_string(), json!("T-1"))]);

        let result = ToolTaskMergeAgent::new()
            .tool_execute(ccx, &"call".to_string(), &args)
            .await
            .unwrap();

        let text = tool_text(&result);
        assert!(text.contains("Agent Work Merged"), "{text}");
        assert!(
            text.contains("stashed before the merge and restored"),
            "{text}"
        );
        assert_eq!(
            std::fs::read_to_string(source.join("file.txt")).unwrap(),
            "agent change\n"
        );
        assert_eq!(
            std::fs::read_to_string(source.join("main-dirty.txt")).unwrap(),
            "main dirty\n"
        );
        assert!(run_git(&source, &["stash", "list"]).trim().is_empty());
    }

    #[tokio::test]
    async fn merge_agent_legacy_overlapping_dirty_target_is_honest_conflict() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let agent_worktree = temp.path().join("agent-worktree-overlap");
        let branch = "refact/task/task-1/card/T-1/agent-overlap";
        create_legacy_agent_worktree(&source, &agent_worktree, branch);
        commit_file(
            &agent_worktree,
            "file.txt",
            "agent change\n",
            "agent change",
        );
        std::fs::write(source.join("file.txt"), "user change\n").unwrap();
        let head_before = run_git(&source, &["rev-parse", "HEAD"]);

        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_workspace(gcx.clone(), &source).await;
        write_task(
            gcx.clone(),
            &source,
            test_card_with_branch(None, branch, &agent_worktree),
        )
        .await;
        let ccx = planner_ccx(gcx.clone()).await;
        let args = HashMap::from([("card_id".to_string(), json!("T-1"))]);

        let result = ToolTaskMergeAgent::new()
            .tool_execute(ccx, &"call".to_string(), &args)
            .await
            .unwrap();

        let text = tool_text(&result);
        assert!(text.contains("Merge Conflicts Detected"), "{text}");
        assert!(text.contains("merge was rolled back"), "{text}");
        assert!(!text.contains("successfully merged"), "{text}");
        assert_eq!(run_git(&source, &["rev-parse", "HEAD"]), head_before);
        assert_eq!(
            std::fs::read_to_string(source.join("file.txt")).unwrap(),
            "user change\n"
        );
        assert!(run_git(&source, &["stash", "list"]).trim().is_empty());
        assert!(agent_worktree.exists());
        assert!(branch_exists(&source, branch));
    }

    async fn registered_dirty_target_fixture(
        target_file: &str,
        target_content: &str,
    ) -> (
        tempfile::TempDir,
        Arc<GlobalContext>,
        std::path::PathBuf,
        std::path::PathBuf,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_workspace(gcx.clone(), &source).await;
        let service =
            WorktreeService::new(gcx.cache_dir.clone(), source.canonicalize().unwrap()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/task/task-1/card/T-1/agent-dirty-target".to_string()),
                kind: Some("task_agent".to_string()),
                task_id: Some("task-1".to_string()),
                card_id: Some("T-1".to_string()),
                agent_id: Some("agent-1".to_string()),
                chat_id: Some("agent-chat-1".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let branch = created.worktree.meta.branch.clone().unwrap();
        let root = created.worktree.meta.root.clone();
        commit_file(&root, "file.txt", "agent change\n", "agent change");
        write_task(
            gcx.clone(),
            &source,
            test_card(&created.worktree.meta.id, &branch, &root),
        )
        .await;
        std::fs::write(source.join(target_file), target_content).unwrap();
        (temp, gcx, source, root)
    }

    #[tokio::test]
    async fn merge_agent_registered_dirty_target_is_stashed_and_restored() {
        let (_temp, gcx, source, root) =
            registered_dirty_target_fixture("notes.txt", "user notes\n").await;

        let result = merge_registered_task_worktree(
            gcx,
            &source.canonicalize().unwrap(),
            "task-1",
            "T-1",
            "squash",
            "tool-call",
            Some("task merge".to_string()),
            false,
            Duration::from_secs(5),
        )
        .await
        .unwrap()
        .unwrap();

        let text = tool_text(&result);
        assert!(text.contains("Agent Work Merged"), "{text}");
        assert_eq!(
            std::fs::read_to_string(source.join("file.txt")).unwrap(),
            "agent change\n"
        );
        assert_eq!(
            std::fs::read_to_string(source.join("notes.txt")).unwrap(),
            "user notes\n"
        );
        assert!(run_git(&source, &["stash", "list"]).trim().is_empty());
        assert!(!root.exists());
    }

    #[tokio::test]
    async fn merge_agent_registered_overlapping_dirty_target_is_honest_conflict() {
        let (_temp, gcx, source, root) =
            registered_dirty_target_fixture("file.txt", "user change\n").await;
        let head_before = run_git(&source, &["rev-parse", "HEAD"]);

        let result = merge_registered_task_worktree(
            gcx.clone(),
            &source.canonicalize().unwrap(),
            "task-1",
            "T-1",
            "squash",
            "tool-call",
            Some("task merge".to_string()),
            false,
            Duration::from_secs(5),
        )
        .await
        .unwrap()
        .unwrap();

        let text = tool_text(&result);
        assert!(text.contains("Merge Conflicts Detected"), "{text}");
        assert!(!text.contains("successfully merged"), "{text}");
        assert_eq!(run_git(&source, &["rev-parse", "HEAD"]), head_before);
        assert_eq!(
            std::fs::read_to_string(source.join("file.txt")).unwrap(),
            "user change\n"
        );
        assert!(run_git(&source, &["stash", "list"]).trim().is_empty());
        assert!(root.exists());
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let card = board.get_card("T-1").unwrap();
        assert_eq!(card.column, "done");
        assert!(card.agent_worktree_name.is_some());
    }

    async fn write_task(gcx: Arc<GlobalContext>, source: &Path, card: BoardCard) {
        let task_dir = source.join(".refact").join("tasks").join("task-1");
        tokio::fs::create_dir_all(&task_dir).await.unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        let cards_done = if card.column == "done" { 1 } else { 0 };
        let cards_failed = if card.column == "failed" || card.column == "regressed" {
            1
        } else {
            0
        };
        let agents_active = if card.column == "doing" && card.assignee.is_some() {
            1
        } else {
            0
        };
        let meta = TaskMeta {
            schema_version: 1,
            id: "task-1".to_string(),
            name: "Task".to_string(),
            status: TaskStatus::Active,
            created_at: now.clone(),
            updated_at: now,
            cards_total: 1,
            cards_done,
            cards_failed,
            agents_active,
            base_branch: Some("main".to_string()),
            base_commit: None,
            default_agent_model: None,
            is_name_generated: false,
            last_agents_summary_at: None,
            planner_session_state: None,
        };
        let mut board = TaskBoard::default();
        board.cards.push(card);
        tokio::fs::write(
            task_dir.join("meta.yaml"),
            serde_yaml::to_string(&meta).unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::write(
            task_dir.join("board.yaml"),
            serde_yaml::to_string(&board).unwrap(),
        )
        .await
        .unwrap();
        set_workspace(gcx, source).await;
    }

    #[tokio::test]
    async fn worktree_merge_merge_agent_uses_service_and_clears_board_mirrors() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_workspace(gcx.clone(), &source).await;
        let cache_dir = gcx.cache_dir.clone();
        let service = WorktreeService::new(cache_dir, source.canonicalize().unwrap()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/task/task-1/card/T-1/agent".to_string()),
                kind: Some("task_agent".to_string()),
                task_id: Some("task-1".to_string()),
                card_id: Some("T-1".to_string()),
                agent_id: Some("agent-1".to_string()),
                chat_id: Some("agent-chat-1".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let branch = created.worktree.meta.branch.clone().unwrap();
        let root = created.worktree.meta.root.clone();
        commit_file(&root, "file.txt", "merged by task tool\n", "agent change");
        write_task(
            gcx.clone(),
            &source,
            test_card(&created.worktree.meta.id, &branch, &root),
        )
        .await;

        let result = merge_registered_task_worktree(
            gcx.clone(),
            &source.canonicalize().unwrap(),
            "task-1",
            "T-1",
            "squash",
            "tool-call",
            Some("task merge".to_string()),
            false,
            Duration::from_secs(5),
        )
        .await
        .unwrap();

        assert!(result.is_some());
        assert_eq!(
            std::fs::read_to_string(source.join("file.txt")).unwrap(),
            "merged by task tool\n"
        );
        assert!(!root.exists());
        assert!(service
            .get_worktree(&created.worktree.meta.id)
            .await
            .is_err());
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let card = board.get_card("T-1").unwrap();
        assert!(card.agent_branch.is_none());
        assert!(card.agent_worktree.is_none());
        assert!(card.agent_worktree_name.is_none());
        assert!(card.target_files.contains(&"file.txt".to_string()));
    }

    #[tokio::test]
    async fn merge_agent_registered_path_auto_commits_dirty_worktree_and_cleans_up() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_workspace(gcx.clone(), &source).await;
        let cache_dir = gcx.cache_dir.clone();
        let service = WorktreeService::new(cache_dir, source.canonicalize().unwrap()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/task/task-1/card/T-1/agent-dirty".to_string()),
                kind: Some("task_agent".to_string()),
                task_id: Some("task-1".to_string()),
                card_id: Some("T-1".to_string()),
                agent_id: Some("agent-1".to_string()),
                chat_id: Some("agent-chat-1".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let branch = created.worktree.meta.branch.clone().unwrap();
        let root = created.worktree.meta.root.clone();
        commit_file(&root, "file.txt", "registered committed\n", "agent change");
        std::fs::write(root.join("dirty.txt"), "registered dirty\n").unwrap();
        write_task(
            gcx.clone(),
            &source,
            test_card(&created.worktree.meta.id, &branch, &root),
        )
        .await;

        let result = merge_registered_task_worktree(
            gcx.clone(),
            &source.canonicalize().unwrap(),
            "task-1",
            "T-1",
            "squash",
            "tool-call",
            Some("task merge".to_string()),
            false,
            Duration::from_secs(5),
        )
        .await
        .unwrap();

        assert!(result.is_some());
        assert_eq!(
            std::fs::read_to_string(source.join("file.txt")).unwrap(),
            "registered committed\n"
        );
        assert_eq!(
            std::fs::read_to_string(source.join("dirty.txt")).unwrap(),
            "registered dirty\n"
        );
        assert!(!root.exists());
        assert!(service
            .get_worktree(&created.worktree.meta.id)
            .await
            .is_err());
        assert!(!branch_exists(&source, &branch));
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let card = board.get_card("T-1").unwrap();
        assert!(card.agent_branch.is_none());
        assert!(card.agent_worktree.is_none());
        assert!(card.agent_worktree_name.is_none());
        assert!(card.target_files.contains(&"dirty.txt".to_string()));
    }

    #[tokio::test]
    async fn merge_agent_registered_path_validates_checkout_before_diff_generation() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_workspace(gcx.clone(), &source).await;
        let cache_dir = gcx.cache_dir.clone();
        let service = WorktreeService::new(cache_dir, source.canonicalize().unwrap()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/task/task-1/card/T-1/agent-wrong-checkout".to_string()),
                kind: Some("task_agent".to_string()),
                task_id: Some("task-1".to_string()),
                card_id: Some("T-1".to_string()),
                agent_id: Some("agent-1".to_string()),
                chat_id: Some("agent-chat-1".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let branch = created.worktree.meta.branch.clone().unwrap();
        let root = created.worktree.meta.root.clone();
        run_git(&root, &["checkout", "-b", "manual-task-branch"]);
        std::fs::write(root.join("wrong-branch.txt"), "do not diff\n").unwrap();
        write_task(
            gcx.clone(),
            &source,
            test_card(&created.worktree.meta.id, &branch, &root),
        )
        .await;

        let err = merge_registered_task_worktree(
            gcx.clone(),
            &source.canonicalize().unwrap(),
            "task-1",
            "T-1",
            "squash",
            "tool-call",
            None,
            false,
            Duration::from_secs(5),
        )
        .await
        .unwrap_err();

        assert!(err.contains("different branch"), "{err}");
        assert_eq!(
            run_git(&root, &["branch", "--show-current"]).trim(),
            "manual-task-branch"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("wrong-branch.txt")).unwrap(),
            "do not diff\n"
        );
    }

    #[tokio::test]
    async fn worktree_merge_registered_error_does_not_fall_back_to_legacy_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let agent_worktree = temp
            .path()
            .join("cache")
            .join("worktrees")
            .join("legacy-agent");
        std::fs::create_dir_all(agent_worktree.parent().unwrap()).unwrap();
        let branch = "refact/task/task-1/card/T-1/legacy-fallback";
        create_legacy_agent_worktree(&source, &agent_worktree, branch);
        commit_file(
            &agent_worktree,
            "file.txt",
            "legacy fallback\n",
            "legacy change",
        );

        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            temp.path().join("cache"),
            temp.path().join("config"),
        )
        .await;
        set_workspace(gcx.clone(), &source).await;
        write_task(
            gcx.clone(),
            &source,
            test_card_with_branch(Some("missing-registry-id"), branch, &agent_worktree),
        )
        .await;
        let ccx = planner_ccx(gcx.clone()).await;
        let args = HashMap::from([("card_id".to_string(), json!("T-1"))]);

        let head_before = run_git(&source, &["rev-parse", "HEAD"]);
        let err = ToolTaskMergeAgent::new()
            .tool_execute(ccx, &"call".to_string(), &args)
            .await
            .unwrap_err();

        assert!(!err.is_empty());
        assert_eq!(run_git(&source, &["rev-parse", "HEAD"]), head_before);
        assert_eq!(
            std::fs::read_to_string(source.join("file.txt")).unwrap(),
            "hello\n"
        );
        assert!(agent_worktree.exists());
        assert!(branch_exists(&source, branch));
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let card = board.get_card("T-1").unwrap();
        assert_eq!(card.agent_branch.as_deref(), Some(branch));
        assert!(card.agent_worktree.is_some());
        assert_eq!(
            card.agent_worktree_name.as_deref(),
            Some("missing-registry-id")
        );
    }

    #[test]
    fn merge_agent_warns_instead_of_refusing_unfinished_cards() {
        let mut card = test_card("wt", "agent", Path::new("/tmp/worktree"));
        for column in ["doing", "failed"] {
            card.column = column.to_string();
            let warning = ensure_card_done_allows_merge(&card).unwrap();
            assert!(warning.contains("not 'done'"), "{warning}");
            assert!(warning.contains(column), "{warning}");
            assert!(!warning.contains("force=true"), "{warning}");
        }
        card.column = "done".to_string();
        card.final_report = None;
        let warning = ensure_card_done_allows_merge(&card).unwrap();
        assert!(warning.contains("no final report"), "{warning}");
    }

    #[test]
    fn merge_agent_allows_done_card_with_final_report() {
        let card = test_card("wt", "agent", Path::new("/tmp/worktree"));

        assert!(ensure_card_done_allows_merge(&card).is_none());
    }

    #[test]
    fn sequential_applied_merges_of_one_card_each_record_their_own_marker() {
        let mut card = test_card("wt", "agent", Path::new("/tmp/worktree"));
        card.column = "doing".to_string();
        card.assignee = None;
        card.agent_chat_id = None;
        let since = chrono::Utc::now() - chrono::Duration::seconds(5);
        let first = AppliedMerge {
            since,
            commit: Some("1111111111111111111111111111111111111111".to_string()),
        };
        let second = AppliedMerge {
            since,
            commit: Some("2222222222222222222222222222222222222222".to_string()),
        };

        assert!(push_merge_marker_once(&mut card, &first, "squash", "main"));
        assert!(!push_merge_marker_once(&mut card, &first, "squash", "main"));
        assert!(push_merge_marker_once(&mut card, &second, "squash", "main"));
        assert!(!push_merge_marker_once(
            &mut card, &second, "squash", "main"
        ));
        assert!(finalize_force_merge_card_if_current(
            &mut card,
            None,
            None,
            Some(&second)
        ));

        let markers = card
            .status_updates
            .iter()
            .filter(|update| update.message.starts_with(MERGE_AGENT_STATUS_PREFIX))
            .collect::<Vec<_>>();
        assert_eq!(markers.len(), 2, "{markers:?}");
        assert_eq!(
            markers[0].message,
            "Merged by planner via merge_agent (squash) into main at 1111111111111111111111111111111111111111."
        );
        assert_eq!(
            markers[1].message,
            "Merged by planner via merge_agent (squash) into main at 2222222222222222222222222222222222222222."
        );
        for marker in markers {
            assert!(chrono::DateTime::parse_from_rfc3339(&marker.timestamp).unwrap() >= since);
        }
        assert_eq!(card.column, "done");
    }

    #[test]
    fn merge_agent_force_finalize_preserves_newer_ownership() {
        let mut card = test_card("wt", "agent", Path::new("/tmp/worktree"));
        card.column = "doing".to_string();
        card.completed_at = None;
        card.assignee = Some("agent-2".to_string());
        card.agent_chat_id = Some("agent-chat-2".to_string());

        let changed = finalize_force_merge_card_if_current(
            &mut card,
            Some("agent-1"),
            Some("agent-chat-1"),
            Some(&AppliedMerge {
                since: chrono::Utc::now(),
                commit: None,
            }),
        );

        assert!(!changed);
        assert_eq!(card.assignee.as_deref(), Some("agent-2"));
        assert_eq!(card.agent_chat_id.as_deref(), Some("agent-chat-2"));
        assert_eq!(card.column, "doing");
        assert!(card.completed_at.is_none());
        assert!(card.status_updates.is_empty());
    }

    #[tokio::test]
    async fn registered_force_merge_auto_revert_clears_stale_refs_without_finishing_card() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut card = test_card("wt", "agent", Path::new("/tmp/worktree"));
        card.column = "doing".to_string();
        card.completed_at = None;
        write_task(gcx.clone(), &source, card).await;
        let mut response = merged_response_with_cleanup_warning();
        response.cleanup.as_mut().unwrap().branch_deleted = true;

        clear_board_mirrors_after_registered_merge(
            gcx.clone(),
            "task-1",
            "T-1",
            &response,
            true,
            false,
            true,
            None,
            None,
            Some("agent-1".to_string()),
            Some("agent-chat-1".to_string()),
            chrono::Utc::now(),
        )
        .await
        .unwrap();

        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let card = board.get_card("T-1").unwrap();
        assert_eq!(card.column, "doing");
        assert!(card.completed_at.is_none());
        assert!(card.assignee.is_none());
        assert!(card.agent_chat_id.is_none());
        assert!(card.agent_branch.is_none());
        assert!(card.agent_worktree.is_none());
        assert!(card.agent_worktree_name.is_none());
        assert!(card.status_updates.iter().any(|update| update
            .message
            .contains("Unfinished or unverified merge was auto-reverted")));
        assert!(!card
            .status_updates
            .iter()
            .any(|update| update.message.contains("marked done for manual review")));
    }

    #[tokio::test]
    async fn registered_force_merge_retains_typed_failure_without_finishing_card() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut card = test_card("wt", "agent", Path::new("/tmp/worktree"));
        card.column = "doing".to_string();
        card.completed_at = None;
        write_task(gcx.clone(), &source, card).await;
        let mut response = merged_response_with_cleanup_warning();
        response.cleanup.as_mut().unwrap().branch_deleted = true;

        clear_board_mirrors_after_registered_merge(
            gcx.clone(),
            "task-1",
            "T-1",
            &response,
            true,
            false,
            false,
            Some("Unfinished or unverified merge remains applied, but post-merge verification did not pass: verification was denied by policy. Stale agent ownership was cleared; the merged work is retained and the card was not marked done.".to_string()),
            None,
            Some("agent-1".to_string()),
            Some("agent-chat-1".to_string()),
            chrono::Utc::now(),
        )
        .await
        .unwrap();

        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let card = board.get_card("T-1").unwrap();
        assert_eq!(card.column, "doing");
        assert!(card.completed_at.is_none());
        assert!(card.assignee.is_none());
        assert!(card.agent_chat_id.is_none());
        assert!(card.agent_branch.is_none());
        assert!(card.agent_worktree.is_none());
        assert!(card.agent_worktree_name.is_none());
        assert!(card
            .status_updates
            .iter()
            .any(|update| update.message.contains("merged work is retained")));
    }

    #[test]
    fn force_merge_finalization_requires_typed_pass_or_absent_check() {
        use crate::chat::post_merge_check::PostMergeCheckResult;
        use crate::tasks::types::VerificationOutcome;

        let result = |checked, outcome| PostMergeCheckResult {
            checked,
            outcome,
            auto_reverted: false,
            command: Some("cargo test".to_string()),
            exit_code: None,
            output_tail: String::new(),
            merge_commit: Some("abc".to_string()),
            revert_commit: None,
            fix_card_id: None,
            skipped_reason: None,
            revert_skipped_reason: None,
        };

        assert!(force_merge_post_check_passed_or_absent(&Ok(None)));
        assert!(force_merge_post_check_passed_or_absent(&Ok(Some(result(
            true,
            VerificationOutcome::Passed,
        )))));
        for outcome in [
            VerificationOutcome::CommandFailed,
            VerificationOutcome::Rejected,
            VerificationOutcome::PolicyDenied,
            VerificationOutcome::InfrastructureFailed,
            VerificationOutcome::NoCommands,
        ] {
            let checked_failure = Ok(Some(result(true, outcome)));
            assert!(!force_merge_post_check_passed_or_absent(&checked_failure));
            let audit = retained_post_merge_verification_failure(&checked_failure).unwrap();
            assert!(audit.contains("merged work is retained"), "{audit}");
        }
        assert!(!force_merge_post_check_passed_or_absent(&Ok(Some(result(
            false,
            VerificationOutcome::Passed,
        )))));
        assert!(!force_merge_post_check_passed_or_absent(&Err(
            "verification unavailable".to_string()
        )));
    }

    #[tokio::test]
    async fn registered_post_merge_check_error_still_reconciles_cleanup_without_finalizing() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut card = test_card("wt", "agent", Path::new("/tmp/worktree"));
        card.column = "doing".to_string();
        card.completed_at = None;
        write_task(gcx.clone(), &source, card).await;
        let mut response = merged_response_with_cleanup_warning();
        response.cleanup.as_mut().unwrap().branch_deleted = true;

        // finalize_force_merge=false models an errored verification result: cleanup has
        // already happened in git, so its mirrors must be reconciled before returning it.
        clear_board_mirrors_after_registered_merge(
            gcx.clone(),
            "task-1",
            "T-1",
            &response,
            true,
            false,
            false,
            None,
            Some("verification service unavailable".to_string()),
            Some("agent-1".to_string()),
            Some("agent-chat-1".to_string()),
            chrono::Utc::now(),
        )
        .await
        .unwrap();

        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let card = board.get_card("T-1").unwrap();
        assert_eq!(card.column, "doing");
        assert!(card.assignee.is_none());
        assert!(card.agent_chat_id.is_none());
        assert!(card.agent_branch.is_none());
        assert!(card.agent_worktree.is_none());
        assert!(card.agent_worktree_name.is_none());
        assert!(card.status_updates.iter().any(|update| update
            .message
            .contains("post-merge verification failed to run")));
    }

    #[tokio::test]
    async fn registered_non_force_cleanup_preserves_rebound_card_pointers() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut card = test_card("wt-new", "agent-new", Path::new("/tmp/worktree-new"));
        card.assignee = Some("agent-2".to_string());
        card.agent_chat_id = Some("agent-chat-2".to_string());
        write_task(gcx.clone(), &source, card).await;
        let mut response = merged_response_with_cleanup_warning();
        response.cleanup.as_mut().unwrap().branch_deleted = true;

        clear_board_mirrors_after_registered_merge(
            gcx.clone(),
            "task-1",
            "T-1",
            &response,
            false,
            false,
            false,
            None,
            None,
            Some("agent-1".to_string()),
            Some("agent-chat-1".to_string()),
            chrono::Utc::now(),
        )
        .await
        .unwrap();

        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let card = board.get_card("T-1").unwrap();
        assert_eq!(card.assignee.as_deref(), Some("agent-2"));
        assert_eq!(card.agent_chat_id.as_deref(), Some("agent-chat-2"));
        assert_eq!(card.agent_branch.as_deref(), Some("agent-new"));
        assert_eq!(card.agent_worktree.as_deref(), Some("/tmp/worktree-new"));
        assert_eq!(card.agent_worktree_name.as_deref(), Some("wt-new"));
    }

    #[tokio::test]
    async fn legacy_squash_nothing_to_commit_does_not_verify_or_revert_existing_head() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let agent_worktree = temp.path().join("agent-worktree-squash-noop");
        let branch = "refact/task/task-1/card/T-1/squash-noop";
        create_legacy_agent_worktree(&source, &agent_worktree, branch);
        commit_file(
            &agent_worktree,
            "file.txt",
            "temporary\n",
            "temporary change",
        );
        commit_file(
            &agent_worktree,
            "file.txt",
            "hello\n",
            "restore base content",
        );

        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_workspace(gcx.clone(), &source).await;
        write_task(
            gcx.clone(),
            &source,
            test_card_with_branch(None, branch, &agent_worktree),
        )
        .await;
        let head_before = run_git(&source, &["rev-parse", "HEAD"]);
        let ccx = planner_ccx(gcx.clone()).await;
        let args = HashMap::from([
            ("card_id".to_string(), json!("T-1")),
            ("strategy".to_string(), json!("squash")),
            ("auto_revert".to_string(), json!(true)),
        ]);

        let result = ToolTaskMergeAgent::new()
            .tool_execute(ccx, &"call".to_string(), &args)
            .await
            .unwrap();

        assert_eq!(run_git(&source, &["rev-parse", "HEAD"]), head_before);
        assert_eq!(
            std::fs::read_to_string(source.join("file.txt")).unwrap(),
            "hello\n"
        );
        assert!(tool_text(&result).contains("Nothing to Merge"));
        assert!(!tool_text(&result).contains("Post-merge check"));
        assert!(!agent_worktree.exists());
        assert!(!branch_exists(&source, branch));
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let card = board.get_card("T-1").unwrap();
        assert!(card.agent_branch.is_none());
        assert!(card.agent_worktree.is_none());
        assert!(card.agent_worktree_name.is_none());
    }

    #[tokio::test]
    async fn merge_agent_merges_doing_card_with_warning() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_workspace(gcx.clone(), &source).await;
        let cache_dir = gcx.cache_dir.clone();
        let service = WorktreeService::new(cache_dir, source.canonicalize().unwrap()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/task/task-1/card/T-1/agent-force-doing".to_string()),
                kind: Some("task_agent".to_string()),
                task_id: Some("task-1".to_string()),
                card_id: Some("T-1".to_string()),
                agent_id: Some("agent-1".to_string()),
                chat_id: Some("agent-chat-1".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let branch = created.worktree.meta.branch.clone().unwrap();
        let root = created.worktree.meta.root.clone();
        commit_file(&root, "file.txt", "force doing\n", "agent change");
        let mut card = test_card(&created.worktree.meta.id, &branch, &root);
        card.column = "doing".to_string();
        card.final_report = None;
        write_task(gcx.clone(), &source, card).await;

        let result = merge_registered_task_worktree(
            gcx.clone(),
            &source.canonicalize().unwrap(),
            "task-1",
            "T-1",
            "squash",
            "tool-call",
            Some("force merge doing".to_string()),
            false,
            Duration::from_secs(5),
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(source.join("file.txt")).unwrap(),
            "force doing\n"
        );
        let text = tool_text(&result);
        assert!(text.contains("Merged card T-1 although"), "{text}");

        assert!(!root.exists());
        assert!(!branch_exists(&source, &branch));
        assert!(service
            .get_worktree(&created.worktree.meta.id)
            .await
            .is_err());

        let board = storage::load_board(gcx.clone(), "task-1").await.unwrap();
        let card = board.get_card("T-1").unwrap();
        assert_eq!(card.column, "done");
        assert!(card.completed_at.is_some());
        assert!(card.assignee.is_none());
        assert!(card.agent_chat_id.is_none());
        assert!(card.agent_branch.is_none());
        assert!(card.agent_worktree.is_none());
        assert!(card.agent_worktree_name.is_none());
        assert!(card.status_updates.iter().any(|update| update
            .message
            .contains("without the normal agent_finish path")));
        assert_eq!(
            card.status_updates
                .iter()
                .filter(|update| update.message.starts_with(MERGE_AGENT_STATUS_PREFIX))
                .count(),
            1
        );
        let ready = board.get_ready_cards();
        assert!(ready.in_progress.is_empty());
        assert_eq!(ready.completed, vec!["T-1".to_string()]);
        let meta = storage::load_task_meta(gcx, "task-1").await.unwrap();
        assert_eq!(meta.cards_done, 1);
        assert_eq!(meta.agents_active, 0);
    }
}
