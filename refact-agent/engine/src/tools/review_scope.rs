use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::process::Command;

use crate::call_validation::SubchatParameters;
use crate::global_context::GlobalContext;
use crate::tools::subagent_phases::DEFAULT_MAX_FILES;

const DEFAULT_MAX_CANDIDATES: usize = 30;
const TOKENS_EXTRA_BUDGET_PERCENT: f32 = 0.06;
const MAX_DIFF_PATCH_BYTES: usize = 512 * 1024;

/// Limits applied across code review stages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewBudgets {
    pub max_files: usize,
    pub tokens_budget: i64,
    pub max_candidates: usize,
}

/// Files, git context, focus, and limits shared by code review stages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewScope {
    pub files: Vec<PathBuf>,
    pub seed_files: Vec<PathBuf>,
    pub focus: Option<String>,
    pub diff_base: Option<String>,
    pub changed_files: Vec<PathBuf>,
    pub diff_patch: Option<String>,
    pub budgets: ReviewBudgets,
}

fn merge_files(gathered: Vec<PathBuf>, seed: Vec<PathBuf>, max_files: usize) -> Vec<PathBuf> {
    let mut files = Vec::with_capacity(gathered.len().saturating_add(seed.len()));
    let mut seen = HashSet::new();
    for path in seed.iter().chain(gathered.iter()) {
        if seen.insert(path.clone()) {
            files.push(path.clone());
        }
    }
    files.truncate(max_files);
    files
}

fn review_budget_components(subchat_params: &SubchatParameters) -> (usize, usize) {
    let extra = (subchat_params.subchat_n_ctx as f32 * TOKENS_EXTRA_BUDGET_PERCENT) as usize;
    let required =
        subchat_params.subchat_max_new_tokens + subchat_params.subchat_tokens_for_rag + extra;
    (extra, required)
}

fn review_tokens_budget(subchat_params: &SubchatParameters) -> i64 {
    let (_, required) = review_budget_components(subchat_params);
    subchat_params.subchat_n_ctx as i64 - required as i64
}

fn repo_root_for_scope(gcx: &GlobalContext, paths: &[PathBuf]) -> Option<PathBuf> {
    let workspace_folders = gcx.documents_state.workspace_folders.lock().ok()?.clone();
    paths
        .iter()
        .map(|path| {
            if path.is_dir() {
                path.as_path()
            } else {
                path.parent().unwrap_or(path.as_path())
            }
        })
        .chain(workspace_folders.iter().map(PathBuf::as_path))
        .find_map(|path| {
            let repo = refact_worktrees::git::discover_repo(path).ok()?;
            refact_worktrees::git::repo_root(&repo).ok()
        })
}

async fn git_output(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!value.is_empty()).then_some(value)
}

async fn detect_diff_base(root: &Path) -> Option<String> {
    for candidate in [
        "HEAD@{upstream}",
        "main",
        "origin/main",
        "master",
        "origin/master",
    ] {
        if let Some(base) = git_output(root, &["merge-base", candidate, "HEAD"]).await {
            return Some(base);
        }
    }
    git_output(root, &["rev-parse", "HEAD"]).await
}

async fn git_context(
    gcx: &GlobalContext,
    files: &[PathBuf],
) -> (Option<String>, Vec<PathBuf>, Option<String>) {
    let Some(root) = repo_root_for_scope(gcx, files) else {
        return (None, Vec::new(), None);
    };
    let Some(diff_base) = detect_diff_base(&root).await else {
        return (None, Vec::new(), None);
    };
    let Ok(diff) =
        refact_worktrees::git::diff_for_path(&root, Some(&diff_base), None, MAX_DIFF_PATCH_BYTES)
    else {
        return (None, Vec::new(), None);
    };
    let mut seen = HashSet::new();
    let changed_files = diff
        .files
        .into_iter()
        .map(|file| crate::files_correction::canonicalize_normalized_path(root.join(file.path)))
        .filter(|path| seen.insert(path.clone()))
        .collect();
    let patch = (!diff.patch.trim().is_empty()).then_some(diff.patch);
    (Some(diff_base), changed_files, patch)
}

/// Build the immutable input shared by the code review pipeline.
///
/// @param gcx Global workspace state used for git repository discovery.
/// @param gathered Files selected by the gather phase.
/// @param seed User-supplied files that take priority under the file cap.
/// @param focus Optional review focus text.
/// @param subchat_params Token limits for review stages.
/// @returns The capped files, git context, focus, and stage budgets.
pub async fn build_review_scope(
    gcx: Arc<GlobalContext>,
    gathered: Vec<PathBuf>,
    seed: Vec<PathBuf>,
    focus: Option<String>,
    subchat_params: &SubchatParameters,
) -> ReviewScope {
    build_review_scope_with_max_files(
        gcx,
        gathered,
        seed,
        focus,
        subchat_params,
        DEFAULT_MAX_FILES,
    )
    .await
}

