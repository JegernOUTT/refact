use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::global_context::GlobalContext;
use crate::tasks::storage;
use crate::tasks::types::BoardCard;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use crate::worktrees::service::WorktreeService;

const DEFAULT_MAX_LINES: usize = 300;
const GIT_DIFF_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentDiffMode {
    Stat,
    Unified,
    NameOnly,
}

impl AgentDiffMode {
    fn parse(value: Option<&Value>) -> Result<Self, String> {
        match value.and_then(|value| value.as_str()).unwrap_or("stat") {
            "stat" => Ok(Self::Stat),
            "unified" => Ok(Self::Unified),
            "name-only" => Ok(Self::NameOnly),
            other => Err(format!("Invalid mode: {}", other)),
        }
    }
}

pub struct ToolAgentDiff;

impl ToolAgentDiff {
    pub fn new() -> Self {
        Self
    }
}

fn required_string(args: &HashMap<String, Value>, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("Missing '{}'", key))
}

fn parse_max_lines(args: &HashMap<String, Value>) -> Result<usize, String> {
    let Some(value) = args.get("max_lines") else {
        return Ok(DEFAULT_MAX_LINES);
    };
    if value.is_null() {
        return Ok(DEFAULT_MAX_LINES);
    }
    let Some(n) = value.as_u64() else {
        return Err("max_lines must be a non-negative number".to_string());
    };
    usize::try_from(n).map_err(|_| "max_lines is too large".to_string())
}

