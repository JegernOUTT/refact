use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use serde_json::json;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::chat::internal_roles::{event, EventSubkind};
use crate::chat::verifier_diff::{git_changed_files_summary, resolve_verifier_diff_base};
use crate::chat::verify_cmd::parse_restricted_argv;
use crate::global_context::{try_load_caps_quickly_if_not_present, GlobalContext};
use crate::tasks::storage;
use crate::tasks::types::{BoardCard, StatusUpdate, VerificationResult, VerifierReport};

const VERIFY_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_OUTPUT_TAIL_CHARS: usize = 4000;
const MAX_DIFF_LINES: usize = 200;
const VERIFIER_SOURCE: &str = "chat.verifier";

#[derive(Clone, Debug)]
pub struct VerifyCardRequest {
    pub task_id: String,
    pub card_id: String,
}

#[async_trait]
trait VerificationCommandRunner: Send {
    async fn run(
        &mut self,
        worktree: &Path,
        command: &str,
        cwd: Option<PathBuf>,
        argv: Vec<String>,
    ) -> VerificationResult;
}

struct SystemVerificationCommandRunner;

#[async_trait]
impl VerificationCommandRunner for SystemVerificationCommandRunner {
    async fn run(
        &mut self,
        worktree: &Path,
        command: &str,
        cwd: Option<PathBuf>,
        argv: Vec<String>,
    ) -> VerificationResult {
        run_verification_argv(worktree, command, cwd, argv).await
    }
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
    report: VerifierReport,
) -> Result<(), String> {
    let card_id = card_id.to_string();
    storage::update_board_atomic(gcx, task_id, move |board| {
        let card = board
            .get_card_mut(&card_id)
            .ok_or_else(|| format!("Card {} not found", card_id))?;
        card.verifier_report = Some(report.clone());
        append_verifier_status(card, &report);
        Ok(())
    })
    .await
    .map(|_| ())
}

