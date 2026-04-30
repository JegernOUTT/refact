use std::path::{Component, Path, PathBuf};

use super::types::WorktreeMeta;

pub trait WorktreeThread {
    fn worktree(&self) -> Option<&WorktreeMeta>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedPath {
    pub path: PathBuf,
    pub raw: PathBuf,
    pub used_absolute_path: bool,
    pub remapped_from: Option<PathBuf>,
    pub outside_absolute_path: bool,
    pub privacy_check_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionScope {
    worktree: WorktreeMeta,
    root: PathBuf,
    source_workspace_root: PathBuf,
    repo_root: PathBuf,
}

struct PathCandidate {
    path: PathBuf,
    used_absolute_path: bool,
    remapped_from: Option<PathBuf>,
    outside_absolute_path: bool,
}

impl ExecutionScope {
    pub fn from_worktree(worktree: &WorktreeMeta) -> Self {
        Self {
            worktree: worktree.clone(),
            root: normalize_existing_or_lexical(&worktree.root),
            source_workspace_root: normalize_existing_or_lexical(&worktree.source_workspace_root),
            repo_root: normalize_existing_or_lexical(&worktree.repo_root),
        }
    }

    pub fn from_thread<T: WorktreeThread + ?Sized>(thread: &T) -> Option<Self> {
        thread.worktree().map(Self::from_worktree)
    }

    pub fn is_enforced(&self) -> bool {
        self.worktree.enforce
    }

    pub fn worktree(&self) -> &WorktreeMeta {
        &self.worktree
    }

    pub fn effective_root(&self) -> &Path {
        self.root.as_path()
    }

    pub fn effective_project_dirs(&self) -> Vec<PathBuf> {
        vec![self.root.clone()]
    }

    pub fn repo_root(&self) -> &Path {
        self.repo_root.as_path()
    }

    pub fn resolve_path(&self, raw: &Path) -> Result<ScopedPath, String> {
        let candidate = self.candidate_for_raw_path(raw)?;
        self.finalize_candidate(raw, candidate, false, false)
    }

    pub fn resolve_existing_path(&self, raw: &Path) -> Result<ScopedPath, String> {
        let candidate = self.candidate_for_raw_path(raw)?;
        self.finalize_candidate(raw, candidate, true, false)
    }

    pub fn resolve_workdir(&self, raw: Option<&str>) -> Result<ScopedPath, String> {
        let raw_path = raw.map(PathBuf::from).unwrap_or_else(|| self.root.clone());
        let candidate = self.candidate_for_raw_path(&raw_path)?;
        self.finalize_candidate(&raw_path, candidate, true, true)
    }

    fn candidate_for_raw_path(&self, raw: &Path) -> Result<PathCandidate, String> {
        let used_absolute_path = raw.is_absolute();
        if !used_absolute_path {
            return Ok(PathCandidate {
                path: self.root.join(raw),
                used_absolute_path,
                remapped_from: None,
                outside_absolute_path: false,
            });
        }

        let normalized_raw = normalize_existing_or_lexical(raw);
        if normalized_raw.starts_with(&self.root) {
            return Ok(PathCandidate {
                path: normalized_raw,
                used_absolute_path,
                remapped_from: None,
                outside_absolute_path: false,
            });
        }

        if normalized_raw.starts_with(&self.source_workspace_root) {
            let relative = normalized_raw
                .strip_prefix(&self.source_workspace_root)
                .map_err(|e| {
                    format!(
                        "Failed to map source path '{}' into worktree '{}': {}",
                        normalized_raw.display(),
                        self.root.display(),
                        e
                    )
                })?;
            return Ok(PathCandidate {
                path: self.root.join(relative),
                used_absolute_path,
                remapped_from: Some(normalized_raw),
                outside_absolute_path: false,
            });
        }

        Ok(PathCandidate {
            path: normalized_raw,
            used_absolute_path,
            remapped_from: None,
            outside_absolute_path: true,
        })
    }

    fn finalize_candidate(
        &self,
        raw: &Path,
        candidate: PathCandidate,
        require_existing: bool,
        require_dir: bool,
    ) -> Result<ScopedPath, String> {
        let path = resolve_final_path(&candidate.path, require_existing)?;
        if require_dir && !path.is_dir() {
            return Err(format!("Path '{}' is not a directory", path.display()));
        }
        if !candidate.outside_absolute_path && !path.starts_with(&self.root) {
            return Err(format!(
                "Path '{}' escapes active worktree root '{}'",
                path.display(),
                self.root.display()
            ));
        }
        Ok(ScopedPath {
            path,
            raw: raw.to_path_buf(),
            used_absolute_path: candidate.used_absolute_path,
            remapped_from: candidate.remapped_from,
            outside_absolute_path: candidate.outside_absolute_path,
            privacy_check_required: candidate.outside_absolute_path,
        })
    }
}

fn resolve_final_path(path: &Path, require_existing: bool) -> Result<PathBuf, String> {
    match std::fs::canonicalize(path) {
        Ok(canonical) => Ok(canonical),
        Err(e) if require_existing => Err(format!(
            "Path '{}' does not exist or cannot be resolved: {}",
            path.display(),
            e
        )),
        Err(_) => {
            let parent = path.parent().ok_or_else(|| {
                format!(
                    "Path '{}' has no parent directory to validate",
                    path.display()
                )
            })?;
            let file_name = path
                .file_name()
                .ok_or_else(|| format!("Path '{}' has no file name to create", path.display()))?;
            let parent = std::fs::canonicalize(parent).map_err(|e| {
                format!(
                    "Parent directory '{}' does not exist or cannot be resolved: {}",
                    parent.display(),
                    e
                )
            })?;
            if !parent.is_dir() {
                return Err(format!(
                    "Parent path '{}' is not a directory",
                    parent.display()
                ));
            }
            Ok(parent.join(file_name))
        }
    }
}

fn normalize_existing_or_lexical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| normalize_lexical(path).unwrap_or_else(|_| path.to_path_buf()))
}