async fn get_task_id(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    args: &HashMap<String, Value>,
) -> Result<String, String> {
    if let Some(id) = args.get("task_id").and_then(|v| v.as_str()) {
        return Ok(id.to_string());
    }
    let ccx_lock = ccx.lock().await;
    if let Some(ref meta) = ccx_lock.task_meta {
        return Ok(meta.task_id.clone());
    }
    storage::infer_task_id_from_chat_id(&ccx_lock.chat_id)
        .ok_or_else(|| "Missing 'task_id' (and chat is not bound to a task)".to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiffBase {
    refish: String,
    label: String,
}

fn present(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn resolve_base(
    worktree_commit: Option<String>,
    worktree_branch: Option<String>,
    task_meta_commit: Option<String>,
    task_meta_branch: Option<String>,
) -> Result<DiffBase, String> {
    if let Some(commit) = present(worktree_commit).or_else(|| present(task_meta_commit)) {
        return Ok(DiffBase {
            refish: commit.clone(),
            label: format!("commit {}", commit),
        });
    }
    if let Some(branch) = present(worktree_branch).or_else(|| present(task_meta_branch)) {
        return Ok(DiffBase {
            refish: branch.clone(),
            label: format!("branch {}", branch),
        });
    }
    Err("Task has no base commit or base branch set".to_string())
}

async fn base_from_worktree_meta(
    gcx: Arc<GlobalContext>,
    card: &BoardCard,
) -> (Option<String>, Option<String>) {
    let Some(worktree_name) = card.agent_worktree_name.as_ref() else {
        return (None, None);
    };
    let project_dirs = crate::files_correction::get_project_dirs(gcx.clone()).await;
    for source_root in project_dirs {
        let Ok(service) = WorktreeService::new(gcx.cache_dir.clone(), source_root) else {
            continue;
        };
        let Ok(registry) = service.load_registry().await else {
            continue;
        };
        if let Some(record) = registry
            .records
            .iter()
            .find(|record| record.meta.id == *worktree_name)
        {
            if record.meta.root.exists() {
                return (
                    record.meta.base_commit.clone(),
                    record.meta.base_branch.clone(),
                );
            }
        }
    }
    (None, None)
}

fn canonical_worktree(card: &BoardCard) -> Result<PathBuf, String> {
    let worktree = card
        .agent_worktree
        .as_ref()
        .ok_or_else(|| format!("Card {} has no agent worktree", card.id))?;
    let path = Path::new(worktree);
    if !path.exists() {
        return Err(format!(
            "Agent worktree '{}' for card {} does not exist",
            worktree, card.id
        ));
    }
    std::fs::canonicalize(path).map_err(|e| {
        format!(
            "Failed to canonicalize agent worktree '{}' for card {}: {}",
            worktree, card.id, e
        )
    })
}

fn join_reader(
    handle: std::thread::JoinHandle<std::io::Result<Vec<u8>>>,
    stream_name: &str,
) -> Result<Vec<u8>, String> {
    handle
        .join()
        .map_err(|_| format!("Failed to capture git {}", stream_name))?
        .map_err(|e| format!("Failed to capture git {}: {}", stream_name, e))
}

fn run_git(worktree: &Path, args: &[&str], deadline: Instant) -> Result<String, String> {
    if Instant::now() >= deadline {
        return Err(format!(
            "git {:?} timed out after {} seconds",
            args,
            GIT_DIFF_TIMEOUT.as_secs()
        ));
    }
    let mut child = Command::new("git")
        .args(args)
        .current_dir(worktree)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            format!(
                "Failed to run git {:?} in '{}': {}",
                args,
                worktree.display(),
                e
            )
        })?;

    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Failed to capture git stdout".to_string())?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Failed to capture git stderr".to_string())?;

    let stdout_task = std::thread::spawn(move || {
        let mut stdout_bytes = Vec::new();
        stdout.read_to_end(&mut stdout_bytes).map(|_| stdout_bytes)
    });
    let stderr_task = std::thread::spawn(move || {
        let mut stderr_bytes = Vec::new();
        stderr.read_to_end(&mut stderr_bytes).map(|_| stderr_bytes)
    });

    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| {
            format!(
                "Failed to run git {:?} in '{}': {}",
                args,
                worktree.display(),
                e
            )
        })? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = join_reader(stdout_task, "stdout");
            let _ = join_reader(stderr_task, "stderr");
            return Err(format!(
                "git {:?} timed out after {} seconds",
                args,
                GIT_DIFF_TIMEOUT.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    };

    let stdout_bytes = join_reader(stdout_task, "stdout")?;
    let stderr_bytes = join_reader(stderr_task, "stderr")?;

    if !status.success() {
        let stderr = String::from_utf8_lossy(&stderr_bytes).trim().to_string();
        return Err(format!(
            "git {:?} failed in '{}': {}",
            args,
            worktree.display(),
            if stderr.is_empty() {
                "unknown git error"
            } else {
                stderr.as_str()
            }
        ));
    }

    Ok(String::from_utf8_lossy(&stdout_bytes).to_string())
}

fn list_untracked(worktree: &Path, deadline: Instant) -> Result<Vec<String>, String> {
    Ok(run_git(
        worktree,
        &["ls-files", "--others", "--exclude-standard"],
        deadline,
    )?
    .lines()
    .map(str::trim)
    .filter(|line| !line.is_empty())
    .map(str::to_string)
    .collect())
}

fn append_section(output: &mut String, title: &str, body: &str) {
    if !output.is_empty() {
        output.push('\n');
    }
    output.push_str("## ");
    output.push_str(title);
    output.push_str("\n");
    if body.trim().is_empty() {
        output.push_str("(no changes)\n");
    } else {
        output.push_str(body.trim_end());
        output.push('\n');
    }
}

fn join_untracked(untracked: &[String]) -> String {
    if untracked.is_empty() {
        String::new()
    } else {
        let mut output = untracked.join("\n");
        output.push('\n');
        output
    }
}

fn push_name_only(names: &mut Vec<String>, seen: &mut HashSet<String>, output: &str) {
    for line in output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if seen.insert(line.to_string()) {
            names.push(line.to_string());
        }
    }
}

