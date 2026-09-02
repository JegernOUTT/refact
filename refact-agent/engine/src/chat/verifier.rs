use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use serde_json::{json, Value};

use crate::chat::internal_roles::{event, EventSubkind};
use crate::chat::verifier_diff::{git_changed_files_summary, resolve_verifier_diff_base};
use crate::chat::verify_cmd::{parse_verification_argv, verification_commands, VerifyCommandPolicy};
use crate::exec::command_policy::{build_exec_request, CommandKind, CommandPolicyInput, ExecSource};
use crate::exec::{ExecOutputStream, ExecStatus};
use crate::global_context::{try_load_caps_quickly_if_not_present, GlobalContext};
use crate::worktrees::service::WorktreeService;
use crate::tasks::storage;
use crate::tasks::types::{
    BoardCard, StatusUpdate, VerificationOutcome, VerificationResult, VerifierReport,
    VerifierReportClassification,
};

const VERIFY_TIMEOUT: Duration = Duration::from_secs(600);
const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_OUTPUT_TAIL_CHARS: usize = 4000;
const MAX_OUTPUT_CAPTURE_BYTES: usize = 512 * 1024;
const MAX_DIFF_LINES: usize = 200;
const VERIFIER_SOURCE: &str = "chat.verifier";
const STALE_VERIFIER_WRITE: &str = "stale verifier write";

#[derive(Clone, Debug, PartialEq, Default)]
pub struct ExpectedCardState {
    pub board_rev: u64,
    card_fingerprint: Value,
}

impl ExpectedCardState {
    pub fn from_card(board_rev: u64, card: &BoardCard) -> Self {
        Self {
            board_rev,
            card_fingerprint: verifier_card_fingerprint(card),
        }
    }

    pub fn matches_card(&self, card: &BoardCard) -> bool {
        self.card_fingerprint == verifier_card_fingerprint(card)
    }

    pub fn matches_board_card(&self, board_rev: u64, card: &BoardCard) -> bool {
        board_rev >= self.board_rev && self.matches_card(card)
    }
}

pub async fn load_expected_card_state(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    card_id: &str,
) -> Result<ExpectedCardState, String> {
    let board = storage::load_board(gcx, task_id).await?;
    let card = board
        .get_card(card_id)
        .ok_or_else(|| format!("Card {} not found", card_id))?;
    Ok(ExpectedCardState::from_card(board.rev, card))
}

fn verifier_card_fingerprint(card: &BoardCard) -> Value {
    json!({
        "id": &card.id,
        "title": &card.title,
        "column": &card.column,
        "priority": &card.priority,
        "depends_on": &card.depends_on,
        "instructions": &card.instructions,
        "assignee": &card.assignee,
        "agent_chat_id": &card.agent_chat_id,
        "final_report": &card.final_report,
        "final_report_structured": &card.final_report_structured,
        "verifier_report": &card.verifier_report,
        "created_at": &card.created_at,
        "started_at": &card.started_at,
        "completed_at": &card.completed_at,
        "agent_branch": &card.agent_branch,
        "agent_worktree": &card.agent_worktree,
        "agent_worktree_name": &card.agent_worktree_name,
        "base_branch": &card.base_branch,
        "base_commit": &card.base_commit,
        "ab_variants": &card.ab_variants,
        "team_members": &card.team_members,
        "target_files": &card.target_files,
        "scope_guard_mode": &card.scope_guard_mode,
    })
}

fn stale_verifier_error(task_id: &str, card_id: &str) -> String {
    format!(
        "{} for task {} card {}: current card state no longer matches the verifier request",
        STALE_VERIFIER_WRITE, task_id, card_id
    )
}

fn is_stale_verifier_error(error: &str) -> bool {
    error.starts_with(STALE_VERIFIER_WRITE)
}

#[derive(Clone, Debug)]
pub struct VerifyCardRequest {
    pub task_id: String,
    pub card_id: String,
    pub expected_state: Option<ExpectedCardState>,
}

#[async_trait]
trait VerificationCommandRunner: Send {
    async fn run(
        &mut self,
        worktree: &Path,
        command: &str,
        cwd: Option<PathBuf>,
        env: Vec<(String, String)>,
        argv: Vec<String>,
    ) -> VerificationResult;
}

struct SystemVerificationCommandRunner {
    gcx: Arc<GlobalContext>,
}

#[async_trait]
impl VerificationCommandRunner for SystemVerificationCommandRunner {
    async fn run(
        &mut self,
        worktree: &Path,
        command: &str,
        cwd: Option<PathBuf>,
        env: Vec<(String, String)>,
        argv: Vec<String>,
    ) -> VerificationResult {
        run_verification_argv(self.gcx.clone(), worktree, command, cwd, env, argv).await
    }
}

async fn check_cwd_in_worktree(worktree: &Path, effective_cwd: &Path) -> Result<(), String> {
    let canonical_worktree = tokio::fs::canonicalize(worktree)
        .await
        .map_err(|e| format!("cannot access worktree '{}': {}", worktree.display(), e))?;
    if let Ok(canonical_cwd) = tokio::fs::canonicalize(effective_cwd).await {
        if !canonical_cwd.starts_with(&canonical_worktree) {
            return Err(format!(
                "cwd '{}' is outside the worktree",
                effective_cwd.display()
            ));
        }
    }
    Ok(())
}

fn append_verifier_status(card: &mut BoardCard, report: &VerifierReport) {
    let message = if report.passed {
        "Verifier: PASS".to_string()
    } else {
        let first = report
            .concerns
            .first()
            .map(|s| s.as_str())
            .unwrap_or("verification failed");
        format!("Verifier: FAIL — {}", first)
    };
    card.status_updates.push(StatusUpdate {
        timestamp: Utc::now().to_rfc3339(),
        message,
    });
}

