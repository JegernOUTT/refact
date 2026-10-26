use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::process::Command;
use tokio::sync::Mutex as AMutex;

use crate::global_context::GlobalContext;
use crate::tools::review_types::{ReviewDiffSummary, ReviewScopeSummary, ScopeMode};
use crate::worktrees::scope::ExecutionScope;

pub(crate) const TARGET_REFUSED: &str =
    "review target is unavailable, unauthorized or changed; request a fresh review target";

const SOURCE_LIMIT_REFUSED: &str = "review source snapshot exceeds configured limits; narrow the review scope or adjust max_files, max_file_size and review_diff_char_cap";

#[derive(Clone, Copy)]
struct SourceLimits {
    file_bytes: usize,
    total_bytes: usize,
}

impl SourceLimits {
    fn current() -> Self {
        let settings = crate::runtime_settings::current();
        Self {
            file_bytes: settings.max_file_size.min(settings.review_diff_char_cap),
            total_bytes: settings.review_diff_char_cap,
        }
    }
}

pub(crate) async fn resolve_review_files(
    gcx: Arc<GlobalContext>,
    execution_scope: Option<&ExecutionScope>,
    selected_root: Option<&Path>,
    files: &[String],
) -> Result<(Option<PathBuf>, Vec<PathBuf>), String> {
    let roots = match selected_root {
        Some(root) => vec![root.to_path_buf()],
        None => match execution_scope.filter(|scope| scope.is_enforced()) {
            Some(scope) => {
                scope.ensure_active_root().map_err(|_| TARGET_REFUSED)?;
                vec![scope.effective_root().to_path_buf()]
            }
            None => crate::files_correction::get_project_dirs(gcx.clone()).await,
        },
    };
    if files.is_empty() {
        return Ok((selected_root.map(Path::to_path_buf), Vec::new()));
    }
    let mut authorized = Vec::new();
    for root in roots {
        let root = tokio::fs::canonicalize(root)
            .await
            .map_err(|_| TARGET_REFUSED)?;
        authorized.push(dunce::simplified(&root).to_path_buf());
    }
    let mut requested = Vec::new();
    let mut repo_root = None;
    for file in files {
        let raw = Path::new(file);
        let candidates = if raw.is_absolute() {
            vec![raw.to_path_buf()]
        } else {
            authorized.iter().map(|root| root.join(raw)).collect()
        };
        let mut matches = Vec::new();
        for path in candidates {
            let Ok(path) = tokio::fs::canonicalize(path).await else {
                continue;
            };
            let path = dunce::simplified(&path).to_path_buf();
            if path.is_file()
                && authorized.iter().any(|root| path.starts_with(root))
                && !matches.contains(&path)
            {
                matches.push(path);
            }
        }
        if matches.len() != 1 {
            return Err(TARGET_REFUSED.into());
        }
        let path = matches.remove(0);
        crate::files_in_workspace::check_file_privacy_for_model_context(gcx.clone(), &path)
            .await
            .map_err(|_| TARGET_REFUSED)?;
        let root = discover_root(&path).ok_or(TARGET_REFUSED)?;
        if repo_root.as_ref().is_some_and(|expected| expected != &root)
            || selected_root.is_some_and(|expected| expected != root)
            || !authorized
                .iter()
                .any(|authorized| discover_root(authorized).as_ref() == Some(&root))
        {
            return Err(TARGET_REFUSED.into());
        }
        repo_root = Some(root);
        if !requested.contains(&path) {
            requested.push(path);
        }
    }
    Ok((repo_root, requested))
}

fn discover_root(path: &Path) -> Option<PathBuf> {
    let path = if path.is_file() { path.parent()? } else { path };
    let repo = refact_worktrees::git::discover_repo(path).ok()?;
    let root = refact_worktrees::git::repo_root(&repo).ok()?;
    Some(dunce::simplified(&std::fs::canonicalize(root).ok()?).to_path_buf())
}

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
    pub checkout: Option<Value>,
    pub snapshots: Vec<(PathBuf, Option<String>)>,
    pub artifacts: Option<Value>,
}