fn run_git_diff(worktree: &Path, mode: AgentDiffMode, base: &DiffBase) -> Result<String, String> {
    let range = format!("{}...HEAD", base.refish);
    let deadline = Instant::now() + GIT_DIFF_TIMEOUT;
    match mode {
        AgentDiffMode::Stat => {
            let committed = run_git(worktree, &["diff", "--stat", &range], deadline)?;
            let staged = run_git(worktree, &["diff", "--stat", "--cached"], deadline)?;
            let unstaged = run_git(worktree, &["diff", "--stat"], deadline)?;
            let untracked = list_untracked(worktree, deadline)?;
            if committed.trim().is_empty()
                && staged.trim().is_empty()
                && unstaged.trim().is_empty()
                && untracked.is_empty()
            {
                return Ok("(no changes detected)".to_string());
            }
            let mut output = String::new();
            append_section(&mut output, "Committed changes since base", &committed);
            append_section(&mut output, "Staged changes", &staged);
            append_section(&mut output, "Unstaged changes", &unstaged);
            append_section(&mut output, "Untracked files", &join_untracked(&untracked));
            Ok(output)
        }
        AgentDiffMode::Unified => {
            let committed = run_git(worktree, &["diff", &range], deadline)?;
            let staged = run_git(worktree, &["diff", "--cached"], deadline)?;
            let unstaged = run_git(worktree, &["diff"], deadline)?;
            let untracked = list_untracked(worktree, deadline)?;
            if committed.trim().is_empty()
                && staged.trim().is_empty()
                && unstaged.trim().is_empty()
                && untracked.is_empty()
            {
                return Ok("(no changes detected)".to_string());
            }
            let mut output = String::new();
            append_section(&mut output, "Committed changes since base", &committed);
            append_section(&mut output, "Staged changes", &staged);
            append_section(&mut output, "Unstaged changes", &unstaged);
            append_section(&mut output, "Untracked files", &join_untracked(&untracked));
            Ok(output)
        }
        AgentDiffMode::NameOnly => {
            let committed = run_git(worktree, &["diff", "--name-only", &range], deadline)?;
            let staged = run_git(worktree, &["diff", "--name-only", "--cached"], deadline)?;
            let unstaged = run_git(worktree, &["diff", "--name-only"], deadline)?;
            let untracked = list_untracked(worktree, deadline)?;
            let mut names = Vec::new();
            let mut seen = HashSet::new();
            push_name_only(&mut names, &mut seen, &committed);
            push_name_only(&mut names, &mut seen, &staged);
            push_name_only(&mut names, &mut seen, &unstaged);
            for path in untracked {
                if seen.insert(path.clone()) {
                    names.push(path);
                }
            }
            if names.is_empty() {
                Ok("(no changes detected)".to_string())
            } else {
                Ok(names.join("\n"))
            }
        }
    }
}

fn output_fence(mode: AgentDiffMode) -> &'static str {
    match mode {
        AgentDiffMode::Unified => "diff",
        AgentDiffMode::Stat | AgentDiffMode::NameOnly => "text",
    }
}

fn truncate_lines(output: &str, max_lines: usize) -> String {
    let lines = output.lines().collect::<Vec<_>>();
    if lines.len() <= max_lines {
        return output.to_string();
    }
    let omitted = lines.len().saturating_sub(max_lines);
    let mut result = lines
        .iter()
        .take(max_lines)
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    if !result.is_empty() {
        result.push('\n');
    }
    result.push_str(&format!(
        "... ({} more lines, use mode='name-only' to see all files)",
        omitted
    ));
    result
}

fn render_agent_diff(
    card: &BoardCard,
    branch: &str,
    base: &DiffBase,
    mode: AgentDiffMode,
    output: &str,
    max_lines: usize,
) -> String {
    let rendered = truncate_lines(output, max_lines);
    let diff = if rendered.trim().is_empty() {
        "(no changes detected)".to_string()
    } else {
        rendered
    };
    format!(
        "# Agent Diff for {}\n\n**Card:** {}\n**Branch:** {}\n**Base:** {}\n\n```{}\n{}\n```",
        card.id,
        card.title,
        branch,
        base.label,
        output_fence(mode),
        diff
    )
}

