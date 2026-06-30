use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::global_context::GlobalContext;
use crate::tasks::storage;
use crate::tasks::types::{AbVariantInfo, BoardCard};
use crate::tools::task_tool_helpers::{required_string, require_bound_planner_task};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use crate::worktrees::service::WorktreeService;
use crate::worktrees::types::{WorktreeRecordView, WorktreeReference};

const DEFAULT_MAX_LINES: usize = 300;
const GIT_DIFF_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_GIT_OUTPUT_BYTES: usize = 1024 * 1024;
const UNTRACKED_PER_FILE_CAP_BYTES: usize = 64 * 1024;
const UNTRACKED_TOTAL_CAP_BYTES: usize = 256 * 1024;
const UNTRACKED_BINARY_PROBE_BYTES: usize = 8 * 1024;

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

fn parse_card_variant_id(input: &str) -> (&str, Option<&str>) {
    if let Some((card_id, variant)) = input.split_once('/') {
        return (card_id, Some(variant));
    }
    if let Some((card_id, variant)) = input.split_once(':') {
        return (card_id, Some(variant));
    }
    (input, None)
}

fn variant_key_from_request(value: Option<&str>) -> Result<Option<&'static str>, String> {
    match value {
        Some("a") | Some("A") => Ok(Some("a")),
        Some("b") | Some("B") => Ok(Some("b")),
        Some(other) => Err(format!(
            "Invalid A/B variant '{}', expected 'a' or 'b'",
            other
        )),
        None => Ok(None),
    }
}

fn card_for_diff_target(card: &BoardCard, requested_id: &str) -> Result<BoardCard, String> {
    let (_, requested_variant) = parse_card_variant_id(requested_id);
    let Some(variant_key) = variant_key_from_request(requested_variant)? else {
        return Ok(card.clone());
    };
    let variants = card
        .ab_variants
        .as_ref()
        .ok_or_else(|| format!("Card {} has no A/B variants", card.id))?;
    let variant = variants
        .variant(variant_key)
        .ok_or_else(|| format!("A/B variant {} not found", variant_key))?;
    Ok(card_with_variant_agent(card, variant_key, variant))
}

