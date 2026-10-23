use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use tokio::process::Command;

use crate::chat::evidence::{
    EvidenceContext, EvidenceDefaults, EvidenceRunner, EvidenceService, EvidenceVerdict,
};
use crate::chat::evidence_cache::{evidence_cache, revalidate_merged_tree, MergedTreeRevalidation};
use crate::chat::verify_cmd::VerifyCommandPolicy;
use crate::chat::verifier::ExpectedCardState;
use crate::global_context::GlobalContext;
use crate::tasks::storage;
use crate::tasks::types::{BoardCard, BoardColumn, StatusUpdate, VerificationOutcome};

pub const DEFAULT_POST_MERGE_CHECK_TIMEOUT_SECS: u64 = 300;
const MAX_OUTPUT_TAIL_CHARS: usize = 6000;
const STALE_POST_MERGE_WRITE: &str = "stale post-merge write";

#[derive(Clone, Debug)]
pub struct PostMergeCheckRequest {
    pub task_id: String,
    pub card_id: String,
    pub workspace_root: PathBuf,
    pub enabled: bool,
    pub timeout: Duration,
    pub expected_merge_commit: String,
    pub expected_card_state: Option<ExpectedCardState>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PostMergeCheckResult {
    pub checked: bool,
    pub auto_reverted: bool,
    pub command: Option<String>,
    pub exit_code: Option<i32>,
    pub output_tail: String,
    pub merge_commit: Option<String>,
    pub revert_commit: Option<String>,
    pub fix_card_id: Option<String>,
    pub skipped_reason: Option<String>,
    pub revert_skipped_reason: Option<String>,
    pub outcome: VerificationOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PostMergeCommand {
    Verify {
        cwd: Option<PathBuf>,
        env: Vec<(String, String)>,
        argv: Vec<String>,
    },
    Git(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PostMergeCommandOutput {
    pub exit_code: Option<i32>,
    pub success: bool,
    pub output: String,
    pub outcome: VerificationOutcome,
}

#[async_trait]
pub trait PostMergeCommandRunner: Send {
    async fn run(
        &mut self,
        workspace_root: &Path,
        command: PostMergeCommand,
        timeout: Duration,
    ) -> PostMergeCommandOutput;
}

struct SystemPostMergeCommandRunner {
    gcx: Arc<GlobalContext>,
}

#[async_trait]
impl PostMergeCommandRunner for SystemPostMergeCommandRunner {
    async fn run(
        &mut self,
        workspace_root: &Path,
        command: PostMergeCommand,
        timeout: Duration,
    ) -> PostMergeCommandOutput {
        match command {
            PostMergeCommand::Verify { cwd, env, argv } => {
                let label = argv.join(" ");
                if argv.is_empty() {
                    return PostMergeCommandOutput {
                        exit_code: None,
                        success: false,
                        output: "empty verification command".to_string(),
                        outcome: VerificationOutcome::Rejected,
                    };
                }
                let result = crate::chat::verifier::run_verification_argv_impl(
                    self.gcx.clone(),
                    workspace_root,
                    &label,
                    cwd,
                    env,
                    argv,
                    timeout,
                    Duration::from_secs(10),
                )
                .await;
                PostMergeCommandOutput {
                    exit_code: result.exit_code,
                    success: result.passed,
                    output: result.output_tail,
                    outcome: result.outcome,
                }
            }
            PostMergeCommand::Git(args) => {
                let label = format!("git {}", args.join(" "));
                let mut cmd = Command::new("git");
                cmd.args(&args).current_dir(workspace_root);
                run_command(cmd, &label, timeout).await
            }
        }
    }
}

struct PostMergeEvidenceRunner<'a, R> {
    inner: &'a mut R,
    timeout: Duration,
}

#[async_trait]
impl<R: PostMergeCommandRunner> EvidenceRunner for PostMergeEvidenceRunner<'_, R> {
    async fn run(
        &mut self,
        worktree: &Path,
        spec: &crate::chat::evidence::VerificationSpec,
    ) -> crate::tasks::types::VerificationResult {
        let output = self
            .inner
            .run(
                worktree,
                PostMergeCommand::Verify {
                    cwd: spec.cwd.clone(),
                    env: spec.env.clone(),
                    argv: spec.argv.clone(),
                },
                self.timeout,
            )
            .await;
        crate::tasks::types::VerificationResult {
            command: spec.command.clone(),
            exit_code: output.exit_code,
            passed: output.success,
            output_tail: output.output,
            outcome: output.outcome,
            from_cache: false,
            executed_at: None,
        }
    }
}

async fn run_command(
    mut command: Command,
    command_label: &str,
    timeout: Duration,
) -> PostMergeCommandOutput {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.kill_on_drop(true);
    #[cfg(unix)]
    {
        command.process_group(0);
    }
    let child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return PostMergeCommandOutput {
                exit_code: None,
                success: false,
                output: format!("failed to execute command '{}': {}", command_label, error),
                outcome: VerificationOutcome::InfrastructureFailed,
            };
        }
    };
    let child_id = child.id();
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(output)) => {
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            PostMergeCommandOutput {
                exit_code: output.status.code(),
                success: output.status.success(),
                output: text,
                outcome: if output.status.success() {
                    VerificationOutcome::Passed
                } else {
                    VerificationOutcome::CommandFailed
                },
            }
        }
        Ok(Err(error)) => PostMergeCommandOutput {
            exit_code: None,
            success: false,
            output: format!("failed to execute command '{}': {}", command_label, error),
            outcome: VerificationOutcome::InfrastructureFailed,
        },
        Err(_) => {
            #[cfg(unix)]
            if let Some(child_id) = child_id {
                unsafe {
                    libc::kill(-(child_id as i32), libc::SIGKILL);
                }
            }
            PostMergeCommandOutput {
                exit_code: None,
                success: false,
                output: format!(
                    "command '{}' timed out after {} seconds",
                    command_label,
                    timeout.as_secs()
                ),
                outcome: VerificationOutcome::InfrastructureFailed,
            }
        }
    }
}

pub async fn post_merge_check(
    gcx: Arc<GlobalContext>,
    request: PostMergeCheckRequest,
) -> Result<PostMergeCheckResult, String> {
    post_merge_check_with_revert_policy(gcx, request, true).await
}

pub async fn post_merge_check_without_revert(
    gcx: Arc<GlobalContext>,
    request: PostMergeCheckRequest,
) -> Result<PostMergeCheckResult, String> {
    post_merge_check_with_revert_policy(gcx, request, false).await
}

async fn post_merge_check_with_revert_policy(
    gcx: Arc<GlobalContext>,
    request: PostMergeCheckRequest,
    allow_revert: bool,
) -> Result<PostMergeCheckResult, String> {
    let mut runner = SystemPostMergeCommandRunner { gcx: gcx.clone() };
    post_merge_check_with_runner_and_revert_policy(gcx, request, &mut runner, allow_revert).await
}