impl ReviewScope {
    pub fn candidate_context(&self) -> Value {
        json!({
            "repo_root": self.repo_root,
            "base": self.base,
            "head": self.head,
            "requested_paths": self.requested,
            "checkout": self.checkout,
            "file_snapshots": self.snapshots,
            "mode": if self.artifacts.is_some() { "artifact_only" } else { "inherited_tools" },
            "artifact_limits": self.artifacts.as_ref().map(|artifacts| &artifacts["limits"]),
            "source_artifact_coverage": self.artifacts.as_ref().map(|artifacts| &artifacts["coverage"]),
            "reviewed_files_basis": "selected scope paths, not proof of full-source inspection; see stage coverage and source_artifact_coverage",
            "omitted_dependencies": self.artifacts.is_some(),
        })
    }

    pub fn disclosed_paths(&self) -> Vec<PathBuf> {
        let mut seen = HashSet::new();
        self.requested
            .iter()
            .chain(self.files.iter())
            .chain(self.dropped_files.iter())
            .chain(self.changed_files.iter())
            .chain(self.snapshots.iter().map(|(path, _)| path))
            .filter(|path| seen.insert((*path).clone()))
            .cloned()
            .collect()
    }

    async fn authorize_paths(&self, gcx: Arc<GlobalContext>) -> Result<(), String> {
        for path in self.disclosed_paths() {
            if self
                .repo_root
                .as_ref()
                .is_some_and(|root| !path.starts_with(root))
            {
                return Err(TARGET_REFUSED.into());
            }
            crate::files_in_workspace::check_file_privacy_for_model_context(gcx.clone(), &path)
                .await
                .map_err(|_| TARGET_REFUSED)?;
            canonical_source_path(&path).await?;
        }
        Ok(())
    }

    pub async fn load_artifacts(&mut self, gcx: Arc<GlobalContext>) -> Result<(), String> {
        self.load_artifacts_with_limits(gcx, SourceLimits::current())
            .await
    }

    async fn load_artifacts_with_limits(
        &mut self,
        gcx: Arc<GlobalContext>,
        limits: SourceLimits,
    ) -> Result<(), String> {
        if self.requested.is_empty() {
            return Err("artifact-only review requires explicit candidate files".into());
        }
        self.ensure_current_with_limits(gcx.clone(), limits).await?;
        let mut remaining = limits.total_bytes;
        let mut artifacts = Vec::new();
        let mut supplied = Vec::new();
        let mut missing = Vec::new();
        for path in &self.files {
            let Some(bytes) = read_source(gcx.clone(), path, limits, &mut remaining).await? else {
                if self.requested.contains(path) {
                    return Err(TARGET_REFUSED.into());
                }
                missing.push(path.clone());
                continue;
            };
            let digest = format!("{:x}", Sha256::digest(&bytes));
            if !self
                .snapshots
                .iter()
                .any(|(known, expected)| known == path && expected.as_ref() == Some(&digest))
            {
                return Err(TARGET_REFUSED.into());
            }
            let content = String::from_utf8(bytes).map_err(|_| TARGET_REFUSED)?;
            supplied.push(path.clone());
            artifacts.push(
                json!({"path": path, "head": self.head, "sha256": digest, "content": content}),
            );
        }
        let diff_only: Vec<_> = self
            .changed_files
            .iter()
            .filter(|path| !supplied.contains(path))
            .cloned()
            .collect();
        let omitted: Vec<_> = self
            .disclosed_paths()
            .into_iter()
            .filter(|path| !supplied.contains(path))
            .collect();
        self.artifacts = Some(json!({
            "limits": {
                "total_bytes": limits.total_bytes, "file_bytes": limits.file_bytes,
                "source_files": self.files.len(),
                "max_read_bytes_per_pass": limits.total_bytes.saturating_add(1),
                "max_file_read_bytes": limits.file_bytes.saturating_add(1),
                "read_probe_bytes": 1,
                "applies_to": "source snapshot, artifact load and revalidation bodies only; legacy git diff capture is separate",
            },
            "files": artifacts,
            "coverage": {
                "source_artifact_files": supplied,
                "source_artifact_basis": "complete contents supplied for every selected existing scope file; this is input coverage, not proof the stage inspected it",
                "diff_only_files": diff_only,
                "diff_only_basis": "changed paths without a source artifact; the supplied diff may be truncated and is not full-source coverage",
                "omitted_source_files": omitted,
                "dropped_scope_files": self.dropped_files,
                "missing_source_files": missing,
                "omitted_dependencies": true,
            },
            "omitted_files": omitted,
            "omitted_dependencies": true,
            "tools_available": false,
        }));
        self.ensure_current_with_limits(gcx, limits).await
    }

