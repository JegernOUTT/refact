use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub const LOG_MAX_AGE: Duration = Duration::from_secs(14 * 24 * 60 * 60);
pub const LOG_DIR_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct CacheSweepSummary {
    pub removed_files: usize,
    pub removed_bytes: u64,
}

pub async fn sweep_logs_dir(cache_dir: &Path) -> Result<CacheSweepSummary, String> {
    let logs_dir = cache_dir.join("logs");
    tokio::task::spawn_blocking(move || sweep_logs_dir_blocking(&logs_dir, SystemTime::now()))
        .await
        .map_err(|error| format!("log sweep task failed: {error}"))?
}

fn log_file_date_age(name: &str, today: chrono::NaiveDate) -> Option<Duration> {
    let suffix = name.strip_prefix("rustbinary.")?;
    let date =
        chrono::NaiveDate::parse_from_str(&suffix[..suffix.len().min(10)], "%Y-%m-%d").ok()?;
    let days = today.signed_duration_since(date).num_days();
    (days > 0).then(|| Duration::from_secs(days as u64 * 24 * 60 * 60))
}

fn sweep_logs_dir_blocking(logs_dir: &Path, now: SystemTime) -> Result<CacheSweepSummary, String> {
    let today_date = chrono::Utc::now().date_naive();
    let today = today_date.format("%Y-%m-%d").to_string();
    let mut files: Vec<(PathBuf, SystemTime, u64)> = Vec::new();
    let Ok(entries) = std::fs::read_dir(logs_dir) else {
        return Ok(CacheSweepSummary::default());
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("rustbinary.") || name.contains(&today) {
            continue;
        }
        let path = entry.path();
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            continue;
        }
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let effective = match log_file_date_age(&name, today_date) {
            Some(age) => modified.min(now.checked_sub(age).unwrap_or(SystemTime::UNIX_EPOCH)),
            None => modified,
        };
        files.push((path, effective, metadata.len()));
    }
    files.sort_by_key(|(_, modified, _)| *modified);
    let mut total: u64 = files.iter().map(|(_, _, size)| *size).sum();
    let mut summary = CacheSweepSummary::default();
    for (path, modified, size) in files {
        let old = now
            .duration_since(modified)
            .map(|age| age > LOG_MAX_AGE)
            .unwrap_or(false);
        if !old && total <= LOG_DIR_MAX_BYTES {
            continue;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(size);
            summary.removed_files += 1;
            summary.removed_bytes = summary.removed_bytes.saturating_add(size);
        }
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sweep_only_removes_eligible_log_files() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("rustbinary.2000-01-01");
        let unrelated = temp.path().join("other.2000-01-01");
        std::fs::write(&old, b"old").unwrap();
        std::fs::write(&unrelated, b"keep").unwrap();
        let summary = sweep_logs_dir_blocking(temp.path(), SystemTime::now()).unwrap();
        assert_eq!(summary.removed_files, 1);
        assert!(!old.exists());
        assert!(unrelated.exists());
    }
}