pub async fn post_merge_check_with_runner<R: PostMergeCommandRunner>(
    gcx: Arc<GlobalContext>,
    request: PostMergeCheckRequest,
    runner: &mut R,
) -> Result<PostMergeCheckResult, String> {
    post_merge_check_with_runner_and_revert_policy(gcx, request, runner, true).await
}

async fn post_merge_check_with_runner_and_revert_policy<R: PostMergeCommandRunner>(
    gcx: Arc<GlobalContext>,
    request: PostMergeCheckRequest,
    runner: &mut R,
    allow_revert: bool,
) -> Result<PostMergeCheckResult, String> {
    if !request.enabled {
        return Ok(skipped_result("auto_revert disabled"));
    }

    let board = storage::load_board(gcx.clone(), &request.task_id).await?;
    let Some(card) = board.get_card(&request.card_id) else {
        if request.expected_card_state.is_some() {
            return Ok(skipped_result("stale card state"));
        }
        return Err(format!("Card {} not found", request.card_id));
    };
    if let Some(expected) = request.expected_card_state.as_ref() {
        if !expected.matches_board_card(board.rev, card) {
            return Ok(skipped_result("stale card state"));
        }
    }
    let card = card.clone();
    let assignment =
        match crate::tasks::assignment::load_assignment(gcx.clone(), &request.task_id, &card.id)
            .await
        {
            Ok(assignment) => assignment,
            Err(error) => {
                return Ok(failed_without_revert(
                    None,
                    VerificationOutcome::InfrastructureFailed,
                    format!("cannot load the dispatched assignment: {error}"),
                ));
            }
        };
    let defaults = EvidenceDefaults::from_settings();
    let specs =
        crate::tasks::assignment::specs_for_validation(assignment.as_ref(), &card, &defaults);
    if specs.is_empty() {
        return Ok(failed_without_revert(
            None,
            VerificationOutcome::NoCommands,
            "no verification command found".to_string(),
        ));
    }
    let expected_merge_commit = request.expected_merge_commit.trim();
    let service = EvidenceService::new(
        VerifyCommandPolicy::load(gcx.clone()).await,
        EvidenceContext {
            worktree: request.workspace_root.clone(),
            commit: (!expected_merge_commit.is_empty()).then(|| expected_merge_commit.to_string()),
            scope: card.id.clone(),
        },
    );
    let mut bounded_specs = specs;
    for spec in &mut bounded_specs {
        spec.timeout_secs = request.timeout.as_secs().max(1);
        spec.output_cap_bytes = MAX_OUTPUT_TAIL_CHARS;
    }
    let mut evidence_runner = PostMergeEvidenceRunner {
        inner: runner,
        timeout: request.timeout,
    };
    let revalidation = revalidate_merged_tree(
        &service,
        bounded_specs,
        evidence_cache(),
        &mut evidence_runner,
    )
    .await;
    let record = match revalidation {
        MergedTreeRevalidation::Refused(record) => {
            let reason = record
                .reason
                .clone()
                .unwrap_or_else(|| record.output_tail.clone());
            return Ok(failed_without_revert(
                Some(record.command),
                record.outcome,
                reason,
            ));
        }
        MergedTreeRevalidation::Ran(records) => records
            .into_iter()
            .last()
            .ok_or_else(|| "revalidation returned no records".to_string())?,
        MergedTreeRevalidation::NoContract => {
            return Ok(failed_without_revert(
                None,
                VerificationOutcome::NoCommands,
                "no verification command found".to_string(),
            ));
        }
    };
    let command = record.command.clone();
    let exit_code = record.exit_code;
    let verification_success = record.verdict == EvidenceVerdict::Passed;
    let output_tail = tail_chars(&record.output_tail, MAX_OUTPUT_TAIL_CHARS);
    let verification_outcome = match record.verdict {
        EvidenceVerdict::Passed => VerificationOutcome::Passed,
        EvidenceVerdict::Failed => VerificationOutcome::CommandFailed,
        EvidenceVerdict::Blocked => record.outcome,
        EvidenceVerdict::Unavailable | EvidenceVerdict::Superseded => {
            VerificationOutcome::InfrastructureFailed
        }
        EvidenceVerdict::NotRun => VerificationOutcome::NoCommands,
    };
    if verification_success {
        return Ok(PostMergeCheckResult {
            checked: true,
            auto_reverted: false,
            command: Some(command),
            exit_code,
            output_tail,
            merge_commit: None,
            revert_commit: None,
            fix_card_id: None,
            skipped_reason: None,
            revert_skipped_reason: None,
            outcome: VerificationOutcome::Passed,
        });
    }
    if verification_outcome != VerificationOutcome::CommandFailed {
        return Ok(failed_without_revert(
            Some(command),
            verification_outcome,
            output_tail,
        ));
    }
    if !allow_revert {
        return Ok(PostMergeCheckResult {
            checked: true,
            auto_reverted: false,
            command: Some(command),
            exit_code,
            output_tail,
            merge_commit: None,
            revert_commit: None,
            fix_card_id: None,
            skipped_reason: None,
            revert_skipped_reason: Some("automatic revert was not enabled".to_string()),
            outcome: VerificationOutcome::CommandFailed,
        });
    }
    let expected_merge_commit = request.expected_merge_commit.trim().to_string();
    if expected_merge_commit.is_empty() {
        return Ok(skipped_result("missing expected merge commit"));
    }

    let current_head = git_text(
        runner
            .run(
                &request.workspace_root,
                PostMergeCommand::Git(vec!["rev-parse".to_string(), "HEAD".to_string()]),
                request.timeout,
            )
            .await,
    );
    let Some(current_head) = current_head else {
        let reason = "unable to verify HEAD before revert";
        let diagnostic = format!("Expected merge commit: {}", expected_merge_commit);
        let fix_card_id = store_regression_result(
            gcx,
            &request.task_id,
            &request.card_id,
            request.expected_card_state.as_ref(),
            &command,
            &output_tail,
            &expected_merge_commit,
            "not reverted",
            &first_error_line(&output_tail),
            Some(reason),
            Some(&diagnostic),
        )
        .await?;
        return Ok(PostMergeCheckResult {
            checked: true,
            auto_reverted: false,
            command: Some(command),
            exit_code,
            output_tail,
            merge_commit: Some(expected_merge_commit),
            revert_commit: None,
            fix_card_id,
            skipped_reason: None,
            revert_skipped_reason: Some(reason.to_string()),
            outcome: VerificationOutcome::CommandFailed,
        });
    };
    if current_head != expected_merge_commit {
        let reason = "HEAD changed since merge";
        let diagnostic = format!(
            "Expected merge commit: {}\nCurrent HEAD: {}",
            expected_merge_commit, current_head
        );
        let fix_card_id = store_regression_result(
            gcx,
            &request.task_id,
            &request.card_id,
            request.expected_card_state.as_ref(),
            &command,
            &output_tail,
            &expected_merge_commit,
            "not reverted",
            &first_error_line(&output_tail),
            Some(reason),
            Some(&diagnostic),
        )
        .await?;
        return Ok(PostMergeCheckResult {
            checked: true,
            auto_reverted: false,
            command: Some(command),
            exit_code,
            output_tail,
            merge_commit: Some(expected_merge_commit),
            revert_commit: None,
            fix_card_id,
            skipped_reason: None,
            revert_skipped_reason: Some(reason.to_string()),
            outcome: VerificationOutcome::CommandFailed,
        });
    }

    let status_output = runner
        .run(
            &request.workspace_root,
            PostMergeCommand::Git(vec!["status".to_string(), "--porcelain".to_string()]),
            request.timeout,
        )
        .await;
    if !status_output.success || !status_output.output.trim().is_empty() {
        let reason = if status_output.success {
            "workspace dirty since merge"
        } else {
            "unable to verify clean workspace before revert"
        };
        let diagnostic = format!(
            "Expected merge commit: {}\nCurrent HEAD: {}\nGit status --porcelain:\n{}",
            expected_merge_commit,
            current_head,
            status_output.output.trim()
        );
        let fix_card_id = store_regression_result(
            gcx,
            &request.task_id,
            &request.card_id,
            request.expected_card_state.as_ref(),
            &command,
            &output_tail,
            &expected_merge_commit,
            "not reverted",
            &first_error_line(&output_tail),
            Some(reason),
            Some(&diagnostic),
        )
        .await?;
        return Ok(PostMergeCheckResult {
            checked: true,
            auto_reverted: false,
            command: Some(command),
            exit_code,
            output_tail,
            merge_commit: Some(expected_merge_commit),
            revert_commit: None,
            fix_card_id,
            skipped_reason: None,
            revert_skipped_reason: Some(reason.to_string()),
            outcome: VerificationOutcome::CommandFailed,
        });
    }

    let parents = runner
        .run(
            &request.workspace_root,
            PostMergeCommand::Git(vec![
                "rev-list".to_string(),
                "--parents".to_string(),
                "-n".to_string(),
                "1".to_string(),
                expected_merge_commit.clone(),
            ]),
            request.timeout,
        )
        .await;
    let parent_count = if parents.success {
        parents.output.split_whitespace().count().checked_sub(1)
    } else {
        None
    };
    let Some(parent_count) = parent_count else {
        return Ok(failed_without_revert(
            Some(command),
            VerificationOutcome::InfrastructureFailed,
            format!(
                "unable to determine merge commit parents: {}",
                parents.output.trim()
            ),
        ));
    };
    let mut revert_args = vec!["revert".to_string()];
    if parent_count > 1 {
        revert_args.extend(["-m".to_string(), "1".to_string()]);
    }
    revert_args.extend(["--no-edit".to_string(), expected_merge_commit.clone()]);
    let revert_output = runner
        .run(
            &request.workspace_root,
            PostMergeCommand::Git(revert_args),
            request.timeout,
        )
        .await;
    if !revert_output.success {
        let reason = "git revert failed";
        let diagnostic = format!(
            "Expected merge commit: {}\nCurrent HEAD: {}\nGit revert output:\n{}",
            expected_merge_commit,
            current_head,
            revert_output.output.trim()
        );
        store_regression_result(
            gcx,
            &request.task_id,
            &request.card_id,
            request.expected_card_state.as_ref(),
            &command,
            &output_tail,
            &expected_merge_commit,
            "not reverted",
            &first_error_line(&output_tail),
            Some(reason),
            Some(&diagnostic),
        )
        .await?;
        return Err(format!(
            "post-merge verification failed, and git revert failed: {}",
            first_error_line(&revert_output.output)
        ));
    }
    let revert_commit = git_text(
        runner
            .run(
                &request.workspace_root,
                PostMergeCommand::Git(vec!["rev-parse".to_string(), "HEAD".to_string()]),
                request.timeout,
            )
            .await,
    );
    let merge_commit = Some(expected_merge_commit);
    let first_error = first_error_line(&output_tail);
    let fix_card_id = store_regression_result(
        gcx,
        &request.task_id,
        &request.card_id,
        request.expected_card_state.as_ref(),
        &command,
        &output_tail,
        merge_commit.as_deref().unwrap_or("unknown"),
        revert_commit.as_deref().unwrap_or("unknown"),
        &first_error,
        None,
        None,
    )
    .await?;

    Ok(PostMergeCheckResult {
        checked: true,
        auto_reverted: true,
        command: Some(command),
        exit_code,
        output_tail,
        merge_commit,
        revert_commit,
        fix_card_id,
        skipped_reason: None,
        revert_skipped_reason: None,
        outcome: VerificationOutcome::CommandFailed,
    })
}

