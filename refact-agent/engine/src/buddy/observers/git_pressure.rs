use chrono::{DateTime, Utc};

use crate::buddy::memory_lifecycle::{
    detect_git_memory_ops, load_memory_doc_snapshots_from_knowledge_dirs, MemoryLifecycleOp,
    MemoryOpType,
};
use crate::buddy::observers::{BuddyObserver, ObserverContext};
use crate::buddy::settings::BuddySettings;
use crate::buddy::types::{BuddyFact, BuddyFactKind};
use crate::app_state::AppState;
use crate::git::operations::{mine_git_history, GitHistoryOptions, GitHistoryReport};

pub struct GitPressureObserver;

pub(crate) const MAX_UNCOMMITTED_STATUS_SCAN: usize = 2000;
pub(crate) const MAX_DIFF_COMMITS: usize = 200;
pub(crate) const MAX_GIT_MEMORY_HISTORY_COMMITS: usize = 120;
pub(crate) const MAX_GIT_MEMORY_OPS_PER_SCAN: usize = 40;

fn empty_git_history_report() -> GitHistoryReport {
    GitHistoryReport {
        commits: Vec::new(),
        cochanges: Vec::new(),
        hotspots: Vec::new(),
        commit_cap_hit: false,
    }
}

#[cfg(test)]
mod tests {

    use std::path::Path;
}

fn path_hash(p: &std::path::Path) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    p.hash(&mut h);
    format!("{:x}", h.finish())
}

pub fn count_uncommitted(project_root: &std::path::Path) -> Option<usize> {
    let repo = git2::Repository::discover(project_root).ok()?;
    count_uncommitted_in_repo(&repo)
}

pub(crate) fn count_uncommitted_in_repo(repo: &git2::Repository) -> Option<usize> {
    use git2::{StatusOptions, StatusShow};
    let mut opts = StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(false)
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

#[cfg_attr(not(test), allow(dead_code))]
pub async fn detect_and_enqueue_git_memory_ops(
    project_root: &std::path::Path,
    now: DateTime<Utc>,
) -> Vec<MemoryLifecycleOp> {
    let root = project_root.to_path_buf();
    let mined = tokio::task::spawn_blocking(move || {
        let report = match mine_git_history(
            &root,
            GitHistoryOptions {
                max_commits: MAX_GIT_MEMORY_HISTORY_COMMITS,
                cochange_threshold: 3,
                max_hotspots: 20,
                max_cochange_pairs: 50,
                ..GitHistoryOptions::default()
            },
        ) {
            Ok(report) => report,
            Err(err) => {
                tracing::debug!("git_pressure: git memory mining skipped: {}", err);
                return (root, empty_git_history_report());
            }
        };
        (root, report)
    })
    .await
    .unwrap_or_else(|_| (project_root.to_path_buf(), empty_git_history_report()));
    let (root, report) = mined;
    if report.commits.is_empty() {
        return Vec::new();
    }
    let knowledge_dirs = vec![root.join(crate::file_filter::KNOWLEDGE_FOLDER_NAME)]
        .into_iter()
        .filter(|dir| dir.exists())
        .collect::<Vec<_>>();
    let docs = load_memory_doc_snapshots_from_knowledge_dirs(&knowledge_dirs).await;
    let mut ops = tokio::task::spawn_blocking(move || detect_git_memory_ops(&report, &docs, now))
        .await
        .unwrap_or_default();
    ops.truncate(MAX_GIT_MEMORY_OPS_PER_SCAN);
    sort_git_memory_ops_for_enqueue(&mut ops);
    let mut enqueued = Vec::new();
    for op in ops.clone() {
        let op_id = op.op_id.clone();
        match crate::buddy::storage::enqueue_memory_op(project_root, op).await {
            Ok(updated) => {
                if let Some(saved) = updated.ops.iter().find(|saved| saved.op_id == op_id) {
                    enqueued.push(saved.clone());
                }
            }
            Err(err) => tracing::warn!("buddy: failed to enqueue git memory op: {}", err),
        }
    }
    enqueued
}

fn sort_git_memory_ops_for_enqueue(ops: &mut [MemoryLifecycleOp]) {
    ops.sort_by(|a, b| {
        git_memory_enqueue_priority(a)
            .cmp(&git_memory_enqueue_priority(b))
            .then_with(|| a.idempotency_key.cmp(&b.idempotency_key))
            .then_with(|| a.op_id.cmp(&b.op_id))
    });
}

fn git_memory_enqueue_priority(op: &MemoryLifecycleOp) -> u8 {
    match op.op_type {
        MemoryOpType::RepairLinks => 0,
        MemoryOpType::MarkStale => 1,
        MemoryOpType::MarkReviewNeeded => 2,
        MemoryOpType::CreateMemory => 3,
        _ => 4,
    }
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

    async fn observe(&self, gcx: AppState, ctx: &ObserverContext) -> Vec<BuddyFact> {
        let root = ctx.project_root.clone();
        let now = ctx.now;
        let facts_root = root.clone();
        let facts =
            tokio::task::spawn_blocking(move || detect_git_pressure_facts(&facts_root, now))
                .await
                .unwrap_or_default();
        let git_ops = detect_and_enqueue_git_memory_ops(&root, now).await;
        if !git_ops.is_empty() {
            let buddy_arc = gcx.buddy.buddy.clone();
            let updated_memory_ops = crate::buddy::storage::load_memory_ops(&root).await;
            let mut buddy = buddy_arc.lock().await;
            if let Some(svc) = buddy.as_mut() {
                svc.memory_ops = updated_memory_ops;
            }
        }
        facts
    }
}