#[async_trait]
impl Tool for ToolAgentDiff {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "agent_diff".to_string(),
            display_name: "Agent Diff".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Show the real git diff for a task agent worktree against the task base commit or branch, including committed, staged, unstaged, and untracked changes. Planner-only; use this to inspect actual agent changes instead of relying on final reports.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "card_id": {"type": "string", "description": "Card ID whose agent worktree diff to inspect"},
                    "mode": {"type": "string", "enum": ["stat", "unified", "name-only"], "description": "Diff mode. Default: stat"},
                    "max_lines": {"type": "number", "description": "Maximum output lines before truncation. Default: 300"},
                    "task_id": {"type": "string", "description": "Task ID (optional if chat is bound to a task)"}
                },
                "required": ["card_id"]
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
        let is_planner = {
            let ccx_lock = ccx.lock().await;
            ccx_lock
                .task_meta
                .as_ref()
                .map(|meta| meta.role == "planner")
                .unwrap_or(false)
        };
        if !is_planner {
            return Err(
                "agent_diff can only be called by the task planner. Switch to the planner chat to inspect agent diffs."
                    .to_string(),
            );
        }

        let card_id = required_string(args, "card_id")?;
        let mode = AgentDiffMode::parse(args.get("mode"))?;
        let max_lines = parse_max_lines(args)?;
        let task_id = get_task_id(&ccx, args).await?;
        let gcx = ccx.lock().await.app.gcx.clone();

        let board = storage::load_board(gcx.clone(), &task_id).await?;
        let card = board
            .get_card(&card_id)
            .ok_or_else(|| format!("Card {} not found", card_id))?;
        let task_meta = storage::load_task_meta(gcx.clone(), &task_id).await?;
        let (worktree_commit, worktree_branch) = base_from_worktree_meta(gcx.clone(), card).await;
        let base = resolve_base(
            worktree_commit,
            worktree_branch,
            task_meta.base_commit,
            task_meta.base_branch,
        )?;
        let branch = card
            .agent_branch
            .as_ref()
            .ok_or_else(|| format!("Card {} has no agent branch", card.id))?;
        let worktree = canonical_worktree(card)?;
        let output = run_git_diff(&worktree, mode, &base)?;
        let result = render_agent_diff(card, branch, &base, mode, &output, max_lines);

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(result),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use crate::chat::types::TaskMeta as ThreadTaskMeta;
    use crate::tasks::types::{TaskBoard, TaskMeta, TaskStatus};
    use crate::tools::tools_description::Tool;
    use std::process::Command as StdCommand;

    fn run_git(cwd: &Path, args: &[&str]) -> String {
        let output = StdCommand::new("git")
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
        run_git(root, &["config", "user.email", "test@example.com"]);
        run_git(root, &["config", "user.name", "Test User"]);
        std::fs::write(root.join("file.txt"), "hello\n").unwrap();
        run_git(root, &["add", "file.txt"]);
        run_git(root, &["commit", "-m", "initial"]);
    }

    fn commit_file(root: &Path, name: &str, content: &str, message: &str) -> String {
        std::fs::write(root.join(name), content).unwrap();
        run_git(root, &["add", name]);
        run_git(root, &["commit", "-m", message]);
        run_git(root, &["rev-parse", "HEAD"]).trim().to_string()
    }

    fn test_card(branch: Option<String>, worktree: Option<String>) -> BoardCard {
        BoardCard {
            id: "T-1".to_string(),
            title: "Diff card".to_string(),
            column: "done".to_string(),
            priority: "P1".to_string(),
            depends_on: vec![],
            instructions: String::new(),
            assignee: Some("agent-1".to_string()),
            agent_chat_id: Some("agent-chat-1".to_string()),
            status_updates: vec![],
            final_report: Some("done".to_string()),
            final_report_structured: None,
            verifier_report: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            started_at: None,
            last_heartbeat_at: None,
            completed_at: Some(chrono::Utc::now().to_rfc3339()),
            agent_branch: branch,
            agent_worktree: worktree,
            agent_worktree_name: None,
            target_files: vec![],
            scope_guard_mode: Default::default(),
        }
    }

    fn write_file(root: &Path, path: &str, content: &str) {
        let full_path = root.join(path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(full_path, content).unwrap();
    }

    fn assert_name_only_has(output: &str, expected: &[&str]) {
        let paths = output.lines().map(str::trim).collect::<Vec<_>>();
        for path in expected {
            assert!(paths.contains(path), "missing {path} in {output}");
        }
    }

    fn task_meta() -> TaskMeta {
        task_meta_with_base(Some("main"), None)
    }

    fn task_meta_with_base(base_branch: Option<&str>, base_commit: Option<&str>) -> TaskMeta {
        let now = chrono::Utc::now().to_rfc3339();
        TaskMeta {
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
            base_branch: base_branch.map(str::to_string),
            base_commit: base_commit.map(str::to_string),
            default_agent_model: None,
            is_name_generated: false,
            last_agents_summary_at: None,
            planner_session_state: None,
        }
    }

    async fn write_task(root: &Path, card: BoardCard) -> Arc<crate::global_context::GlobalContext> {
        write_task_with_meta(root, card, task_meta()).await
    }

    async fn write_task_with_meta(
        root: &Path,
        card: BoardCard,
        meta: TaskMeta,
    ) -> Arc<crate::global_context::GlobalContext> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let task_dir = root.join(".refact").join("tasks").join("task-1");
        tokio::fs::create_dir_all(&task_dir).await.unwrap();
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
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![root.canonicalize().unwrap()];
        gcx
    }

    async fn planner_ccx(
        gcx: Arc<crate::global_context::GlobalContext>,
        role: &str,
    ) -> Arc<AMutex<AtCommandsContext>> {
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
                    role: role.to_string(),
                    agent_id: None,
                    card_id: None,
                    planner_chat_id: None,
                }),
                None,
            )
            .await,
        ))
    }

    fn tool_output_text(result: (bool, Vec<ContextEnum>)) -> String {
        match result.1.into_iter().next().unwrap() {
            ContextEnum::ChatMessage(message) => match message.content {
                ChatContent::SimpleText(text) => text,
                _ => panic!("expected text output"),
            },
            _ => panic!("expected chat message"),
        }
    }

    #[test]
    fn tool_agent_diff_description_is_correct() {
        let desc = ToolAgentDiff::new().tool_description();

        assert_eq!(desc.name, "agent_diff");
        assert_eq!(desc.input_schema["required"], json!(["card_id"]));
        assert_eq!(
            desc.input_schema["properties"]["mode"]["enum"],
            json!(["stat", "unified", "name-only"])
        );
        assert!(desc.description.contains("real git diff"));
    }

    #[tokio::test]
    async fn tool_agent_diff_rejects_non_planner_role() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = write_task(temp.path(), test_card(None, None)).await;
        let ccx = planner_ccx(gcx, "agents").await;
        let mut tool = ToolAgentDiff::new();
        let args = HashMap::from([("card_id".to_string(), json!("T-1"))]);

        let err = tool
            .tool_execute(ccx, &"call".to_string(), &args)
            .await
            .unwrap_err();

        assert!(err.contains("can only be called by the task planner"));
    }

    #[tokio::test]
    async fn tool_agent_diff_missing_card_id_returns_error() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = write_task(temp.path(), test_card(None, None)).await;
        let ccx = planner_ccx(gcx, "planner").await;
        let mut tool = ToolAgentDiff::new();
        let args = HashMap::new();

        let err = tool
            .tool_execute(ccx, &"call".to_string(), &args)
            .await
            .unwrap_err();

        assert_eq!(err, "Missing 'card_id'");
    }

    #[tokio::test]
    async fn tool_agent_diff_git_diff_between_branches_works() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        init_repo(&repo);
        run_git(&repo, &["checkout", "-b", "agent-branch"]);
        commit_file(&repo, "file.txt", "hello\nagent\n", "agent change");
        let card = test_card(
            Some("agent-branch".to_string()),
            Some(repo.to_string_lossy().to_string()),
        );
        let gcx = write_task(&repo, card).await;
        let ccx = planner_ccx(gcx, "planner").await;
        let mut tool = ToolAgentDiff::new();

        let stat = tool_output_text(
            tool.tool_execute(
                ccx.clone(),
                &"call".to_string(),
                &HashMap::from([
                    ("card_id".to_string(), json!("T-1")),
                    ("mode".to_string(), json!("stat")),
                ]),
            )
            .await
            .unwrap(),
        );
        assert!(stat.contains("# Agent Diff for T-1"));
        assert!(stat.contains("**Branch:** agent-branch"));
        assert!(stat.contains("**Base:** branch main"));
        assert!(stat.contains("file.txt"));

        let unified = tool_output_text(
            tool.tool_execute(
                ccx.clone(),
                &"call".to_string(),
                &HashMap::from([
                    ("card_id".to_string(), json!("T-1")),
                    ("mode".to_string(), json!("unified")),
                ]),
            )
            .await
            .unwrap(),
        );
        assert!(unified.contains("+agent"));

        let name_only = tool_output_text(
            tool.tool_execute(
                ccx,
                &"call".to_string(),
                &HashMap::from([
                    ("card_id".to_string(), json!("T-1")),
                    ("mode".to_string(), json!("name-only")),
                ]),
            )
            .await
            .unwrap(),
        );
        assert!(name_only.contains("file.txt"));
    }

    #[tokio::test]
    async fn tool_agent_diff_name_only_includes_all_worktree_states() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        init_repo(&repo);
        commit_file(&repo, "unstaged.txt", "base\n", "add tracked unstaged file");
        run_git(&repo, &["checkout", "-b", "agent-branch"]);
        commit_file(&repo, "committed.txt", "committed\n", "committed change");
        write_file(&repo, "staged.txt", "staged\n");
        run_git(&repo, &["add", "staged.txt"]);
        write_file(&repo, "unstaged.txt", "base\nunstaged\n");
        write_file(&repo, "untracked.txt", "untracked\n");
        let card = test_card(
            Some("agent-branch".to_string()),
            Some(repo.to_string_lossy().to_string()),
        );
        let gcx = write_task(&repo, card).await;
        let ccx = planner_ccx(gcx, "planner").await;
        let mut tool = ToolAgentDiff::new();

        let name_only = tool_output_text(
            tool.tool_execute(
                ccx,
                &"call".to_string(),
                &HashMap::from([
                    ("card_id".to_string(), json!("T-1")),
                    ("mode".to_string(), json!("name-only")),
                ]),
            )
            .await
            .unwrap(),
        );

        assert_name_only_has(
            &name_only,
            &[
                "committed.txt",
                "staged.txt",
                "unstaged.txt",
                "untracked.txt",
            ],
        );
    }

    #[tokio::test]
    async fn tool_agent_diff_unified_sections_include_dirty_state() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        init_repo(&repo);
        run_git(&repo, &["checkout", "-b", "agent-branch"]);
        commit_file(&repo, "committed.txt", "committed\n", "committed change");
        write_file(&repo, "staged.txt", "staged\n");
        run_git(&repo, &["add", "staged.txt"]);
        std::fs::write(repo.join("file.txt"), "hello\nunstaged\n").unwrap();
        write_file(&repo, "untracked.txt", "untracked\n");
        let card = test_card(
            Some("agent-branch".to_string()),
            Some(repo.to_string_lossy().to_string()),
        );
        let gcx = write_task(&repo, card).await;
        let ccx = planner_ccx(gcx, "planner").await;
        let mut tool = ToolAgentDiff::new();

        let unified = tool_output_text(
            tool.tool_execute(
                ccx,
                &"call".to_string(),
                &HashMap::from([
                    ("card_id".to_string(), json!("T-1")),
                    ("mode".to_string(), json!("unified")),
                ]),
            )
            .await
            .unwrap(),
        );

        assert!(unified.contains("## Committed changes since base"));
        assert!(unified.contains("## Staged changes"));
        assert!(unified.contains("## Unstaged changes"));
        assert!(unified.contains("## Untracked files"));
        assert!(unified.contains("+committed"));
        assert!(unified.contains("+staged"));
        assert!(unified.contains("+unstaged"));
        assert!(unified.contains("untracked.txt"));
    }

    #[tokio::test]
    async fn tool_agent_diff_prefers_original_base_commit_after_base_branch_advances() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        init_repo(&repo);
        let base_commit = run_git(&repo, &["rev-parse", "HEAD"]).trim().to_string();
        run_git(&repo, &["checkout", "-b", "agent-branch"]);
        commit_file(&repo, "agent.txt", "agent\n", "agent change");
        run_git(&repo, &["checkout", "main"]);
        commit_file(&repo, "main.txt", "main advanced\n", "advance main");
        run_git(&repo, &["checkout", "agent-branch"]);
        let card = test_card(
            Some("agent-branch".to_string()),
            Some(repo.to_string_lossy().to_string()),
        );
        let gcx = write_task_with_meta(
            &repo,
            card,
            task_meta_with_base(Some("main"), Some(&base_commit)),
        )
        .await;
        let ccx = planner_ccx(gcx, "planner").await;
        let mut tool = ToolAgentDiff::new();

        let name_only = tool_output_text(
            tool.tool_execute(
                ccx,
                &"call".to_string(),
                &HashMap::from([
                    ("card_id".to_string(), json!("T-1")),
                    ("mode".to_string(), json!("name-only")),
                ]),
            )
            .await
            .unwrap(),
        );

        assert!(name_only.contains(&format!("**Base:** commit {}", base_commit)));
        assert!(name_only.contains("agent.txt"));
        assert!(!name_only.contains("main.txt"));
    }

    #[tokio::test]
    async fn tool_agent_diff_reports_no_changes() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        init_repo(&repo);
        run_git(&repo, &["checkout", "-b", "agent-branch"]);
        let card = test_card(
            Some("agent-branch".to_string()),
            Some(repo.to_string_lossy().to_string()),
        );
        let gcx = write_task(&repo, card).await;
        let ccx = planner_ccx(gcx, "planner").await;
        let mut tool = ToolAgentDiff::new();

        let output = tool_output_text(
            tool.tool_execute(
                ccx,
                &"call".to_string(),
                &HashMap::from([
                    ("card_id".to_string(), json!("T-1")),
                    ("mode".to_string(), json!("stat")),
                ]),
            )
            .await
            .unwrap(),
        );

        assert!(output.contains("(no changes detected)"));
    }

    #[test]
    fn tool_agent_diff_truncates_output() {
        let card = test_card(Some("agent".to_string()), Some("/tmp/wt".to_string()));
        let output = "a\nb\nc\nd\n";

        let rendered = render_agent_diff(
            &card,
            "agent",
            &DiffBase {
                refish: "main".to_string(),
                label: "branch main".to_string(),
            },
            AgentDiffMode::Unified,
            output,
            2,
        );

        assert!(
            rendered.contains("a\nb\n... (2 more lines, use mode='name-only' to see all files)")
        );
    }
}