fn normalize_lexical(path: &Path) -> Result<PathBuf, String> {
    let mut normalized = if path.is_absolute() {
        PathBuf::new()
    } else {
        std::env::current_dir().map_err(|e| format!("Failed to read current dir: {}", e))?
    };

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::RootDir | Component::Prefix(_) | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }

    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_scope(root: &Path, source_root: &Path, repo_root: &Path) -> ExecutionScope {
        ExecutionScope::from_worktree(&WorktreeMeta {
            id: "wt-1".to_string(),
            kind: "chat".to_string(),
            root: root.to_path_buf(),
            source_workspace_root: source_root.to_path_buf(),
            repo_root: repo_root.to_path_buf(),
            branch: Some("feature".to_string()),
            base_branch: Some("main".to_string()),
            base_commit: Some("base".to_string()),
            task_id: None,
            card_id: None,
            agent_id: None,
            enforce: true,
        })
    }

    fn setup_dirs() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("worktree");
        let source = temp.path().join("source");
        let outside = temp.path().join("outside");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(source.join("src")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(root.join("src").join("main.rs"), "fn main() {}").unwrap();
        fs::write(outside.join("secret.txt"), "secret").unwrap();
        let root = fs::canonicalize(root).unwrap();
        let source = fs::canonicalize(source).unwrap();
        let outside = fs::canonicalize(outside).unwrap();
        let repo = source.clone();
        (temp, root, source, repo, outside)
    }

    #[test]
    fn worktree_execution_scope_reports_enforcement_and_project_dirs() {
        let (_temp, root, source, repo, _outside) = setup_dirs();
        let scope = make_scope(&root, &source, &repo);
        assert!(scope.is_enforced());
        assert_eq!(scope.effective_root(), root.as_path());
        assert_eq!(scope.effective_project_dirs(), vec![root]);
        assert_eq!(scope.repo_root(), repo.as_path());
    }

    #[test]
    fn worktree_execution_scope_resolves_relative_path_under_root() {
        let (_temp, root, source, repo, _outside) = setup_dirs();
        let scope = make_scope(&root, &source, &repo);
        let resolved = scope
            .resolve_existing_path(Path::new("src/main.rs"))
            .unwrap();
        assert_eq!(resolved.path, root.join("src").join("main.rs"));
        assert!(!resolved.used_absolute_path);
        assert!(!resolved.outside_absolute_path);
        assert!(!resolved.privacy_check_required);
        assert!(resolved.remapped_from.is_none());
    }

    #[test]
    fn worktree_execution_scope_marks_absolute_path_under_root() {
        let (_temp, root, source, repo, _outside) = setup_dirs();
        let scope = make_scope(&root, &source, &repo);
        let raw = root.join("src").join("main.rs");
        let resolved = scope.resolve_existing_path(&raw).unwrap();
        assert_eq!(resolved.path, raw);
        assert!(resolved.used_absolute_path);
        assert!(!resolved.outside_absolute_path);
        assert!(resolved.remapped_from.is_none());
    }

    #[test]
    fn worktree_execution_scope_remaps_absolute_source_path() {
        let (_temp, root, source, repo, _outside) = setup_dirs();
        let scope = make_scope(&root, &source, &repo);
        let raw = source.join("src").join("main.rs");
        let resolved = scope.resolve_existing_path(&raw).unwrap();
        assert_eq!(resolved.path, root.join("src").join("main.rs"));
        assert!(resolved.used_absolute_path);
        assert_eq!(resolved.remapped_from, Some(raw));
        assert!(!resolved.outside_absolute_path);
        assert!(!resolved.privacy_check_required);
    }

    #[test]
    fn worktree_execution_scope_marks_outside_absolute_existing_path() {
        let (_temp, root, source, repo, outside) = setup_dirs();
        let scope = make_scope(&root, &source, &repo);
        let raw = outside.join("secret.txt");
        let resolved = scope.resolve_existing_path(&raw).unwrap();
        assert_eq!(resolved.path, raw);
        assert!(resolved.used_absolute_path);
        assert!(resolved.outside_absolute_path);
        assert!(resolved.privacy_check_required);
        assert!(resolved.remapped_from.is_none());
    }

    #[test]
    fn worktree_execution_scope_marks_outside_absolute_create_path() {
        let (_temp, root, source, repo, outside) = setup_dirs();
        let scope = make_scope(&root, &source, &repo);
        let raw = outside.join("new.txt");
        let resolved = scope.resolve_path(&raw).unwrap();
        assert_eq!(resolved.path, raw);
        assert!(resolved.used_absolute_path);
        assert!(resolved.outside_absolute_path);
        assert!(resolved.privacy_check_required);
    }

    #[test]
    fn worktree_execution_scope_rejects_relative_traversal() {
        let (_temp, root, source, repo, _outside) = setup_dirs();
        let scope = make_scope(&root, &source, &repo);
        let result = scope.resolve_path(Path::new("../escaped.txt"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("escapes active worktree"));
    }

    #[test]
    fn worktree_execution_scope_rejects_non_existing_parent() {
        let (_temp, root, source, repo, _outside) = setup_dirs();
        let scope = make_scope(&root, &source, &repo);
        let result = scope.resolve_path(Path::new("missing/new.txt"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Parent directory"));
    }

    #[test]
    fn worktree_execution_scope_resolves_non_existing_file_with_existing_parent() {
        let (_temp, root, source, repo, _outside) = setup_dirs();
        let scope = make_scope(&root, &source, &repo);
        let resolved = scope.resolve_path(Path::new("src/new.rs")).unwrap();
        assert_eq!(resolved.path, root.join("src").join("new.rs"));
        assert!(!resolved.used_absolute_path);
        assert!(!resolved.outside_absolute_path);
    }

    #[test]
    fn worktree_execution_scope_resolves_workdir() {
        let (_temp, root, source, repo, _outside) = setup_dirs();
        let scope = make_scope(&root, &source, &repo);
        let default_workdir = scope.resolve_workdir(None).unwrap();
        assert_eq!(default_workdir.path, root);
        let src_workdir = scope.resolve_workdir(Some("src")).unwrap();
        assert!(src_workdir.path.ends_with("src"));
        assert!(!src_workdir.outside_absolute_path);
    }

    #[test]
    fn worktree_execution_scope_rejects_file_as_workdir() {
        let (_temp, root, source, repo, _outside) = setup_dirs();
        let scope = make_scope(&root, &source, &repo);
        let result = scope.resolve_workdir(Some("src/main.rs"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not a directory"));
    }

    #[cfg(unix)]
    #[test]
    fn worktree_execution_scope_rejects_symlink_escape() {
        let (_temp, root, source, repo, outside) = setup_dirs();
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
        let scope = make_scope(&root, &source, &repo);
        let result = scope.resolve_existing_path(Path::new("link/secret.txt"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("escapes active worktree"));
    }

    #[cfg(windows)]
    #[test]
    fn worktree_execution_scope_rejects_symlink_escape() {
        let (_temp, root, source, repo, outside) = setup_dirs();
        if std::os::windows::fs::symlink_dir(&outside, root.join("link")).is_err() {
            return;
        }
        let scope = make_scope(&root, &source, &repo);
        let result = scope.resolve_existing_path(Path::new("link").join("secret.txt").as_path());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("escapes active worktree"));
    }
}