    pub async fn ensure_current(&self, gcx: Arc<GlobalContext>) -> Result<(), String> {
        self.ensure_current_with_limits(gcx, SourceLimits::current())
            .await
    }

    async fn ensure_current_with_limits(
        &self,
        gcx: Arc<GlobalContext>,
        limits: SourceLimits,
    ) -> Result<(), String> {
        if let Some(root) = self.repo_root.as_ref() {
            if discover_root(root).as_ref() != Some(root)
                || git_output(root, &["rev-parse", "--verify", "HEAD"]).await != self.head
            {
                return Err(TARGET_REFUSED.into());
            }
        }
        self.authorize_paths(gcx.clone()).await?;
        if self.snapshots.len() != self.files.len()
            || self.requested.iter().any(|path| !self.files.contains(path))
            || self.requested.iter().any(|path| {
                !self
                    .snapshots
                    .iter()
                    .any(|(known, digest)| known == path && digest.is_some())
            })
        {
            return Err(TARGET_REFUSED.into());
        }
        let mut remaining = limits.total_bytes;
        for (path, expected) in &self.snapshots {
            if !self.files.contains(path)
                || snapshot(gcx.clone(), path, limits, &mut remaining).await? != *expected
            {
                return Err(TARGET_REFUSED.into());
            }
        }
        if let (Some(root), Some(base)) = (&self.repo_root, &self.base) {
            let diff = refact_worktrees::git::diff_for_path(
                root,
                Some(base),
                None,
                max_diff_patch_bytes(),
            )
            .map_err(|_| TARGET_REFUSED)?;
            let mut changed = Vec::new();
            for file in diff.files {
                let path =
                    crate::files_correction::canonicalize_normalized_path(root.join(file.path));
                if !changed.contains(&path) {
                    changed.push(path);
                }
            }
            if changed != self.changed_files
                || (!diff.patch.trim().is_empty()).then_some(diff.patch) != self.diff_patch
            {
                return Err(TARGET_REFUSED.into());
            }
        }
        Ok(())
    }

    pub fn in_scope(&self, file: &str) -> bool {
        if let Some(root) = &self.repo_root {
            if Path::new(file).is_absolute() && !Path::new(file).starts_with(root) {
                return false;
            }
        }
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
            candidate: Some(self.candidate_context()),
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
    pub repo_root: Option<PathBuf>,
    pub expected_head: Option<String>,
    pub checkout: Option<Value>,
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
        let commit = git_output(
            root,
            &[
                "rev-parse",
                "--verify",
                "--end-of-options",
                &format!("{requested}^{{commit}}"),
            ],
        )
        .await?;
        return git_output(root, &["merge-base", &commit, "HEAD"]).await;
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

async fn canonical_source_path(path: &Path) -> Result<bool, String> {
    match tokio::fs::canonicalize(path).await {
        Ok(canonical) if dunce::simplified(&canonical) == path => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        _ => Err(TARGET_REFUSED.into()),
    }
}

fn read_bounded_source(source: impl Read, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    source
        .take(limit.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| TARGET_REFUSED)?;
    if bytes.len() > limit {
        return Err(SOURCE_LIMIT_REFUSED.into());
    }
    Ok(bytes)
}

async fn read_source(
    gcx: Arc<GlobalContext>,
    path: &PathBuf,
    limits: SourceLimits,
    remaining: &mut usize,
) -> Result<Option<Vec<u8>>, String> {
    crate::files_in_workspace::check_file_privacy_for_model_context(gcx, path)
        .await
        .map_err(|_| TARGET_REFUSED)?;
    if !canonical_source_path(path).await? {
        return Ok(None);
    }
    if !tokio::fs::symlink_metadata(path)
        .await
        .map_err(|_| TARGET_REFUSED)?
        .is_file()
    {
        return Err(TARGET_REFUSED.into());
    }
    let limit = limits.file_bytes.min(*remaining);
    let source_path = path.clone();
    let bytes = tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(source_path).map_err(|_| TARGET_REFUSED)?;
        let metadata = file.metadata().map_err(|_| TARGET_REFUSED)?;
        if !metadata.is_file() {
            return Err(TARGET_REFUSED.into());
        }
        if metadata.len() > limit as u64 {
            return Err(SOURCE_LIMIT_REFUSED.into());
        }
        read_bounded_source(file, limit)
    })
    .await
    .map_err(|_| TARGET_REFUSED)??;
    if !canonical_source_path(path).await? {
        return Err(TARGET_REFUSED.into());
    }
    *remaining -= bytes.len();
    Ok(Some(bytes))
}

