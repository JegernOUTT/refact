use std::collections::HashMap;
use std::path::Path;

use git2::Repository;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlameOwner {
    pub author: String,
    pub lines: u32,
    pub pct: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeAge {
    pub median_age_days: f64,
    pub volatility: f64,
}

pub fn blame_ownership(repo_path: &Path, file: &str) -> Result<Vec<BlameOwner>, String> {
    let repo = Repository::open(repo_path).map_err(|e| format!("git open: {e}"))?;
    let blame = match repo.blame_file(Path::new(file), None) {
        Ok(blame) => blame,
        Err(_) => return Ok(Vec::new()),
    };

    let mut counts: HashMap<String, u32> = HashMap::new();
    for hunk in blame.iter() {
        let lines = hunk.lines_in_hunk() as u32;
        if lines == 0 {
            continue;
        }
        let author = hunk
            .final_signature()
            .email()
            .filter(|email| !email.is_empty())
            .unwrap_or("unknown")
            .to_string();
        *counts.entry(author).or_default() += lines;
    }

    let total: u32 = counts.values().sum();
    if total == 0 {
        return Ok(Vec::new());
    }

    let mut owners: Vec<BlameOwner> = counts
        .into_iter()
        .map(|(author, lines)| BlameOwner {
            author,
            lines,
            pct: lines as f64 / total as f64,
        })
        .collect();
    owners.sort_by(|a, b| b.lines.cmp(&a.lines).then_with(|| a.author.cmp(&b.author)));
    Ok(owners)
}

pub fn top_owner(repo_path: &Path, file: &str) -> Option<(String, f64)> {
    blame_ownership(repo_path, file).ok().and_then(|owners| {
        owners
            .into_iter()
            .next()
            .map(|owner| (owner.author, owner.pct))
    })
}

pub fn code_age(repo_path: &Path, file: &str, now_ts: i64) -> Result<CodeAge, String> {
    let repo = Repository::open(repo_path).map_err(|e| format!("git open: {e}"))?;
    let blame = match repo.blame_file(Path::new(file), None) {
        Ok(blame) => blame,
        Err(_) => {
            return Ok(CodeAge {
                median_age_days: 0.0,
                volatility: 0.0,
            })
        }
    };

    let mut age_buckets = Vec::new();
    let mut total_lines = 0_u64;
    for hunk in blame.iter() {
        let lines = hunk.lines_in_hunk();
        if lines == 0 {
            continue;
        }
        let ts = hunk.final_signature().when().seconds();
        let age = now_ts.saturating_sub(ts).max(0) as f64 / 86_400.0;
        let lines = lines as u64;
        total_lines = total_lines.saturating_add(lines);
        age_buckets.push((age, lines));
    }

    if total_lines == 0 {
        return Ok(CodeAge {
            median_age_days: 0.0,
            volatility: 0.0,
        });
    }

    age_buckets.sort_by(|a, b| a.0.total_cmp(&b.0));
    let median = weighted_median_sorted(&age_buckets, total_lines);
    let mean = age_buckets
        .iter()
        .map(|(age, lines)| age * *lines as f64)
        .sum::<f64>()
        / total_lines as f64;
    let variance = age_buckets
        .iter()
        .map(|(age, lines)| {
            let diff = age - mean;
            diff * diff * *lines as f64
        })
        .sum::<f64>()
        / total_lines as f64;
    let std_dev = variance.sqrt();
    let volatility = std_dev / (median + 1.0);

    Ok(CodeAge {
        median_age_days: median,
        volatility,
    })
}

fn weighted_median_sorted(age_buckets: &[(f64, u64)], total_lines: u64) -> f64 {
    let lower_pos = (total_lines - 1) / 2;
    let upper_pos = total_lines / 2;
    let lower = weighted_age_at(age_buckets, lower_pos);
    let upper = weighted_age_at(age_buckets, upper_pos);
    (lower + upper) / 2.0
}

fn weighted_age_at(age_buckets: &[(f64, u64)], pos: u64) -> f64 {
    let mut seen = 0_u64;
    for (age, lines) in age_buckets {
        seen = seen.saturating_add(*lines);
        if pos < seen {
            return *age;
        }
    }
    age_buckets.last().map(|(age, _)| *age).unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::{Repository, Signature, Time};

    fn commit_file(
        repo: &Repository,
        path: &str,
        content: &str,
        msg: &str,
        name: &str,
        email: &str,
        ts: i64,
    ) {
        let workdir = repo.workdir().unwrap().to_path_buf();
        std::fs::write(workdir.join(path), content).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new(path)).unwrap();
        index.write().unwrap();
        let tree_oid = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let time = Time::new(ts, 0);
        let sig = Signature::new(name, email, &time).unwrap();
        let parents: Vec<git2::Commit> = repo
            .head()
            .ok()
            .and_then(|head| head.target())
            .and_then(|oid| repo.find_commit(oid).ok())
            .into_iter()
            .collect();
        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &parent_refs)
            .unwrap();
    }

    fn mixed_age_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        commit_file(
            &repo,
            "src.txt",
            "one\ntwo\nthree\nfour\nfive\n",
            "alice base",
            "Alice",
            "alice@example.com",
            1_600_000_000,
        );
        commit_file(
            &repo,
            "src.txt",
            "one\nTWO\nthree\nFOUR\nfive\n",
            "bob edits",
            "Bob",
            "bob@example.com",
            1_600_086_400,
        );
        dir
    }

    #[test]
    fn blame_ownership_counts_lines_and_sorts_descending() {
        let dir = mixed_age_repo();
        let owners = blame_ownership(dir.path(), "src.txt").unwrap();
        let total: u32 = owners.iter().map(|owner| owner.lines).sum();

        assert_eq!(total, 5);
        assert_eq!(owners[0].author, "alice@example.com");
        assert_eq!(owners[0].lines, 3);
        assert!(owners
            .iter()
            .any(|owner| owner.author == "bob@example.com" && owner.lines == 2));
        assert!(owners[0].lines >= owners[1].lines);
    }

    #[test]
    fn top_owner_returns_dominant_author_and_pct() {
        let dir = mixed_age_repo();
        let (author, pct) = top_owner(dir.path(), "src.txt").unwrap();

        assert_eq!(author, "alice@example.com");
        assert!(pct > 0.0 && pct <= 1.0);
    }

    #[test]
    fn code_age_is_finite_for_mixed_age_file() {
        let dir = mixed_age_repo();
        let age = code_age(dir.path(), "src.txt", 1_600_172_800).unwrap();

        assert!(age.median_age_days.is_finite());
        assert!(age.median_age_days >= 0.0);
        assert!(age.volatility.is_finite());
        assert!(!age.volatility.is_nan());
    }

    #[test]
    fn code_age_clamps_future_dated_blame_to_zero_age() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        commit_file(
            &repo,
            "future.txt",
            "one\ntwo\n",
            "future",
            "Future",
            "future@example.com",
            1_700_086_400,
        );

        let age = code_age(dir.path(), "future.txt", 1_700_000_000).unwrap();

        assert_eq!(age.median_age_days, 0.0);
        assert_eq!(age.volatility, 0.0);
    }
}