pub(crate) async fn build_review_scope_with_max_files(
    gcx: Arc<GlobalContext>,
    gathered: Vec<PathBuf>,
    seed: Vec<PathBuf>,
    focus: Option<String>,
    subchat_params: &SubchatParameters,
    max_files: usize,
) -> ReviewScope {
    let files = merge_files(gathered, seed.clone(), max_files);
    let (diff_base, changed_files, diff_patch) = git_context(gcx.as_ref(), &files).await;
    ReviewScope {
        files,
        seed_files: seed,
        focus: focus
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        diff_base,
        changed_files,
        diff_patch,
        budgets: ReviewBudgets {
            max_files,
            tokens_budget: review_tokens_budget(subchat_params),
            max_candidates: DEFAULT_MAX_CANDIDATES,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as StdCommand;

    use crate::call_validation::ChatModelType;
    use crate::llm::params::CacheControl;

    fn subchat_params(
        n_ctx: usize,
        max_new_tokens: usize,
        tokens_for_rag: usize,
    ) -> SubchatParameters {
        SubchatParameters {
            subchat_model_type: ChatModelType::Default,
            subchat_model: String::new(),
            subchat_n_ctx: n_ctx,
            subchat_max_new_tokens: max_new_tokens,
            subchat_temperature: None,
            subchat_tokens_for_rag: tokens_for_rag,
            subchat_reasoning_effort: None,
            subchat_cache_control: CacheControl::Off,
        }
    }

    fn run_git(root: &Path, args: &[&str]) -> String {
        let output = StdCommand::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn init_repo(root: &Path) {
        run_git(root, &["init"]);
        run_git(root, &["checkout", "-b", "main"]);
        run_git(root, &["config", "user.email", "test@example.com"]);
        run_git(root, &["config", "user.name", "Test User"]);
        std::fs::write(root.join("base.txt"), "base\n").unwrap();
        run_git(root, &["add", "base.txt"]);
        run_git(root, &["commit", "-m", "base"]);
    }

    #[tokio::test]
    async fn tool_review_scope_merges_seeds_first_deduplicates_and_caps() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
        let seed = temp.path().join("seed.rs");
        let gathered_first = temp.path().join("first.rs");
        let gathered_last = temp.path().join("last.rs");

        let scope = build_review_scope_with_max_files(
            gcx,
            vec![gathered_first.clone(), seed.clone(), gathered_last],
            vec![seed.clone()],
            None,
            &subchat_params(10_000, 2_000, 1_000),
            2,
        )
        .await;

        assert_eq!(scope.files, vec![seed.clone(), gathered_first]);
        assert_eq!(scope.seed_files, vec![seed]);
        assert_eq!(scope.budgets.max_files, 2);
        assert_eq!(scope.budgets.max_candidates, 30);
    }

    #[tokio::test]
    async fn tool_review_scope_detects_git_changed_files_against_base() {
        let temp = tempfile::tempdir().unwrap();
        init_repo(temp.path());
        run_git(temp.path(), &["checkout", "-b", "feature"]);
        std::fs::write(temp.path().join("changed.rs"), "changed\n").unwrap();
        run_git(temp.path(), &["add", "changed.rs"]);
        run_git(temp.path(), &["commit", "-m", "change"]);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];

        let scope = build_review_scope(
            gcx,
            Vec::new(),
            Vec::new(),
            None,
            &subchat_params(10_000, 2_000, 1_000),
        )
        .await;

        assert!(scope.diff_base.is_some());
        assert_eq!(
            scope.changed_files,
            vec![crate::files_correction::canonicalize_normalized_path(
                temp.path().join("changed.rs")
            )]
        );
        assert!(scope
            .diff_patch
            .as_deref()
            .is_some_and(|patch| patch.contains("changed.rs")));
    }

    #[tokio::test]
    async fn tool_review_scope_supports_non_git_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];

        let scope = build_review_scope(
            gcx,
            Vec::new(),
            Vec::new(),
            None,
            &subchat_params(10_000, 2_000, 1_000),
        )
        .await;

        assert_eq!(scope.diff_base, None);
        assert!(scope.changed_files.is_empty());
        assert_eq!(scope.diff_patch, None);
    }

    #[test]
    fn tool_review_scope_budget_math_matches_existing_values() {
        let params = subchat_params(10_000, 2_000, 1_000);

        assert_eq!(review_tokens_budget(&params), 6_400);
    }
}
