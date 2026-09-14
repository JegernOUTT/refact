//! Work-root resolution.
//!
//! A *work root* is the directory work actually happens in: a registered
//! worktree checkout, or the source project checkout when the work is not
//! worktree-scoped. Every root carries the *project identity* of its source
//! workspace root, which is what lets the catalog span roots while staying
//! inside one project.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::global_context::GlobalContext;
use crate::worktrees::service::{project_hash_for_path, WorktreeService};
use crate::worktrees::types::WorktreeMeta;

use super::types::{WorkRootKind, WorkRootRef};

/// Canonical project identity for a source workspace root.
///
/// Uses the same hash the worktree registry keys on, so a worktree and its
/// source checkout always agree on which project they belong to.
pub fn project_identity(source_workspace_root: &Path) -> String {
    project_hash_for_path(source_workspace_root)
}

fn display_path(path: &Path) -> String {
    dunce::simplified(path).to_string_lossy().to_string()
}

/// The root for a plain (non-worktree) project checkout.
pub fn project_root_ref(source_workspace_root: &Path) -> WorkRootRef {
    let project = project_identity(source_workspace_root);
    WorkRootRef {
        id: project.clone(),
        kind: WorkRootKind::Project,
        path: Some(display_path(source_workspace_root)),
        branch: None,
        project,
        source_path: Some(display_path(source_workspace_root)),
    }
}

/// The root described by a chat's own worktree metadata.
pub fn worktree_root_ref(meta: &WorktreeMeta) -> WorkRootRef {
    WorkRootRef {
        id: meta.id.clone(),
        kind: WorkRootKind::Worktree,
        path: Some(display_path(&meta.root)),
        branch: meta.branch.clone(),
        project: project_identity(&meta.source_workspace_root),
        source_path: Some(display_path(&meta.source_workspace_root)),
    }
}

/// Every work root of one source project: the checkout itself plus each
/// worktree registered against it.
///
/// Reads the on-disk worktree registry once. Registry problems are reported as
/// a degradation string instead of failing the whole catalog.
pub struct ProjectRoots {
    pub project: String,
    pub source_workspace_root: PathBuf,
    /// Keyed by [`WorkRootRef::id`].
    pub by_id: BTreeMap<String, WorkRootRef>,
    pub degraded: Vec<String>,
}

impl ProjectRoots {
    pub fn get(&self, id: &str) -> Option<&WorkRootRef> {
        self.by_id.get(id)
    }

    pub fn project_root(&self) -> WorkRootRef {
        project_root_ref(&self.source_workspace_root)
    }

    /// Deterministic listing: the project checkout first, then worktrees by id.
    pub fn all(&self) -> Vec<WorkRootRef> {
        let project_root = self.project_root();
        let mut out = vec![project_root.clone()];
        out.extend(
            self.by_id
                .values()
                .filter(|root| root.id != project_root.id)
                .cloned(),
        );
        out
    }

    /// True when `root` belongs to this project.
    pub fn contains_project(&self, root: &WorkRootRef) -> bool {
        root.project == self.project
    }
}

/// Resolve the active source project root.
///
/// When the engine itself runs inside a worktree, the *source* checkout is the
/// project identity we key on, not the worktree directory.
pub async fn resolve_source_workspace_root(
    gcx: Arc<GlobalContext>,
    requested: Option<&str>,
) -> Result<PathBuf, String> {
    let project_dirs = crate::files_correction::get_project_dirs(gcx.clone()).await;
    match requested {
        Some(requested) => {
            let canonical = tokio::fs::canonicalize(requested)
                .await
                .map(|path| dunce::simplified(&path).to_path_buf())
                .map_err(|e| format!("invalid source workspace root '{requested}': {e}"))?;
            for dir in &project_dirs {
                if let Ok(dir_canonical) = tokio::fs::canonicalize(dir).await {
                    if dunce::simplified(&dir_canonical) == canonical.as_path() {
                        return Ok(canonical);
                    }
                }
            }
            Err("invalid source workspace root: not a workspace directory".to_string())
        }
        None => {
            let first = project_dirs
                .into_iter()
                .next()
                .ok_or_else(|| "no project root available".to_string())?;
            tokio::fs::canonicalize(&first)
                .await
                .map(|path| dunce::simplified(&path).to_path_buf())
                .map_err(|e| format!("invalid project root '{}': {e}", first.display()))
        }
    }
}