fn card_with_variant_agent(
    card: &BoardCard,
    variant_key: &str,
    variant: &AbVariantInfo,
) -> BoardCard {
    let mut card = card.clone();
    card.agent_chat_id = Some(variant.chat_id.clone());
    card.assignee = Some(variant.agent_id.clone());
    card.agent_worktree = Some(variant.worktree.clone());
    card.agent_worktree_name = variant.worktree_name.clone();
    card.agent_branch = variant.branch.clone();
    card.title = format!("{} [A/B {}]", card.title, variant_key.to_ascii_uppercase());
    card
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

fn reference_matches_card(task_id: &str, card: &BoardCard, reference: &WorktreeReference) -> bool {
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
    card: &BoardCard,
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

fn registered_error_allows_legacy_fallback(err: &str) -> bool {
    !err.contains("does not match task")
}

async fn base_from_worktree_meta(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    card: &BoardCard,
) -> (Option<String>, Option<String>) {
    let project_dirs = crate::files_correction::get_project_dirs(gcx.clone()).await;
    for source_root in project_dirs {
        let Ok(service) = WorktreeService::new(gcx.cache_dir.clone(), source_root) else {
            continue;
        };
        let Ok(registry) = service.load_registry().await else {
            continue;
        };
        if let Some(record) = registry.records.iter().find(|record| {
            card.agent_worktree_name.as_deref() == Some(record.meta.id.as_str())
                || (record.meta.task_id.as_deref() == Some(task_id)
                    && record.meta.card_id.as_deref() == Some(card.id.as_str()))
                || record
                    .references
                    .iter()
                    .any(|reference| reference_matches_card(task_id, card, reference))
        }) {
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

fn canonical_existing_path(path: &Path) -> Result<PathBuf, String> {
    std::fs::canonicalize(path)
        .map_err(|e| format!("Failed to canonicalize '{}': {}", path.display(), e))
}

fn validated_cache_root(gcx: &GlobalContext) -> Result<PathBuf, String> {
    if !gcx.cache_dir.exists() {
        std::fs::create_dir_all(&gcx.cache_dir).map_err(|e| {
            format!(
                "Failed to create cache directory '{}': {}",
                gcx.cache_dir.display(),
                e
            )
        })?;
    }
    let cache_root = gcx.cache_dir.join("worktrees");
    if !cache_root.exists() {
        std::fs::create_dir_all(&cache_root).map_err(|e| {
            format!(
                "Failed to create worktree cache '{}': {}",
                cache_root.display(),
                e
            )
        })?;
    }
    canonical_existing_path(&cache_root)
}

fn validate_fallback_worktree_path(
    gcx: &GlobalContext,
    workspace_root: &Path,
    card_id: &str,
    worktree: &Path,
    branch: &str,
) -> Result<PathBuf, String> {
    if !worktree.exists() {
        return Err(format!(
            "Agent worktree '{}' for card {} does not exist",
            worktree.display(),
            card_id
        ));
    }
    let worktree = canonical_existing_path(worktree)?;
    let cache_root = validated_cache_root(gcx)?;
    let cache_dir = canonical_existing_path(&gcx.cache_dir)?;
    if worktree == Path::new("/") || worktree == cache_dir || worktree == cache_root {
        return Err(format!(
            "Refusing to use unsafe agent worktree path '{}' for card {}.",
            worktree.display(),
            card_id
        ));
    }
    if !worktree.starts_with(&cache_root) {
        return Err(format!(
            "Agent worktree '{}' for card {} is outside worktree cache '{}'.",
            worktree.display(),
            card_id,
            cache_root.display()
        ));
    }
    let status = crate::worktrees::git::status_for_path(&worktree);
    if !status.is_git_worktree {
        return Err(format!(
            "Agent worktree '{}' for card {} is not a git worktree.",
            worktree.display(),
            card_id
        ));
    }
    let top_level = crate::worktrees::git::run_git(&worktree, &["rev-parse", "--show-toplevel"])?;
    let top_level = canonical_existing_path(Path::new(top_level.trim()))?;
    if top_level != worktree {
        return Err(format!(
            "Agent worktree '{}' for card {} is not the git worktree root.",
            worktree.display(),
            card_id
        ));
    }
    let workspace_root = canonical_existing_path(workspace_root)?;
    let common_dir = |root: &Path| -> Result<PathBuf, String> {
        let output = crate::worktrees::git::run_git(root, &["rev-parse", "--git-common-dir"])?;
        let path = Path::new(output.trim());
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        };
        canonical_existing_path(&path)
    };
    if common_dir(&workspace_root)? != common_dir(&worktree)? {
        return Err(format!(
            "Agent worktree '{}' for card {} belongs to a different repository.",
            worktree.display(),
            card_id
        ));
    }
    crate::worktrees::git::ensure_checkout_on_branch(&worktree, "Agent worktree", branch)?;
    Ok(worktree)
}

async fn primary_workspace_root(gcx: Arc<GlobalContext>) -> Result<PathBuf, String> {
    let project_dirs = crate::files_correction::get_project_dirs(gcx).await;
    let workspace_root = project_dirs.first().ok_or("No workspace folder found")?;
    canonical_existing_path(workspace_root)
}

fn legacy_worktree_path(
    gcx: &GlobalContext,
    workspace_root: &Path,
    card: &BoardCard,
) -> Result<PathBuf, String> {
    let worktree = card
        .agent_worktree
        .as_ref()
        .ok_or_else(|| format!("Card {} has no agent worktree", card.id))?;
    let branch = card
        .agent_branch
        .as_ref()
        .ok_or_else(|| format!("Card {} has no agent branch", card.id))?;
    validate_fallback_worktree_path(gcx, workspace_root, &card.id, Path::new(worktree), branch)
}

async fn registered_worktree_path(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    card: &BoardCard,
    worktree_name: &str,
) -> Result<PathBuf, String> {
    let project_dirs = crate::files_correction::get_project_dirs(gcx.clone()).await;
    for source_root in project_dirs {
        let Ok(service) = WorktreeService::new(gcx.cache_dir.clone(), source_root) else {
            continue;
        };
        let Ok(view) = service.get_worktree(worktree_name).await else {
            continue;
        };
        if !registered_record_matches_card(&view, task_id, card) {
            return Err(format!(
                "Registered worktree '{}' does not match task {} card {}.",
                worktree_name, task_id, card.id
            ));
        }
        if !view.meta.root.exists() {
            return Err(format!(
                "Registered worktree '{}' path '{}' for card {} does not exist",
                worktree_name,
                view.meta.root.display(),
                card.id
            ));
        }
        service
            .validate_registered_worktree_checkout(worktree_name)
            .await?;
        return canonical_existing_path(&view.meta.root);
    }
    Err(format!(
        "Registered worktree '{}' for card {} was not found",
        worktree_name, card.id
    ))
}

async fn registered_worktree_path_for_card(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    card: &BoardCard,
    excluded_id: Option<&str>,
) -> Result<Option<PathBuf>, String> {
    let project_dirs = crate::files_correction::get_project_dirs(gcx.clone()).await;
    for source_root in project_dirs {
        let Ok(service) = WorktreeService::new(gcx.cache_dir.clone(), source_root) else {
            continue;
        };
        let Ok(registry) = service.load_registry().await else {
            continue;
        };
        for record in &registry.records {
            if excluded_id == Some(record.meta.id.as_str()) {
                continue;
            }
            let Ok(view) = service.get_worktree(&record.meta.id).await else {
                continue;
            };
            let branch_matches = card
                .agent_branch
                .as_deref()
                .map(|branch| {
                    view.meta.branch.as_deref() == Some(branch)
                        && view
                            .meta
                            .task_id
                            .as_deref()
                            .map(|record_task_id| record_task_id == task_id)
                            .unwrap_or(true)
                })
                .unwrap_or(false);
            if !registered_record_matches_card(&view, task_id, card) && !branch_matches {
                continue;
            }
            if !view.meta.root.exists() {
                return Err(format!(
                    "Registered worktree '{}' path '{}' for card {} does not exist",
                    view.meta.id,
                    view.meta.root.display(),
                    card.id
                ));
            }
            service
                .validate_registered_worktree_checkout(&view.meta.id)
                .await?;
            return canonical_existing_path(&view.meta.root).map(Some);
        }
    }
    Ok(None)
}

async fn canonical_worktree(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    card: &BoardCard,
) -> Result<PathBuf, String> {
    if let Some(worktree_name) = card
        .agent_worktree_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return match registered_worktree_path(gcx.clone(), task_id, card, worktree_name).await {
            Ok(path) => Ok(path),
            Err(registered_err) => {
                if !registered_error_allows_legacy_fallback(&registered_err) {
                    return Err(registered_err);
                }
                let card_registered_err = match registered_worktree_path_for_card(
                    gcx.clone(),
                    task_id,
                    card,
                    Some(worktree_name),
                )
                .await
                {
                    Ok(Some(path)) => return Ok(path),
                    Ok(None) => None,
                    Err(err) => Some(err),
                };
                let workspace_root = primary_workspace_root(gcx.clone()).await?;
                legacy_worktree_path(&gcx, &workspace_root, card).map_err(|legacy_err| {
                    if let Some(card_registered_err) = card_registered_err.as_deref() {
                        format!(
                            "{}; related registered lookup failed: {}; legacy fallback failed: {}",
                            registered_err, card_registered_err, legacy_err
                        )
                    } else {
                        format!("{}; legacy fallback failed: {}", registered_err, legacy_err)
                    }
                })
            }
        };
    }
    match registered_worktree_path_for_card(gcx.clone(), task_id, card, None).await {
        Ok(Some(path)) => return Ok(path),
        Ok(None) => {}
        Err(registered_err) => {
            if !registered_error_allows_legacy_fallback(&registered_err) {
                return Err(registered_err);
            }
            let workspace_root = primary_workspace_root(gcx.clone()).await?;
            return legacy_worktree_path(&gcx, &workspace_root, card).map_err(|legacy_err| {
                format!("{}; legacy fallback failed: {}", registered_err, legacy_err)
            });
        }
    }
    let workspace_root = primary_workspace_root(gcx.clone()).await?;
    legacy_worktree_path(&gcx, &workspace_root, card)
}

async fn run_with_timeout<F, T>(fut: F, timeout: Duration) -> Result<T, ()>
where
    F: std::future::Future<Output = T>,
{
    tokio::time::timeout(timeout, fut).await.map_err(|_| ())
}

async fn run_git(worktree: &Path, args: &[&str], timeout: Duration) -> Result<String, String> {
    use tokio::io::AsyncReadExt;

    let mut child = tokio::process::Command::new("git")
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

    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let mut buf = Vec::with_capacity(64 * 1024);
    let mut chunk = [0u8; 8192];
    let mut capped = false;

    let read_result = run_with_timeout(
        async {
            loop {
                let n = stdout
                    .read(&mut chunk)
                    .await
                    .map_err(|e| format!("git stdout read failed: {e}"))?;
                if n == 0 {
                    break;
                }
                if buf.len() + n > MAX_GIT_OUTPUT_BYTES {
                    let remaining = MAX_GIT_OUTPUT_BYTES - buf.len();
                    buf.extend_from_slice(&chunk[..remaining]);
                    buf.extend_from_slice(b"\n... (truncated by byte cap)\n");
                    capped = true;
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            Ok::<_, String>(())
        },
        timeout,
    )
    .await;

    let _ = child.kill().await;
    let status = child
        .wait()
        .await
        .map_err(|e| format!("git wait failed: {e}"))?;

    match read_result {
        Err(()) => {
            return Err(format!(
                "git {:?} timed out after {} seconds",
                args,
                timeout.as_secs()
            ));
        }
        Ok(Err(e)) => return Err(e),
        Ok(Ok(())) => {}
    }

    if capped {
        return String::from_utf8(buf).map_err(|e| format!("git output not utf-8: {e}"));
    }

    if !status.success() {
        let mut stderr_buf = Vec::new();
        let _ = stderr.read_to_end(&mut stderr_buf).await;
        let stderr_text = String::from_utf8_lossy(&stderr_buf).trim().to_string();
        let msg = if stderr_text.is_empty() {
            "unknown git error".to_string()
        } else {
            stderr_text
        };
        return Err(format!(
            "git {:?} failed in '{}': {}",
            args,
            worktree.display(),
            msg
        ));
    }

    String::from_utf8(buf).map_err(|e| format!("git output not utf-8: {e}"))
}

async fn list_untracked(worktree: &Path, timeout: Duration) -> Result<Vec<String>, String> {
    Ok(run_git(
        worktree,
        &["ls-files", "--others", "--exclude-standard"],
        timeout,
    )
    .await?
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

fn usize_from_file_len(len: u64) -> usize {
    usize::try_from(len).unwrap_or(usize::MAX)
}

fn read_file_prefix(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path)
        .map_err(|e| format!("Failed to open '{}': {}", path.display(), e))?;
    let mut bytes = Vec::new();
    file.take(limit as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Failed to read '{}': {}", path.display(), e))?;
    Ok(bytes)
}

fn render_untracked_text_diff(relative_path: &str, bytes: &[u8], more_bytes: u64) -> String {
    let mut lines = String::from_utf8_lossy(bytes)
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    if more_bytes > 0 {
        lines.push(format!("... (truncated, {} more bytes)", more_bytes));
    }
    let mut output = String::new();
    output.push_str("--- /dev/null\n");
    output.push_str(&format!("+++ b/{}\n", relative_path));
    output.push_str(&format!("@@ -0,0 +1,{} @@\n", lines.len()));
    for line in lines {
        output.push('+');
        output.push_str(&line);
        output.push('\n');
    }
    output
}

fn render_untracked_unified_entry(
    worktree: &Path,
    relative_path: &str,
    remaining_budget: usize,
) -> (String, usize) {
    let path = worktree.join(relative_path);
    let metadata = match std::fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(e) => return (format!("{} (failed to stat: {})\n", relative_path, e), 0),
    };
    if !metadata.is_file() {
        return (format!("{} (not a regular file)\n", relative_path), 0);
    }

    let file_len = metadata.len();
    let file_len_usize = usize_from_file_len(file_len);
    let content_limit = remaining_budget.min(UNTRACKED_PER_FILE_CAP_BYTES);
    let read_limit = file_len_usize
        .min(UNTRACKED_PER_FILE_CAP_BYTES)
        .min(content_limit.max(UNTRACKED_BINARY_PROBE_BYTES));
    let bytes = match read_file_prefix(&path, read_limit) {
        Ok(bytes) => bytes,
        Err(e) => return (format!("{} ({})\n", relative_path, e), 0),
    };
    let probe_len = bytes.len().min(UNTRACKED_BINARY_PROBE_BYTES);
    if bytes[..probe_len].contains(&0) {
        return (
            format!("{} (binary, {} bytes)\n", relative_path, file_len),
            0,
        );
    }

    let embedded_len = bytes.len().min(content_limit);
    let more_bytes = file_len.saturating_sub(embedded_len as u64);
    (
        render_untracked_text_diff(relative_path, &bytes[..embedded_len], more_bytes),
        embedded_len,
    )
}

fn render_untracked_unified(worktree: &Path, untracked: &[String]) -> String {
    let mut output = String::new();
    let mut used_bytes = 0usize;
    for (index, relative_path) in untracked.iter().enumerate() {
        let remaining_budget = UNTRACKED_TOTAL_CAP_BYTES.saturating_sub(used_bytes);
        if remaining_budget == 0 {
            for omitted_path in &untracked[index..] {
                output.push_str(omitted_path);
                output.push('\n');
            }
            output.push_str(&format!(
                "({} more untracked files omitted due to size cap)\n",
                untracked.len().saturating_sub(index)
            ));
            break;
        }
        let (entry, consumed_bytes) =
            render_untracked_unified_entry(worktree, relative_path, remaining_budget);
        output.push_str(&entry);
        if !entry.ends_with('\n') {
            output.push('\n');
        }
        used_bytes = used_bytes.saturating_add(consumed_bytes);
    }
    output
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

async fn run_git_diff(
    worktree: &Path,
    mode: AgentDiffMode,
    base: &DiffBase,
) -> Result<String, String> {
    let range = format!("{}...HEAD", base.refish);
    match mode {
        AgentDiffMode::Stat => {
            let committed =
                run_git(worktree, &["diff", "--stat", &range], GIT_DIFF_TIMEOUT).await?;
            let staged =
                run_git(worktree, &["diff", "--stat", "--cached"], GIT_DIFF_TIMEOUT).await?;
            let unstaged = run_git(worktree, &["diff", "--stat"], GIT_DIFF_TIMEOUT).await?;
            let untracked = list_untracked(worktree, GIT_DIFF_TIMEOUT).await?;
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
            let committed = run_git(worktree, &["diff", &range], GIT_DIFF_TIMEOUT).await?;
            let staged = run_git(worktree, &["diff", "--cached"], GIT_DIFF_TIMEOUT).await?;
            let unstaged = run_git(worktree, &["diff"], GIT_DIFF_TIMEOUT).await?;
            let untracked = list_untracked(worktree, GIT_DIFF_TIMEOUT).await?;
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
            append_section(
                &mut output,
                "Untracked files",
                &render_untracked_unified(worktree, &untracked),
            );
            Ok(output)
        }
        AgentDiffMode::NameOnly => {
            let committed =
                run_git(worktree, &["diff", "--name-only", &range], GIT_DIFF_TIMEOUT).await?;
            let staged = run_git(
                worktree,
                &["diff", "--name-only", "--cached"],
                GIT_DIFF_TIMEOUT,
            )
            .await?;
            let unstaged = run_git(worktree, &["diff", "--name-only"], GIT_DIFF_TIMEOUT).await?;
            let untracked = list_untracked(worktree, GIT_DIFF_TIMEOUT).await?;
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

        let requested_card_id = required_string(args, "card_id")?;
        let (card_id, _) = parse_card_variant_id(&requested_card_id);
        let mode = AgentDiffMode::parse(args.get("mode"))?;
        let max_lines = parse_max_lines(args)?;
        let task_id = require_bound_planner_task(&ccx, args).await?;
        let gcx = ccx.lock().await.app.gcx.clone();

        let board = storage::load_board(gcx.clone(), &task_id).await?;
        let card = board
            .get_card(card_id)
            .ok_or_else(|| format!("Card {} not found", card_id))?;
        let card = card_for_diff_target(card, &requested_card_id)?;
        let worktree = canonical_worktree(gcx.clone(), &task_id, &card).await?;
        let task_meta = storage::load_task_meta(gcx.clone(), &task_id).await?;
        let (worktree_commit, worktree_branch) =
            base_from_worktree_meta(gcx.clone(), &task_id, &card).await;
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
        let output = run_git_diff(&worktree, mode, &base).await?;
        let result = render_agent_diff(&card, branch, &base, mode, &output, max_lines);

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
    use crate::worktrees::types::CreateWorktreeRequest;
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
        std::fs::write(root.join(".git").join("info").join("exclude"), ".refact/\n").unwrap();
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

    fn legacy_repo_path(root: &Path) -> PathBuf {
        root.join("cache").join("worktrees").join("repo")
    }

    async fn make_gcx_for_root(root: &Path) -> Arc<crate::global_context::GlobalContext> {
        if let Some(worktrees_dir) = root.parent() {
            if worktrees_dir.file_name().and_then(|value| value.to_str()) == Some("worktrees") {
                if let Some(cache_dir) = worktrees_dir.parent() {
                    return crate::global_context::tests::make_test_gcx_with_dirs(
                        cache_dir.to_path_buf(),
                        cache_dir.join("config"),
                    )
                    .await;
                }
            }
        }
        crate::global_context::tests::make_test_gcx().await
    }

    fn test_card(branch: Option<String>, worktree: Option<String>) -> BoardCard {
        test_card_with_id("T-1", branch, worktree, None)
    }

    fn test_card_with_worktree_name(
        branch: Option<String>,
        worktree: Option<String>,
        worktree_name: Option<String>,
    ) -> BoardCard {
        test_card_with_id("T-1", branch, worktree, worktree_name)
    }

    fn test_card_with_id(
        id: &str,
        branch: Option<String>,
        worktree: Option<String>,
        worktree_name: Option<String>,
    ) -> BoardCard {
        BoardCard {
            id: id.to_string(),
            title: "Diff card".to_string(),
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
            verifier_report: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            started_at: None,
            last_heartbeat_at: None,
            completed_at: Some(chrono::Utc::now().to_rfc3339()),
            agent_branch: branch,
            agent_worktree: worktree,
            agent_worktree_name: worktree_name,
            ab_variants: None,
            team_members: vec![],
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

    fn count_lines_with_prefix(output: &str, prefix: &str) -> usize {
        output
            .lines()
            .filter(|line| line.starts_with(prefix))
            .count()
    }

    async fn execute_diff(
        gcx: Arc<crate::global_context::GlobalContext>,
        mode: &str,
    ) -> Result<String, String> {
        execute_diff_with_max_lines(gcx, mode, 2000).await
    }

    async fn execute_diff_with_max_lines(
        gcx: Arc<crate::global_context::GlobalContext>,
        mode: &str,
        max_lines: usize,
    ) -> Result<String, String> {
        let ccx = planner_ccx(gcx, "planner").await;
        let mut tool = ToolAgentDiff::new();
        let output = tool
            .tool_execute(
                ccx,
                &"call".to_string(),
                &HashMap::from([
                    ("card_id".to_string(), json!("T-1")),
                    ("mode".to_string(), json!(mode)),
                    ("max_lines".to_string(), json!(max_lines)),
                ]),
            )
            .await
            .map(tool_output_text)?;
        Ok(output)
    }

    async fn create_registered_worktree(
        temp: &tempfile::TempDir,
        task_id: &str,
        card_id: &str,
    ) -> (
        Arc<crate::global_context::GlobalContext>,
        PathBuf,
        crate::worktrees::types::WorktreeRecordView,
    ) {
        let source = temp.path().join("source");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let cache_dir = temp.path().join("cache");
        let config_dir = temp.path().join("config");
        let gcx =
            crate::global_context::tests::make_test_gcx_with_dirs(cache_dir, config_dir).await;
        let service =
            WorktreeService::new(gcx.cache_dir.clone(), source.canonicalize().unwrap()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some(format!("refact/task/{}/card/{}/agent", task_id, card_id)),
                kind: Some("task_agent".to_string()),
                task_id: Some(task_id.to_string()),
                card_id: Some(card_id.to_string()),
                agent_id: Some("agent-1".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        (gcx, source, created.worktree)
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
        let gcx = make_gcx_for_root(root).await;
        write_task_with_gcx(gcx, root, card, meta).await
    }

    async fn write_task_with_gcx(
        gcx: Arc<crate::global_context::GlobalContext>,
        root: &Path,
        card: BoardCard,
        meta: TaskMeta,
    ) -> Arc<crate::global_context::GlobalContext> {
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
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.canonicalize().unwrap()];
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
    async fn tool_agent_diff_rejects_mismatched_task_id() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = write_task(temp.path(), test_card(None, None)).await;
        let ccx = planner_ccx(gcx, "planner").await;
        let mut tool = ToolAgentDiff::new();
        let args = HashMap::from([
            ("card_id".to_string(), json!("T-1")),
            ("task_id".to_string(), json!("task-2")),
        ]);

        let err = tool
            .tool_execute(ccx, &"call".to_string(), &args)
            .await
            .unwrap_err();

        assert_eq!(
            err,
            "task_id override is not allowed from this planner chat"
        );
    }

    #[tokio::test]
    async fn tool_agent_diff_git_diff_between_branches_works() {
        let temp = tempfile::tempdir().unwrap();
        let repo = legacy_repo_path(temp.path());
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
                    ("task_id".to_string(), json!("task-1")),
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
        let repo = legacy_repo_path(temp.path());
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
        let repo = legacy_repo_path(temp.path());
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
    async fn agent_diff_unified_includes_untracked_file_content() {
        let temp = tempfile::tempdir().unwrap();
        let repo = legacy_repo_path(temp.path());
        std::fs::create_dir_all(&repo).unwrap();
        init_repo(&repo);
        run_git(&repo, &["checkout", "-b", "agent-branch"]);
        write_file(&repo, "new.txt", "alpha\nbeta\n");
        let card = test_card(
            Some("agent-branch".to_string()),
            Some(repo.to_string_lossy().to_string()),
        );
        let gcx = write_task(&repo, card).await;

        let unified = execute_diff(gcx, "unified").await.unwrap();

        assert!(unified.contains("--- /dev/null"));
        assert!(unified.contains("+++ b/new.txt"));
        assert!(unified.contains("+alpha"));
        assert!(unified.contains("+beta"));
    }

    #[tokio::test]
    async fn agent_diff_unified_truncates_large_untracked_files() {
        let temp = tempfile::tempdir().unwrap();
        let repo = legacy_repo_path(temp.path());
        std::fs::create_dir_all(&repo).unwrap();
        init_repo(&repo);
        run_git(&repo, &["checkout", "-b", "agent-branch"]);
        write_file(&repo, "big.txt", &"a".repeat(200 * 1024));
        let card = test_card(
            Some("agent-branch".to_string()),
            Some(repo.to_string_lossy().to_string()),
        );
        let gcx = write_task(&repo, card).await;

        let unified = execute_diff_with_max_lines(gcx, "unified", 20)
            .await
            .unwrap();

        assert!(unified.contains("+++ b/big.txt"));
        assert!(unified.contains("... (truncated, 139264 more bytes)"));
        assert!(count_lines_with_prefix(&unified, "+a") <= 1);
    }

    #[tokio::test]
    async fn agent_diff_unified_skips_binary_untracked_files() {
        let temp = tempfile::tempdir().unwrap();
        let repo = legacy_repo_path(temp.path());
        std::fs::create_dir_all(&repo).unwrap();
        init_repo(&repo);
        run_git(&repo, &["checkout", "-b", "agent-branch"]);
        std::fs::write(repo.join("binary.bin"), b"abc\0def").unwrap();
        let card = test_card(
            Some("agent-branch".to_string()),
            Some(repo.to_string_lossy().to_string()),
        );
        let gcx = write_task(&repo, card).await;

        let unified = execute_diff(gcx, "unified").await.unwrap();

        assert!(unified.contains("binary.bin (binary, 7 bytes)"));
        assert!(!unified.contains("+++ b/binary.bin"));
        assert!(!unified.contains("+abc"));
    }

    #[tokio::test]
    async fn agent_diff_unified_respects_total_size_budget() {
        let temp = tempfile::tempdir().unwrap();
        let repo = legacy_repo_path(temp.path());
        std::fs::create_dir_all(&repo).unwrap();
        init_repo(&repo);
        run_git(&repo, &["checkout", "-b", "agent-branch"]);
        for index in 0..6 {
            write_file(&repo, &format!("file-{index}.txt"), &"x".repeat(64 * 1024));
        }
        let card = test_card(
            Some("agent-branch".to_string()),
            Some(repo.to_string_lossy().to_string()),
        );
        let gcx = write_task(&repo, card).await;

        let unified = execute_diff(gcx, "unified").await.unwrap();

        assert_eq!(
            unified
                .matches("more untracked files omitted due to size cap")
                .count(),
            1
        );
        assert!(unified.contains("(2 more untracked files omitted due to size cap)"));
        assert!(unified.contains("file-4.txt"));
        assert!(unified.contains("file-5.txt"));
    }

    #[tokio::test]
    async fn tool_agent_diff_prefers_original_base_commit_after_base_branch_advances() {
        let temp = tempfile::tempdir().unwrap();
        let repo = legacy_repo_path(temp.path());
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
        let repo = legacy_repo_path(temp.path());
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

    #[tokio::test]
    async fn agent_diff_rejects_path_outside_worktree_cache() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let outside = temp.path().join("random-repo");
        std::fs::create_dir_all(&outside).unwrap();
        init_repo(&outside);
        run_git(&outside, &["checkout", "-b", "agent-branch"]);
        write_file(&outside, "evil.txt", "nope\n");
        let cache_dir = temp.path().join("cache");
        let config_dir = temp.path().join("config");
        let gcx =
            crate::global_context::tests::make_test_gcx_with_dirs(cache_dir, config_dir).await;
        let card = test_card(
            Some("agent-branch".to_string()),
            Some(outside.to_string_lossy().to_string()),
        );
        let gcx = write_task_with_gcx(gcx, &source, card, task_meta()).await;
        let ccx = planner_ccx(gcx, "planner").await;
        let mut tool = ToolAgentDiff::new();

        let err = tool
            .tool_execute(
                ccx,
                &"call".to_string(),
                &HashMap::from([
                    ("card_id".to_string(), json!("T-1")),
                    ("mode".to_string(), json!("unified")),
                ]),
            )
            .await
            .unwrap_err();

        assert!(err.contains("outside worktree cache"), "{err}");
    }

    #[tokio::test]
    async fn agent_diff_uses_registry_when_worktree_name_set() {
        let temp = tempfile::tempdir().unwrap();
        let (gcx, source, registered) = create_registered_worktree(&temp, "task-1", "T-1").await;
        write_file(&registered.meta.root, "trusted.txt", "trusted\n");
        let stale = temp.path().join("stale");
        std::fs::create_dir_all(&stale).unwrap();
        init_repo(&stale);
        run_git(&stale, &["checkout", "-b", "agent-branch"]);
        write_file(&stale, "stale.txt", "stale\n");
        let card = test_card_with_worktree_name(
            registered.meta.branch.clone(),
            Some(stale.to_string_lossy().to_string()),
            Some(registered.meta.id.clone()),
        );
        let gcx = write_task_with_gcx(gcx, &source, card, task_meta()).await;

        let unified = execute_diff(gcx, "unified").await.unwrap();

        assert!(unified.contains("trusted.txt"));
        assert!(unified.contains("+trusted"));
        assert!(!unified.contains("stale.txt"));
    }

    #[tokio::test]
    async fn agent_diff_falls_back_to_valid_legacy_path_when_registry_id_is_stale() {
        let temp = tempfile::tempdir().unwrap();
        let repo = legacy_repo_path(temp.path());
        std::fs::create_dir_all(&repo).unwrap();
        init_repo(&repo);
        run_git(&repo, &["checkout", "-b", "agent-branch"]);
        commit_file(&repo, "legacy.txt", "legacy\n", "legacy change");
        let card = test_card_with_worktree_name(
            Some("agent-branch".to_string()),
            Some(repo.to_string_lossy().to_string()),
            Some("missing-registry-id".to_string()),
        );
        let gcx = write_task(&repo, card).await;

        let name_only = execute_diff(gcx, "name-only").await.unwrap();

        assert!(name_only.contains("legacy.txt"), "{name_only}");
    }

    #[tokio::test]
    async fn agent_diff_resolves_registered_worktree_by_reference_when_meta_is_missing() {
        let temp = tempfile::tempdir().unwrap();
        let (gcx, source, registered) = create_registered_worktree(&temp, "task-1", "T-1").await;
        let service =
            WorktreeService::new(gcx.cache_dir.clone(), source.canonicalize().unwrap()).unwrap();
        let mut registry = service.load_registry().await.unwrap();
        registry.records[0].meta.task_id = None;
        registry.records[0].meta.card_id = None;
        registry.records[0].references = vec![crate::worktrees::types::WorktreeReference {
            kind: "task_agent".to_string(),
            task_id: Some("task-1".to_string()),
            card_id: Some("T-1".to_string()),
            chat_id: Some("agent-chat-1".to_string()),
            agent_id: Some("agent-1".to_string()),
        }];
        service.save_registry(&registry).await.unwrap();
        write_file(&registered.meta.root, "referenced.txt", "referenced\n");
        let card = test_card_with_worktree_name(registered.meta.branch.clone(), None, None);
        let gcx = write_task_with_gcx(gcx, &source, card, task_meta()).await;

        let unified = execute_diff(gcx, "unified").await.unwrap();

        assert!(unified.contains("referenced.txt"), "{unified}");
        assert!(unified.contains("+referenced"), "{unified}");
    }

    #[tokio::test]
    async fn agent_diff_registry_metadata_mismatch_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let (gcx, source, registered) =
            create_registered_worktree(&temp, "other-task", "T-1").await;
        let card = test_card_with_worktree_name(
            registered.meta.branch.clone(),
            Some(registered.meta.root.to_string_lossy().to_string()),
            Some(registered.meta.id.clone()),
        );
        let gcx = write_task_with_gcx(gcx, &source, card, task_meta()).await;
        let ccx = planner_ccx(gcx, "planner").await;
        let mut tool = ToolAgentDiff::new();

        let err = tool
            .tool_execute(
                ccx,
                &"call".to_string(),
                &HashMap::from([
                    ("card_id".to_string(), json!("T-1")),
                    ("mode".to_string(), json!("unified")),
                ]),
            )
            .await
            .unwrap_err();

        assert!(err.contains("does not match task task-1 card T-1"), "{err}");
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

    #[tokio::test]
    async fn run_git_truncates_output_at_byte_cap() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path();
        init_repo(repo);
        let content_a = "a".repeat(600_000);
        std::fs::write(repo.join("big.txt"), &content_a).unwrap();
        run_git(repo, &["add", "big.txt"]);
        run_git(repo, &["commit", "-m", "add big"]);
        let content_b = "b".repeat(600_000);
        std::fs::write(repo.join("big.txt"), &content_b).unwrap();

        let output = super::run_git(repo, &["diff", "HEAD", "--text"], GIT_DIFF_TIMEOUT)
            .await
            .unwrap();

        assert!(
            output.contains("truncated by byte cap"),
            "expected truncation footer"
        );
        assert!(
            output.len() <= MAX_GIT_OUTPUT_BYTES + 64,
            "output {} bytes exceeds cap + overhead",
            output.len()
        );
    }

    #[tokio::test]
    async fn run_git_respects_timeout() {
        let result =
            super::run_with_timeout(std::future::pending::<()>(), Duration::from_nanos(1)).await;

        assert!(result.is_err(), "expected timeout");
    }

    #[tokio::test]
    async fn agent_diff_is_async_compatible() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path();
        init_repo(repo);

        let result = super::run_git(
            repo,
            &["rev-parse", "--is-inside-work-tree"],
            GIT_DIFF_TIMEOUT,
        )
        .await
        .unwrap();

        assert_eq!(result.trim(), "true");
    }
}