pub async fn schedule_card_verifier(gcx: Arc<GlobalContext>, request: VerifyCardRequest) {
    tokio::spawn(async move {
        if let Err(error) = verify_card(gcx.clone(), request.clone()).await {
            let report = launch_failure_report(error);
            if let Err(store_error) =
                store_verifier_report(gcx, &request.task_id, &request.card_id, report).await
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

pub async fn schedule_card_verifier_after_finish(
    gcx: Arc<GlobalContext>,
    task_id: String,
    card_id: String,
) {
    schedule_card_verifier(gcx, VerifyCardRequest { task_id, card_id }).await;
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
    let worktree = card
        .agent_worktree
        .as_ref()
        .map(PathBuf::from)
        .ok_or_else(|| format!("Card {} has no agent worktree", card.id))?;
    if !worktree.is_dir() {
        return Err(format!(
            "Card {} worktree '{}' does not exist",
            card.id,
            worktree.display()
        ));
    }

    let commands = verification_commands(&card);
    let mut command_results = Vec::new();
    let mut concerns = Vec::new();

    if commands.is_empty() {
        concerns.push(
            "No verification commands found in card instructions or final report".to_string(),
        );
    }

    for command in commands {
        let result = run_verification_command(&worktree, &command).await;
        if !result.passed {
            concerns.push(format!("Verification command failed: {}", result.command));
        }
        command_results.push(result);
    }

    let diff_base = resolve_verifier_diff_base(task_meta.base_commit, task_meta.base_branch)?;
    let diff = git_changed_files_summary(&worktree, &diff_base, MAX_DIFF_LINES)
        .await
        .unwrap_or_else(|error| format!("diff unavailable: {}", error));
    let prompt = verifier_prompt(&card, &command_results, &diff);
    let model_concerns = run_verifier_review(gcx.clone(), prompt)
        .await
        .unwrap_or_else(|error| {
            vec![format!(
                "Verifier review subchat unavailable; human review recommended: {}",
                error
            )]
        });
    concerns.extend(model_concerns);

    let failed_commands = command_results.iter().any(|result| !result.passed);
    let review_blocked = concerns
        .iter()
        .any(|concern| !concern.to_lowercase().contains("human review recommended"));
    let passed = !failed_commands && !review_blocked;
    let recommendation = if passed {
        "merge"
    } else if failed_commands || review_blocked {
        "fix-needed"
    } else {
        "human-review"
    }
    .to_string();

    let report = VerifierReport {
        passed,
        command_results,
        concerns,
        recommendation,
    };
    store_verifier_report(gcx, &request.task_id, &request.card_id, report.clone()).await?;
    Ok(report)
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
    }
}

fn verification_commands(card: &BoardCard) -> Vec<String> {
    let mut commands = Vec::new();
    for command in commands_from_instructions(&card.instructions) {
        push_unique(&mut commands, command);
    }
    if let Some(report) = card.final_report_structured.as_ref() {
        for result in &report.verification {
            push_unique(&mut commands, result.command.clone());
        }
    }
    commands
}

fn push_unique(commands: &mut Vec<String>, command: String) {
    let command = command.trim();
    if command.is_empty() {
        return;
    }
    if !commands.iter().any(|existing| existing == command) {
        commands.push(command.to_string());
    }
}

fn commands_from_instructions(instructions: &str) -> Vec<String> {
    let lines = instructions.lines().collect::<Vec<_>>();
    let mut commands = Vec::new();
    let mut in_acceptance = false;
    let mut in_fence = false;
    let mut fence_lines: Vec<String> = Vec::new();

    for line in lines {
        let trimmed = line.trim();
        let heading = trimmed.trim_start_matches('#').trim().to_lowercase();
        if trimmed.starts_with('#') {
            in_acceptance = heading.contains("acceptance criteria") || heading.contains("verify");
            continue;
        }
        if !in_acceptance
            && (trimmed.eq_ignore_ascii_case("acceptance criteria")
                || trimmed.eq_ignore_ascii_case("verify:"))
        {
            in_acceptance = true;
            continue;
        }
        if !in_acceptance {
            continue;
        }
        if trimmed.starts_with("```") {
            if in_fence {
                for command in &fence_lines {
                    push_unique(&mut commands, command.clone());
                }
                fence_lines.clear();
                in_fence = false;
            } else {
                in_fence = true;
            }
            continue;
        }
        if in_fence {
            if !trimmed.is_empty() {
                fence_lines.push(trimmed.to_string());
            }
            continue;
        }
        if let Some(command) = parse_verify_line(trimmed) {
            push_unique(&mut commands, command);
        }
    }
    commands
}

fn parse_verify_line(line: &str) -> Option<String> {
    let line = line.trim_start_matches(['-', '*', ' ']).trim();
    let lower = line.to_lowercase();
    if let Some((_, command)) = line.split_once("Verify:") {
        return Some(command.trim().trim_matches('`').to_string()).filter(|s| !s.is_empty());
    }
    if let Some((_, command)) = line.split_once("verify:") {
        return Some(command.trim().trim_matches('`').to_string()).filter(|s| !s.is_empty());
    }
    if lower.contains("cargo ")
        || lower.contains("npm ")
        || lower.contains("pytest")
        || lower.contains("bun ")
    {
        return Some(line.trim_matches('`').to_string());
    }
    None
}

async fn run_verification_command(worktree: &Path, command: &str) -> VerificationResult {
    let mut runner = SystemVerificationCommandRunner;
    run_verification_command_with_runner(worktree, command, &mut runner).await
}

async fn run_verification_command_with_runner<R: VerificationCommandRunner>(
    worktree: &Path,
    command: &str,
    runner: &mut R,
) -> VerificationResult {
    let (cwd, argv) = match parse_restricted_argv(command) {
        Ok(parsed) => parsed,
        Err(reason) => {
            return VerificationResult {
                command: command.to_string(),
                exit_code: None,
                passed: false,
                output_tail: format!("Rejected by command filter: {}", reason),
            };
        }
    };
    runner.run(worktree, command, cwd, argv).await
}

async fn run_verification_argv(
    worktree: &Path,
    command: &str,
    cwd: Option<PathBuf>,
    argv: Vec<String>,
) -> VerificationResult {
    let Some(program) = argv.first() else {
        return VerificationResult {
            command: command.to_string(),
            exit_code: None,
            passed: false,
            output_tail: "empty verification command".to_string(),
        };
    };
    let mut child = match Command::new(program)
        .args(&argv[1..])
        .current_dir(cwd.map_or_else(|| worktree.to_path_buf(), |cwd| worktree.join(cwd)))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            return VerificationResult {
                command: command.to_string(),
                exit_code: None,
                passed: false,
                output_tail: format!("failed to spawn command: {}", error),
            };
        }
    };
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let stdout_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        if let Some(ref mut stdout) = stdout {
            let _ = stdout.read_to_end(&mut bytes).await;
        }
        bytes
    });
    let stderr_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        if let Some(ref mut stderr) = stderr {
            let _ = stderr.read_to_end(&mut bytes).await;
        }
        bytes
    });
    let status = match tokio::time::timeout(VERIFY_TIMEOUT, child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => {
            return VerificationResult {
                command: command.to_string(),
                exit_code: None,
                passed: false,
                output_tail: format!("failed to wait for command: {}", error),
            }
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return VerificationResult {
                command: command.to_string(),
                exit_code: None,
                passed: false,
                output_tail: format!(
                    "command timed out after {} seconds",
                    VERIFY_TIMEOUT.as_secs()
                ),
            };
        }
    };
    let stdout = stdout_task.await.unwrap_or_default();
    let stderr = stderr_task.await.unwrap_or_default();
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr)
    );
    VerificationResult {
        command: command.to_string(),
        exit_code: status.code(),
        passed: status.success(),
        output_tail: tail_chars(&output, MAX_OUTPUT_TAIL_CHARS),
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
) -> Result<Vec<String>, String> {
    let model = resolve_verifier_model(gcx.clone()).await?;
    let config = crate::subchat::SubchatConfig {
        tool_name: "verifier".to_string(),
        stateful: false,
        autonomous_no_confirm: true,
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
        parent_tool_call_id: None,
        parent_subchat_tx: None,
        abort_flag: None,
        subchat_depth: 1,
        buddy_meta: None,
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
    use crate::tasks::types::{FinalReport, ScopeGuardMode};

    #[derive(Default)]
    struct MockVerificationRunner {
        calls: Vec<(PathBuf, String, Option<PathBuf>, Vec<String>)>,
    }

    #[async_trait]
    impl VerificationCommandRunner for MockVerificationRunner {
        async fn run(
            &mut self,
            worktree: &Path,
            command: &str,
            cwd: Option<PathBuf>,
            argv: Vec<String>,
        ) -> VerificationResult {
            self.calls
                .push((worktree.to_path_buf(), command.to_string(), cwd, argv));
            VerificationResult {
                command: command.to_string(),
                exit_code: Some(0),
                passed: true,
                output_tail: "ok".to_string(),
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
            ab_variants: None,
            team_members: vec![],
            target_files: Vec::new(),
            scope_guard_mode: ScopeGuardMode::Off,
        }
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
            ..Default::default()
        };
        append_verifier_status(&mut pass_card, &pass);
        assert_eq!(pass_card.status_updates[0].message, "Verifier: PASS");

        let mut fail_card = card("");
        let fail = VerifierReport {
            passed: false,
            concerns: vec!["command failed".to_string()],
            recommendation: "fix-needed".to_string(),
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
    fn mock_verifier_passed_case_recommends_merge() {
        let report = VerifierReport {
            passed: true,
            command_results: vec![VerificationResult {
                command: "cargo test".to_string(),
                exit_code: Some(0),
                passed: true,
                output_tail: "ok".to_string(),
            }],
            concerns: Vec::new(),
            recommendation: "merge".to_string(),
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
            }],
            concerns: vec!["Verification command failed: cargo test".to_string()],
            recommendation: "fix-needed".to_string(),
        };

        assert!(!report.passed);
        assert_eq!(report.recommendation, "fix-needed");
    }

    #[tokio::test]
    async fn verifier_runs_argv_not_shell() {
        let temp = tempfile::tempdir().unwrap();
        let mut runner = MockVerificationRunner::default();

        let result = run_verification_command_with_runner(
            temp.path(),
            "cd refact-agent/engine && cargo check",
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
        assert_eq!(runner.calls[0].3, vec!["cargo", "check"]);
    }

    #[tokio::test]
    async fn verifier_rejects_shell_syntax() {
        let temp = tempfile::tempdir().unwrap();

        let result = run_verification_command(temp.path(), "cargo test | tee f").await;

        assert!(!result.passed);
        assert!(result
            .output_tail
            .starts_with("Rejected by command filter:"));
    }

    #[tokio::test]
    async fn agent_finish_spawns_verifier_through_helper() {
        let gcx = crate::global_context::tests::make_test_gcx().await;

        schedule_card_verifier_after_finish(
            gcx,
            "missing-task".to_string(),
            "T-missing".to_string(),
        )
        .await;
    }
}