pub async fn store_verifier_report(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    card_id: &str,
    expected_state: Option<&ExpectedCardState>,
    report: VerifierReport,
) -> Result<bool, String> {
    let card_id = card_id.to_string();
    let task_id_owned = task_id.to_string();
    let expected_state = expected_state.cloned();
    match storage::update_board_atomic(gcx, task_id, move |board| {
        let board_rev = board.rev;
        let Some(card) = board.get_card_mut(&card_id) else {
            if expected_state.is_some() {
                return Err(stale_verifier_error(&task_id_owned, &card_id));
            }
            return Err(format!("Card {} not found", card_id));
        };
        if let Some(expected) = expected_state.as_ref() {
            if !expected.matches_board_card(board_rev, card) {
                return Err(stale_verifier_error(&task_id_owned, &card_id));
            }
        }
        card.verifier_report = Some(report.clone());
        append_verifier_status(card, &report);
        Ok(())
    })
    .await
    {
        Ok(_) => Ok(true),
        Err(error) if is_stale_verifier_error(&error) => {
            tracing::info!("{}", error);
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

pub async fn schedule_card_verifier(gcx: Arc<GlobalContext>, request: VerifyCardRequest) {
    if gcx.shutdown_flag.load(Ordering::SeqCst) {
        return;
    }
    let task_gcx = gcx.clone();
    let mut tasks = gcx.card_verifier_tasks.lock().await;
    while let Some(result) = tasks.try_join_next() {
        if let Err(error) = result {
            tracing::warn!("card verifier task failed: {error}");
        }
    }
    if gcx.shutdown_flag.load(Ordering::SeqCst) {
        return;
    }
    tasks.spawn(async move {
        if let Err(error) = verify_card(task_gcx.clone(), request.clone()).await {
            if is_stale_verifier_error(&error) {
                tracing::info!("{}", error);
                return;
            }
            let report = launch_failure_report(error);
            if let Err(store_error) = store_verifier_report(
                task_gcx,
                &request.task_id,
                &request.card_id,
                request.expected_state.as_ref(),
                report,
            )
            .await
            {
                tracing::warn!(
                    "failed to store verifier launch-failure report for card {}: {}",
                    request.card_id,
                    store_error
                );
            }
        }
    });
}

pub async fn shutdown_card_verifiers(gcx: Arc<GlobalContext>) {
    let mut tasks = gcx.card_verifier_tasks.lock().await;
    tasks.abort_all();
    while let Some(result) = tasks.join_next().await {
        if let Err(error) = result {
            if !error.is_cancelled() {
                tracing::warn!("card verifier task failed during shutdown: {error}");
            }
        }
    }
}

pub async fn schedule_card_verifier_after_finish(
    gcx: Arc<GlobalContext>,
    task_id: String,
    card_id: String,
    expected_state: ExpectedCardState,
) {
    schedule_card_verifier(
        gcx,
        VerifyCardRequest {
            task_id,
            card_id,
            expected_state: Some(expected_state),
        },
    )
    .await;
}

pub async fn verify_card(
    gcx: Arc<GlobalContext>,
    request: VerifyCardRequest,
) -> Result<VerifierReport, String> {
    let task_meta = storage::load_task_meta(gcx.clone(), &request.task_id).await?;
    let board = storage::load_board(gcx.clone(), &request.task_id).await?;
    let card = board
        .get_card(&request.card_id)
        .ok_or_else(|| format!("Card {} not found", request.card_id))?
        .clone();
    if let Some(expected) = request.expected_state.as_ref() {
        if !expected.matches_board_card(board.rev, &card) {
            return Err(stale_verifier_error(&request.task_id, &request.card_id));
        }
    }
    let worktree = card
        .agent_worktree
        .as_ref()
        .map(PathBuf::from)
        .ok_or_else(|| format!("Card {} has no agent worktree", card.id))?;
    if !tokio::fs::metadata(&worktree)
        .await
        .is_ok_and(|metadata| metadata.is_dir())
    {
        return Err(format!(
            "Card {} worktree '{}' does not exist",
            card.id,
            worktree.display()
        ));
    }

    let commands = verification_commands(&card);
    let no_commands = commands.is_empty();
    let mut command_results = Vec::new();
    let mut concerns = Vec::new();

    if no_commands {
        concerns.push(
            "No verification commands found in card instructions or final report".to_string(),
        );
    }

    let command_policy = VerifyCommandPolicy::load(gcx.clone()).await;
    for command in commands {
        let result =
            run_verification_command_with_policy(gcx.clone(), &worktree, &command, &command_policy)
                .await;
        if !result.passed {
            concerns.push(format!("Verification command failed: {}", result.command));
        }
        command_results.push(result);
    }

    let worktree_base = if let Some(worktree_id) = card.agent_worktree_name.as_deref() {
        let project_root = crate::files_correction::get_project_dirs(gcx.clone())
            .await
            .into_iter()
            .next();
        if let Some(project_root) = project_root {
            match WorktreeService::new_async(gcx.cache_dir.clone(), project_root).await {
                Ok(service) => service
                    .get_worktree(worktree_id)
                    .await
                    .ok()
                    .map(|view| (view.meta.base_commit, view.meta.base_branch)),
                Err(_) => None,
            }
        } else {
            None
        }
    } else {
        None
    };
    let card_pair_present = card.base_commit.is_some() || card.base_branch.is_some();
    let (base_commit, base_branch) = if card_pair_present {
        (card.base_commit.clone(), card.base_branch.clone())
    } else if let Some((commit, branch)) = worktree_base {
        if commit.is_some() || branch.is_some() {
            (commit, branch)
        } else {
            (task_meta.base_commit, task_meta.base_branch)
        }
    } else {
        (task_meta.base_commit, task_meta.base_branch)
    };
    let diff_base = resolve_verifier_diff_base(base_commit, base_branch)?;
    let diff = git_changed_files_summary(&worktree, &diff_base, MAX_DIFF_LINES)
        .await
        .unwrap_or_else(|error| format!("diff unavailable: {}", error));
    let prompt = verifier_prompt(&card, &command_results, &diff);
    let review = run_verifier_review(gcx.clone(), prompt, card.agent_chat_id.as_deref()).await;
    let review_unavailable = review.is_err();
    let model_concerns = review.unwrap_or_else(|error| {
        vec![format!(
            "Verifier review subchat unavailable; human review recommended: {}",
            error
        )]
    });
    let review_has_concerns = !review_unavailable && !model_concerns.is_empty();
    concerns.extend(model_concerns);

    let classification = classify_verifier_report(
        no_commands,
        &command_results,
        review_unavailable,
        review_has_concerns,
    );
    let passed = classification == VerifierReportClassification::Passed;
    let recommendation = match classification {
        VerifierReportClassification::Passed => "merge",
        VerifierReportClassification::HumanReview => "human-review",
        VerifierReportClassification::VerificationFailed
        | VerifierReportClassification::Unknown => "fix-needed",
    }
    .to_string();

    let report = VerifierReport {
        passed,
        command_results,
        concerns,
        recommendation,
        classification,
    };
    let stored = store_verifier_report(
        gcx,
        &request.task_id,
        &request.card_id,
        request.expected_state.as_ref(),
        report.clone(),
    )
    .await?;
    if !stored {
        return Err(stale_verifier_error(&request.task_id, &request.card_id));
    }
    Ok(report)
}

pub(crate) fn outcome_is_infrastructure(outcome: &VerificationOutcome) -> bool {
    matches!(
        outcome,
        VerificationOutcome::InfrastructureFailed
            | VerificationOutcome::Rejected
            | VerificationOutcome::PolicyDenied
    )
}

fn classify_verifier_report(
    no_commands: bool,
    command_results: &[VerificationResult],
    review_unavailable: bool,
    review_has_concerns: bool,
) -> VerifierReportClassification {
    let command_verification_failed = no_commands
        || command_results.iter().any(|result| {
            matches!(
                result.outcome,
                VerificationOutcome::CommandFailed | VerificationOutcome::NoCommands
            ) || (!result.passed && !outcome_is_infrastructure(&result.outcome))
        });
    if command_verification_failed || review_has_concerns {
        return VerifierReportClassification::VerificationFailed;
    }
    let infrastructure_failed = command_results
        .iter()
        .any(|result| outcome_is_infrastructure(&result.outcome) || !result.passed);
    if infrastructure_failed || review_unavailable {
        return VerifierReportClassification::HumanReview;
    }
    if !command_results.is_empty()
        && command_results.iter().all(|result| {
            result.passed
                && matches!(
                    result.outcome,
                    VerificationOutcome::Passed | VerificationOutcome::Unknown
                )
        })
    {
        VerifierReportClassification::Passed
    } else {
        VerifierReportClassification::VerificationFailed
    }
}

fn launch_failure_report(error: String) -> VerifierReport {
    VerifierReport {
        passed: false,
        command_results: Vec::new(),
        concerns: vec![format!(
            "Verifier failed to launch; human review recommended: {}",
            error
        )],
        recommendation: "human-review".to_string(),
        classification: VerifierReportClassification::HumanReview,
    }
}

#[cfg(test)]
async fn run_verification_command(
    gcx: Arc<GlobalContext>,
    worktree: &Path,
    command: &str,
) -> VerificationResult {
    let policy = VerifyCommandPolicy::load(gcx.clone()).await;
    run_verification_command_with_policy(gcx, worktree, command, &policy).await
}

async fn run_verification_command_with_policy(
    gcx: Arc<GlobalContext>,
    worktree: &Path,
    command: &str,
    policy: &VerifyCommandPolicy,
) -> VerificationResult {
    let mut runner = SystemVerificationCommandRunner { gcx };
    run_verification_command_with_runner(worktree, command, policy, &mut runner).await
}

async fn run_verification_command_with_runner<R: VerificationCommandRunner>(
    worktree: &Path,
    command: &str,
    policy: &VerifyCommandPolicy,
    runner: &mut R,
) -> VerificationResult {
    let parsed = match parse_verification_argv(command) {
        Ok(parsed) => parsed,
        Err(reason) => {
            return VerificationResult {
                command: command.to_string(),
                exit_code: None,
                passed: false,
                output_tail: format!("Cannot run as a command: {}", reason),
                outcome: VerificationOutcome::Rejected,
            };
        }
    };
    if let Err(reason) = policy.check(command) {
        return VerificationResult {
            command: command.to_string(),
            exit_code: None,
            passed: false,
            output_tail: format!("Denied by shell policy: {}", reason),
            outcome: VerificationOutcome::PolicyDenied,
        };
    }
    runner
        .run(worktree, command, parsed.cwd, parsed.env, parsed.argv)
        .await
}

async fn run_verification_argv(
    gcx: Arc<GlobalContext>,
    worktree: &Path,
    command: &str,
    cwd: Option<PathBuf>,
    env: Vec<(String, String)>,
    argv: Vec<String>,
) -> VerificationResult {
    run_verification_argv_impl(
        gcx,
        worktree,
        command,
        cwd,
        env,
        argv,
        VERIFY_TIMEOUT,
        DRAIN_TIMEOUT,
    )
    .await
}

pub(crate) async fn run_verification_argv_impl(
    gcx: Arc<GlobalContext>,
    worktree: &Path,
    command: &str,
    cwd: Option<PathBuf>,
    env: Vec<(String, String)>,
    argv: Vec<String>,
    timeout: Duration,
    drain_timeout: Duration,
) -> VerificationResult {
    if argv.is_empty() {
        return VerificationResult {
            command: command.to_string(),
            exit_code: None,
            passed: false,
            output_tail: "empty verification command".to_string(),
            outcome: VerificationOutcome::Rejected,
        };
    };
    let effective_cwd = cwd.map_or_else(|| worktree.to_path_buf(), |cwd| worktree.join(cwd));
    if let Err(reason) = check_cwd_in_worktree(worktree, &effective_cwd).await {
        return VerificationResult {
            command: command.to_string(),
            exit_code: None,
            passed: false,
            output_tail: reason,
            outcome: VerificationOutcome::InfrastructureFailed,
        };
    }
    let request = match build_exec_request(
        gcx.clone(),
        CommandPolicyInput {
            source: ExecSource::Verifier,
            command: CommandKind::Argv(&argv),
            cwd: Some(effective_cwd),
            env: env.into_iter().collect::<HashMap<String, String>>(),
            chat_mode: None,
            escalation: None,
        },
    )
    .await
    {
        Ok(policy) => policy
            .request
            .with_timeout(timeout)
            .with_output_drain_timeout(drain_timeout)
            .with_transcript_limit(MAX_OUTPUT_CAPTURE_BYTES * 2),
        Err(error) => {
            return VerificationResult {
                command: command.to_string(),
                exit_code: None,
                passed: false,
                output_tail: format!("failed to spawn command: {}", error.message),
                outcome: VerificationOutcome::PolicyDenied,
            };
        }
    };
    let result = match gcx.exec_registry.spawn(request).await {
        Ok(result) => result,
        Err(error) => {
            return VerificationResult {
                command: command.to_string(),
                exit_code: None,
                passed: false,
                output_tail: format!("failed to spawn command: {}", error),
                outcome: VerificationOutcome::InfrastructureFailed,
            };
        }
    };
    let read = gcx
        .exec_registry
        .read(&result.snapshot.meta.process_id, 0, None)
        .await;
    let mut stdout = String::new();
    let mut stderr = String::new();
    for chunk in read.chunks {
        match chunk.stream {
            ExecOutputStream::Stdout | ExecOutputStream::Combined => stdout.push_str(&chunk.text),
            ExecOutputStream::Stderr => stderr.push_str(&chunk.text),
        }
    }
    let mut output = format!("{stdout}{stderr}");
    let (exit_code, passed, outcome) = match &result.snapshot.status {
        ExecStatus::Exited { exit_code } => (
            *exit_code,
            exit_code == &Some(0),
            if exit_code == &Some(0) {
                VerificationOutcome::Passed
            } else {
                VerificationOutcome::CommandFailed
            },
        ),
        ExecStatus::SandboxLauncherFailed { exit_code } => {
            output.push_str(&format!(
                "sandbox launcher failed before command execution with exit code {exit_code}"
            ));
            (
                Some(*exit_code),
                false,
                VerificationOutcome::InfrastructureFailed,
            )
        }
        ExecStatus::TimedOut => {
            output.push_str(&format!(
                "command timed out after {} seconds",
                timeout.as_secs()
            ));
            (None, false, VerificationOutcome::InfrastructureFailed)
        }
        ExecStatus::Failed { message } => {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(message);
            (None, false, VerificationOutcome::InfrastructureFailed)
        }
        ExecStatus::Killed | ExecStatus::Starting | ExecStatus::Running => {
            (None, false, VerificationOutcome::InfrastructureFailed)
        }
    };
    VerificationResult {
        command: command.to_string(),
        exit_code,
        passed,
        output_tail: tail_chars(&output, MAX_OUTPUT_TAIL_CHARS),
        outcome,
    }
}

fn verifier_prompt(card: &BoardCard, commands: &[VerificationResult], diff: &str) -> String {
    format!(
        "Review this completed task card. Return concise concerns only. If the changed files look safe and commands passed, answer exactly PASS.\n\nCard: {} - {}\n\nInstructions:\n{}\n\nFinal report:\n{}\n\nCommand results:\n{}\n\nChanged files:\n{}",
        card.id,
        card.title,
        card.instructions,
        card.final_report.as_deref().unwrap_or(""),
        serde_json::to_string_pretty(commands).unwrap_or_default(),
        diff
    )
}

async fn run_verifier_review(
    gcx: Arc<GlobalContext>,
    prompt: String,
    parent_chat_id: Option<&str>,
) -> Result<Vec<String>, String> {
    let model = resolve_verifier_model(gcx.clone()).await?;
    let config = crate::subchat::SubchatConfig {
        tool_name: "verifier".to_string(),
        stateful: false,
        autonomous_no_confirm: true,
        auto_approve_editing_tools: false,
        auto_approve_dangerous_commands: false,
        chat_id: None,
        title: None,
        parent_id: None,
        link_type: None,
        root_chat_id: None,
        tools: crate::subchat::ToolsPolicy::None,
        max_steps: 1,
        prepend_system_prompt: false,
        wrap_up: None,
        task_meta: None,
        worktree: None,
        model,
        mode: "agent".to_string(),
        n_ctx: 32_000,
        max_new_tokens: 1024,
        temperature: Some(0.0),
        reasoning_effort: None,
        cache_control: crate::llm::params::CacheControl::Ephemeral,
        parent_tool_call_id: None,
        parent_subchat_tx: None,
        abort_flag: None,
        background_agent_id: None,
        subchat_depth: 1,
        final_step_force_answer: false,
        buddy_meta: None,
        step_progress: None,
        trace_parent: crate::subchat::TraceParent::from_parts(parent_chat_id, None),
    };
    let messages = vec![event(
        EventSubkind::VerifierReport,
        VERIFIER_SOURCE,
        json!({ "kind": "verifier_review_prompt" }),
        prompt,
    )];
    let result = crate::subchat::run_subchat(gcx, messages, config).await?;
    let answer = result
        .messages
        .iter()
        .rev()
        .find(|message| message.role == "assistant")
        .map(|message| message.content.content_text_only())
        .unwrap_or_default();
    Ok(parse_review_concerns(&answer))
}

async fn resolve_verifier_model(gcx: Arc<GlobalContext>) -> Result<String, String> {
    let caps = try_load_caps_quickly_if_not_present(gcx, 0)
        .await
        .map_err(|e| e.message.clone())?;
    if !caps.defaults.chat_light_model.is_empty() {
        return Ok(caps.defaults.chat_light_model.clone());
    }
    if !caps.defaults.chat_default_model.is_empty() {
        return Ok(caps.defaults.chat_default_model.clone());
    }
    Err("no light/default model configured for verifier".to_string())
}

fn parse_review_concerns(answer: &str) -> Vec<String> {
    let trimmed = answer.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("pass") {
        return Vec::new();
    }
    trimmed
        .lines()
        .map(|line| line.trim().trim_start_matches(['-', '*', ' ']).trim())
        .filter(|line| !line.is_empty() && !line.eq_ignore_ascii_case("pass"))
        .map(str::to_string)
        .collect()
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
    use crate::tasks::types::{FinalReport, ScopeGuardMode, TaskBoard, TaskMeta, TaskStatus};

    #[derive(Default)]
    struct MockVerificationRunner {
        calls: Vec<(
            PathBuf,
            String,
            Option<PathBuf>,
            Vec<(String, String)>,
            Vec<String>,
        )>,
    }

    #[async_trait]
    impl VerificationCommandRunner for MockVerificationRunner {
        async fn run(
            &mut self,
            worktree: &Path,
            command: &str,
            cwd: Option<PathBuf>,
            env: Vec<(String, String)>,
            argv: Vec<String>,
        ) -> VerificationResult {
            self.calls
                .push((worktree.to_path_buf(), command.to_string(), cwd, env, argv));
            VerificationResult {
                command: command.to_string(),
                exit_code: Some(0),
                passed: true,
                output_tail: "ok".to_string(),
                outcome: VerificationOutcome::Unknown,
            }
        }
    }

    fn card(instructions: &str) -> BoardCard {
        BoardCard {
            id: "T-verify".to_string(),
            title: "Verifier card".to_string(),
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
            team_members: vec![],
            target_files: Vec::new(),
            scope_guard_mode: ScopeGuardMode::Off,
        }
    }

    fn verifier_report(passed: bool) -> VerifierReport {
        VerifierReport {
            passed,
            command_results: vec![VerificationResult {
                command: "cargo test verifier".to_string(),
                exit_code: Some(if passed { 0 } else { 1 }),
                passed,
                output_tail: "ok".to_string(),
                outcome: VerificationOutcome::Unknown,
            }],
            concerns: if passed {
                Vec::new()
            } else {
                vec!["failed".to_string()]
            },
            recommendation: if passed { "merge" } else { "fix-needed" }.to_string(),
            classification: if passed {
                VerifierReportClassification::Passed
            } else {
                VerifierReportClassification::VerificationFailed
            },
        }
    }

    async fn write_task(root: &Path, gcx: Arc<GlobalContext>, card: BoardCard) {
        let task_id = "task-1";
        let task_dir = root.join(".refact").join("tasks").join(task_id);
        tokio::fs::create_dir_all(&task_dir).await.unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.to_path_buf()];
        let now = Utc::now().to_rfc3339();
        let meta = TaskMeta {
            schema_version: 1,
            id: task_id.to_string(),
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
        storage::save_task_meta(gcx.clone(), task_id, &meta)
            .await
            .unwrap();
        storage::save_board(
            gcx,
            task_id,
            &TaskBoard {
                cards: vec![card],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }

    #[test]
    fn verifier_commands_include_acceptance_verify_lines() {
        let card = card(
            "## Acceptance Criteria\n- verifier.rs created\n- Verify: `cargo test --lib -p refact-lsp -- verifier merge_agent`",
        );

        assert_eq!(
            verification_commands(&card),
            vec!["cargo test --lib -p refact-lsp -- verifier merge_agent".to_string()]
        );
    }

    #[test]
    fn verifier_commands_include_structured_final_report_commands() {
        let mut card = card("## Acceptance Criteria\n- [ ] done");
        card.final_report_structured = Some(FinalReport {
            verification: vec![VerificationResult {
                command: "cargo test --lib -p refact-lsp -- verifier".to_string(),
                exit_code: Some(0),
                passed: true,
                output_tail: "ok".to_string(),
                outcome: VerificationOutcome::Unknown,
            }],
            ..Default::default()
        });

        assert_eq!(
            verification_commands(&card),
            vec!["cargo test --lib -p refact-lsp -- verifier".to_string()]
        );
    }

    #[test]
    fn verifier_status_records_pass_and_fail() {
        let mut pass_card = card("");
        let pass = VerifierReport {
            passed: true,
            recommendation: "merge".to_string(),
            classification: VerifierReportClassification::Passed,
            ..Default::default()
        };
        append_verifier_status(&mut pass_card, &pass);
        assert_eq!(pass_card.status_updates[0].message, "Verifier: PASS");

        let mut fail_card = card("");
        let fail = VerifierReport {
            passed: false,
            concerns: vec!["command failed".to_string()],
            recommendation: "fix-needed".to_string(),
            classification: VerifierReportClassification::VerificationFailed,
            ..Default::default()
        };
        append_verifier_status(&mut fail_card, &fail);
        assert_eq!(
            fail_card.status_updates[0].message,
            "Verifier: FAIL — command failed"
        );
    }

    #[test]
    fn launch_failure_report_returns_passed_false() {
        let report = launch_failure_report("no model configured".to_string());

        assert!(!report.passed);
        assert_eq!(report.recommendation, "human-review");
        assert!(report.command_results.is_empty());
        assert!(report.concerns[0].contains("Verifier failed to launch"));
        assert!(report.concerns[0].contains("no model configured"));
    }

    #[test]
    fn typed_classification_does_not_infer_state_from_concern_text() {
        let infrastructure = VerificationResult {
            command: "cargo test".to_string(),
            passed: false,
            outcome: VerificationOutcome::InfrastructureFailed,
            ..Default::default()
        };
        assert_eq!(
            classify_verifier_report(false, &[infrastructure], false, false),
            VerifierReportClassification::HumanReview
        );
        let passed = VerificationResult {
            command: "cargo test".to_string(),
            passed: true,
            outcome: VerificationOutcome::Passed,
            ..Default::default()
        };
        assert_eq!(
            classify_verifier_report(false, &[passed.clone()], true, false),
            VerifierReportClassification::HumanReview
        );
        assert_eq!(
            classify_verifier_report(false, &[passed], false, true),
            VerifierReportClassification::VerificationFailed
        );
    }

    #[test]
    fn typed_classification_requires_commands_and_real_success() {
        assert_eq!(
            classify_verifier_report(true, &[], false, false),
            VerifierReportClassification::VerificationFailed
        );
        let result = VerificationResult {
            command: "cargo test".to_string(),
            passed: false,
            outcome: VerificationOutcome::CommandFailed,
            ..Default::default()
        };
        assert_eq!(
            classify_verifier_report(false, &[result], false, false),
            VerifierReportClassification::VerificationFailed
        );
    }

    #[test]
    fn mock_verifier_passed_case_recommends_merge() {
        let report = VerifierReport {
            passed: true,
            command_results: vec![VerificationResult {
                command: "cargo test".to_string(),
                exit_code: Some(0),
                passed: true,
                output_tail: "ok".to_string(),
                outcome: VerificationOutcome::Unknown,
            }],
            concerns: Vec::new(),
            recommendation: "merge".to_string(),
            classification: VerifierReportClassification::Passed,
        };

        assert!(report.passed);
        assert_eq!(report.recommendation, "merge");
    }

    #[test]
    fn mock_verifier_failed_case_recommends_fix_needed() {
        let report = VerifierReport {
            passed: false,
            command_results: vec![VerificationResult {
                command: "cargo test".to_string(),
                exit_code: Some(1),
                passed: false,
                output_tail: "failed".to_string(),
                outcome: VerificationOutcome::Unknown,
            }],
            concerns: vec!["Verification command failed: cargo test".to_string()],
            recommendation: "fix-needed".to_string(),
            classification: VerifierReportClassification::VerificationFailed,
        };

        assert!(!report.passed);
        assert_eq!(report.recommendation, "fix-needed");
    }

    #[tokio::test]
    async fn store_verifier_report_lands_when_expected_card_state_matches() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut card = card("## Acceptance Criteria\n- Verify: `cargo test verifier`");
        card.id = "T-1".to_string();
        write_task(temp.path(), gcx.clone(), card).await;
        let expected = load_expected_card_state(gcx.clone(), "task-1", "T-1")
            .await
            .unwrap();

        let stored = store_verifier_report(
            gcx.clone(),
            "task-1",
            "T-1",
            Some(&expected),
            verifier_report(true),
        )
        .await
        .unwrap();

        assert!(stored);
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let card = board.get_card("T-1").unwrap();
        assert!(card.verifier_report.as_ref().unwrap().passed);
        assert!(card
            .status_updates
            .iter()
            .any(|update| update.message == "Verifier: PASS"));
    }

    #[tokio::test]
    async fn store_verifier_report_noops_when_card_restarted() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut card = card("## Acceptance Criteria\n- Verify: `cargo test verifier`");
        card.id = "T-1".to_string();
        card.agent_chat_id = Some("agent-old".to_string());
        write_task(temp.path(), gcx.clone(), card).await;
        let expected = load_expected_card_state(gcx.clone(), "task-1", "T-1")
            .await
            .unwrap();

        storage::update_board_atomic(gcx.clone(), "task-1", |board| {
            let card = board.get_card_mut("T-1").unwrap();
            card.column = "doing".to_string();
            card.agent_chat_id = Some("agent-new".to_string());
            card.assignee = Some("agent-new".to_string());
            card.final_report = None;
            card.final_report_structured = None;
            Ok(())
        })
        .await
        .unwrap();

        let stored = store_verifier_report(
            gcx.clone(),
            "task-1",
            "T-1",
            Some(&expected),
            verifier_report(false),
        )
        .await
        .unwrap();

        assert!(!stored);
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let card = board.get_card("T-1").unwrap();
        assert!(card.verifier_report.is_none());
        assert_eq!(card.column, "doing");
        assert_eq!(card.agent_chat_id.as_deref(), Some("agent-new"));
    }

    #[tokio::test]
    async fn store_verifier_report_noops_when_card_cleaned_up_after_merge() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut card = card("## Acceptance Criteria\n- Verify: `cargo test verifier`");
        card.id = "T-1".to_string();
        card.agent_branch = Some("refact/task/task-1/card/T-1/agent".to_string());
        card.agent_worktree = Some(temp.path().join("agent").to_string_lossy().to_string());
        card.agent_worktree_name = Some("wt-1".to_string());
        write_task(temp.path(), gcx.clone(), card).await;
        let expected = load_expected_card_state(gcx.clone(), "task-1", "T-1")
            .await
            .unwrap();

        storage::update_board_atomic(gcx.clone(), "task-1", |board| {
            let card = board.get_card_mut("T-1").unwrap();
            card.agent_branch = None;
            card.agent_worktree = None;
            card.agent_worktree_name = None;
            Ok(())
        })
        .await
        .unwrap();

        let stored = store_verifier_report(
            gcx.clone(),
            "task-1",
            "T-1",
            Some(&expected),
            verifier_report(false),
        )
        .await
        .unwrap();

        assert!(!stored);
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let card = board.get_card("T-1").unwrap();
        assert!(card.verifier_report.is_none());
        assert!(card.agent_worktree.is_none());
        assert!(card.agent_worktree_name.is_none());
    }

    #[tokio::test]
    async fn store_verifier_report_noops_when_report_already_changed() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut card = card("## Acceptance Criteria\n- Verify: `cargo test verifier`");
        card.id = "T-1".to_string();
        write_task(temp.path(), gcx.clone(), card).await;
        let expected = load_expected_card_state(gcx.clone(), "task-1", "T-1")
            .await
            .unwrap();

        store_verifier_report(gcx.clone(), "task-1", "T-1", None, verifier_report(true))
            .await
            .unwrap();

        let stored = store_verifier_report(
            gcx.clone(),
            "task-1",
            "T-1",
            Some(&expected),
            verifier_report(false),
        )
        .await
        .unwrap();

        assert!(!stored);
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let card = board.get_card("T-1").unwrap();
        assert!(card.verifier_report.as_ref().unwrap().passed);
    }

    #[tokio::test]
    async fn verifier_runs_argv_not_shell() {
        let temp = tempfile::tempdir().unwrap();
        let mut runner = MockVerificationRunner::default();

        let result = run_verification_command_with_runner(
            temp.path(),
            "cd refact-agent/engine && cargo check",
            &VerifyCommandPolicy::permissive(),
            &mut runner,
        )
        .await;

        assert!(result.passed);
        assert_eq!(runner.calls.len(), 1);
        assert_eq!(runner.calls[0].0, temp.path());
        assert_eq!(runner.calls[0].1, "cd refact-agent/engine && cargo check");
        assert_eq!(
            runner.calls[0].2,
            Some(PathBuf::from("refact-agent/engine"))
        );
        assert!(runner.calls[0].3.is_empty());
        assert_eq!(runner.calls[0].4, vec!["cargo", "check"]);
    }

    #[tokio::test]
    async fn verifier_threads_env_prefix_to_the_runner() {
        let temp = tempfile::tempdir().unwrap();
        let mut runner = MockVerificationRunner::default();

        let result = run_verification_command_with_runner(
            temp.path(),
            "cd ui && FLEXUS_PYTHON=/abs/py pnpm typecheck",
            &VerifyCommandPolicy::permissive(),
            &mut runner,
        )
        .await;

        assert!(result.passed, "{}", result.output_tail);
        assert_eq!(runner.calls[0].2, Some(PathBuf::from("ui")));
        assert_eq!(
            runner.calls[0].3,
            vec![("FLEXUS_PYTHON".to_string(), "/abs/py".to_string())]
        );
        assert_eq!(runner.calls[0].4, vec!["pnpm", "typecheck"]);
    }

    #[tokio::test]
    async fn verifier_runs_previously_unsupported_binaries() {
        let temp = tempfile::tempdir().unwrap();
        let mut runner = MockVerificationRunner::default();

        for command in [
            "dotnet test",
            "go test ./...",
            "make check",
            "gradlew build",
        ] {
            let result = run_verification_command_with_runner(
                temp.path(),
                command,
                &VerifyCommandPolicy::permissive(),
                &mut runner,
            )
            .await;
            assert!(result.passed, "{command}: {}", result.output_tail);
        }
        assert_eq!(runner.calls.len(), 4);
    }

    #[tokio::test]
    async fn verifier_denies_commands_matching_shell_policy_deny_rules() {
        let temp = tempfile::tempdir().unwrap();
        let mut runner = MockVerificationRunner::default();
        let policy = VerifyCommandPolicy::from_shell_policy(
            &crate::tools::shell_gate::ShellGatePolicy::default(),
        );

        let result = run_verification_command_with_runner(
            temp.path(),
            "sudo rm -rf /",
            &policy,
            &mut runner,
        )
        .await;

        assert!(!result.passed);
        assert_eq!(result.outcome, VerificationOutcome::PolicyDenied);
        assert!(
            result.output_tail.starts_with("Denied by shell policy:"),
            "{}",
            result.output_tail
        );
        assert!(runner.calls.is_empty());
    }

    fn command_result(
        command: &str,
        passed: bool,
        outcome: VerificationOutcome,
    ) -> VerificationResult {
        VerificationResult {
            command: command.to_string(),
            exit_code: None,
            passed,
            output_tail: String::new(),
            outcome,
        }
    }

    #[test]
    fn rejected_and_denied_commands_classify_as_human_review_not_verification_failure() {
        for outcome in [
            VerificationOutcome::Rejected,
            VerificationOutcome::PolicyDenied,
            VerificationOutcome::InfrastructureFailed,
        ] {
            let results = vec![command_result("dotnet test", false, outcome.clone())];
            assert_eq!(
                classify_verifier_report(false, &results, false, false),
                VerifierReportClassification::HumanReview,
                "{outcome:?} must be infrastructure, not a verification failure"
            );
        }
    }

    #[test]
    fn genuine_command_failure_still_classifies_as_verification_failure() {
        let results = vec![command_result(
            "cargo test",
            false,
            VerificationOutcome::CommandFailed,
        )];
        assert_eq!(
            classify_verifier_report(false, &results, false, false),
            VerifierReportClassification::VerificationFailed
        );

        let mixed = vec![
            command_result("cargo build", true, VerificationOutcome::Passed),
            command_result("cargo test", false, VerificationOutcome::CommandFailed),
        ];
        assert_eq!(
            classify_verifier_report(false, &mixed, false, false),
            VerifierReportClassification::VerificationFailed
        );
    }

    #[test]
    fn passed_and_rejected_mix_is_human_review() {
        let results = vec![
            command_result("cargo build", true, VerificationOutcome::Passed),
            command_result("dotnet test", false, VerificationOutcome::Rejected),
        ];
        assert_eq!(
            classify_verifier_report(false, &results, false, false),
            VerifierReportClassification::HumanReview
        );
    }

    #[tokio::test]
    async fn verifier_yolo_mode_allows_arbitrary_binary() {
        let temp = tempfile::tempdir().unwrap();
        let mut runner = MockVerificationRunner::default();
        let policy =
            VerifyCommandPolicy::from_shell_policy(&crate::tools::shell_gate::ShellGatePolicy {
                mode: crate::tools::shell_gate::ApprovalMode::Yolo,
                ..crate::tools::shell_gate::ShellGatePolicy::default()
            });

        let result = run_verification_command_with_runner(
            temp.path(),
            "mycompany-verify --all",
            &policy,
            &mut runner,
        )
        .await;

        assert!(result.passed, "{}", result.output_tail);
        assert_eq!(runner.calls[0].4, vec!["mycompany-verify", "--all"]);
    }

    #[tokio::test]
    async fn verifier_rejects_shell_syntax() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;

        let result = run_verification_command(gcx, temp.path(), "cargo test | tee f").await;

        assert!(!result.passed);
        assert_eq!(result.outcome, VerificationOutcome::Rejected);
        assert!(result.output_tail.starts_with("Cannot run as a command:"));
    }

    #[tokio::test]
    async fn agent_finish_spawns_verifier_through_helper() {
        let mut card = card("");
        card.id = "T-missing".to_string();
        let expected_state = ExpectedCardState::from_card(0, &card);
        let gcx = crate::global_context::tests::make_test_gcx().await;

        schedule_card_verifier_after_finish(
            gcx.clone(),
            "missing-task".to_string(),
            "T-missing".to_string(),
            expected_state,
        )
        .await;

        assert_eq!(gcx.card_verifier_tasks.lock().await.len(), 1);
        shutdown_card_verifiers(gcx.clone()).await;
        assert!(gcx.card_verifier_tasks.lock().await.is_empty());
    }

    #[tokio::test]
    async fn verifier_is_not_scheduled_after_shutdown_starts() {
        let mut card = card("");
        card.id = "T-stopped".to_string();
        let expected_state = ExpectedCardState::from_card(0, &card);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        gcx.shutdown_flag.store(true, Ordering::SeqCst);

        schedule_card_verifier_after_finish(
            gcx.clone(),
            "missing-task".to_string(),
            "T-stopped".to_string(),
            expected_state,
        )
        .await;

        assert!(gcx.card_verifier_tasks.lock().await.is_empty());
    }

    #[tokio::test]
    async fn cwd_outside_worktree_is_rejected() {
        let worktree = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;

        let result = run_verification_argv_impl(
            gcx,
            worktree.path(),
            "cargo check",
            Some(outside.path().to_path_buf()),
            Vec::new(),
            vec!["cargo".to_string(), "check".to_string()],
            Duration::from_secs(30),
            DRAIN_TIMEOUT,
        )
        .await;

        assert!(!result.passed);
        assert!(result.output_tail.contains("outside the worktree"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stdin_null_does_not_hang() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;

        let result = tokio::time::timeout(
            Duration::from_secs(5),
            run_verification_argv_impl(
                gcx,
                temp.path(),
                "cat",
                None,
                Vec::new(),
                vec!["cat".to_string()],
                Duration::from_secs(30),
                DRAIN_TIMEOUT,
            ),
        )
        .await
        .expect("should not hang with stdin=null");

        assert!(result.passed);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_kills_process_group() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;

        let result = run_verification_argv_impl(
            gcx,
            temp.path(),
            "sleep 30",
            None,
            Vec::new(),
            vec!["sleep".to_string(), "30".to_string()],
            Duration::from_millis(200),
            DRAIN_TIMEOUT,
        )
        .await;

        assert!(!result.passed);
        assert!(result.output_tail.contains("timed out"));
        assert!(result.exit_code.is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn drain_times_out_when_descendant_holds_pipe() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;

        // bash spawns a background sleep that inherits the stdout/stderr pipes and then exits;
        // without the drain timeout the verifier hangs waiting for EOF from the background sleep
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            run_verification_argv_impl(
                gcx,
                temp.path(),
                "bash -c 'sleep 60 &'",
                None,
                Vec::new(),
                vec![
                    "bash".to_string(),
                    "-c".to_string(),
                    "sleep 60 &".to_string(),
                ],
                Duration::from_secs(30),
                Duration::from_millis(500),
            ),
        )
        .await
        .expect("verifier must not hang when descendant holds pipe open");

        assert!(result.output_tail.contains("drain timed out"));
        assert!(!result.passed);
    }
}
