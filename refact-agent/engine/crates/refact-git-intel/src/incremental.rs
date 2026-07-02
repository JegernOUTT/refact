use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::{push_head_or_empty, CommitRecord, GitIntel};
use git2::{Commit, Repository, Sort};
use serde::{Deserialize, Serialize};

const MAX_FILES_PER_COMMIT_FOR_COCHANGE: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GitTier {
    Essential,
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncrementalConfig {
    pub tier: GitTier,
    pub since_ts: Option<i64>,
    #[serde(default)]
    pub seen_oids: HashSet<String>,
    pub max_commits: usize,
    pub deep_walk_limit: usize,
}

impl Default for IncrementalConfig {
    fn default() -> Self {
        Self {
            tier: GitTier::Full,
            since_ts: None,
            seen_oids: HashSet::new(),
            max_commits: 500,
            deep_walk_limit: 20_000,
        }
    }
}

pub fn mine_incremental(repo_path: &Path, cfg: &IncrementalConfig) -> Result<GitIntel, String> {
    let repo = Repository::open(repo_path).map_err(|e| format!("git open: {e}"))?;
    let mut revwalk = repo.revwalk().map_err(|e| format!("git revwalk: {e}"))?;
    if !push_head_or_empty(&mut revwalk)? {
        return Ok(GitIntel::default());
    }
    revwalk
        .set_sorting(Sort::TIME)
        .map_err(|e| format!("git sort: {e}"))?;

    let mut intel = GitIntel::default();
    let mut walked = 0usize;
    for oid in revwalk {
        if walked >= cfg.deep_walk_limit || intel.commits_analyzed as usize >= cfg.max_commits {
            break;
        }
        walked += 1;

        let oid = oid.map_err(|e| format!("git oid: {e}"))?;
        let oid_string = oid.to_string();
        if cfg.seen_oids.contains(&oid_string) {
            continue;
        }
        let commit = repo
            .find_commit(oid)
            .map_err(|e| format!("git find_commit: {e}"))?;
        let ts = commit.time().seconds();
        if let Some(since_ts) = cfg.since_ts {
            if ts < since_ts || (ts == since_ts && cfg.seen_oids.is_empty()) {
                break;
            }
        }

        collect_commit(
            &repo,
            &commit,
            oid_string,
            &mut intel,
            cfg.tier == GitTier::Full,
        );
    }

    Ok(intel)
}

pub fn newest_commit_ts(repo_path: &Path) -> Result<Option<i64>, String> {
    let repo = Repository::open(repo_path).map_err(|e| format!("git open: {e}"))?;
    let head = match repo.head() {
        Ok(head) => head,
        Err(e) if crate::is_empty_head_error(&e) => {
            return Ok(None);
        }
        Err(e) => return Err(format!("git head: {e}")),
    };
    let Some(oid) = head.target() else {
        return Ok(None);
    };
    let commit = repo
        .find_commit(oid)
        .map_err(|e| format!("git find_commit: {e}"))?;
    Ok(Some(commit.time().seconds()))
}

pub fn merge_intel(base: &mut GitIntel, delta: &GitIntel) {
    for (path, churn) in &delta.file_churn {
        *base.file_churn.entry(path.clone()).or_default() += churn;
    }

    for (path, authors) in &delta.file_authors {
        let base_authors = base.file_authors.entry(path.clone()).or_default();
        for (author, count) in authors {
            *base_authors.entry(author.clone()).or_default() += count;
        }
    }

    for (pair, count) in &delta.co_change {
        *base.co_change.entry(pair.clone()).or_default() += count;
    }

    base.commits_analyzed = base.commits_analyzed.saturating_add(delta.commits_analyzed);
    base.commit_records
        .extend(delta.commit_records.iter().cloned());
    base.commit_records.sort_by(|a, b| {
        b.ts.cmp(&a.ts)
            .then_with(|| a.author.cmp(&b.author))
            .then_with(|| a.committer.cmp(&b.committer))
            .then_with(|| a.message.cmp(&b.message))
            .then_with(|| a.files.cmp(&b.files))
    });
}

fn collect_commit(
    repo: &Repository,
    commit: &Commit,
    oid: String,
    intel: &mut GitIntel,
    include_co_change: bool,
) {
    let author = commit.author().email().unwrap_or("unknown").to_string();
    let committer = commit.committer().email().unwrap_or("unknown").to_string();
    let ts = commit.time().seconds();
    let message = commit.message().unwrap_or("").to_string();
    let file_stats = changed_files_with_stats(repo, commit);
    let files: Vec<String> = file_stats.iter().map(|(path, _, _)| path.clone()).collect();

    intel.commits_analyzed += 1;
    for f in &files {
        *intel.file_churn.entry(f.clone()).or_default() += 1;
        *intel
            .file_authors
            .entry(f.clone())
            .or_default()
            .entry(author.clone())
            .or_default() += 1;
    }

    if include_co_change && files.len() <= MAX_FILES_PER_COMMIT_FOR_COCHANGE {
        for a in 0..files.len() {
            for b in (a + 1)..files.len() {
                let key = (files[a].clone(), files[b].clone());
                *intel.co_change.entry(key).or_default() += 1;
            }
        }
    }

    intel.commit_records.push(CommitRecord {
        oid: Some(oid),
        ts,
        author,
        committer,
        message,
        files: file_stats,
    });
}

fn changed_files_with_stats(repo: &Repository, commit: &Commit) -> Vec<(String, u32, u32)> {
    let tree = match commit.tree() {
        Ok(t) => t,
        Err(_) => return vec![],
    };
    let parent_tree = commit.parent(0).ok().and_then(|p| p.tree().ok());
    let diff = match repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None) {
        Ok(d) => d,
        Err(_) => return vec![],
    };
    let mut files: HashMap<String, (u32, u32)> = HashMap::new();
    for delta in diff.deltas() {
        if let Some(path) = delta.new_file().path().or_else(|| delta.old_file().path()) {
            files.entry(path.to_string_lossy().to_string()).or_default();
        }
    }

    let _ = diff.foreach(
        &mut |_delta, _progress| true,
        None,
        None,
        Some(&mut |delta, _hunk, line| {
            let Some(path) = delta.new_file().path().or_else(|| delta.old_file().path()) else {
                return true;
            };
            let entry = files.entry(path.to_string_lossy().to_string()).or_default();
            match line.origin() {
                '+' => entry.0 = entry.0.saturating_add(1),
                '-' => entry.1 = entry.1.saturating_add(1),
                _ => {}
            }
            true
        }),
    );

    let mut files: Vec<(String, u32, u32)> = files
        .into_iter()
        .map(|(path, (added, deleted))| (path, added, deleted))
        .collect();
    files.sort();
    files.dedup();
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::{Signature, Time};
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_REPO_ID: AtomicU64 = AtomicU64::new(0);

    struct TempRepo {
        path: PathBuf,
    }

    impl TempRepo {
        fn new() -> Self {
            let id = NEXT_REPO_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "refact_git_intel_incremental_{}_{}",
                std::process::id(),
                id
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn commit_files_at(
        repo: &Repository,
        files: &[(&str, &str)],
        msg: &str,
        name: &str,
        email: &str,
        ts: i64,
    ) -> git2::Oid {
        let workdir = repo.workdir().unwrap().to_path_buf();
        for (p, c) in files {
            let path = workdir.join(p);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, c).unwrap();
        }
        let mut index = repo.index().unwrap();
        for (p, _) in files {
            index.add_path(Path::new(p)).unwrap();
        }
        index.write().unwrap();
        let tree_oid = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let time = Time::new(ts, 0);
        let sig = Signature::new(name, email, &time).unwrap();
        let parents: Vec<git2::Commit> = repo
            .head()
            .ok()
            .and_then(|h| h.target())
            .and_then(|oid| repo.find_commit(oid).ok())
            .into_iter()
            .collect();
        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &parent_refs)
            .unwrap()
    }

    fn fixture_repo() -> TempRepo {
        let dir = TempRepo::new();
        let repo = Repository::init(dir.path()).unwrap();
        commit_files_at(
            &repo,
            &[("a.rs", "1\n"), ("b.rs", "1\n")],
            "first",
            "Alice",
            "alice@x.com",
            1_700_000_000,
        );
        commit_files_at(
            &repo,
            &[("a.rs", "2\n"), ("b.rs", "2\n")],
            "second",
            "Bob",
            "bob@x.com",
            1_700_000_100,
        );
        commit_files_at(
            &repo,
            &[("a.rs", "3\n")],
            "third",
            "Cara",
            "cara@x.com",
            1_700_000_200,
        );
        dir
    }

    #[test]
    fn since_ts_excludes_that_commit() {
        let dir = fixture_repo();
        let cfg = IncrementalConfig {
            since_ts: Some(1_700_000_000),
            max_commits: 10,
            ..IncrementalConfig::default()
        };

        let intel = mine_incremental(dir.path(), &cfg).unwrap();

        assert_eq!(intel.commits_analyzed, 2);
        assert_eq!(intel.commit_records.len(), 2);
        assert!(intel.commit_records.iter().all(|c| c.ts > 1_700_000_000));
        assert_eq!(intel.file_churn.get("a.rs"), Some(&2));
        assert_eq!(intel.file_churn.get("b.rs"), Some(&1));
    }

    #[test]
    fn unborn_repo_returns_empty_incremental_history() {
        let dir = TempRepo::new();
        Repository::init(dir.path()).unwrap();

        let intel = mine_incremental(dir.path(), &IncrementalConfig::default()).unwrap();

        assert_eq!(intel.commits_analyzed, 0);
        assert!(intel.commit_records.is_empty());
        assert_eq!(newest_commit_ts(dir.path()).unwrap(), None);
    }

    #[test]
    fn same_second_frontier_keeps_unseen_commit() {
        let dir = TempRepo::new();
        let repo = Repository::init(dir.path()).unwrap();
        let ts = 1_700_000_000;
        let first_oid = commit_files_at(
            &repo,
            &[("same.rs", "old\n")],
            "first same second",
            "Alice",
            "alice@x.com",
            ts,
        );
        let second_oid = commit_files_at(
            &repo,
            &[("same.rs", "new\n")],
            "second same second",
            "Bob",
            "bob@x.com",
            ts,
        );
        let cfg = IncrementalConfig {
            since_ts: Some(ts),
            seen_oids: HashSet::from([first_oid.to_string()]),
            max_commits: 10,
            ..IncrementalConfig::default()
        };

        let intel = mine_incremental(dir.path(), &cfg).unwrap();

        assert_eq!(intel.commits_analyzed, 1);
        let second_oid = second_oid.to_string();
        assert_eq!(
            intel.commit_records[0].oid.as_deref(),
            Some(second_oid.as_str())
        );
        assert_eq!(intel.commit_records[0].message, "second same second");
    }

    #[test]
    fn essential_skips_co_change_but_full_populates_it() {
        let dir = fixture_repo();
        let essential = mine_incremental(
            dir.path(),
            &IncrementalConfig {
                tier: GitTier::Essential,
                ..IncrementalConfig::default()
            },
        )
        .unwrap();
        let full = mine_incremental(
            dir.path(),
            &IncrementalConfig {
                tier: GitTier::Full,
                ..IncrementalConfig::default()
            },
        )
        .unwrap();

        assert!(essential.co_change.is_empty());
        assert_eq!(
            full.co_change.get(&("a.rs".into(), "b.rs".into())),
            Some(&2)
        );
    }

    #[test]
    fn merge_intel_sums_churn() {
        let mut base = GitIntel::default();
        base.file_churn.insert("a.rs".into(), 2);
        base.file_churn.insert("b.rs".into(), 1);
        base.commits_analyzed = 2;

        let mut delta = GitIntel::default();
        delta.file_churn.insert("a.rs".into(), 3);
        delta.file_churn.insert("c.rs".into(), 4);
        delta.commits_analyzed = 3;

        merge_intel(&mut base, &delta);

        assert_eq!(base.file_churn.get("a.rs"), Some(&5));
        assert_eq!(base.file_churn.get("b.rs"), Some(&1));
        assert_eq!(base.file_churn.get("c.rs"), Some(&4));
        assert_eq!(base.commits_analyzed, 5);
    }

    #[test]
    fn newest_commit_ts_returns_head_time() {
        let dir = fixture_repo();

        let ts = newest_commit_ts(dir.path()).unwrap();

        assert_eq!(ts, Some(1_700_000_200));
    }
}
