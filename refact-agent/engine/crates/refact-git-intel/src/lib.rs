use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use git2::{Commit, ErrorClass, ErrorCode, Oid, Repository, Revwalk};
use serde::{Deserialize, Serialize};

pub mod blame;
pub mod change_risk;
pub mod coupling;
pub mod incremental;
pub mod paths;
pub mod provenance;

pub use incremental::mine_history_incremental;
pub use provenance::{classify_commit, AgentProvenance};

const MAX_FILES_PER_COMMIT_FOR_COCHANGE: usize = 200;
const MAX_FILES_PER_COMMIT_FOR_ENTROPY: usize = 30;
const TEMPORAL_HALFLIFE_DAYS: f64 = 180.0;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GitIntel {
    pub file_churn: HashMap<String, u32>,
    #[serde(default)]
    pub fix_commit_counts: HashMap<String, u32>,
    pub file_authors: HashMap<String, HashMap<String, u32>>,
    pub co_change: HashMap<(String, String), u32>,
    pub commits_analyzed: u32,
    pub commit_records: Vec<CommitRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_commit_id: Option<String>,
    #[serde(default)]
    pub author_commit_counts: HashMap<String, u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommitRisk {
    pub commit_id: String,
    pub summary: String,
    pub author: String,
    pub ts: i64,
    pub risk: f64,
    pub inputs: change_risk::RiskInputs,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommitRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oid: Option<String>,
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

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn contains_fix_word(message: &str, word: &str) -> bool {
    message.match_indices(word).any(|(idx, _)| {
        let before = idx
            .checked_sub(1)
            .and_then(|prev| message.as_bytes().get(prev))
            .is_some_and(|byte| is_word_byte(*byte));
        let after = message
            .as_bytes()
            .get(idx + word.len())
            .is_some_and(|byte| is_word_byte(*byte));
        !before && !after
    })
}

fn is_fix_commit_message(message: &str) -> bool {
    const FIX_WORDS: &[&str] = &[
        "fix",
        "bugfix",
        "hotfix",
        "patch",
        "patched",
        "patching",
        "regression",
        "crash",
        "oops",
        "revert",
    ];
    let message = message.to_ascii_lowercase();
    FIX_WORDS
        .iter()
        .any(|word| contains_fix_word(&message, word))
}

pub fn is_empty_head_error(error: &git2::Error) -> bool {
    error.code() == ErrorCode::UnbornBranch
        || error.code() == ErrorCode::NotFound
        || (error.class() == ErrorClass::Reference
            && error.message().to_lowercase().contains("not found"))
        || error.message().to_lowercase().contains("unborn")
}

pub(crate) fn push_head_or_empty(revwalk: &mut Revwalk) -> Result<bool, String> {
    match revwalk.push_head() {
        Ok(()) => Ok(true),
        Err(error) if is_empty_head_error(&error) => Ok(false),
        Err(error) => Err(format!("git push_head: {error}")),
    }
}

fn push_oid_or_empty(revwalk: &mut Revwalk, oid: Option<Oid>) -> Result<bool, String> {
    match oid {
        Some(oid) => revwalk
            .push(oid)
            .map(|()| true)
            .map_err(|e| format!("git push: {e}")),
        None => Ok(false),
    }
}

fn temporal_weight(now_ts: i64, ts: i64) -> f64 {
    let age_days = now_ts.saturating_sub(ts).max(0) as f64 / 86_400.0;
    (-std::f64::consts::LN_2 * age_days / TEMPORAL_HALFLIFE_DAYS)
        .exp()
        .clamp(0.0, 1.0)
}

fn current_unix_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
        .unwrap_or(0)
}

fn saturating_usize_to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn saturating_sum_u32(values: impl Iterator<Item = u32>) -> u32 {
    values.fold(0_u32, |total, value| total.saturating_add(value))
}

fn window_secs(days: i64) -> Option<i64> {
    if days < 0 {
        return None;
    }
    days.checked_mul(86_400)
}

fn commit_in_window(commit: &CommitRecord, now_ts: i64, window_secs: i64) -> bool {
    commit
        .ts
        .le(&now_ts)
        .then(|| now_ts.checked_sub(commit.ts))
        .flatten()
        .is_some_and(|age_secs| age_secs <= window_secs)
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
    if !push_head_or_empty(&mut revwalk)? {
        return Ok(Vec::new());
    }
    collect_commit_messages_from_revwalk(&repo, revwalk, max)
}

pub fn collect_commit_messages_at(
    repo_path: &Path,
    head: Option<Oid>,
    max: usize,
) -> Result<Vec<String>, String> {
    let repo = Repository::open(repo_path).map_err(|e| format!("git open: {e}"))?;
    let mut revwalk = repo.revwalk().map_err(|e| format!("git revwalk: {e}"))?;
    if !push_oid_or_empty(&mut revwalk, head)? {
        return Ok(Vec::new());
    }
    collect_commit_messages_from_revwalk(&repo, revwalk, max)
}

fn collect_commit_messages_from_revwalk(
    repo: &Repository,
    revwalk: Revwalk,
    max: usize,
) -> Result<Vec<String>, String> {
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
    if !push_head_or_empty(&mut revwalk)? {
        return Ok(GitIntel::default());
    }
    mine_history_from_revwalk(&repo, revwalk, max_commits)
}

pub fn mine_history_at(
    repo_path: &Path,
    head: Option<Oid>,
    max_commits: usize,
) -> Result<GitIntel, String> {
    let repo = Repository::open(repo_path).map_err(|e| format!("git open: {e}"))?;
    let mut revwalk = repo.revwalk().map_err(|e| format!("git revwalk: {e}"))?;
    if !push_oid_or_empty(&mut revwalk, head)? {
        return Ok(GitIntel::default());
    }
    mine_history_from_revwalk(&repo, revwalk, max_commits)
}

fn mine_history_from_revwalk(
    repo: &Repository,
    revwalk: Revwalk,
    max_commits: usize,
) -> Result<GitIntel, String> {
    let mut intel = GitIntel::default();
    for (i, oid) in revwalk.enumerate() {
        if i >= max_commits {
            break;
        }
        let oid = oid.map_err(|e| format!("git oid: {e}"))?;
        let commit = repo
            .find_commit(oid)
            .map_err(|e| format!("git find_commit: {e}"))?;
        let oid_string = oid.to_string();
        if intel.last_commit_id.is_none() {
            intel.last_commit_id = Some(oid_string.clone());
        }
        let author = commit.author().email().unwrap_or("unknown").to_string();
        let committer = commit.committer().email().unwrap_or("unknown").to_string();
        let ts = commit.time().seconds();
        let message = commit.message().unwrap_or("").to_string();
        let file_stats = changed_files_with_stats(repo, &commit);
        let files: Vec<String> = file_stats.iter().map(|(path, _, _)| path.clone()).collect();
        let is_fix_commit = is_fix_commit_message(&message);

        intel.commits_analyzed = intel.commits_analyzed.saturating_add(1);
        let author_count = intel
            .author_commit_counts
            .entry(author.clone())
            .or_default();
        *author_count = author_count.saturating_add(1);
        for f in &files {
            let churn = intel.file_churn.entry(f.clone()).or_default();
            *churn = churn.saturating_add(1);
            if is_fix_commit {
                let fix_count = intel.fix_commit_counts.entry(f.clone()).or_default();
                *fix_count = fix_count.saturating_add(1);
            }
            let author_count = intel
                .file_authors
                .entry(f.clone())
                .or_default()
                .entry(author.clone())
                .or_default();
            *author_count = author_count.saturating_add(1);
        }
        if files.len() <= MAX_FILES_PER_COMMIT_FOR_COCHANGE {
            for a in 0..files.len() {
                for b in (a + 1)..files.len() {
                    let key = (files[a].clone(), files[b].clone());
                    let count = intel.co_change.entry(key).or_default();
                    *count = count.saturating_add(1);
                }
            }
        }
        intel.commit_records.push(CommitRecord {
            oid: Some(oid_string),
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
    pub fn prior_defects(&self, path: &str) -> u32 {
        self.fix_commit_counts.get(path).copied().unwrap_or(0)
    }

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
        let total: u64 = authors.values().map(|commits| u64::from(*commits)).sum();
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
        let Some(window_secs) = window_secs(days) else {
            return 0;
        };
        self.commit_records
            .iter()
            .filter(|commit| commit_in_window(commit, now_ts, window_secs))
            .filter(|commit| commit.files.iter().any(|(path, _, _)| path == file))
            .fold(0_u32, |total, _| total.saturating_add(1))
    }

    pub fn lines_in_window(&self, file: &str, now_ts: i64, days: i64) -> (u32, u32) {
        let Some(window_secs) = window_secs(days) else {
            return (0, 0);
        };
        self.commit_records
            .iter()
            .filter(|commit| commit_in_window(commit, now_ts, window_secs))
            .flat_map(|commit| commit.files.iter())
            .filter(|(path, _, _)| path == file)
            .fold(
                (0_u32, 0_u32),
                |(total_added, total_deleted), (_, added, deleted)| {
                    (
                        total_added.saturating_add(*added),
                        total_deleted.saturating_add(*deleted),
                    )
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
        let Some(window_secs) = window_secs(days) else {
            return (String::new(), 0.0);
        };
        let mut counts: HashMap<String, u32> = HashMap::new();
        for commit in self
            .commit_records
            .iter()
            .filter(|commit| commit_in_window(commit, now_ts, window_secs))
        {
            if commit.files.iter().any(|(path, _, _)| path == file) {
                let count = counts.entry(commit.author.clone()).or_default();
                *count = count.saturating_add(1);
            }
        }
        let total: u64 = counts.values().map(|count| u64::from(*count)).sum();
        if total == 0 {
            return (String::new(), 0.0);
        }
        let mut owners: Vec<(String, u32)> = counts.into_iter().collect();
        owners.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let (author, commits) = owners.remove(0);
        (author, commits as f64 / total as f64)
    }

    pub fn active_contributors_in_window(&self, now_ts: i64, days: i64) -> u32 {
        let Some(window_secs) = window_secs(days) else {
            return 0;
        };
        self.commit_records
            .iter()
            .filter(|commit| commit_in_window(commit, now_ts, window_secs))
            .map(|commit| commit.author.clone())
            .collect::<HashSet<_>>()
            .len()
            .min(u32::MAX as usize) as u32
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
                let lines =
                    (u64::from(*added).saturating_add(u64::from(*deleted)) as f64 / 100.0).min(3.0);
                *scores.entry(path.clone()).or_default() += weight * lines;
            }
        }
        let mut v: Vec<(String, f64)> = scores.into_iter().collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        v.truncate(top_n);
        v
    }

    pub fn change_entropy(&self) -> HashMap<String, f64> {
        self.change_entropy_at(current_unix_ts())
    }

    pub fn change_entropy_at(&self, now_ts: i64) -> HashMap<String, f64> {
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
                let la = saturating_sum_u32(commit.files.iter().map(|(_, added, _)| *added));
                let ld = saturating_sum_u32(commit.files.iter().map(|(_, _, deleted)| *deleted));
                let churns: Vec<u64> = commit
                    .files
                    .iter()
                    .map(|(_, added, deleted)| {
                        u64::from(*added).saturating_add(u64::from(*deleted))
                    })
                    .collect();
                let total = churns
                    .iter()
                    .fold(0_u64, |total, churn| total.saturating_add(*churn));
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
                        nf: saturating_usize_to_u32(commit.files.len()),
                        entropy,
                    },
                )
            })
            .collect()
    }

    pub fn recent_commit_risks(&self, last_n: usize) -> Vec<CommitRisk> {
        let inputs_by_idx = self.commit_risk_inputs();
        let mut newest: Vec<usize> = (0..self.commit_records.len()).collect();
        newest.sort_by(|left, right| {
            compare_commits_newest(&self.commit_records[*left], &self.commit_records[*right])
        });
        newest.truncate(last_n);

        let mut risks: Vec<CommitRisk> = newest
            .into_iter()
            .map(|idx| {
                let commit = &self.commit_records[idx];
                let inputs = inputs_by_idx[idx];
                CommitRisk {
                    commit_id: commit.oid.clone().unwrap_or_default(),
                    summary: commit
                        .message
                        .lines()
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_string(),
                    author: commit.author.clone(),
                    ts: commit.ts,
                    risk: change_risk::score_change(&inputs),
                    inputs,
                }
            })
            .collect();
        risks.sort_by(|left, right| {
            right
                .risk
                .total_cmp(&left.risk)
                .then_with(|| right.ts.cmp(&left.ts))
                .then_with(|| left.commit_id.cmp(&right.commit_id))
        });
        risks
    }

    fn commit_risk_inputs(&self) -> Vec<change_risk::RiskInputs> {
        let features_by_idx: HashMap<usize, ChangeFeatures> =
            self.commit_features().into_iter().collect();
        let mut chronological: Vec<usize> = (0..self.commit_records.len()).collect();
        chronological.sort_by(|left, right| {
            compare_commits_oldest(&self.commit_records[*left], &self.commit_records[*right])
        });

        let mut record_author_counts: HashMap<String, u32> = HashMap::new();
        for commit in &self.commit_records {
            let count = record_author_counts
                .entry(commit.author.clone())
                .or_default();
            *count = count.saturating_add(1);
        }
        let mut author_counts: HashMap<String, u32> = self
            .author_commit_counts
            .iter()
            .filter_map(|(author, total)| {
                let known = record_author_counts.get(author).copied().unwrap_or(0);
                total
                    .checked_sub(known)
                    .filter(|seed| *seed > 0)
                    .map(|seed| (author.clone(), seed))
            })
            .collect();
        let mut out = vec![
            change_risk::RiskInputs {
                la: 0.0,
                ld: 0.0,
                nf: 0.0,
                nd: 0.0,
                ns: 0.0,
                entropy: 0.0,
                exp: 0.0,
            };
            self.commit_records.len()
        ];

        for idx in chronological {
            let commit = &self.commit_records[idx];
            let features = features_by_idx
                .get(&idx)
                .cloned()
                .unwrap_or(ChangeFeatures {
                    la: 0,
                    ld: 0,
                    nf: 0,
                    entropy: 0.0,
                });
            let exp = *author_counts.get(&commit.author).unwrap_or(&0) as f64;
            out[idx] = change_risk::from_change_features(
                &features,
                distinct_directories(commit) as f64,
                distinct_subsystems(commit) as f64,
                exp,
            );
            let count = author_counts.entry(commit.author.clone()).or_default();
            *count = count.saturating_add(1);
        }

        out
    }

    pub fn ownership_risk(&self, file: &str) -> bool {
        let owners = self.ownership(file);
        let total: u64 = owners.iter().map(|o| u64::from(o.commits)).sum();
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
        let mut churn_by_file: HashMap<String, u64> = HashMap::new();
        for commit in &self.commit_records {
            for (path, added, deleted) in &commit.files {
                let churn = u64::from(*added).saturating_add(u64::from(*deleted));
                let total = churn_by_file.entry(path.clone()).or_default();
                *total = total.saturating_add(churn);
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

fn compare_commits_newest(left: &CommitRecord, right: &CommitRecord) -> std::cmp::Ordering {
    right
        .ts
        .cmp(&left.ts)
        .then_with(|| left.oid.cmp(&right.oid))
        .then_with(|| left.author.cmp(&right.author))
        .then_with(|| left.message.cmp(&right.message))
}

fn compare_commits_oldest(left: &CommitRecord, right: &CommitRecord) -> std::cmp::Ordering {
    left.ts
        .cmp(&right.ts)
        .then_with(|| left.oid.cmp(&right.oid))
        .then_with(|| left.author.cmp(&right.author))
        .then_with(|| left.message.cmp(&right.message))
}

fn distinct_directories(commit: &CommitRecord) -> usize {
    commit
        .files
        .iter()
        .map(|(path, _, _)| path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("."))
        .collect::<HashSet<_>>()
        .len()
}

fn distinct_subsystems(commit: &CommitRecord) -> usize {
    commit
        .files
        .iter()
        .map(|(path, _, _)| path.split_once('/').map(|(top, _)| top).unwrap_or("."))
        .collect::<HashSet<_>>()
        .len()
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
                oid: None,
                ts: now - 40 * day,
                author: "alice@x.com".into(),
                committer: "alice@x.com".into(),
                message: "old".into(),
                files: vec![("a.rs".into(), 10, 1)],
            },
            CommitRecord {
                oid: None,
                ts: now - 3 * day,
                author: "alice@x.com".into(),
                committer: "alice@x.com".into(),
                message: "recent alice".into(),
                files: vec![("a.rs".into(), 2, 3), ("b.rs".into(), 1, 0)],
            },
            CommitRecord {
                oid: None,
                ts: now - day,
                author: "bob@x.com".into(),
                committer: "bob@x.com".into(),
                message: "recent bob".into(),
                files: vec![("a.rs".into(), 5, 1)],
            },
            CommitRecord {
                oid: None,
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
    fn fix_commits_counted_per_file() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        commit_files(
            &repo,
            &[("src/a.rs", "pub fn a() -> u32 { 1 }\n")],
            "feature: add a",
            "Alice",
            "alice@x.com",
        );
        commit_files(
            &repo,
            &[("src/a.rs", "pub fn a() -> u32 { 2 }\n")],
            "fix typo in docs",
            "Bob",
            "bob@x.com",
        );
        commit_files(
            &repo,
            &[("src/a.rs", "pub fn a() -> u32 { 3 }\n")],
            "Hotfix crash in parser",
            "Carol",
            "carol@x.com",
        );

        let intel = mine_history(dir.path(), 10).unwrap();

        assert_eq!(intel.prior_defects("src/a.rs"), 2);
        assert_eq!(intel.prior_defects("missing.rs"), 0);
    }

    #[test]
    fn prefix_is_not_a_fix() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        commit_files(
            &repo,
            &[("src/a.rs", "pub fn a() -> u32 { 1 }\n")],
            "prefix cache keys",
            "Alice",
            "alice@x.com",
        );

        let intel = mine_history(dir.path(), 10).unwrap();

        assert_eq!(intel.prior_defects("src/a.rs"), 0);
    }

    #[test]
    fn empty_repo_history_helpers_return_empty_results() {
        let dir = tempfile::tempdir().unwrap();
        Repository::init(dir.path()).unwrap();

        assert_eq!(
            collect_commit_messages(dir.path(), 10).unwrap(),
            Vec::<String>::new()
        );
        let intel = mine_history(dir.path(), 10).unwrap();
        assert_eq!(intel.commits_analyzed, 0);
        assert!(intel.commit_records.is_empty());
    }

    #[test]
    fn windowed_helpers_reject_invalid_day_windows_without_overflow() {
        let now = 1_700_000_000;
        let intel = windowed_intel();

        assert_eq!(intel.commit_count_in_window("a.rs", now, -1), 0);
        assert_eq!(intel.commit_count_in_window("a.rs", now, i64::MAX), 0);
        assert_eq!(intel.lines_in_window("a.rs", now, -1), (0, 0));
        assert_eq!(intel.lines_in_window("a.rs", now, i64::MAX), (0, 0));
        assert_eq!(intel.recent_owner("a.rs", now, -1), (String::new(), 0.0));
        assert_eq!(
            intel.recent_owner("a.rs", now, i64::MAX),
            (String::new(), 0.0)
        );
        assert_eq!(intel.active_contributors_in_window(now, -1), 0);
        assert_eq!(intel.active_contributors_in_window(now, i64::MAX), 0);
    }

    #[test]
    fn temporal_weight_clamps_future_commits_to_one() {
        let future = CommitRecord {
            oid: None,
            ts: 2_000_000,
            author: "future@x.com".into(),
            committer: "future@x.com".into(),
            message: "future".into(),
            files: vec![("future.rs".into(), 100, 0)],
        };
        let past = CommitRecord {
            oid: None,
            ts: 1_000_000,
            author: "past@x.com".into(),
            committer: "past@x.com".into(),
            message: "past".into(),
            files: vec![("past.rs".into(), 100, 0)],
        };
        let mut intel = GitIntel::default();
        intel.commit_records = vec![future, past];

        let hotspots = intel.temporal_hotspots(1_500_000, 2);

        assert_eq!(hotspots[0], ("future.rs".into(), 1.0));
        assert!(hotspots[1].1 < 1.0);
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
        assert!(newest.oid.is_some());
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
                oid: None,
                ts: 1_000_000,
                author: "a@x.com".into(),
                committer: "a@x.com".into(),
                message: "old".into(),
                files: vec![("old.rs".into(), 300, 0)],
            },
            CommitRecord {
                oid: None,
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
                oid: None,
                ts: 100,
                author: "a@x.com".into(),
                committer: "a@x.com".into(),
                message: "small".into(),
                files: vec![("a.rs".into(), 1, 0), ("b.rs".into(), 1, 0)],
            },
            CommitRecord {
                oid: None,
                ts: 100,
                author: "a@x.com".into(),
                committer: "a@x.com".into(),
                message: "large".into(),
                files: (0..31).map(|i| (format!("l{i}.rs"), 1, 0)).collect(),
            },
        ];
        let entropy = intel.change_entropy_at(100);
        assert!((entropy["a.rs"] - 0.5).abs() < 1e-9);
        assert!(!entropy.contains_key("l0.rs"));
    }

    #[test]
    fn change_entropy_uses_injected_now_not_latest_commit_timestamp() {
        let now = 1_700_000_000;
        let day = 86_400;
        let past = CommitRecord {
            oid: None,
            ts: now - 180 * day,
            author: "a@x.com".into(),
            committer: "a@x.com".into(),
            message: "past".into(),
            files: vec![(("past_a.rs").into(), 1, 0), (("past_b.rs").into(), 1, 0)],
        };
        let future = CommitRecord {
            oid: None,
            ts: now + 365 * day,
            author: "a@x.com".into(),
            committer: "a@x.com".into(),
            message: "future".into(),
            files: vec![
                (("future_a.rs").into(), 1, 0),
                (("future_b.rs").into(), 1, 0),
            ],
        };
        let mut with_future = GitIntel::default();
        with_future.commit_records = vec![past.clone(), future];
        let mut without_future = GitIntel::default();
        without_future.commit_records = vec![past];

        let entropy_with_future = with_future.change_entropy_at(now);
        let entropy_without_future = without_future.change_entropy_at(now);

        assert!(
            (entropy_with_future["past_a.rs"] - entropy_without_future["past_a.rs"]).abs() < 1e-9
        );
        assert!((entropy_with_future["past_a.rs"] - 0.25).abs() < 1e-9);
        assert!((entropy_with_future["future_a.rs"] - 0.5).abs() < 1e-9);
    }

    #[test]
    fn commit_features_compute_kamei_values() {
        let mut intel = GitIntel::default();
        intel.commit_records.push(CommitRecord {
            oid: None,
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
    fn commit_risks_scores_and_sorts() {
        let mut intel = GitIntel::default();
        for i in 0..20 {
            intel.commit_records.push(CommitRecord {
                oid: Some(format!("dominant-{i}")),
                ts: i,
                author: "dominant@x.com".into(),
                committer: "dominant@x.com".into(),
                message: format!("dominant prior {i}"),
                files: vec![("core/stable.rs".into(), 1, 0)],
            });
        }
        intel.commit_records.push(CommitRecord {
            oid: Some("tiny".into()),
            ts: 100,
            author: "dominant@x.com".into(),
            committer: "dominant@x.com".into(),
            message: "tiny dominant".into(),
            files: vec![("core/stable.rs".into(), 1, 0)],
        });
        intel.commit_records.push(CommitRecord {
            oid: Some("large".into()),
            ts: 101,
            author: "new@x.com".into(),
            committer: "new@x.com".into(),
            message: "large scattered".into(),
            files: vec![
                ("api/http/router.rs".into(), 400, 30),
                ("engine/core/mod.rs".into(), 300, 40),
                ("gui/src/app.tsx".into(), 250, 20),
                ("docs/guide.md".into(), 100, 10),
            ],
        });

        let risks = intel.recent_commit_risks(2);

        assert_eq!(risks.len(), 2);
        assert_eq!(risks[0].commit_id, "large");
        assert_eq!(risks[1].commit_id, "tiny");
        assert!(risks[0].risk > risks[1].risk);
        assert_eq!(risks[0].inputs.exp, 0.0);
        assert_eq!(risks[1].inputs.exp, 20.0);
        assert_eq!(risks[0].inputs.nd, 4.0);
        assert_eq!(risks[0].inputs.ns, 4.0);
    }

    #[test]
    fn line_aggregations_saturate_large_values() {
        let mut intel = GitIntel::default();
        intel.file_churn.insert("huge.rs".into(), 1);
        intel.file_churn.insert("other.rs".into(), 1);
        intel.commit_records.push(CommitRecord {
            oid: None,
            ts: 1,
            author: "a@x.com".into(),
            committer: "a@x.com".into(),
            message: "huge".into(),
            files: vec![
                ("huge.rs".into(), u32::MAX, 1),
                ("other.rs".into(), 1, u32::MAX),
            ],
        });

        let hotspots = intel.temporal_hotspots(1, 2);
        let features = intel.commit_features();

        assert_eq!(hotspots[0].1, 3.0);
        assert_eq!(features[0].1.la, u32::MAX);
        assert_eq!(features[0].1.ld, u32::MAX);
        assert!(features[0].1.entropy.is_finite());
        assert_eq!(intel.churn_risk("huge.rs"), 1.0);
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
            oid: None,
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
                oid: None,
                ts: 1,
                author: "dev@x.com".into(),
                committer: "dev@x.com".into(),
                message: "introduce parser architecture".into(),
                files: vec![],
            },
            CommitRecord {
                oid: None,
                ts: 2,
                author: "dev@x.com".into(),
                committer: "dev@x.com".into(),
                message: "chore: update snapshots".into(),
                files: vec![],
            },
            CommitRecord {
                oid: None,
                ts: 3,
                author: "dev@x.com".into(),
                committer: "dev@x.com".into(),
                message: "ci: migrate to new runner".into(),
                files: vec![],
            },
            CommitRecord {
                oid: None,
                ts: 4,
                author: "dependabot@github.com".into(),
                committer: "dependabot@github.com".into(),
                message: "replace dependency versions".into(),
                files: vec![],
            },
            CommitRecord {
                oid: None,
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
            classify_commit("Claude", "claude@example.com", "change"),
            None
        );
        assert_eq!(
            classify_commit("claude@example.com", "human@x.com", "change"),
            None
        );
        assert_eq!(
            classify_commit("copilot-swe-agent@github.com", "human@x.com", "change")
                .unwrap()
                .tier,
            1
        );
        assert_eq!(
            classify_commit(
                "claude-code[bot]@users.noreply.github.com",
                "human@x.com",
                "change"
            )
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
        assert_eq!(
            classify_commit(
                "human@x.com",
                "human@x.com",
                "fix\n\nco-authored-by: claude <bot@anthropic.com>"
            )
            .unwrap()
            .tier,
            2
        );
        assert_eq!(
            classify_commit("Dev (AIDER)", "human@x.com", "change")
                .unwrap()
                .tier,
            2
        );

        let mut intel = GitIntel::default();
        intel.commit_records = vec![
            CommitRecord {
                oid: None,
                ts: 1,
                author: "copilot-swe-agent@github.com".into(),
                committer: "human@x.com".into(),
                message: "change".into(),
                files: vec![],
            },
            CommitRecord {
                oid: None,
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
