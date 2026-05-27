use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use tokio::process::Command;

use crate::chat::verify_cmd::parse_restricted_argv;
use crate::global_context::GlobalContext;
use crate::tasks::storage;
use crate::tasks::types::{BoardCard, BoardColumn, StatusUpdate};

pub const DEFAULT_POST_MERGE_CHECK_TIMEOUT_SECS: u64 = 300;
const MAX_OUTPUT_TAIL_CHARS: usize = 6000;

#[derive(Clone, Debug)]
pub struct PostMergeCheckRequest {
    pub task_id: String,
    pub card_id: String,
    pub workspace_root: PathBuf,
    pub enabled: bool,
    pub timeout: Duration,
    pub expected_merge_commit: String,
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PostMergeCommand {
    Verify {
        cwd: Option<PathBuf>,
        argv: Vec<String>,
    },
    Git(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PostMergeCommandOutput {
    pub exit_code: Option<i32>,
    pub success: bool,
    pub output: String,
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

struct SystemPostMergeCommandRunner;

#[async_trait]
impl PostMergeCommandRunner for SystemPostMergeCommandRunner {
    async fn run(
        &mut self,
        workspace_root: &Path,
        command: PostMergeCommand,
        timeout: Duration,
    ) -> PostMergeCommandOutput {
        match command {
            PostMergeCommand::Verify { cwd, argv } => {
                let label = argv.join(" ");
                let Some(program) = argv.first() else {
                    return PostMergeCommandOutput {
                        exit_code: None,
                        success: false,
                        output: "empty verification command".to_string(),
                    };
                };
                let mut cmd = Command::new(program);
                cmd.args(&argv[1..]);
                cmd.current_dir(cwd.map_or_else(
                    || workspace_root.to_path_buf(),
                    |cwd| workspace_root.join(cwd),
                ));
                run_command(cmd, &label, timeout).await
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

async fn run_command(
    mut command: Command,
    command_label: &str,
    timeout: Duration,
) -> PostMergeCommandOutput {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    match tokio::time::timeout(timeout, command.output()).await {
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
            }
        }
        Ok(Err(error)) => PostMergeCommandOutput {
            exit_code: None,
            success: false,
            output: format!("failed to execute command '{}': {}", command_label, error),
        },
        Err(_) => PostMergeCommandOutput {
            exit_code: None,
            success: false,
            output: format!(
                "command '{}' timed out after {} seconds",
                command_label,
                timeout.as_secs()
            ),
        },
    }
}

pub async fn post_merge_check(
    gcx: Arc<GlobalContext>,
    request: PostMergeCheckRequest,
) -> Result<PostMergeCheckResult, String> {
    let mut runner = SystemPostMergeCommandRunner;
    post_merge_check_with_runner(gcx, request, &mut runner).await
}

pub async fn post_merge_check_with_runner<R: PostMergeCommandRunner>(
    gcx: Arc<GlobalContext>,
    request: PostMergeCheckRequest,
    runner: &mut R,
) -> Result<PostMergeCheckResult, String> {
    if !request.enabled {
        return Ok(skipped_result("auto_revert disabled"));
    }

    let board = storage::load_board(gcx.clone(), &request.task_id).await?;
    let card = board
        .get_card(&request.card_id)
        .ok_or_else(|| format!("Card {} not found", request.card_id))?
        .clone();
    let commands = extract_verification_commands(&card.instructions);
    if commands.is_empty() {
        return Ok(skipped_result("no verification command found"));
    }
    let mut rejected = None;
    let mut selected = None;
    for command in commands {
        match parse_restricted_argv(&command) {
            Ok((cwd, argv)) => {
                selected = Some((command, cwd, argv));
                break;
            }
            Err(reason) => {
                rejected.get_or_insert((command, reason));
            }
        }
    }
    let Some((command, cwd, argv)) = selected else {
        if let Some((command, reason)) = rejected {
            return Ok(rejected_result(command, reason));
        }
        return Ok(skipped_result(
            "no supported deterministic verification command found",
        ));
    };
    let verification = runner
        .run(
            &request.workspace_root,
            PostMergeCommand::Verify { cwd, argv },
            request.timeout,
        )
        .await;
    let exit_code = verification.exit_code;
    let verification_success = verification.success;
    let output_tail = tail_chars(&verification.output, MAX_OUTPUT_TAIL_CHARS);
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
            fix_card_id: Some(fix_card_id),
            skipped_reason: None,
            revert_skipped_reason: Some(reason.to_string()),
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
            fix_card_id: Some(fix_card_id),
            skipped_reason: None,
            revert_skipped_reason: Some(reason.to_string()),
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
            fix_card_id: Some(fix_card_id),
            skipped_reason: None,
            revert_skipped_reason: Some(reason.to_string()),
        });
    }

    let revert_output = runner
        .run(
            &request.workspace_root,
            PostMergeCommand::Git(vec![
                "revert".to_string(),
                "--no-edit".to_string(),
                expected_merge_commit.clone(),
            ]),
            request.timeout,
        )
        .await;
    if !revert_output.success {
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
        fix_card_id: Some(fix_card_id),
        skipped_reason: None,
        revert_skipped_reason: None,
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
    }
}

fn rejected_result(command: String, reason: String) -> PostMergeCheckResult {
    PostMergeCheckResult {
        checked: true,
        auto_reverted: false,
        command: Some(command),
        exit_code: None,
        output_tail: format!("Rejected by command filter: {}", reason),
        merge_commit: None,
        revert_commit: None,
        fix_card_id: None,
        skipped_reason: None,
        revert_skipped_reason: None,
    }
}

fn git_text(output: PostMergeCommandOutput) -> Option<String> {
    output
        .success
        .then(|| output.output.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub fn extract_verification_commands(instructions: &str) -> Vec<String> {
    let mut commands = Vec::new();
    let mut in_acceptance = false;
    let mut in_fence = false;
    for line in instructions.lines() {
        let trimmed = line.trim();
        let heading = trimmed.trim_start_matches('#').trim().to_lowercase();
        if trimmed.starts_with('#') {
            in_acceptance = heading.contains("acceptance criteria")
                || heading == "verify"
                || heading.contains("verification");
            continue;
        }
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if let Some(command) = parse_verify_line(trimmed) {
            push_unique(&mut commands, command);
            continue;
        }
        if in_acceptance && in_fence && !trimmed.is_empty() {
            push_unique(&mut commands, stripped_command(trimmed));
            continue;
        }
        if in_acceptance && looks_like_verification_command(trimmed) {
            push_unique(&mut commands, stripped_command(trimmed));
        }
    }
    commands
}

fn parse_verify_line(line: &str) -> Option<String> {
    let lower = line.to_lowercase();
    let index = lower.find("verify:")?;
    let command = &line[index + "verify:".len()..];
    Some(stripped_command(command)).filter(|command| !command.is_empty())
}

fn stripped_command(command: &str) -> String {
    let command = command.trim();
    if let Some(start) = command.find('`') {
        if let Some(end) = command[start + 1..].find('`') {
            return command[start + 1..start + 1 + end].trim().to_string();
        }
    }
    command
        .trim_matches('`')
        .trim_matches('"')
        .trim_end_matches('.')
        .trim()
        .to_string()
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

fn looks_like_verification_command(line: &str) -> bool {
    let line = line
        .trim_start_matches(['-', '*', ' ', '\t'])
        .trim_matches('`')
        .trim();
    line.starts_with("cargo ")
        || line.starts_with("npm test")
        || line.starts_with("npm run lint")
        || line.starts_with("cd ")
}

pub fn is_supported_deterministic_command(command: &str) -> bool {
    let command = command.trim();
    if command.is_empty()
        || command.contains(';')
        || command.contains('|')
        || command.contains('>')
        || command.contains('<')
        || command.contains('\n')
    {
        return false;
    }
    parse_restricted_argv(command).is_ok()
}

async fn store_regression_result(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    card_id: &str,
    command: &str,
    output_tail: &str,
    merge_commit: &str,
    revert_commit: &str,
    first_error: &str,
    revert_skipped_reason: Option<&str>,
    diagnostic: Option<&str>,
) -> Result<String, String> {
    let card_id_owned = card_id.to_string();
    let command_owned = command.to_string();
    let output_owned = output_tail.to_string();
    let merge_commit_owned = merge_commit.to_string();
    let revert_commit_owned = revert_commit.to_string();
    let first_error_owned = first_error.to_string();
    let revert_skipped_reason_owned = revert_skipped_reason.map(str::to_string);
    let diagnostic_owned = diagnostic.map(str::to_string);
    let (_, fix_card_id) = storage::update_board_atomic(gcx.clone(), task_id, move |board| {
        ensure_regressed_column(board);
        let source_card = board
            .get_card(&card_id_owned)
            .ok_or_else(|| format!("Card {} not found", card_id_owned))?
            .clone();
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
    .await?;
    let _ = storage::update_task_stats(gcx, task_id).await;
    Ok(fix_card_id)
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
        ab_variants: None,
        target_files: source_card.target_files.clone(),
        scope_guard_mode: source_card.scope_guard_mode,
        team_members: Vec::new(),
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

    use crate::tasks::types::{ScopeGuardMode, TaskBoard, TaskMeta, TaskStatus};

    #[derive(Default)]
    struct MockRunner {
        outputs: VecDeque<PostMergeCommandOutput>,
        calls: Vec<PostMergeCommand>,
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

    fn output(success: bool, exit_code: Option<i32>, text: &str) -> PostMergeCommandOutput {
        PostMergeCommandOutput {
            success,
            exit_code,
            output: text.to_string(),
        }
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
            ab_variants: None,
            target_files: vec!["src/lib.rs".to_string()],
            scope_guard_mode: ScopeGuardMode::Off,
            team_members: vec![],
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
        }
    }

    #[test]
    fn post_merge_check_extracts_verification_command() {
        let instructions = "## Acceptance Criteria\n- post_merge_check.rs implements logic\n- Verify: `cargo test --lib -p refact-lsp -- post_merge_check`";

        assert_eq!(
            extract_verification_commands(instructions),
            vec!["cargo test --lib -p refact-lsp -- post_merge_check".to_string()]
        );
    }

    #[test]
    fn post_merge_check_allows_only_supported_commands() {
        assert!(is_supported_deterministic_command("cargo test --lib"));
        assert!(is_supported_deterministic_command(
            "cd refact-agent/engine && npm test -- --run"
        ));
        assert!(is_supported_deterministic_command(
            "npm run lint -- --quiet"
        ));
        assert!(is_supported_deterministic_command("pytest -q"));
        assert!(!is_supported_deterministic_command(
            "cargo test && rm -rf target"
        ));
        assert!(!is_supported_deterministic_command("cargo test > out.txt"));
        assert!(!is_supported_deterministic_command(
            "cargo test && cd refact-agent/engine"
        ));
    }

    #[tokio::test]
    async fn post_merge_check_no_command_is_noop() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        write_task(gcx.clone(), temp.path(), card("No verify command")).await;
        let mut runner = MockRunner::default();

        let result = post_merge_check_with_runner(gcx.clone(), request(temp.path()), &mut runner)
            .await
            .unwrap();

        assert!(!result.checked);
        assert!(!result.auto_reverted);
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
                argv: vec!["cargo".to_string(), "check".to_string()]
            }]
        );
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
