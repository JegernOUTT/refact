//! Work-root resolution.
//!
//! A *work root* is the directory work actually happens in: a registered
//! worktree checkout, or the source project checkout when the work is not
//! worktree-scoped. Every root carries the *project identity* of its source
//! workspace root, which is what lets the catalog span roots while staying
//! inside one project.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

use crate::global_context::GlobalContext;
use crate::worktrees::service::{project_hash_for_path, WorktreeService};
use crate::worktrees::types::WorktreeMeta;

use super::types::{RootPresence, WorkRootKind, WorkRootRef};

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

/// One cached existence answer for a work root.
struct PresenceEntry {
    presence: RootPresence,
    probed_at_ms: u64,
    /// Filesystem probes actually performed for this root. Lets a test prove
    /// that projecting many roots repeatedly does not stat them every turn.
    probes: u64,
}

fn presence_cache() -> &'static RwLock<HashMap<String, PresenceEntry>> {
    static SLOT: OnceLock<RwLock<HashMap<String, PresenceEntry>>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(HashMap::new()))
}

fn with_presence_cache<R>(f: impl FnOnce(&mut HashMap<String, PresenceEntry>) -> R) -> R {
    let mut guard = presence_cache()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut guard)
}

fn now_ms() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}

fn presence_ttl_ms() -> u64 {
    crate::file_activity::settings().work_root_presence_ttl_ms
}

/// Drop answers older than the configured TTL so the cache stays proportional
/// to the roots still being projected.
fn prune_presence_cache(now: u64, ttl: u64) {
    with_presence_cache(|cache| {
        cache.retain(|_, entry| now.saturating_sub(entry.probed_at_ms) < ttl);
    });
}

/// Decide presence from the outcome of a single `stat`.
///
/// `Ok(is_dir)` answers the question directly: a path that resolves to
/// something other than a directory is not a checkout. Only a conclusive
/// `NotFound` is `Missing` — every other error (permission denied, IO failure)
/// fails closed to `Unknown` rather than guessing.
fn presence_from(probe: Result<bool, std::io::ErrorKind>) -> RootPresence {
    match probe {
        Ok(true) => RootPresence::Present,
        Ok(false) => RootPresence::Missing,
        Err(std::io::ErrorKind::NotFound) => RootPresence::Missing,
        Err(_) => RootPresence::Unknown,
    }
}

/// Classify one recorded root path without touching it.
///
/// Existence only: no pruning, no cleanup, no repair.
async fn classify(path: &Path) -> RootPresence {
    presence_from(
        tokio::fs::metadata(path)
            .await
            .map(|metadata| metadata.is_dir())
            .map_err(|error| error.kind()),
    )
}

/// Read-through TTL cache over [`classify`].
async fn probe_presence(key: &str, path: &Path, now: u64, ttl: u64) -> RootPresence {
    let cached = with_presence_cache(|cache| {
        cache
            .get(key)
            .filter(|entry| now.saturating_sub(entry.probed_at_ms) < ttl)
            .map(|entry| entry.presence)
    });
    if let Some(presence) = cached {
        return presence;
    }
    let presence = classify(path).await;
    with_presence_cache(|cache| {
        let probes = cache.get(key).map_or(0, |entry| entry.probes);
        cache.insert(
            key.to_string(),
            PresenceEntry {
                presence,
                probed_at_ms: now,
                probes: probes.saturating_add(1),
            },
        );
    });
    presence
}

#[cfg(test)]
pub fn reset_presence_cache_for_test() {
    with_presence_cache(|cache| cache.clear());
}

#[cfg(test)]
pub fn presence_probes_for_test(path: &Path) -> u64 {
    with_presence_cache(|cache| {
        cache
            .get(&display_path(path))
            .map_or(0, |entry| entry.probes)
    })
}

/// The root for a plain (non-worktree) project checkout.
///
/// Presence is left `Unknown`: only [`load_project_roots`] probes the disk.
pub fn project_root_ref(source_workspace_root: &Path) -> WorkRootRef {
    let project = project_identity(source_workspace_root);
    WorkRootRef {
        id: project.clone(),
        kind: WorkRootKind::Project,
        path: Some(display_path(source_workspace_root)),
        branch: None,
        project,
        source_path: Some(display_path(source_workspace_root)),
        presence: RootPresence::Unknown,
    }
}

/// The root described by a chat's own worktree metadata.
///
/// Presence is left `Unknown`: only [`load_project_roots`] probes the disk.
pub fn worktree_root_ref(meta: &WorktreeMeta) -> WorkRootRef {
    WorkRootRef {
        id: meta.id.clone(),
        kind: WorkRootKind::Worktree,
        path: Some(display_path(&meta.root)),
        branch: meta.branch.clone(),
        project: project_identity(&meta.source_workspace_root),
        source_path: Some(display_path(&meta.source_workspace_root)),
        presence: RootPresence::Unknown,
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

    /// Prefers the stored entry so the probed presence survives.
    pub fn project_root(&self) -> WorkRootRef {
        let fresh = project_root_ref(&self.source_workspace_root);
        self.by_id.get(&fresh.id).cloned().unwrap_or(fresh)
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

    let now = now_ms();
    let ttl = presence_ttl_ms();
    prune_presence_cache(now, ttl);
    for root in by_id.values_mut() {
        let Some(path) = root.path.clone() else {
            continue;
        };
        root.presence = probe_presence(&path, Path::new(&path), now, ttl).await;
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
    fn redacting_a_root_keeps_identity_and_drops_locations() {
        let root = worktree_root_ref(&meta("wt-1", "/tmp/wt-1", "/tmp/src"));
        let redacted = root.redacted();

        assert_eq!(redacted.id, root.id);
        assert_eq!(redacted.project, root.project);
        assert_eq!(redacted.path, None);
        assert_eq!(redacted.source_path, None);
        assert_eq!(redacted.branch, None);
        // Whether a withheld path exists on disk is itself a disclosure.
        assert_eq!(redacted.presence, RootPresence::Unknown);
    }

    #[test]
    fn only_a_conclusive_absence_is_missing_and_every_other_failure_is_unknown() {
        use std::io::ErrorKind;

        assert_eq!(presence_from(Ok(true)), RootPresence::Present);
        assert_eq!(presence_from(Ok(false)), RootPresence::Missing);
        assert_eq!(
            presence_from(Err(ErrorKind::NotFound)),
            RootPresence::Missing
        );
        for kind in [
            ErrorKind::PermissionDenied,
            ErrorKind::Other,
            ErrorKind::TimedOut,
            ErrorKind::Interrupted,
        ] {
            assert_eq!(presence_from(Err(kind)), RootPresence::Unknown, "{kind:?}");
            assert!(!presence_from(Err(kind)).is_active_location());
        }
    }
}
