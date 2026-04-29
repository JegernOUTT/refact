use std::sync::Arc;
use chrono::{DateTime, Utc};
use tokio::sync::RwLock;

use crate::buddy::observers::{BuddyObserver, ObserverContext};
use crate::buddy::settings::BuddySettings;
use crate::buddy::types::{BuddyFact, BuddyFactKind};
use crate::global_context::GlobalContext;

pub struct GitPressureObserver;

pub(crate) const MAX_UNCOMMITTED_STATUS_SCAN: usize = 2000;
pub(crate) const MAX_DIFF_COMMITS: usize = 200;

fn path_hash(p: &std::path::Path) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    p.hash(&mut h);
    format!("{:x}", h.finish())
}

pub fn count_uncommitted(project_root: &std::path::Path) -> Option<usize> {
    use git2::{Repository, StatusOptions, StatusShow};
    let repo = Repository::discover(project_root).ok()?;
    let mut opts = StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false)
        .show(StatusShow::IndexAndWorkdir);
    let statuses = repo.statuses(Some(&mut opts)).ok()?;
    let count = statuses
        .iter()
        .filter(|s| !s.status().is_empty())
        .take(MAX_UNCOMMITTED_STATUS_SCAN)
        .count();
    Some(count)
}

pub(crate) fn git_diff_widening(
    project_root: &std::path::Path,
    now: DateTime<Utc>,
) -> Option<(u32, Vec<String>)> {
    let repo = git2::Repository::discover(project_root).ok()?;
    let head = repo.head().ok()?.peel_to_commit().ok()?;
    let cutoff_ts = (now - chrono::Duration::hours(4)).timestamp();

    let mut walker = repo.revwalk().ok()?;
    walker
        .set_sorting(git2::Sort::TIME | git2::Sort::TOPOLOGICAL)
        .ok()?;
    walker.push(head.id()).ok()?;

    let mut oldest_in_window = None;
    let mut first_before_cutoff = None;
    for oid in walker.take(MAX_DIFF_COMMITS) {
        let oid = oid.ok()?;
        let commit = repo.find_commit(oid).ok()?;
        if commit.time().seconds() >= cutoff_ts {
            oldest_in_window = Some(oid);
        } else {
            first_before_cutoff = Some(oid);
            break;
        }
    }

    let oldest_oid = oldest_in_window?;
    let head_tree = head.tree().ok()?;
    let base_tree = if let Some(oid) = first_before_cutoff {
        repo.find_commit(oid).ok()?.tree().ok()
    } else {
        repo.find_commit(oldest_oid)
            .ok()?
            .parent(0)
            .ok()
            .and_then(|parent| parent.tree().ok())
    };

    let diff = repo
        .diff_tree_to_tree(base_tree.as_ref(), Some(&head_tree), None)
        .ok()?;
    let stats = diff.stats().ok()?;
    let lines = (stats.insertions() + stats.deletions()) as u32;

    let mut dirs = std::collections::HashSet::new();
    let _ = diff.foreach(
        &mut |delta, _| {
            if let Some(path) = delta.new_file().path() {
                if let Some(parent) = path.parent() {
                    let s = parent.to_string_lossy().into_owned();
                    if !s.is_empty() {
                        dirs.insert(s);
                    }
                }
            }
            true
        },
        None,
        None,
        None,
    );

    if lines > 500 && dirs.len() >= 3 {
        let mut top: Vec<String> = dirs.into_iter().collect();
        top.sort();
        top.truncate(5);
        Some((lines, top))
    } else {
        None
    }
}

pub fn detect_git_pressure_facts(
    project_root: &std::path::Path,
    now: DateTime<Utc>,
) -> Vec<BuddyFact> {
    let mut facts = vec![];
    let hash = path_hash(project_root);

    if let Some(count) = count_uncommitted(project_root) {
        if count > 25 {
            tracing::debug!("git_pressure: uncommitted files={}", count);
            facts.push(BuddyFact {
                kind: BuddyFactKind::UncommittedPressure,
                key: format!("git:pressure:{}", hash),
                source: "git_pressure",
                payload: serde_json::json!({
                    "files": count,
                    "lines": 0,
                    "dirs": [],
                }),
                seen_at: now,
                confidence: 0.9,
            });
        }
    }

    if let Some((lines, dirs)) = git_diff_widening(project_root, now) {
        tracing::debug!("git_pressure: diff widening lines={}", lines);
        facts.push(BuddyFact {
            kind: BuddyFactKind::GitDiffWidening,
            key: format!("git:widening:{}", hash),
            source: "git_pressure",
            payload: serde_json::json!({
                "files": 0,
                "lines": lines,
                "dirs": dirs,
            }),
            seen_at: now,
            confidence: 0.8,
        });
    }

    facts
}

#[async_trait::async_trait]
impl BuddyObserver for GitPressureObserver {
    fn id(&self) -> &'static str {
        "git_pressure"
    }

    fn cadence_seconds(&self) -> u64 {
        300
    }

    fn requires_setting(&self, settings: &BuddySettings) -> bool {
        settings.observers.git_pressure
    }

    async fn observe(
        &self,
        gcx: Arc<RwLock<GlobalContext>>,
        ctx: &ObserverContext,
    ) -> Vec<BuddyFact> {
        let root = ctx.project_root.clone();
        let now = ctx.now;
        let _ = gcx;
        tokio::task::spawn_blocking(move || detect_git_pressure_facts(&root, now))
            .await
            .unwrap_or_default()
    }
}