fn skipped_result(reason: &str) -> PostMergeCheckResult {
    PostMergeCheckResult {
        checked: false,
        auto_reverted: false,
        command: None,
        exit_code: None,
        output_tail: String::new(),
        merge_commit: None,
        revert_commit: None,
        fix_card_id: None,
        skipped_reason: Some(reason.to_string()),
        revert_skipped_reason: None,

        outcome: VerificationOutcome::Unknown,
    }
}

fn failed_without_revert(
    command: Option<String>,
    outcome: VerificationOutcome,
    output_tail: String,
) -> PostMergeCheckResult {
    PostMergeCheckResult {
        checked: true,
        auto_reverted: false,
        command,
        exit_code: None,
        output_tail,
        merge_commit: None,
        revert_commit: None,
        fix_card_id: None,
        skipped_reason: None,
        revert_skipped_reason: Some("verification was not a genuine command failure".to_string()),
        outcome,
    }
}

fn git_text(output: PostMergeCommandOutput) -> Option<String> {
    output
        .success
        .then(|| output.output.trim().to_string())
        .filter(|value| !value.is_empty())
}

async fn store_regression_result(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    card_id: &str,
    expected_card_state: Option<&ExpectedCardState>,
    command: &str,
    output_tail: &str,
    merge_commit: &str,
    revert_commit: &str,
    first_error: &str,
    revert_skipped_reason: Option<&str>,
    diagnostic: Option<&str>,
) -> Result<Option<String>, String> {
    let card_id_owned = card_id.to_string();
    let task_id_owned = task_id.to_string();
    let expected_card_state = expected_card_state.cloned();
    let command_owned = command.to_string();
    let output_owned = output_tail.to_string();
    let merge_commit_owned = merge_commit.to_string();
    let revert_commit_owned = revert_commit.to_string();
    let first_error_owned = first_error.to_string();
    let revert_skipped_reason_owned = revert_skipped_reason.map(str::to_string);
    let diagnostic_owned = diagnostic.map(str::to_string);
    let result = storage::update_board_atomic(gcx.clone(), task_id, move |board| {
        let Some(source_card_ref) = board.get_card(&card_id_owned) else {
            if expected_card_state.is_some() {
                return Err(format!(
                    "{} for task {} card {}: card no longer exists",
                    STALE_POST_MERGE_WRITE, task_id_owned, card_id_owned
                ));
            }
            return Err(format!("Card {} not found", card_id_owned));
        };
        if let Some(expected) = expected_card_state.as_ref() {
            if !expected.matches_board_card(board.rev, source_card_ref) {
                return Err(format!(
                    "{} for task {} card {}: current card state no longer matches post-merge request",
                    STALE_POST_MERGE_WRITE, task_id_owned, card_id_owned
                ));
            }
        }
        let source_card = source_card_ref.clone();
        ensure_regressed_column(board);
        let fix_card_id = next_fix_card_id(board, &card_id_owned);
        let now = Utc::now().to_rfc3339();
        let original = board
            .get_card_mut(&card_id_owned)
            .ok_or_else(|| format!("Card {} not found", card_id_owned))?;
        original.column = "regressed".to_string();
        original.status_updates.push(StatusUpdate {
            timestamp: now.clone(),
            message: match revert_skipped_reason_owned.as_deref() {
                Some(reason) => format!("Auto-revert skipped: {}", reason),
                None => format!("Auto-reverted: {}", first_error_owned),
            },
        });
        board.cards.push(build_fix_card(
            &source_card,
            &fix_card_id,
            &command_owned,
            &output_owned,
            &merge_commit_owned,
            &revert_commit_owned,
            revert_skipped_reason_owned.as_deref(),
            diagnostic_owned.as_deref(),
            &now,
        ));
        Ok(fix_card_id)
    })
    .await;
    let fix_card_id = match result {
        Ok((_, fix_card_id)) => fix_card_id,
        Err(error) if error.starts_with(STALE_POST_MERGE_WRITE) => {
            tracing::info!("{}", error);
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let _ = storage::update_task_stats(gcx, task_id).await;
    Ok(Some(fix_card_id))
}

fn ensure_regressed_column(board: &mut crate::tasks::types::TaskBoard) {
    if !board.columns.iter().any(|column| column.id == "regressed") {
        board.columns.push(BoardColumn {
            id: "regressed".to_string(),
            title: "Regressed".to_string(),
        });
    }
}

fn next_fix_card_id(board: &crate::tasks::types::TaskBoard, card_id: &str) -> String {
    let base = format!("{}-fix", card_id);
    if board.get_card(&base).is_none() {
        return base;
    }
    for index in 2.. {
        let candidate = format!("{}-{}", base, index);
        if board.get_card(&candidate).is_none() {
            return candidate;
        }
    }
    unreachable!()
}

fn build_fix_card(
    source_card: &BoardCard,
    fix_card_id: &str,
    command: &str,
    output_tail: &str,
    merge_commit: &str,
    revert_commit: &str,
    revert_skipped_reason: Option<&str>,
    diagnostic: Option<&str>,
    now: &str,
) -> BoardCard {
    let fence = markdown_code_fence(output_tail);
    let revert_summary = match revert_skipped_reason {
        Some(reason) => format!(
            "Auto-revert was skipped: {}. A human should inspect the current branch before reverting or fixing.",
            reason
        ),
        None => "The merge was automatically reverted after post-merge verification failed.".to_string(),
    };
    let diagnostic_section = diagnostic
        .map(|diagnostic| {
            format!(
                "\n## Revert diagnostic\n{}text\n{}\n{}\n",
                fence,
                diagnostic.trim(),
                fence
            )
        })
        .unwrap_or_default();
    BoardCard {
        id: fix_card_id.to_string(),
        title: format!("Fix regression in {}", source_card.id),
        column: "planned".to_string(),
        priority: source_card.priority.clone(),
        depends_on: Vec::new(),
        instructions: format!(
            "# Fix regression in {}\n\n{}\n\n- Original card: {} — {}\n- Merge commit: {}\n- Revert commit: {}\n- Failing command: `{}`\n{}\n## Failing output\n{}text\n{}\n{}\n\nFix the regression and run the verification command before merging again.\n",
            source_card.id,
            revert_summary,
            source_card.id,
            source_card.title,
            merge_commit,
            revert_commit,
            command,
            diagnostic_section,
            fence,
            output_tail.trim(),
            fence
        ),
        assignee: None,
        agent_chat_id: None,
        status_updates: vec![StatusUpdate {
            timestamp: now.to_string(),
            message: format!("Created after post-merge regression in {}", source_card.id),
        }],
        comments: Vec::new(),
        final_report: None,
        final_report_structured: None,
        verifier_report: None,
        created_at: now.to_string(),
        started_at: None,
        last_heartbeat_at: None,
        completed_at: None,
        agent_branch: None,
        agent_worktree: None,
        agent_worktree_name: None,
        base_branch: source_card.base_branch.clone(),
        base_commit: source_card.base_commit.clone(),
        ab_variants: None,
        target_files: source_card.target_files.clone(),
        scope_guard_mode: source_card.scope_guard_mode,
        team_members: Vec::new(),
        attention: None,
        deliverable: None,
        verification: Vec::new(),
    }
}

fn markdown_code_fence(text: &str) -> String {
    "`".repeat(max_backtick_run(text).max(2) + 1)
}

fn max_backtick_run(text: &str) -> usize {
    let mut max_run = 0;
    let mut current = 0;
    for c in text.chars() {
        if c == '`' {
            current += 1;
            max_run = max_run.max(current);
        } else {
            current = 0;
        }
    }
    max_run
}

pub fn first_error_line(output: &str) -> String {
    let mut fallback = None;
    for line in output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if fallback.is_none() {
            fallback = Some(line.to_string());
        }
        let lower = line.to_lowercase();
        if lower.contains("error") || lower.contains("failed") || lower.contains("failure") {
            return truncate_line(line, 240);
        }
    }
    fallback
        .map(|line| truncate_line(&line, 240))
        .unwrap_or_else(|| "verification failed".to_string())
}

fn truncate_line(line: &str, max_chars: usize) -> String {
    if line.chars().count() <= max_chars {
        return line.to_string();
    }
    let mut truncated = line.chars().take(max_chars).collect::<String>();
    truncated.push('…');
    truncated
}

fn tail_chars(text: &str, max_chars: usize) -> String {
    let len = text.chars().count();
    if len <= max_chars {
        return text.to_string();
    }
    text.chars().skip(len - max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    use crate::chat::evidence::{EvidenceContext, EvidenceRunner, VerificationSpec};
    use crate::chat::evidence_cache::{execute_cached, CacheKeyInputs, CacheMode};
    use crate::tasks::types::{ScopeGuardMode, TaskBoard, TaskMeta, TaskStatus};

    #[derive(Default)]
    struct MockRunner {
        outputs: VecDeque<PostMergeCommandOutput>,
        calls: Vec<PostMergeCommand>,
    }

    struct MutatingRunner {
        outputs: VecDeque<PostMergeCommandOutput>,
        calls: Vec<PostMergeCommand>,
        gcx: Arc<GlobalContext>,
        mutated: bool,
    }

    #[async_trait]
    impl PostMergeCommandRunner for MockRunner {
        async fn run(
            &mut self,
            _workspace_root: &Path,
            command: PostMergeCommand,
            _timeout: Duration,
        ) -> PostMergeCommandOutput {
            self.calls.push(command);
            self.outputs
                .pop_front()
                .expect("mock output must be queued")
        }
    }

    #[async_trait]
    impl PostMergeCommandRunner for MutatingRunner {
        async fn run(
            &mut self,
            _workspace_root: &Path,
            command: PostMergeCommand,
            _timeout: Duration,
        ) -> PostMergeCommandOutput {
            let should_mutate = matches!(
                &command,
                PostMergeCommand::Git(args)
                    if args.first().map(String::as_str) == Some("rev-parse")
            );
            self.calls.push(command);
            if should_mutate && !self.mutated {
                storage::update_board_atomic(self.gcx.clone(), "task-1", |board| {
                    board.get_card_mut("T-1").unwrap().final_report =
                        Some("newer finish".to_string());
                    Ok(())
                })
                .await
                .unwrap();
                self.mutated = true;
            }
            self.outputs
                .pop_front()
                .expect("mock output must be queued")
        }
    }

    #[derive(Default)]
    struct CacheEvidenceRunner {
        exit_code: i32,
        runs: usize,
    }

    #[async_trait]
    impl EvidenceRunner for CacheEvidenceRunner {
        async fn run(
            &mut self,
            _worktree: &Path,
            spec: &VerificationSpec,
        ) -> crate::tasks::types::VerificationResult {
            self.runs += 1;
            crate::tasks::types::VerificationResult {
                command: spec.command.clone(),
                exit_code: Some(self.exit_code),
                passed: self.exit_code == 0,
                output_tail: format!("cache runner {}", self.runs),
                outcome: if self.exit_code == 0 {
                    VerificationOutcome::Passed
                } else {
                    VerificationOutcome::CommandFailed
                },
                from_cache: false,
                executed_at: None,
            }
        }
    }

    fn output(success: bool, exit_code: Option<i32>, text: &str) -> PostMergeCommandOutput {
        PostMergeCommandOutput {
            success,
            exit_code,
            output: text.to_string(),
            outcome: if success {
                VerificationOutcome::Passed
            } else if exit_code.is_some() {
                VerificationOutcome::CommandFailed
            } else {
                VerificationOutcome::InfrastructureFailed
            },
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn system_runner_timeout_kills_process_group() {
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join("descendant-survived");
        let mut command = Command::new("sh");
        command.args([
            "-c",
            &format!("(sleep 1; touch '{}') & wait", marker.display()),
        ]);

        let result = run_command(command, "timeout process group", Duration::from_millis(50)).await;
        tokio::time::sleep(Duration::from_millis(1200)).await;

        assert_eq!(result.outcome, VerificationOutcome::InfrastructureFailed);
        assert!(result.output.contains("timed out"));
        assert!(!marker.exists(), "timed-out descendant survived");
    }

    fn card(instructions: &str) -> BoardCard {
        BoardCard {
            id: "T-1".to_string(),
            title: "Merge card".to_string(),
            column: "done".to_string(),
            priority: "P1".to_string(),
            depends_on: Vec::new(),
            instructions: instructions.to_string(),
            assignee: None,
            agent_chat_id: None,
            status_updates: Vec::new(),
            comments: vec![],
            final_report: Some("done".to_string()),
            final_report_structured: None,
            verifier_report: None,
            created_at: Utc::now().to_rfc3339(),
            started_at: None,
            last_heartbeat_at: None,
            completed_at: Some(Utc::now().to_rfc3339()),
            agent_branch: None,
            agent_worktree: None,
            agent_worktree_name: None,
            base_branch: None,
            base_commit: None,
            ab_variants: None,
            target_files: vec!["src/lib.rs".to_string()],
            scope_guard_mode: ScopeGuardMode::Off,
            team_members: vec![],
            attention: None,
            deliverable: None,
            verification: Vec::new(),
        }
    }

    async fn write_task(gcx: Arc<GlobalContext>, root: &Path, card: BoardCard) {
        let task_dir = root.join(".refact").join("tasks").join("task-1");
        tokio::fs::create_dir_all(&task_dir).await.unwrap();
        let now = Utc::now().to_rfc3339();
        let meta = TaskMeta {
            schema_version: 1,
            id: "task-1".to_string(),
            name: "Task".to_string(),
            status: TaskStatus::Active,
            created_at: now.clone(),
            updated_at: now,
            cards_total: 1,
            cards_done: 1,
            cards_failed: 0,
            agents_active: 0,
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
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.to_path_buf()];
    }

    fn request(root: &Path) -> PostMergeCheckRequest {
        PostMergeCheckRequest {
            task_id: "task-1".to_string(),
            card_id: "T-1".to_string(),
            workspace_root: root.to_path_buf(),
            enabled: true,
            timeout: Duration::from_secs(5),
            expected_merge_commit: "mergehash".to_string(),
            expected_card_state: None,
        }
    }

    #[tokio::test]
    async fn post_merge_check_no_command_is_checked_failure_without_revert() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        write_task(gcx.clone(), temp.path(), card("No verify command")).await;
        let mut runner = MockRunner::default();

        let result = post_merge_check_with_runner(gcx.clone(), request(temp.path()), &mut runner)
            .await
            .unwrap();

        assert!(result.checked);
        assert!(!result.auto_reverted);
        assert_eq!(result.outcome, VerificationOutcome::NoCommands);
        assert_eq!(
            result.revert_skipped_reason.as_deref(),
            Some("verification was not a genuine command failure")
        );
        assert!(runner.calls.is_empty());
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        assert_eq!(board.get_card("T-1").unwrap().column, "done");
    }

    #[tokio::test]
    async fn post_merge_check_does_not_revert_when_verification_passes() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        write_task(
            gcx.clone(),
            temp.path(),
            card("## Acceptance Criteria\n- Verify: `cargo test --lib`"),
        )
        .await;
        let mut runner = MockRunner {
            outputs: VecDeque::from([output(true, Some(0), "ok")]),
            calls: Vec::new(),
        };

        let result = post_merge_check_with_runner(gcx.clone(), request(temp.path()), &mut runner)
            .await
            .unwrap();

        assert!(result.checked);
        assert!(!result.auto_reverted);
        assert_eq!(
            runner.calls,
            vec![PostMergeCommand::Verify {
                cwd: None,
                env: Vec::new(),
                argv: vec!["cargo".to_string(), "test".to_string(), "--lib".to_string()]
            }]
        );
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        assert_eq!(board.get_card("T-1").unwrap().column, "done");
        assert!(board.get_card("T-1-fix").is_none());
    }

    #[tokio::test]
    async fn post_merge_runs_argv_not_shell() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        write_task(
            gcx.clone(),
            temp.path(),
            card("## Acceptance Criteria\n- Verify: `cd refact-agent/engine && cargo check`"),
        )
        .await;
        let mut runner = MockRunner {
            outputs: VecDeque::from([output(true, Some(0), "ok")]),
            calls: Vec::new(),
        };

        let result = post_merge_check_with_runner(gcx, request(temp.path()), &mut runner)
            .await
            .unwrap();

        assert!(result.checked);
        assert_eq!(
            runner.calls,
            vec![PostMergeCommand::Verify {
                cwd: Some(PathBuf::from("refact-agent/engine")),
                env: Vec::new(),
                argv: vec!["cargo".to_string(), "check".to_string()]
            }]
        );
    }

    #[tokio::test]
    async fn post_merge_runs_previously_unsupported_binaries() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        write_task(
            gcx.clone(),
            temp.path(),
            card("## Acceptance Criteria\n- Verify: `dotnet test`"),
        )
        .await;
        let mut runner = MockRunner {
            outputs: VecDeque::from([output(true, Some(0), "ok")]),
            calls: Vec::new(),
        };

        let result = post_merge_check_with_runner(gcx, request(temp.path()), &mut runner)
            .await
            .unwrap();

        assert!(result.checked);
        assert_eq!(
            runner.calls,
            vec![PostMergeCommand::Verify {
                cwd: None,
                env: Vec::new(),
                argv: vec!["dotnet".to_string(), "test".to_string()]
            }]
        );
    }

    #[tokio::test]
    async fn post_merge_uses_live_planner_verify_lines_after_dispatch() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut card = card("## Verification\n- Verify: `cargo test --assignment`");
        card.target_files = vec!["src/lib.rs".to_string()];
        write_task(gcx.clone(), temp.path(), {
            let mut edited = card.clone();
            edited.instructions =
                "## Verification\n- Verify: `cargo test --edited-card`".to_string();
            edited
        })
        .await;
        crate::tasks::assignment::save_assignment(
            gcx.clone(),
            "task-1",
            &crate::tasks::assignment::Assignment::from_card(&card),
        )
        .await
        .unwrap();
        let mut runner = MockRunner {
            outputs: VecDeque::from([output(true, Some(0), "ok")]),
            calls: Vec::new(),
        };

        let result = post_merge_check_with_runner(gcx, request(temp.path()), &mut runner)
            .await
            .unwrap();

        assert_eq!(result.outcome, VerificationOutcome::Passed);
        assert_eq!(
            runner.calls,
            vec![PostMergeCommand::Verify {
                cwd: None,
                env: Vec::new(),
                argv: vec![
                    "cargo".to_string(),
                    "test".to_string(),
                    "--edited-card".to_string()
                ]
            }],
            "planner Verify corrections are also used by post-merge validation"
        );
    }

    #[tokio::test]
    #[serial_test::serial(evidence_cache)]
    async fn post_merge_revalidates_even_when_the_same_tree_has_cached_green_evidence() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let unique = uuid::Uuid::new_v4().to_string();
        let input = format!("input-{unique}.txt");
        tokio::fs::write(temp.path().join(&input), &unique)
            .await
            .unwrap();
        tokio::fs::create_dir_all(temp.path().join("src"))
            .await
            .unwrap();
        tokio::fs::write(temp.path().join("src/lib.rs"), &unique)
            .await
            .unwrap();
        let mut card = card("## Verification\n- Verify: `cargo test --lib`");
        card.target_files = vec![input];
        let assignment = crate::tasks::assignment::Assignment::from_card(&card);
        write_task(gcx.clone(), temp.path(), card.clone()).await;
        crate::tasks::assignment::save_assignment(gcx.clone(), "task-1", &assignment)
            .await
            .unwrap();

        let defaults = EvidenceDefaults::from_settings();
        let mut specs =
            crate::tasks::assignment::specs_for_validation(Some(&assignment), &card, &defaults);
        for spec in &mut specs {
            spec.timeout_secs = request(temp.path()).timeout.as_secs().max(1);
            spec.output_cap_bytes = MAX_OUTPUT_TAIL_CHARS;
        }
        let policy = VerifyCommandPolicy::load(gcx.clone()).await;
        let service = EvidenceService::new(
            policy,
            EvidenceContext {
                worktree: temp.path().to_path_buf(),
                commit: Some("mergehash".to_string()),
                scope: card.id.clone(),
            },
        );
        let cache = evidence_cache();
        let key = CacheKeyInputs::collect(
            service.worktree(),
            &service.accept(specs[0].clone()).unwrap(),
            &service.policy_fingerprint(),
        )
        .await
        .key();
        let mut green_runner = CacheEvidenceRunner {
            exit_code: 0,
            runs: 0,
        };
        let cached = execute_cached(
            &service,
            cache,
            specs[0].clone(),
            &mut green_runner,
            CacheMode::Replay,
            crate::chat::evidence_cache::now_ms(),
        )
        .await;
        assert_eq!(cached.verdict, EvidenceVerdict::Passed);
        assert_eq!(green_runner.runs, 1);
        assert!(cache.lock().await.contains(&key));
        let entries_after_seed = cache.lock().await.len();

        let mut red_runner = MockRunner {
            outputs: VecDeque::from([
                output(false, Some(1), "error: fresh failure"),
                output(true, Some(0), "newhead"),
            ]),
            calls: Vec::new(),
        };
        let result = post_merge_check_with_runner(gcx, request(temp.path()), &mut red_runner)
            .await
            .unwrap();

        assert_eq!(result.outcome, VerificationOutcome::CommandFailed);
        assert!(!result.auto_reverted);
        assert!(result.output_tail.contains("error: fresh failure"));
        assert_eq!(red_runner.calls.len(), 2, "verify then guarded HEAD check");
        assert!(matches!(
            red_runner.calls[0],
            PostMergeCommand::Verify { .. }
        ));
        assert!(matches!(red_runner.calls[1], PostMergeCommand::Git(_)));
        assert_eq!(
            cache.lock().await.len(),
            entries_after_seed,
            "revalidation must not write or evict cache entries"
        );
    }

    #[tokio::test]
    async fn post_merge_command_failure_without_exit_code_is_infrastructure_failure() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        write_task(
            gcx.clone(),
            temp.path(),
            card("## Verification\n- Verify: `cargo test --lib`"),
        )
        .await;
        let mut runner = MockRunner {
            outputs: VecDeque::from([PostMergeCommandOutput {
                exit_code: None,
                success: false,
                output: "killed before exit status".to_string(),
                outcome: VerificationOutcome::CommandFailed,
            }]),
            calls: Vec::new(),
        };

        let result = post_merge_check_with_runner(gcx, request(temp.path()), &mut runner)
            .await
            .unwrap();

        assert_eq!(result.outcome, VerificationOutcome::InfrastructureFailed);
        assert!(!result.auto_reverted);
        assert_eq!(runner.calls.len(), 1);
        assert!(matches!(runner.calls[0], PostMergeCommand::Verify { .. }));
    }

    #[tokio::test]
    async fn post_merge_denies_command_matching_deny_rule() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        write_task(
            gcx.clone(),
            temp.path(),
            card("## Acceptance Criteria\n- Verify: `sudo rm -rf /`"),
        )
        .await;
        let mut runner = MockRunner::default();

        let result = post_merge_check_with_runner(gcx, request(temp.path()), &mut runner)
            .await
            .unwrap();

        assert!(result.checked);
        assert!(!result.auto_reverted);
        assert_eq!(result.outcome, VerificationOutcome::PolicyDenied);
        assert!(
            result.output_tail.starts_with("Denied by shell policy:"),
            "{}",
            result.output_tail
        );
        assert!(runner.calls.is_empty());
    }

    #[tokio::test]
    async fn post_merge_rejects_shell_syntax() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        write_task(
            gcx.clone(),
            temp.path(),
            card("## Acceptance Criteria\n- Verify: `cargo test | tee f`"),
        )
        .await;
        let mut runner = MockRunner::default();

        let result = post_merge_check_with_runner(gcx, request(temp.path()), &mut runner)
            .await
            .unwrap();

        assert_eq!(result.outcome, VerificationOutcome::Rejected);
        assert!(
            result.output_tail.starts_with("Cannot run as a command:"),
            "{}",
            result.output_tail
        );
        assert!(runner.calls.is_empty());
    }

    #[tokio::test]
    async fn post_merge_check_skips_revert_when_head_changed() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        write_task(
            gcx.clone(),
            temp.path(),
            card("## Acceptance Criteria\n- Verify: `cargo test --lib`"),
        )
        .await;
        let mut runner = MockRunner {
            outputs: VecDeque::from([
                output(false, Some(1), "running\nerror: regression failed\n"),
                output(true, Some(0), "newhead\n"),
            ]),
            calls: Vec::new(),
        };

        let result = post_merge_check_with_runner(gcx.clone(), request(temp.path()), &mut runner)
            .await
            .unwrap();

        assert!(result.checked);
        assert!(!result.auto_reverted);
        assert_eq!(result.merge_commit.as_deref(), Some("mergehash"));
        assert_eq!(
            result.revert_skipped_reason.as_deref(),
            Some("HEAD changed since merge")
        );
        assert_eq!(result.fix_card_id.as_deref(), Some("T-1-fix"));
        assert!(!runner.calls.iter().any(|call| matches!(
            call,
            PostMergeCommand::Git(args) if args.first().map(String::as_str) == Some("revert")
        )));
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let original = board.get_card("T-1").unwrap();
        assert_eq!(original.column, "regressed");
        assert!(original
            .status_updates
            .iter()
            .any(|update| update.message == "Auto-revert skipped: HEAD changed since merge"));
        let fix = board.get_card("T-1-fix").unwrap();
        assert!(fix.instructions.contains("Current HEAD: newhead"));
    }

    #[tokio::test]
    async fn post_merge_check_skips_revert_when_dirty_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        write_task(
            gcx.clone(),
            temp.path(),
            card("## Acceptance Criteria\n- Verify: `cargo test --lib`"),
        )
        .await;
        let mut runner = MockRunner {
            outputs: VecDeque::from([
                output(false, Some(1), "running\nerror: regression failed\n"),
                output(true, Some(0), "mergehash\n"),
                output(true, Some(0), " M src/lib.rs\n"),
            ]),
            calls: Vec::new(),
        };

        let result = post_merge_check_with_runner(gcx.clone(), request(temp.path()), &mut runner)
            .await
            .unwrap();

        assert!(result.checked);
        assert!(!result.auto_reverted);
        assert_eq!(result.merge_commit.as_deref(), Some("mergehash"));
        assert_eq!(
            result.revert_skipped_reason.as_deref(),
            Some("workspace dirty since merge")
        );
        assert_eq!(result.fix_card_id.as_deref(), Some("T-1-fix"));
        assert!(!runner.calls.iter().any(|call| matches!(
            call,
            PostMergeCommand::Git(args) if args.first().map(String::as_str) == Some("revert")
        )));
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let original = board.get_card("T-1").unwrap();
        assert_eq!(original.column, "regressed");
        assert!(original
            .status_updates
            .iter()
            .any(|update| update.message == "Auto-revert skipped: workspace dirty since merge"));
        let fix = board.get_card("T-1-fix").unwrap();
        assert!(fix.instructions.contains("Git status --porcelain"));
        assert!(fix.instructions.contains("M src/lib.rs"));
    }

    #[tokio::test]
    async fn post_merge_check_reverts_when_head_matches_and_clean() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        write_task(
            gcx.clone(),
            temp.path(),
            card("## Acceptance Criteria\n- Verify: `cargo test --lib`"),
        )
        .await;
        let mut runner = MockRunner {
            outputs: VecDeque::from([
                output(false, Some(1), "running\nerror: regression failed\n"),
                output(true, Some(0), "mergehash\n"),
                output(true, Some(0), ""),
                output(true, Some(0), "mergehash parenthash\n"),
                output(true, Some(0), "revert ok\n"),
                output(true, Some(0), "reverthash\n"),
            ]),
            calls: Vec::new(),
        };

        let result = post_merge_check_with_runner(gcx.clone(), request(temp.path()), &mut runner)
            .await
            .unwrap();

        assert!(result.auto_reverted);
        assert_eq!(result.merge_commit.as_deref(), Some("mergehash"));
        assert_eq!(result.revert_commit.as_deref(), Some("reverthash"));
        assert_eq!(result.fix_card_id.as_deref(), Some("T-1-fix"));
        assert!(runner.calls.contains(&PostMergeCommand::Git(vec![
            "revert".to_string(),
            "--no-edit".to_string(),
            "mergehash".to_string()
        ])));
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let original = board.get_card("T-1").unwrap();
        assert_eq!(original.column, "regressed");
        assert!(original
            .status_updates
            .iter()
            .any(|update| update.message == "Auto-reverted: error: regression failed"));
        assert!(board.columns.iter().any(|column| column.id == "regressed"));
    }

    #[tokio::test]
    async fn post_merge_check_reverts_merge_commit_with_mainline() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        write_task(
            gcx.clone(),
            temp.path(),
            card("## Acceptance Criteria\n- Verify: `cargo test --lib`"),
        )
        .await;
        let mut runner = MockRunner {
            outputs: VecDeque::from([
                output(false, Some(1), "error: regression\n"),
                output(true, Some(0), "mergehash\n"),
                output(true, Some(0), ""),
                output(true, Some(0), "mergehash parent1 parent2\n"),
                output(true, Some(0), "revert ok\n"),
                output(true, Some(0), "reverthash\n"),
            ]),
            calls: Vec::new(),
        };

        let result = post_merge_check_with_runner(gcx, request(temp.path()), &mut runner)
            .await
            .unwrap();

        assert!(result.auto_reverted);
        assert!(runner.calls.contains(&PostMergeCommand::Git(vec![
            "revert".to_string(),
            "-m".to_string(),
            "1".to_string(),
            "--no-edit".to_string(),
            "mergehash".to_string(),
        ])));
    }

    #[tokio::test]
    async fn post_merge_check_skips_stale_card_state_before_running_commands() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        write_task(
            gcx.clone(),
            temp.path(),
            card("## Acceptance Criteria\n- Verify: `cargo test --lib`"),
        )
        .await;
        let expected_card_state =
            crate::chat::verifier::load_expected_card_state(gcx.clone(), "task-1", "T-1")
                .await
                .unwrap();
        storage::update_board_atomic(gcx.clone(), "task-1", |board| {
            board.get_card_mut("T-1").unwrap().final_report = Some("newer finish".to_string());
            Ok(())
        })
        .await
        .unwrap();
        let mut req = request(temp.path());
        req.expected_card_state = Some(expected_card_state);
        let mut runner = MockRunner::default();

        let result = post_merge_check_with_runner(gcx.clone(), req, &mut runner)
            .await
            .unwrap();

        assert!(!result.checked);
        assert_eq!(result.skipped_reason.as_deref(), Some("stale card state"));
        assert!(runner.calls.is_empty());
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        assert_eq!(board.get_card("T-1").unwrap().column, "done");
        assert!(board.get_card("T-1-fix").is_none());
    }

    #[tokio::test]
    async fn post_merge_check_noops_regression_write_when_card_changed_after_failure() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        write_task(
            gcx.clone(),
            temp.path(),
            card("## Acceptance Criteria\n- Verify: `cargo test --lib`"),
        )
        .await;
        let expected_card_state =
            crate::chat::verifier::load_expected_card_state(gcx.clone(), "task-1", "T-1")
                .await
                .unwrap();
        let mut req = request(temp.path());
        req.expected_card_state = Some(expected_card_state);
        let mut runner = MutatingRunner {
            outputs: VecDeque::from([
                output(false, Some(1), "running\nerror: regression failed\n"),
                output(true, Some(0), "newhead\n"),
            ]),
            calls: Vec::new(),
            gcx: gcx.clone(),
            mutated: false,
        };

        let result = post_merge_check_with_runner(gcx.clone(), req, &mut runner)
            .await
            .unwrap();

        assert!(result.checked);
        assert_eq!(
            result.revert_skipped_reason.as_deref(),
            Some("HEAD changed since merge")
        );
        assert!(result.fix_card_id.is_none());
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let original = board.get_card("T-1").unwrap();
        assert_eq!(original.column, "done");
        assert_eq!(original.final_report.as_deref(), Some("newer finish"));
        assert!(board.get_card("T-1-fix").is_none());
    }

    #[tokio::test]
    async fn post_merge_check_persists_regression_when_revert_fails() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        write_task(
            gcx.clone(),
            temp.path(),
            card("## Acceptance Criteria\n- Verify: `cargo test --lib`"),
        )
        .await;
        let mut runner = MockRunner {
            outputs: VecDeque::from([
                output(false, Some(1), "running\nerror: regression failed\n"),
                output(true, Some(0), "mergehash\n"),
                output(true, Some(0), ""),
                output(true, Some(0), "mergehash parenthash\n"),
                output(false, Some(1), "CONFLICT in src/lib.rs\n"),
            ]),
            calls: Vec::new(),
        };

        let error = post_merge_check_with_runner(gcx.clone(), request(temp.path()), &mut runner)
            .await
            .unwrap_err();

        assert!(
            error.contains("post-merge verification failed, and git revert failed"),
            "{error}"
        );
        assert!(runner.calls.contains(&PostMergeCommand::Git(vec![
            "revert".to_string(),
            "--no-edit".to_string(),
            "mergehash".to_string()
        ])));
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let original = board.get_card("T-1").unwrap();
        assert_eq!(original.column, "regressed");
        assert!(original
            .status_updates
            .iter()
            .any(|update| update.message == "Auto-revert skipped: git revert failed"));
        let fix = board.get_card("T-1-fix").unwrap();
        assert!(fix
            .instructions
            .contains("Auto-revert was skipped: git revert failed"));
        assert!(fix.instructions.contains("Git revert output"));
        assert!(fix.instructions.contains("CONFLICT in src/lib.rs"));
    }

    #[tokio::test]
    async fn post_merge_check_fix_card_contains_revert_info() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        write_task(
            gcx.clone(),
            temp.path(),
            card("## Acceptance Criteria\n- Verify: `npm test`"),
        )
        .await;
        let mut req = request(temp.path());
        req.expected_merge_commit = "abc123".to_string();
        let mut runner = MockRunner {
            outputs: VecDeque::from([
                output(false, Some(1), "FAIL test_a\nexpected true\n"),
                output(true, Some(0), "abc123\n"),
                output(true, Some(0), ""),
                output(true, Some(0), "abc123 parenthash\n"),
                output(true, Some(0), "reverted\n"),
                output(true, Some(0), "def456\n"),
            ]),
            calls: Vec::new(),
        };

        post_merge_check_with_runner(gcx.clone(), req, &mut runner)
            .await
            .unwrap();

        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let fix = board.get_card("T-1-fix").unwrap();
        assert_eq!(fix.title, "Fix regression in T-1");
        assert!(fix.instructions.contains("Merge commit: abc123"));
        assert!(fix.instructions.contains("Revert commit: def456"));
        assert!(fix.instructions.contains("Failing command: `npm test`"));
        assert!(fix.instructions.contains("FAIL test_a"));
        assert_eq!(fix.target_files, vec!["src/lib.rs".to_string()]);
    }
}
