use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::process::Command;
use tokio::sync::Mutex as AMutex;

use crate::global_context::GlobalContext;
use crate::tools::review_types::{ReviewDiffSummary, ReviewScopeSummary, ScopeMode};

const ADJACENT_EXPANSION_CAP: usize = 40;

fn max_diff_patch_bytes() -> usize {
    crate::runtime_settings::current().review_max_diff_patch_bytes
}

const GENERATED_MARKERS: &[&str] = &[
    "/node_modules/",
    "/target/debug/",
    "/target/release/",
    "/dist/",
    "/build/",
    "/__snapshots__/",
    "/.next/",
    "/vendor/",
];

const GENERATED_SUFFIXES: &[&str] = &[
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "cargo.lock",
    "poetry.lock",
    ".snap",
    ".min.js",
    ".min.css",
    ".generated.rs",
    ".generated.ts",
    "_pb2.py",
];

pub fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
        .trim_start_matches("./")
        .trim_end_matches('/')
        .to_string()
}

pub fn paths_match(left: &str, right: &str) -> bool {
    let left = normalize_path(left);
    let right = normalize_path(right);
    left == right || left.ends_with(&format!("/{right}")) || right.ends_with(&format!("/{left}"))
}

pub fn is_generated_path(path: &str) -> bool {
    let lowered = normalize_path(path).to_ascii_lowercase();
    if GENERATED_MARKERS.iter().any(|m| lowered.contains(m)) {
        return true;
    }
    GENERATED_SUFFIXES
        .iter()
        .any(|suffix| lowered.ends_with(suffix))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffHunks {
    by_path: HashMap<String, Vec<(u32, u32)>>,
}

impl DiffHunks {
    pub fn parse(patch: &str) -> Self {
        let mut by_path: HashMap<String, Vec<(u32, u32)>> = HashMap::new();
        let mut current: Option<String> = None;
        for line in patch.lines() {
            if let Some(rest) = line.strip_prefix("+++ ") {
                let path = rest.trim();
                current = (path != "/dev/null").then(|| {
                    normalize_path(
                        path.strip_prefix("b/")
                            .unwrap_or(path)
                            .trim_end_matches('\t'),
                    )
                });
                continue;
            }
            if !line.starts_with("@@") {
                continue;
            }
            let Some(path) = current.clone() else {
                continue;
            };
            let Some(range) = parse_new_hunk_range(line) else {
                continue;
            };
            by_path.entry(path).or_default().push(range);
        }
        Self { by_path }
    }

    pub fn contains(&self, file: &str, line_start: u32, line_end: u32) -> bool {
        let file = normalize_path(file);
        self.by_path
            .iter()
            .filter(|(path, _)| paths_match(path, &file))
            .any(|(_, ranges)| {
                ranges
                    .iter()
                    .any(|(start, end)| line_start <= *end && *start <= line_end)
            })
    }

    pub fn touches_file(&self, file: &str) -> bool {
        let file = normalize_path(file);
        self.by_path.keys().any(|path| paths_match(path, &file))
    }

    pub fn hunk_count(&self) -> usize {
        self.by_path.values().map(Vec::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.by_path.is_empty()
    }
}

fn parse_new_hunk_range(header: &str) -> Option<(u32, u32)> {
    let plus = header.split('+').nth(1)?;
    let spec = plus.split(&[' ', '@'][..]).next()?;
    let mut parts = spec.split(',');
    let start: u32 = parts.next()?.trim().parse().ok()?;
    let count: u32 = parts
        .next()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(1);
    Some((start, start + count.saturating_sub(1).max(0)))
}

#[derive(Debug, Clone)]
pub struct ReviewScope {
    pub mode: ScopeMode,
    pub requested: Vec<PathBuf>,
    pub files: Vec<PathBuf>,
    pub dropped_files: Vec<PathBuf>,
    pub changed_files: Vec<PathBuf>,
    pub focus: Option<String>,
    pub plan: Option<String>,
    pub base: Option<String>,
    pub head: Option<String>,
    pub diff_patch: Option<String>,
    pub patch_total_bytes: usize,
    pub hunks: DiffHunks,
    pub repo_root: Option<PathBuf>,
    pub expansion: Option<String>,
}

impl ReviewScope {
    pub fn in_scope(&self, file: &str) -> bool {
        if self.mode == ScopeMode::Broad || self.files.is_empty() {
            return true;
        }
        self.files
            .iter()
            .any(|path| paths_match(&path.to_string_lossy(), file))
    }

    pub fn file_strings(&self) -> Vec<String> {
        self.files
            .iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect()
    }

    pub fn dropped_file_strings(&self) -> Vec<String> {
        self.dropped_files
            .iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect()
    }

    pub fn summary(&self) -> ReviewScopeSummary {
        ReviewScopeSummary {
            mode: self.mode.as_str().to_string(),
            requested_files: self.requested.len(),
            reviewed_files: self.files.len(),
            files: self.file_strings(),
            focus: self.focus.clone(),
            expansion: self.expansion.clone(),
            out_of_scope_findings: 0,
            dropped_files: self.dropped_file_strings(),
        }
    }

    pub fn diff_summary(&self) -> ReviewDiffSummary {
        ReviewDiffSummary {
            base: self.base.clone(),
            head: self.head.clone(),
            changed_files: self.changed_files.len(),
            hunks: self.hunks.hunk_count(),
        }
    }
}

pub struct ScopeRequest {
    pub requested: Vec<PathBuf>,
    pub mode: ScopeMode,
    pub base: Option<String>,
    pub focus: Option<String>,
    pub plan: Option<String>,
    pub max_files: usize,
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

pub async fn resolve_base(root: &Path, requested: Option<&str>) -> Option<String> {
    if let Some(requested) = requested.map(str::trim).filter(|v| !v.is_empty()) {
        if let Some(base) = git_output(root, &["merge-base", requested, "HEAD"]).await {
            return Some(base);
        }
        return git_output(root, &["rev-parse", requested]).await;
    }
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

async fn adjacent_expansion(
    gcx: Arc<GlobalContext>,
    seed: &[PathBuf],
) -> (Vec<PathBuf>, Option<String>) {
    let service = gcx.codegraph.lock().await.clone();
    let Some(service) = service else {
        return (Vec::new(), Some("codegraph unavailable".to_string()));
    };
    let seed_strings: Vec<String> = seed
        .iter()
        .map(|path| path.to_string_lossy().to_string())
        .collect();
    let report = match service.pr_blast(&seed_strings, 1).await {
        Ok(report) => report,
        Err(error) => return (Vec::new(), Some(format!("pr_blast failed: {error}"))),
    };
    let mut seen: HashSet<String> = seed_strings.iter().map(|p| normalize_path(p)).collect();
    let mut expanded = Vec::new();
    for impact in report
        .directly_impacted
        .iter()
        .chain(report.transitively_impacted.iter())
    {
        if expanded.len() >= ADJACENT_EXPANSION_CAP {
            break;
        }
        if is_generated_path(&impact.path) || !seen.insert(normalize_path(&impact.path)) {
            continue;
        }
        expanded.push(PathBuf::from(&impact.path));
    }
    let note = (!expanded.is_empty()).then(|| format!("+{} dependency edges", expanded.len()));
    (expanded, note)
}

pub async fn build_review_scope(gcx: Arc<GlobalContext>, request: ScopeRequest) -> ReviewScope {
    let ScopeRequest {
        requested,
        mode,
        base,
        focus,
        plan,
        max_files,
    } = request;
    let max_files = max_files.max(1);
    let repo_root = repo_root_for_scope(gcx.as_ref(), &requested);

    let (base, head, changed_files, diff_patch, patch_total_bytes) = match repo_root.as_ref() {
        Some(root) => {
            let base = resolve_base(root, base.as_deref()).await;
            let head = git_output(root, &["rev-parse", "--short", "HEAD"]).await;
            let max_patch_bytes = max_diff_patch_bytes();
            match base.as_ref().and_then(|base| {
                refact_worktrees::git::diff_for_path(root, Some(base), None, max_patch_bytes).ok()
            }) {
                Some(diff) => {
                    let mut seen = HashSet::new();
                    let changed = diff
                        .files
                        .into_iter()
                        .map(|file| {
                            crate::files_correction::canonicalize_normalized_path(
                                root.join(file.path),
                            )
                        })
                        .filter(|path| seen.insert(path.clone()))
                        .collect::<Vec<_>>();
                    let total = diff.patch_total_bytes.max(diff.patch_shown_bytes);
                    let patch = (!diff.patch.trim().is_empty()).then_some(diff.patch);
                    (base, head, changed, patch, total)
                }
                None => (base, head, Vec::new(), None, 0),
            }
        }
        None => (None, None, Vec::new(), None, 0),
    };

    let hunks = diff_patch
        .as_deref()
        .map(DiffHunks::parse)
        .unwrap_or_default();

    let mut files: Vec<PathBuf> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let push = |files: &mut Vec<PathBuf>, seen: &mut HashSet<String>, path: &PathBuf| {
        let key = normalize_path(&path.to_string_lossy());
        if !key.is_empty() && seen.insert(key) {
            files.push(path.clone());
        }
    };
    for path in &requested {
        push(&mut files, &mut seen, path);
    }
    for path in &changed_files {
        if is_generated_path(&path.to_string_lossy()) {
            continue;
        }
        push(&mut files, &mut seen, path);
    }

    let mut expansion = None;
    if mode == ScopeMode::Adjacent {
        let (expanded, note) = adjacent_expansion(gcx.clone(), &files).await;
        for path in &expanded {
            push(&mut files, &mut seen, path);
        }
        expansion = note;
    }
    let mut dropped_files: Vec<PathBuf> = Vec::new();
    if files.len() > max_files {
        dropped_files = files.split_off(max_files);
        let dropped = dropped_files.len();
        expansion = Some(match expansion {
            Some(note) => format!("{note}, truncated to {max_files} files, {dropped} not reviewed"),
            None => format!("truncated to {max_files} files, {dropped} not reviewed"),
        });
    }

    ReviewScope {
        mode,
        requested,
        files,
        dropped_files,
        changed_files,
        focus: focus
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        plan: plan
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        base,
        head,
        diff_patch,
        patch_total_bytes,
        hunks,
        repo_root,
        expansion,
    }
}

pub struct DiffAttribution {
    root: Option<PathBuf>,
    base: Option<String>,
    hunks: DiffHunks,
    blame: AMutex<HashMap<String, HashSet<u32>>>,
}

impl DiffAttribution {
    pub fn new(scope: &ReviewScope) -> Self {
        Self {
            root: scope.repo_root.clone(),
            base: scope.base.clone(),
            hunks: scope.hunks.clone(),
            blame: AMutex::new(HashMap::new()),
        }
    }

    pub async fn introduced(&self, file: &str, line_start: u32, line_end: u32) -> bool {
        if self.hunks.is_empty() {
            return true;
        }
        if self.hunks.contains(file, line_start, line_end) {
            return true;
        }
        if !self.hunks.touches_file(file) {
            return false;
        }
        let touched = self.blame_lines(file).await;
        (line_start..=line_end.max(line_start)).any(|line| touched.contains(&line))
    }

    async fn blame_lines(&self, file: &str) -> HashSet<u32> {
        let key = normalize_path(file);
        if let Some(cached) = self.blame.lock().await.get(&key) {
            return cached.clone();
        }
        let lines = self.compute_blame_lines(file).await;
        self.blame.lock().await.insert(key, lines.clone());
        lines
    }

    async fn compute_blame_lines(&self, file: &str) -> HashSet<u32> {
        let (Some(root), Some(base)) = (self.root.as_ref(), self.base.as_ref()) else {
            return HashSet::new();
        };
        let range = format!("{base}..HEAD");
        let Some(revisions) = git_output(root, &["rev-list", &range]).await else {
            return HashSet::new();
        };
        let branch_commits: HashSet<String> =
            revisions.lines().map(|line| line.to_string()).collect();
        if branch_commits.is_empty() {
            return HashSet::new();
        }
        let Some(blame) = git_output(root, &["blame", "--line-porcelain", "--", file.trim()]).await
        else {
            return HashSet::new();
        };
        let mut touched = HashSet::new();
        for line in blame.lines() {
            let mut parts = line.split_whitespace();
            let (Some(sha), Some(_original), Some(final_line)) =
                (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            if sha.len() < 40 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
                continue;
            }
            let Ok(final_line) = final_line.parse::<u32>() else {
                continue;
            };
            if branch_commits.contains(sha) {
                touched.insert(final_line);
            }
        }
        touched
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as StdCommand;

    const PATCH: &str = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -10,3 +10,5 @@ fn thing() {\n context\n+added\n+added\n@@ -80,0 +90,1 @@\n+one more\ndiff --git a/src/other.rs b/src/other.rs\n--- /dev/null\n+++ b/src/other.rs\n@@ -0,0 +1,4 @@\n+new file\n";

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

    #[test]
    fn review_scope_hunk_membership_uses_new_side_ranges() {
        let hunks = DiffHunks::parse(PATCH);

        assert_eq!(hunks.hunk_count(), 3);
        assert!(hunks.contains("src/lib.rs", 10, 10));
        assert!(hunks.contains("src/lib.rs", 14, 20));
        assert!(!hunks.contains("src/lib.rs", 15, 20));
        assert!(hunks.contains("src/lib.rs", 90, 90));
        assert!(hunks.contains("/abs/repo/src/other.rs", 1, 4));
        assert!(!hunks.contains("src/untouched.rs", 1, 400));
        assert!(hunks.touches_file("src/lib.rs"));
        assert!(!hunks.touches_file("src/untouched.rs"));
    }

    #[tokio::test]
    async fn review_scope_attribution_falls_back_to_blame_for_moved_lines() {
        let temp = tempfile::tempdir().unwrap();
        init_repo(temp.path());
        let base = run_git(temp.path(), &["rev-parse", "HEAD"]);
        std::fs::write(temp.path().join("moved.rs"), "alpha\nbeta\ngamma\n").unwrap();
        run_git(temp.path(), &["add", "moved.rs"]);
        run_git(temp.path(), &["commit", "-m", "add moved"]);

        let scope = ReviewScope {
            mode: ScopeMode::Strict,
            requested: vec![],
            files: vec![],
            dropped_files: vec![],
            changed_files: vec![],
            focus: None,
            plan: None,
            base: Some(base),
            head: None,
            diff_patch: None,
            patch_total_bytes: 0,
            hunks: DiffHunks::parse("--- a/moved.rs\n+++ b/moved.rs\n@@ -0,0 +1,1 @@\n+alpha\n"),
            repo_root: Some(temp.path().to_path_buf()),
            expansion: None,
        };
        let attribution = DiffAttribution::new(&scope);

        assert!(attribution.introduced("moved.rs", 1, 1).await);
        assert!(attribution.introduced("moved.rs", 3, 3).await);
        assert!(!attribution.introduced("base.txt", 1, 1).await);
    }

    #[tokio::test]
    async fn review_scope_strict_keeps_requested_files_and_flags_outsiders() {
        let temp = tempfile::tempdir().unwrap();
        init_repo(temp.path());
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
        let requested = vec![temp.path().join("base.txt")];

        let scope = build_review_scope(
            gcx,
            ScopeRequest {
                requested: requested.clone(),
                mode: ScopeMode::Strict,
                base: None,
                focus: Some("  safety  ".to_string()),
                plan: None,
                max_files: 10,
            },
        )
        .await;

        assert_eq!(scope.requested, requested);
        assert!(scope.in_scope(&temp.path().join("base.txt").to_string_lossy()));
        assert!(!scope.in_scope("src/elsewhere.rs"));
        assert_eq!(scope.focus.as_deref(), Some("safety"));
        assert!(scope.base.is_some());
    }

    #[tokio::test]
    async fn review_scope_broad_accepts_any_file_and_records_changed_set() {
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
            ScopeRequest {
                requested: vec![],
                mode: ScopeMode::Broad,
                base: Some("main".to_string()),
                focus: None,
                plan: None,
                max_files: 10,
            },
        )
        .await;

        assert_eq!(
            scope.changed_files,
            vec![crate::files_correction::canonicalize_normalized_path(
                temp.path().join("changed.rs")
            )]
        );
        assert!(scope.in_scope("anything/at/all.rs"));
        assert!(scope
            .diff_patch
            .as_deref()
            .is_some_and(|patch| patch.contains("changed.rs")));
        assert!(!scope.hunks.is_empty());
    }

    #[tokio::test]
    async fn review_scope_records_every_file_it_dropped_to_the_max_files_cap() {
        let temp = tempfile::tempdir().unwrap();
        init_repo(temp.path());
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
        let requested: Vec<PathBuf> = (0..5)
            .map(|index| temp.path().join(format!("file{index}.rs")))
            .collect();

        let scope = build_review_scope(
            gcx,
            ScopeRequest {
                requested: requested.clone(),
                mode: ScopeMode::Strict,
                base: None,
                focus: None,
                plan: None,
                max_files: 3,
            },
        )
        .await;

        assert_eq!(scope.files.len(), 3);
        assert_eq!(scope.dropped_files, requested[3..].to_vec());
        assert_eq!(scope.summary().dropped_files.len(), 2);
        assert!(scope
            .expansion
            .as_deref()
            .is_some_and(|note| note.contains("2 not reviewed")));
    }

    #[test]
    fn review_scope_generated_paths_are_excluded_from_review() {
        assert!(is_generated_path("Cargo.lock"));
        assert!(is_generated_path("gui/package-lock.json"));
        assert!(is_generated_path("src/__snapshots__/App.test.tsx.snap"));
        assert!(is_generated_path("web/node_modules/left-pad/index.js"));
        assert!(!is_generated_path("src/lib.rs"));
        assert!(!is_generated_path("gui/package.json"));
    }

    #[tokio::test]
    async fn review_scope_without_git_still_reviews_requested_files() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];

        let scope = build_review_scope(
            gcx,
            ScopeRequest {
                requested: vec![temp.path().join("solo.rs")],
                mode: ScopeMode::Strict,
                base: None,
                focus: None,
                plan: None,
                max_files: 10,
            },
        )
        .await;

        assert_eq!(scope.base, None);
        assert!(scope.changed_files.is_empty());
        assert_eq!(scope.files.len(), 1);
        assert!(scope.hunks.is_empty());
    }
}