async fn snapshot(
    gcx: Arc<GlobalContext>,
    path: &PathBuf,
    limits: SourceLimits,
    remaining: &mut usize,
) -> Result<Option<String>, String> {
    Ok(read_source(gcx, path, limits, remaining)
        .await?
        .map(|bytes| format!("{:x}", Sha256::digest(bytes))))
}

pub async fn build_review_scope(
    gcx: Arc<GlobalContext>,
    request: ScopeRequest,
) -> Result<ReviewScope, String> {
    build_review_scope_with_limits(gcx, request, SourceLimits::current()).await
}

async fn build_review_scope_with_limits(
    gcx: Arc<GlobalContext>,
    request: ScopeRequest,
    limits: SourceLimits,
) -> Result<ReviewScope, String> {
    let ScopeRequest {
        requested,
        mode,
        base,
        focus,
        plan,
        max_files,
        repo_root,
        expected_head,
        checkout,
    } = request;
    let max_files = max_files.max(1);
    if requested.len() > max_files {
        return Err(SOURCE_LIMIT_REFUSED.into());
    }
    let explicit_base = base.is_some();
    if base.as_ref().is_some_and(|base| base.trim().is_empty()) {
        return Err("review base is unavailable or invalid".into());
    }
    let repo_root = repo_root
        .or_else(|| repo_root_for_scope(gcx.as_ref(), &requested))
        .and_then(|root| discover_root(&root));
    if (!requested.is_empty() || explicit_base || expected_head.is_some()) && repo_root.is_none() {
        return Err(TARGET_REFUSED.into());
    }

    let (base, head, changed_files, diff_patch, patch_total_bytes) = match repo_root.as_ref() {
        Some(root) => {
            let base = resolve_base(root, base.as_deref()).await;
            if explicit_base && base.is_none() {
                return Err("review base is unavailable or invalid".into());
            }
            let head = git_output(root, &["rev-parse", "--verify", "HEAD"]).await;
            if expected_head
                .as_ref()
                .is_some_and(|expected| head.as_ref() != Some(expected))
            {
                return Err(TARGET_REFUSED.into());
            }
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
                None if !requested.is_empty() || explicit_base || expected_head.is_some() => {
                    return Err(TARGET_REFUSED.into());
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
            Some(note) => format!("{note}, truncated to {max_files} scope files; {dropped} excluded from source snapshots but may appear in the diff"),
            None => format!("truncated to {max_files} scope files; {dropped} excluded from source snapshots but may appear in the diff"),
        });
    }

    let mut scope = ReviewScope {
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
        checkout,
        snapshots: Vec::new(),
        artifacts: None,
    };
    scope.authorize_paths(gcx.clone()).await?;
    let mut remaining = limits.total_bytes;
    for path in &scope.files {
        let digest = snapshot(gcx.clone(), path, limits, &mut remaining).await?;
        if digest.is_none() && scope.requested.contains(path) {
            return Err(TARGET_REFUSED.into());
        }
        scope.snapshots.push((path.clone(), digest));
    }
    scope.ensure_current_with_limits(gcx, limits).await?;
    Ok(scope)
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
            checkout: None,
            snapshots: vec![],
            artifacts: None,
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
        let foreign = tempfile::tempdir().unwrap();
        init_repo(foreign.path());
        let root = dunce::simplified(&temp.path().canonicalize().unwrap()).to_path_buf();
        let foreign_root = dunce::simplified(&foreign.path().canonicalize().unwrap()).to_path_buf();
        let base = run_git(&root, &["rev-parse", "HEAD"]);
        std::fs::write(root.join("base.txt"), "candidate\n").unwrap();
        run_git(&root, &["commit", "-am", "candidate"]);
        let head = run_git(&root, &["rev-parse", "HEAD"]);
        std::fs::write(foreign_root.join("base.txt"), "foreign provider\n").unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![foreign_root.clone()];
        *gcx.privacy_settings.write().unwrap() = Arc::new(crate::privacy::PrivacySettings {
            privacy_rules: crate::privacy::FilePrivacySettings {
                only_send_to_servers_I_control: vec![],
                blocked: vec!["*.blocked".into()],
            },
            loaded_ts: u64::MAX / 2,
        });
        std::fs::write(root.join("private.blocked"), "private\n").unwrap();
        let requested = vec![root.join("base.txt")];
        let files = vec![requested[0].to_string_lossy().into_owned()];
        assert_eq!(
            resolve_review_files(gcx.clone(), None, None, &files)
                .await
                .unwrap_err(),
            TARGET_REFUSED,
        );
        let (repo_root, resolved) = resolve_review_files(gcx.clone(), None, Some(&root), &files)
            .await
            .unwrap();
        assert_eq!(resolved, requested);
        assert_eq!(repo_root, Some(root.clone()));
        let partial = vec![
            files[0].clone(),
            root.join("missing.txt").to_string_lossy().into_owned(),
        ];
        assert_eq!(
            resolve_review_files(gcx.clone(), None, Some(&root), &partial)
                .await
                .unwrap_err(),
            TARGET_REFUSED,
        );
        assert_eq!(
            resolve_review_files(gcx.clone(), None, Some(&root), &["private.blocked".into()])
                .await
                .unwrap_err(),
            TARGET_REFUSED,
        );
        std::fs::remove_file(root.join("private.blocked")).unwrap();
        let request = |base: Option<String>| ScopeRequest {
            requested: requested.clone(),
            mode: ScopeMode::Strict,
            base,
            focus: Some("  safety  ".to_string()),
            plan: None,
            max_files: 10,
            repo_root: Some(root.clone()),
            expected_head: Some(head.clone()),
            checkout: None,
        };
        assert_eq!(
            build_review_scope(gcx.clone(), request(Some("missing-base".into())))
                .await
                .unwrap_err(),
            "review base is unavailable or invalid",
        );
        let scope = build_review_scope(gcx.clone(), request(Some(base.clone())))
            .await
            .unwrap();
        assert_eq!(scope.requested, requested);
        assert!(scope.in_scope(&root.join("base.txt").to_string_lossy()));
        assert!(!scope.in_scope(&foreign_root.join("base.txt").to_string_lossy()));
        assert!(!scope.in_scope("src/elsewhere.rs"));
        assert_eq!(scope.focus.as_deref(), Some("safety"));
        assert_eq!(scope.diff_summary().base, Some(base.clone()));
        assert_eq!(scope.diff_summary().head, Some(head.clone()));
        assert!(scope.diff_patch.as_deref().unwrap().contains("+candidate"));
        assert!(!scope
            .diff_patch
            .as_deref()
            .unwrap()
            .contains("foreign provider"));
        let identity = scope.candidate_context();
        let catalog = crate::tools::review_agents::stages::embedded_catalog();
        assert!(!catalog.is_empty());
        for stage in &catalog {
            let prompt =
                crate::tools::review_agents::prompts::build_stage_prompt(stage, &scope, None);
            assert!(prompt.contains(&identity.to_string()));
        }
        let adversarial = catalog
            .iter()
            .find(|stage| stage.id == "adversarial")
            .unwrap();
        assert!(
            crate::tools::review_agents::prompts::build_adversarial_prompt(
                adversarial,
                &scope,
                &[]
            )
            .contains(&identity.to_string())
        );
        assert_eq!(scope.diff_summary().candidate, Some(identity));
        scope.ensure_current(gcx.clone()).await.unwrap();
        std::fs::write(root.join("base.txt"), "changed during review\n").unwrap();
        assert_eq!(
            scope.ensure_current(gcx.clone()).await.unwrap_err(),
            TARGET_REFUSED
        );
        std::fs::write(root.join("base.txt"), "candidate\n").unwrap();
        let peer = root.join("peer.txt");
        std::fs::write(&peer, "oversize peer body\n").unwrap();
        let limits = SourceLimits {
            file_bytes: 16,
            total_bytes: 16,
        };
        let mut capped_request = request(Some(base.clone()));
        capped_request.max_files = 1;
        let mut capped = build_review_scope_with_limits(gcx.clone(), capped_request, limits)
            .await
            .unwrap();
        assert_eq!(capped.files, requested);
        assert_eq!(capped.dropped_files, vec![peer.clone()]);
        assert_eq!(capped.snapshots, scope.snapshots);
        assert!(capped.changed_files.contains(&peer));
        assert!(capped
            .diff_patch
            .as_deref()
            .unwrap()
            .contains("+oversize peer body"));
        capped
            .load_artifacts_with_limits(gcx.clone(), limits)
            .await
            .unwrap();
        let artifacts = capped.artifacts.as_ref().unwrap();
        assert_eq!(
            artifacts["coverage"]["source_artifact_files"],
            json!(requested)
        );
        assert_eq!(artifacts["coverage"]["diff_only_files"], json!([peer]));
        assert_eq!(artifacts["coverage"]["omitted_source_files"], json!([peer]));
        assert_eq!(artifacts["files"][0]["content"], "candidate\n");
        assert_eq!(artifacts["files"].as_array().unwrap().len(), 1);
        let records =
            crate::privacy::records::declared_file_records(&gcx, capped.disclosed_paths()).unwrap();
        assert_eq!(records.len(), 2);
        assert!(records
            .iter()
            .any(|record| record.path == peer.to_string_lossy().as_ref()));
        assert_eq!(
            capped.candidate_context()["source_artifact_coverage"],
            artifacts["coverage"]
        );
        let prompt =
            crate::tools::review_agents::prompts::build_stage_prompt(&catalog[0], &capped, None);
        assert!(prompt.contains(&artifacts.to_string()));
        assert_eq!(
            build_review_scope_with_limits(gcx.clone(), request(Some(base.clone())), limits)
                .await
                .unwrap_err(),
            SOURCE_LIMIT_REFUSED,
        );
        let mut explicit_overflow = request(Some(base.clone()));
        explicit_overflow.max_files = 1;
        explicit_overflow.requested.push(peer.clone());
        assert_eq!(
            build_review_scope_with_limits(gcx.clone(), explicit_overflow, limits)
                .await
                .unwrap_err(),
            SOURCE_LIMIT_REFUSED,
        );
        std::fs::write(&peer, "peer body\n").unwrap();
        assert_eq!(
            build_review_scope_with_limits(gcx.clone(), request(Some(base.clone())), limits)
                .await
                .unwrap_err(),
            SOURCE_LIMIT_REFUSED,
        );
        let fitting = SourceLimits {
            file_bytes: 16,
            total_bytes: 20,
        };
        let mut complete =
            build_review_scope_with_limits(gcx.clone(), request(Some(base.clone())), fitting)
                .await
                .unwrap();
        complete
            .load_artifacts_with_limits(gcx.clone(), fitting)
            .await
            .unwrap();
        let coverage = &complete.artifacts.as_ref().unwrap()["coverage"];
        assert_eq!(
            coverage["source_artifact_files"],
            json!([requested[0], peer])
        );
        assert_eq!(coverage["diff_only_files"], json!([]));
        assert_eq!(coverage["omitted_source_files"], json!([]));
        std::fs::write(root.join("base.txt"), "source grows past its cap\n").unwrap();
        assert_eq!(
            capped
                .ensure_current_with_limits(gcx.clone(), limits)
                .await
                .unwrap_err(),
            SOURCE_LIMIT_REFUSED,
        );
        let mut growing = std::fs::File::open(&peer).unwrap();
        let original_len = growing.metadata().unwrap().len() as usize;
        assert_eq!(original_len, 10);
        {
            use std::io::Write;
            std::fs::OpenOptions::new()
                .append(true)
                .open(&peer)
                .unwrap()
                .write_all(b"appended after metadata\n")
                .unwrap();
        }
        assert_eq!(
            read_bounded_source(&mut growing, original_len).unwrap_err(),
            SOURCE_LIMIT_REFUSED
        );
        use std::io::Seek;
        assert_eq!(growing.stream_position().unwrap(), original_len as u64 + 1);
        std::fs::write(root.join("base.txt"), "candidate\n").unwrap();
        std::fs::write(root.join("private.blocked"), "present private body\n").unwrap();
        assert_eq!(
            capped
                .ensure_current_with_limits(gcx.clone(), limits)
                .await
                .unwrap_err(),
            TARGET_REFUSED,
        );
        let mut private_request = request(Some(base.clone()));
        private_request.max_files = 1;
        assert_eq!(
            build_review_scope_with_limits(gcx.clone(), private_request, limits)
                .await
                .unwrap_err(),
            TARGET_REFUSED,
        );
        let default_scope = build_review_scope(
            gcx,
            ScopeRequest {
                requested: vec![],
                mode: ScopeMode::Broad,
                base: None,
                focus: None,
                plan: None,
                max_files: 10,
                repo_root: None,
                expected_head: None,
                checkout: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(default_scope.repo_root, Some(foreign_root));
        assert!(default_scope.requested.is_empty());
    }
}
