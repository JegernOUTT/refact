use std::collections::{HashMap, HashSet};
use std::path::Path;

use git2::{Commit, Repository};
use serde::{Deserialize, Serialize};

pub mod blame;
pub mod change_risk;
pub mod coupling;
pub mod incremental;
pub mod paths;
pub mod provenance;

pub use provenance::{classify_commit, AgentProvenance};

const MAX_FILES_PER_COMMIT_FOR_COCHANGE: usize = 200;
const MAX_FILES_PER_COMMIT_FOR_ENTROPY: usize = 30;
const TEMPORAL_HALFLIFE_DAYS: f64 = 180.0;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitIntel {
    pub file_churn: HashMap<String, u32>,
    pub file_authors: HashMap<String, HashMap<String, u32>>,
    pub co_change: HashMap<(String, String), u32>,
    pub commits_analyzed: u32,
    pub commit_records: Vec<CommitRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommitRecord {
    pub ts: i64,
    pub author: String,
    pub committer: String,
    pub message: String,
    pub files: Vec<(String, u32, u32)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChangeFeatures {
    pub la: u32,
    pub ld: u32,
    pub nf: u32,
    pub entropy: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hotspot {
    pub path: String,
    pub churn: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ownership {
    pub author: String,
    pub commits: u32,
    pub share: f64,
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

fn changed_files(repo: &Repository, commit: &Commit) -> Vec<String> {
    changed_files_with_stats(repo, commit)
        .into_iter()
        .map(|(path, _, _)| path)
        .collect()
}

fn temporal_weight(now_ts: i64, ts: i64) -> f64 {
    let age_days = (now_ts - ts) as f64 / 86_400.0;
    (-std::f64::consts::LN_2 * age_days / TEMPORAL_HALFLIFE_DAYS).exp()
}

fn percentile(values: &[f64], target: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().filter(|v| **v <= target).count() as f64 / values.len() as f64
}

pub fn collect_commit_messages(repo_path: &Path, max: usize) -> Result<Vec<String>, String> {
    let repo = Repository::open(repo_path).map_err(|e| format!("git open: {e}"))?;
    let mut revwalk = repo.revwalk().map_err(|e| format!("git revwalk: {e}"))?;
    revwalk
        .push_head()
        .map_err(|e| format!("git push_head: {e}"))?;
    let mut out = Vec::new();
    for (i, oid) in revwalk.enumerate() {
        if i >= max {
            break;
        }
        let oid = oid.map_err(|e| format!("git oid: {e}"))?;
        let commit = repo
            .find_commit(oid)
            .map_err(|e| format!("git find_commit: {e}"))?;
        out.push(commit.message().unwrap_or("").to_string());
    }
    Ok(out)
}

pub fn mine_history(repo_path: &Path, max_commits: usize) -> Result<GitIntel, String> {
    let repo = Repository::open(repo_path).map_err(|e| format!("git open: {e}"))?;
    let mut revwalk = repo.revwalk().map_err(|e| format!("git revwalk: {e}"))?;
    revwalk
        .push_head()
        .map_err(|e| format!("git push_head: {e}"))?;

    let mut intel = GitIntel::default();
    for (i, oid) in revwalk.enumerate() {
        if i >= max_commits {
            break;
        }
        let oid = oid.map_err(|e| format!("git oid: {e}"))?;
        let commit = repo
            .find_commit(oid)
            .map_err(|e| format!("git find_commit: {e}"))?;
        let author = commit.author().email().unwrap_or("unknown").to_string();
        let committer = commit.committer().email().unwrap_or("unknown").to_string();
        let ts = commit.time().seconds();
        let message = commit.message().unwrap_or("").to_string();
        let file_stats = changed_files_with_stats(&repo, &commit);
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
        if files.len() <= MAX_FILES_PER_COMMIT_FOR_COCHANGE {
            for a in 0..files.len() {
                for b in (a + 1)..files.len() {
                    let key = (files[a].clone(), files[b].clone());
                    *intel.co_change.entry(key).or_default() += 1;
                }
            }
        }
        intel.commit_records.push(CommitRecord {
            ts,
            author,
            committer,
            message,
            files: file_stats,
        });
    }
    Ok(intel)
}

impl GitIntel {
    pub fn hotspots(&self, top_n: usize) -> Vec<Hotspot> {
        let mut v: Vec<Hotspot> = self
            .file_churn
            .iter()
            .map(|(path, churn)| Hotspot {
                path: path.clone(),
                churn: *churn,
            })
            .collect();
        v.sort_by(|x, y| y.churn.cmp(&x.churn).then_with(|| x.path.cmp(&y.path)));
        v.truncate(top_n);
        v
    }

    pub fn ownership(&self, file: &str) -> Vec<Ownership> {
        let Some(authors) = self.file_authors.get(file) else {
            return vec![];
        };
        let total: u32 = authors.values().sum();
        let mut v: Vec<Ownership> = authors
            .iter()
            .map(|(author, commits)| Ownership {
                author: author.clone(),
                commits: *commits,
                share: if total > 0 {
                    *commits as f64 / total as f64
                } else {
                    0.0
                },
            })
            .collect();
        v.sort_by(|x, y| {
            y.commits
                .cmp(&x.commits)
                .then_with(|| x.author.cmp(&y.author))
        });
        v
    }

    pub fn commit_count_in_window(&self, file: &str, now_ts: i64, days: i64) -> u32 {
        let window_secs = days * 86_400;
        self.commit_records
            .iter()
            .filter(|commit| commit.ts <= now_ts && now_ts - commit.ts <= window_secs)
            .filter(|commit| commit.files.iter().any(|(path, _, _)| path == file))
            .count() as u32
    }

    pub fn lines_in_window(&self, file: &str, now_ts: i64, days: i64) -> (u32, u32) {
        let window_secs = days * 86_400;
        self.commit_records
            .iter()
            .filter(|commit| commit.ts <= now_ts && now_ts - commit.ts <= window_secs)
            .flat_map(|commit| commit.files.iter())
            .filter(|(path, _, _)| path == file)
            .fold(
                (0_u32, 0_u32),
                |(total_added, total_deleted), (_, added, deleted)| {
                    (total_added + *added, total_deleted + *deleted)
                },
            )
    }

    pub fn primary_owner(&self, file: &str) -> (String, f64) {
        self.ownership(file)
            .first()
            .map(|owner| (owner.author.clone(), owner.share))
            .unwrap_or_else(|| (String::new(), 0.0))
    }

    pub fn recent_owner(&self, file: &str, now_ts: i64, days: i64) -> (String, f64) {
        let window_secs = days * 86_400;
        let mut counts: HashMap<String, u32> = HashMap::new();
        for commit in self
            .commit_records
            .iter()
            .filter(|commit| commit.ts <= now_ts && now_ts - commit.ts <= window_secs)
        {
            if commit.files.iter().any(|(path, _, _)| path == file) {
                *counts.entry(commit.author.clone()).or_default() += 1;
            }
        }
        let total: u32 = counts.values().sum();
        if total == 0 {
            return (String::new(), 0.0);
        }
        let mut owners: Vec<(String, u32)> = counts.into_iter().collect();
        owners.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let (author, commits) = owners.remove(0);
        (author, commits as f64 / total as f64)
    }

    pub fn active_contributors_in_window(&self, now_ts: i64, days: i64) -> u32 {
        let window_secs = days * 86_400;
        self.commit_records
            .iter()
            .filter(|commit| commit.ts <= now_ts && now_ts - commit.ts <= window_secs)
            .map(|commit| commit.author.clone())
            .collect::<HashSet<_>>()
            .len() as u32
    }

    pub fn churn_percentile(&self, file: &str) -> f64 {
        let Some(target) = self.file_churn.get(file) else {
            return 0.0;
        };
        let values: Vec<f64> = self
            .file_churn
            .values()
            .map(|churn| *churn as f64)
            .collect();
        percentile(&values, *target as f64)
    }

    pub fn change_entropy_pct(&self, file: &str) -> f64 {
        let entropy = self.change_entropy();
        let Some(target) = entropy.get(file) else {
            return 0.0;
        };
        let values: Vec<f64> = entropy.values().copied().collect();
        percentile(&values, *target)
    }

    pub fn co_change_partners(&self, file: &str, min_count: u32) -> Vec<(String, u32)> {
        let mut partners: Vec<(String, u32)> = self
            .co_change
            .iter()
            .filter_map(|((left, right), count)| {
                if *count < min_count {
                    None
                } else if left == file {
                    Some((right.clone(), *count))
                } else if right == file {
                    Some((left.clone(), *count))
                } else {
                    None
                }
            })
            .collect();
        partners.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        partners
    }

    pub fn is_hotspot_file(&self, file: &str, top_n: usize) -> bool {
        self.hotspots(top_n)
            .iter()
            .any(|hotspot| hotspot.path == file)
    }

    pub fn bus_factor(&self, file: &str) -> usize {
        const BUS_FACTOR_COVERAGE: f64 = 0.8;
        let owners = self.ownership(file);
        let mut cumulative = 0.0;
        let mut count = 0;
        for o in owners {
            cumulative += o.share;
            count += 1;
            if cumulative >= BUS_FACTOR_COVERAGE {
                break;
            }
        }
        count.max(if self.file_authors.contains_key(file) {
            1
        } else {
            0
        })
    }

    pub fn co_change_pairs(&self, min_count: u32) -> Vec<((String, String), u32)> {
        let mut v: Vec<((String, String), u32)> = self
            .co_change
            .iter()
            .filter(|(_, c)| **c >= min_count)
            .map(|(k, c)| (k.clone(), *c))
            .collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        v
    }

    pub fn temporal_hotspots(&self, now_ts: i64, top_n: usize) -> Vec<(String, f64)> {
        let mut scores: HashMap<String, f64> = HashMap::new();
        for commit in &self.commit_records {
            let weight = temporal_weight(now_ts, commit.ts);
            for (path, added, deleted) in &commit.files {
                let lines = ((*added + *deleted) as f64 / 100.0).min(3.0);
                *scores.entry(path.clone()).or_default() += weight * lines;
            }
        }
        let mut v: Vec<(String, f64)> = scores.into_iter().collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        v.truncate(top_n);
        v
    }

    pub fn change_entropy(&self) -> HashMap<String, f64> {
        let now_ts = self.commit_records.iter().map(|c| c.ts).max().unwrap_or(0);
        let mut out = HashMap::new();
        for commit in &self.commit_records {
            let n = commit.files.len();
            if n == 0 || n > MAX_FILES_PER_COMMIT_FOR_ENTROPY {
                continue;
            }
            let contribution = temporal_weight(now_ts, commit.ts) * (n as f64).log2() / n as f64;
            for (path, _, _) in &commit.files {
                *out.entry(path.clone()).or_default() += contribution;
            }
        }
        out
    }

    pub fn commit_features(&self) -> Vec<(usize, ChangeFeatures)> {
        self.commit_records
            .iter()
            .enumerate()
            .map(|(idx, commit)| {
                let la: u32 = commit.files.iter().map(|(_, added, _)| *added).sum();
                let ld: u32 = commit.files.iter().map(|(_, _, deleted)| *deleted).sum();
                let churns: Vec<u32> = commit
                    .files
                    .iter()
                    .map(|(_, added, deleted)| *added + *deleted)
                    .collect();
                let total: u32 = churns.iter().sum();
                let entropy = if total == 0 {
                    0.0
                } else {
                    churns
                        .iter()
                        .filter(|c| **c > 0)
                        .map(|c| {
                            let p = *c as f64 / total as f64;
                            -p * p.log2()
                        })
                        .sum()
                };
                (
                    idx,
                    ChangeFeatures {
                        la,
                        ld,
                        nf: commit.files.len() as u32,
                        entropy,
                    },
                )
            })
            .collect()
    }

    pub fn ownership_risk(&self, file: &str) -> bool {
        let owners = self.ownership(file);
        let total: u32 = owners.iter().map(|o| o.commits).sum();
        if total < 5 {
            return false;
        }
        let minor_contributors = owners.iter().filter(|o| o.share < 0.05).count();
        let top_owner_share = owners.first().map(|o| o.share).unwrap_or(0.0);
        minor_contributors >= 3 || top_owner_share < 0.4
    }

    pub fn developer_congestion(&self, file: &str, commits_90d: u32, primary_share: f64) -> bool {
        let contributor_count = self.file_authors.get(file).map(|a| a.len()).unwrap_or(0);
        contributor_count >= 5 && commits_90d >= 6 && primary_share < 0.5
    }

    pub fn knowledge_loss(&self, file: &str) -> bool {
        self.bus_factor(file) <= 1 && self.ownership(file).first().is_some()
    }

    pub fn churn_risk(&self, file: &str) -> f64 {
        let mut churn_by_file: HashMap<String, u32> = HashMap::new();
        for commit in &self.commit_records {
            for (path, added, deleted) in &commit.files {
                *churn_by_file.entry(path.clone()).or_default() += added + deleted;
            }
        }
        let mut max_relative: f64 = 0.0;
        let mut target_relative = 0.0;
        for (path, churn) in churn_by_file {
            let commits = *self.file_churn.get(&path).unwrap_or(&0);
            if commits == 0 {
                continue;
            }
            let relative = churn as f64 / commits as f64;
            max_relative = max_relative.max(relative);
            if path == file {
                target_relative = relative;
            }
        }
        if max_relative == 0.0 {
            0.0
        } else {
            (target_relative / max_relative).clamp(0.0, 1.0)
        }
    }

    pub fn significant_commits(&self, max: usize) -> Vec<usize> {
        let decision_keywords = [
            "migrate",
            "switch to",
            "introduce",
            "deprecate",
            "rewrite",
            "replace",
        ];
        let boring_prefixes = ["chore", "ci", "style", "build"];
        let mut out = Vec::new();
        for (idx, commit) in self.commit_records.iter().enumerate() {
            let message = commit.message.trim();
            let lower_message = message.to_lowercase();
            let lower_author = commit.author.to_lowercase();
            if message.len() < 12
                || message.starts_with("Merge ")
                || lower_author.contains("dependabot")
                || lower_author.contains("renovate")
                || lower_author.contains("github-actions")
            {
                continue;
            }
            let has_decision = decision_keywords.iter().any(|k| lower_message.contains(k));
            let conventional_boring = boring_prefixes.iter().any(|p| {
                lower_message.starts_with(&format!("{}:", p))
                    || lower_message.starts_with(&format!("{}(", p))
            });
            if !conventional_boring || has_decision {
                out.push(idx);
                if out.len() >= max {
                    break;
                }
            }
        }
        out
    }

    pub fn agent_authored_pct(&self) -> f64 {
        if self.commit_records.is_empty() {
            return 0.0;
        }
        let agent_count = self
            .commit_records
            .iter()
            .filter(|c| classify_commit(&c.author, &c.committer, &c.message).is_some())
            .count();
        agent_count as f64 / self.commit_records.len() as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::{Repository, Signature, Time};
    use std::path::Path;

    fn commit_files(repo: &Repository, files: &[(&str, &str)], msg: &str, name: &str, email: &str) {
        commit_files_at(repo, files, msg, name, email, 1_700_000_000);
    }

    fn commit_files_at(
        repo: &Repository,
        files: &[(&str, &str)],
        msg: &str,
        name: &str,
        email: &str,
        ts: i64,
    ) {
        let workdir = repo.workdir().unwrap().to_path_buf();
        for (p, c) in files {
            std::fs::write(workdir.join(p), c).unwrap();
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
            .unwrap();
    }

    fn fixture_repo() -> (tempfile::TempDir, GitIntel) {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        commit_files(
            &repo,
            &[("a.rs", "1"), ("b.rs", "1")],
            "c1",
            "Alice",
            "alice@x.com",
        );
        commit_files(
            &repo,
            &[("a.rs", "2"), ("b.rs", "2")],
            "c2",
            "Alice",
            "alice@x.com",
        );
        commit_files(&repo, &[("a.rs", "3")], "c3", "Bob", "bob@x.com");
        let intel = mine_history(dir.path(), 100).unwrap();
        (dir, intel)
    }

    fn windowed_intel() -> GitIntel {
        let now = 1_700_000_000;
        let day = 86_400;
        let mut intel = GitIntel::default();
        intel.file_churn.insert("a.rs".into(), 3);
        intel.file_churn.insert("b.rs".into(), 2);
        intel.file_churn.insert("c.rs".into(), 1);
        intel.file_authors.insert(
            "a.rs".into(),
            HashMap::from([("alice@x.com".into(), 2), ("bob@x.com".into(), 1)]),
        );
        intel.co_change.insert(("a.rs".into(), "b.rs".into()), 2);
        intel.co_change.insert(("c.rs".into(), "a.rs".into()), 1);
        intel.commit_records = vec![
            CommitRecord {
                ts: now - 40 * day,
                author: "alice@x.com".into(),
                committer: "alice@x.com".into(),
                message: "old".into(),
                files: vec![("a.rs".into(), 10, 1)],
            },
            CommitRecord {
                ts: now - 3 * day,
                author: "alice@x.com".into(),
                committer: "alice@x.com".into(),
                message: "recent alice".into(),
                files: vec![("a.rs".into(), 2, 3), ("b.rs".into(), 1, 0)],
            },
            CommitRecord {
                ts: now - day,
                author: "bob@x.com".into(),
                committer: "bob@x.com".into(),
                message: "recent bob".into(),
                files: vec![("a.rs".into(), 5, 1)],
            },
            CommitRecord {
                ts: now + day,
                author: "carol@x.com".into(),
                committer: "carol@x.com".into(),
                message: "future".into(),
                files: vec![("a.rs".into(), 100, 100)],
            },
        ];
        intel
    }

    #[test]
    fn windowed_helpers_use_only_commits_in_window() {
        let now = 1_700_000_000;
        let intel = windowed_intel();
        assert_eq!(intel.commit_count_in_window("a.rs", now, 10), 2);
        assert_eq!(intel.commit_count_in_window("a.rs", now, 1), 1);
        assert_eq!(intel.commit_count_in_window("missing.rs", now, 10), 0);
        assert_eq!(intel.lines_in_window("a.rs", now, 10), (7, 4));
        assert_eq!(intel.lines_in_window("a.rs", now, 1), (5, 1));
        assert_eq!(intel.lines_in_window("missing.rs", now, 10), (0, 0));
        assert_eq!(intel.active_contributors_in_window(now, 10), 2);
        assert_eq!(intel.active_contributors_in_window(now, 1), 1);
    }

    #[test]
    fn ownership_percentile_and_partner_helpers_return_expected_values() {
        let now = 1_700_000_000;
        let intel = windowed_intel();
        let (primary_author, primary_share) = intel.primary_owner("a.rs");
        assert_eq!(primary_author, "alice@x.com");
        assert!((primary_share - 2.0 / 3.0).abs() < 1e-9);
        assert_eq!(intel.primary_owner("missing.rs"), (String::new(), 0.0));

        let (recent_author, recent_share) = intel.recent_owner("a.rs", now, 10);
        assert_eq!(recent_author, "alice@x.com");
        assert!((recent_share - 0.5).abs() < 1e-9);
        assert_eq!(intel.recent_owner("a.rs", now, 0), (String::new(), 0.0));

        assert!((intel.churn_percentile("b.rs") - 2.0 / 3.0).abs() < 1e-9);
        assert_eq!(intel.churn_percentile("missing.rs"), 0.0);
        assert_eq!(intel.change_entropy_pct("missing.rs"), 0.0);
        assert_eq!(
            intel.co_change_partners("a.rs", 1),
            vec![("b.rs".into(), 2), ("c.rs".into(), 1)]
        );
        assert_eq!(
            intel.co_change_partners("a.rs", 2),
            vec![("b.rs".into(), 2)]
        );
        assert!(intel.is_hotspot_file("a.rs", 1));
        assert!(!intel.is_hotspot_file("b.rs", 1));
    }

    #[test]
    fn hotspots_rank_by_churn() {
        let (_dir, intel) = fixture_repo();
        let hs = intel.hotspots(10);
        assert_eq!(hs[0].path, "a.rs", "a.rs changed 3x, b.rs 2x: {hs:?}");
        assert_eq!(hs[0].churn, 3);
    }

    #[test]
    fn ownership_reflects_authors() {
        let (_dir, intel) = fixture_repo();
        let owners = intel.ownership("a.rs");
        assert_eq!(
            owners[0].author, "alice@x.com",
            "Alice has 2/3 of a.rs: {owners:?}"
        );
        assert_eq!(owners[0].commits, 2);
        assert!((owners[0].share - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn co_change_detects_files_changed_together() {
        let (_dir, intel) = fixture_repo();
        let pairs = intel.co_change_pairs(1);
        assert!(
            pairs
                .iter()
                .any(|((x, y), c)| x == "a.rs" && y == "b.rs" && *c == 2),
            "a.rs & b.rs co-changed twice: {pairs:?}"
        );
    }

    #[test]
    fn bus_factor_for_single_owner_is_one() {
        let (_dir, intel) = fixture_repo();
        assert_eq!(intel.bus_factor("b.rs"), 1, "b.rs only by Alice");
    }

    #[test]
    fn mine_history_records_commit_metadata_and_line_stats() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        commit_files_at(
            &repo,
            &[("lines.rs", "one\ntwo\nthree\n")],
            "introduce line stats",
            "Alice",
            "alice@x.com",
            1_600_000_000,
        );
        commit_files_at(
            &repo,
            &[("lines.rs", "one\nthree\n")],
            "rewrite line stats",
            "Bob",
            "bob@x.com",
            1_600_000_100,
        );

        let intel = mine_history(dir.path(), 10).unwrap();
        assert_eq!(intel.commit_records.len(), 2);
        let newest = &intel.commit_records[0];
        assert_eq!(newest.ts, 1_600_000_100);
        assert_eq!(newest.author, "bob@x.com");
        assert_eq!(newest.committer, "bob@x.com");
        assert_eq!(newest.message, "rewrite line stats");
        assert!(newest
            .files
            .iter()
            .any(|(p, a, d)| p == "lines.rs" && *a == 0 && *d == 1));
    }

    #[test]
    fn huge_commits_are_kept_but_not_co_changed() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let names: Vec<String> = (0..201).map(|i| format!("f{i}.txt")).collect();
        let files: Vec<(&str, &str)> = names.iter().map(|s| (s.as_str(), "x\n")).collect();
        commit_files(
            &repo,
            &files,
            "introduce many files",
            "Alice",
            "alice@x.com",
        );
        let intel = mine_history(dir.path(), 10).unwrap();
        assert_eq!(intel.commit_records[0].files.len(), 201);
        assert!(intel.co_change_pairs(1).is_empty());
    }

    #[test]
    fn temporal_hotspots_use_decay_and_line_weight() {
        let mut intel = GitIntel::default();
        intel.commit_records = vec![
            CommitRecord {
                ts: 1_000_000,
                author: "a@x.com".into(),
                committer: "a@x.com".into(),
                message: "old".into(),
                files: vec![("old.rs".into(), 300, 0)],
            },
            CommitRecord {
                ts: 1_000_000 + 180 * 86_400,
                author: "b@x.com".into(),
                committer: "b@x.com".into(),
                message: "new".into(),
                files: vec![("new.rs".into(), 200, 0)],
            },
        ];
        let hs = intel.temporal_hotspots(1_000_000 + 180 * 86_400, 2);
        assert_eq!(hs[0].0, "new.rs");
        assert!((hs[0].1 - 2.0).abs() < 1e-9);
        assert!((hs[1].1 - 1.5).abs() < 1e-9);
    }

    #[test]
    fn change_entropy_skips_large_commits() {
        let mut intel = GitIntel::default();
        intel.commit_records = vec![
            CommitRecord {
                ts: 100,
                author: "a@x.com".into(),
                committer: "a@x.com".into(),
                message: "small".into(),
                files: vec![("a.rs".into(), 1, 0), ("b.rs".into(), 1, 0)],
            },
            CommitRecord {
                ts: 100,
                author: "a@x.com".into(),
                committer: "a@x.com".into(),
                message: "large".into(),
                files: (0..31).map(|i| (format!("l{i}.rs"), 1, 0)).collect(),
            },
        ];
        let entropy = intel.change_entropy();
        assert!((entropy["a.rs"] - 0.5).abs() < 1e-9);
        assert!(!entropy.contains_key("l0.rs"));
    }

    #[test]
    fn commit_features_compute_kamei_values() {
        let mut intel = GitIntel::default();
        intel.commit_records.push(CommitRecord {
            ts: 1,
            author: "a@x.com".into(),
            committer: "a@x.com".into(),
            message: "feature".into(),
            files: vec![("a.rs".into(), 3, 1), ("b.rs".into(), 2, 0)],
        });
        let features = intel.commit_features();
        assert_eq!(features[0].0, 0);
        assert_eq!(features[0].1.la, 5);
        assert_eq!(features[0].1.ld, 1);
        assert_eq!(features[0].1.nf, 2);
        let expected =
            -((4.0_f64 / 6.0) * (4.0_f64 / 6.0).log2() + (2.0_f64 / 6.0) * (2.0_f64 / 6.0).log2());
        assert!((features[0].1.entropy - expected).abs() < 1e-9);
    }

    #[test]
    fn organizational_metrics_flag_risks() {
        let mut intel = GitIntel::default();
        intel.file_churn.insert("risk.rs".into(), 6);
        intel.file_churn.insert("calm.rs".into(), 2);
        intel.file_authors.insert(
            "risk.rs".into(),
            HashMap::from([
                ("a@x.com".into(), 2),
                ("b@x.com".into(), 2),
                ("c@x.com".into(), 2),
                ("d@x.com".into(), 0),
                ("e@x.com".into(), 0),
            ]),
        );
        intel
            .file_authors
            .insert("calm.rs".into(), HashMap::from([("a@x.com".into(), 2)]));
        intel.commit_records = vec![CommitRecord {
            ts: 1,
            author: "a@x.com".into(),
            committer: "a@x.com".into(),
            message: "churn".into(),
            files: vec![("risk.rs".into(), 12, 0), ("calm.rs".into(), 2, 0)],
        }];
        assert!(intel.ownership_risk("risk.rs"));
        assert!(intel.developer_congestion("risk.rs", 6, 0.4));
        assert!(intel.knowledge_loss("calm.rs"));
        assert_eq!(intel.churn_risk("risk.rs"), 1.0);
        assert!(intel.churn_risk("calm.rs") < 1.0);
    }

    #[test]
    fn significant_commits_filter_noise_but_keep_decisions() {
        let mut intel = GitIntel::default();
        intel.commit_records = vec![
            CommitRecord {
                ts: 1,
                author: "dev@x.com".into(),
                committer: "dev@x.com".into(),
                message: "introduce parser architecture".into(),
                files: vec![],
            },
            CommitRecord {
                ts: 2,
                author: "dev@x.com".into(),
                committer: "dev@x.com".into(),
                message: "chore: update snapshots".into(),
                files: vec![],
            },
            CommitRecord {
                ts: 3,
                author: "dev@x.com".into(),
                committer: "dev@x.com".into(),
                message: "ci: migrate to new runner".into(),
                files: vec![],
            },
            CommitRecord {
                ts: 4,
                author: "dependabot@github.com".into(),
                committer: "dependabot@github.com".into(),
                message: "replace dependency versions".into(),
                files: vec![],
            },
            CommitRecord {
                ts: 5,
                author: "dev@x.com".into(),
                committer: "dev@x.com".into(),
                message: "Merge branch main".into(),
                files: vec![],
            },
        ];
        assert_eq!(intel.significant_commits(10), vec![0, 2]);
        assert_eq!(intel.significant_commits(1), vec![0]);
    }

    #[test]
    fn provenance_classifies_tiers_and_agent_percentage() {
        assert_eq!(
            classify_commit("copilot-swe-agent@github.com", "human@x.com", "change")
                .unwrap()
                .tier,
            1
        );
        assert_eq!(
            classify_commit("Dev (aider)", "human@x.com", "change")
                .unwrap()
                .tier,
            2
        );
        assert_eq!(
            classify_commit(
                "human@x.com",
                "human@x.com",
                "Fix\n\nCo-authored-by: Bot <bot@anthropic.com>"
            )
            .unwrap()
            .tier,
            3
        );

        let mut intel = GitIntel::default();
        intel.commit_records = vec![
            CommitRecord {
                ts: 1,
                author: "copilot-swe-agent@github.com".into(),
                committer: "human@x.com".into(),
                message: "change".into(),
                files: vec![],
            },
            CommitRecord {
                ts: 2,
                author: "human@x.com".into(),
                committer: "human@x.com".into(),
                message: "manual change".into(),
                files: vec![],
            },
        ];
        assert!((intel.agent_authored_pct() - 0.5).abs() < 1e-9);
    }
}