/// Load every work root for the given source project.
pub async fn load_project_roots(
    gcx: Arc<GlobalContext>,
    source_workspace_root: PathBuf,
) -> ProjectRoots {
    let project = project_identity(&source_workspace_root);
    let mut by_id = BTreeMap::new();
    let mut degraded = Vec::new();

    let project_root = project_root_ref(&source_workspace_root);
    by_id.insert(project_root.id.clone(), project_root);

    match WorktreeService::new_async(gcx.cache_dir.clone(), source_workspace_root.clone()).await {
        Ok(service) => match service.load_registry().await {
            Ok(registry) => {
                for record in registry.records {
                    let root = worktree_root_ref(&record.meta);
                    by_id.insert(root.id.clone(), root);
                }
            }
            Err(error) => degraded.push(format!("worktree registry unavailable: {error}")),
        },
        Err(error) => degraded.push(format!("worktree service unavailable: {error}")),
    }

    ProjectRoots {
        project,
        source_workspace_root,
        by_id,
        degraded,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(id: &str, root: &str, source: &str) -> WorktreeMeta {
        WorktreeMeta {
            id: id.to_string(),
            kind: "chat".to_string(),
            root: PathBuf::from(root),
            source_workspace_root: PathBuf::from(source),
            repo_root: PathBuf::from(source),
            branch: Some("feature".to_string()),
            base_branch: None,
            base_commit: None,
            task_id: None,
            card_id: None,
            agent_id: None,
            enforce: false,
        }
    }

    #[test]
    fn a_worktree_and_its_source_checkout_share_one_project_identity() {
        let source = PathBuf::from("/tmp/refact-coordination-project");
        let worktree = worktree_root_ref(&meta(
            "wt-1",
            "/tmp/wt-1",
            "/tmp/refact-coordination-project",
        ));
        let project = project_root_ref(&source);

        assert_eq!(worktree.project, project.project);
        assert_ne!(worktree.id, project.id);
        assert_eq!(worktree.kind, WorkRootKind::Worktree);
        assert_eq!(project.kind, WorkRootKind::Project);
    }

    #[test]
    fn different_source_checkouts_get_different_project_identities() {
        let a = project_root_ref(Path::new("/tmp/project-a"));
        let b = project_root_ref(Path::new("/tmp/project-b"));
        assert_ne!(a.project, b.project);
    }

    #[test]
    fn all_lists_the_project_checkout_first_then_worktrees_by_id() {
        let source = PathBuf::from("/tmp/refact-coordination-order");
        let mut by_id = BTreeMap::new();
        let project_root = project_root_ref(&source);
        by_id.insert(project_root.id.clone(), project_root.clone());
        for id in ["wt-b", "wt-a"] {
            let root = worktree_root_ref(&meta(id, "/tmp/x", "/tmp/refact-coordination-order"));
            by_id.insert(root.id.clone(), root);
        }
        let roots = ProjectRoots {
            project: project_root.project.clone(),
            source_workspace_root: source,
            by_id,
            degraded: Vec::new(),
        };

        let ids: Vec<_> = roots.all().into_iter().map(|root| root.id).collect();
        assert_eq!(ids[0], project_root.id);
        assert_eq!(&ids[1..], &["wt-a".to_string(), "wt-b".to_string()]);
    }

    #[test]
    fn redacting_a_root_keeps_identity_and_drops_locations() {
        let root = worktree_root_ref(&meta("wt-1", "/tmp/wt-1", "/tmp/src"));
        let redacted = root.redacted();

        assert_eq!(redacted.id, root.id);
        assert_eq!(redacted.project, root.project);
        assert_eq!(redacted.path, None);
        assert_eq!(redacted.source_path, None);
        assert_eq!(redacted.branch, None);
    }
}
