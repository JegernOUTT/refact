use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};

use crate::event::LlmCallEvent;

pub type StatsDirFn =
    Arc<dyn Fn() -> Pin<Box<dyn std::future::Future<Output = PathBuf> + Send>> + Send + Sync>;

pub const MAX_FILE_SIZE: u64 = 1024 * 1024;
pub const RETENTION_MAX_DAYS: u64 = 180;
pub const RETENTION_MAX_AGE: Duration = Duration::from_secs(RETENTION_MAX_DAYS * 24 * 60 * 60);
pub const RETENTION_MAX_BYTES: u64 = 200 * 1024 * 1024;
const BATCH_SIZE: usize = 32;
const BATCH_TIMEOUT_MS: u64 = 100;

struct StatsFileState {
    stats_dir: PathBuf,
    seq: u32,
    file: fs::File,
    current_size: u64,
}

async fn find_current_sequence(stats_dir: &PathBuf) -> u32 {
    let mut max_seq: u32 = 0;
    let mut rd = match fs::read_dir(stats_dir).await {
        Ok(rd) => rd,
        Err(_) => return 0,
    };
    while let Ok(Some(entry)) = rd.next_entry().await {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.ends_with(".jsonl") {
            let stem = name_str.trim_end_matches(".jsonl");
            if let Ok(n) = stem.parse::<u32>() {
                if n > max_seq {
                    max_seq = n;
                }
            }
        }
    }
    max_seq
}

fn seq_filename(stats_dir: &PathBuf, seq: u32) -> PathBuf {
    stats_dir.join(format!("{:08}.jsonl", seq))
}

async fn get_initial_file_size(path: &PathBuf) -> u64 {
    fs::metadata(path).await.map(|m| m.len()).unwrap_or(0)
}

async fn open_stats_file_state(stats_dir: PathBuf) -> Option<StatsFileState> {
    if let Err(e) = fs::create_dir_all(&stats_dir).await {
        warn!("stats: failed to create stats dir {:?}: {}", stats_dir, e);
        return None;
    }

    let mut seq = find_current_sequence(&stats_dir).await;
    if seq == 0 {
        seq = 1;
    }

    let current_path = seq_filename(&stats_dir, seq);
    let file = match open_append(&current_path).await {
        Ok(file) => file,
        Err(e) => {
            warn!("stats: failed to open {:?}: {}", current_path, e);
            return None;
        }
    };
    let current_size = get_initial_file_size(&current_path).await;

    info!("stats: writing to {:?}", stats_dir);
    Some(StatsFileState {
        stats_dir,
        seq,
        file,
        current_size,
    })
}

async fn rotate_stats_file(state: &mut StatsFileState) -> std::io::Result<()> {
    state.file.flush().await?;

    let next_seq = state.seq + 1;
    let next_path = seq_filename(&state.stats_dir, next_seq);
    let next_file = open_append(&next_path).await?;

    state.seq = next_seq;
    state.file = next_file;
    state.current_size = 0;
    let stats_dir = state.stats_dir.clone();
    let pruning = tokio::task::spawn_blocking(move || {
        prune_stats_files(
            &stats_dir,
            next_seq,
            RETENTION_MAX_AGE,
            RETENTION_MAX_BYTES,
            None,
        )
    })
    .await;
    match pruning {
        Ok(Ok(())) => {}
        Ok(Err(error)) => warn!("stats: retention failed: {}", error),
        Err(error) => warn!("stats: retention task failed: {}", error),
    }
    Ok(())
}

struct RetentionFile {
    path: PathBuf,
    sequence: u32,
    size: u64,
    modified: SystemTime,
}

fn prune_stats_files(
    stats_dir: &PathBuf,
    current_sequence: u32,
    max_age: Duration,
    max_bytes: u64,
    max_files: Option<usize>,
) -> std::io::Result<()> {
    let now = SystemTime::now();
    let mut files = Vec::new();
    for entry in std::fs::read_dir(stats_dir)? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        let Some(sequence) = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(|stem| stem.parse::<u32>().ok())
        else {
            continue;
        };
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        files.push(RetentionFile {
            path,
            sequence,
            size: metadata.len(),
            modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        });
    }
    files.sort_by_key(|file| file.sequence);

    let mut retained_bytes = files.iter().map(|file| file.size).sum::<u64>();
    let mut retained_count = files.len();
    for file in files {
        if file.sequence == current_sequence {
            continue;
        }
        let expired = now
            .duration_since(file.modified)
            .map(|age| age > max_age)
            .unwrap_or(false);
        let over_bytes = retained_bytes > max_bytes;
        let over_files = max_files.is_some_and(|budget| retained_count > budget);
        if expired || over_bytes || over_files {
            match std::fs::remove_file(&file.path) {
                Ok(()) => {
                    retained_bytes = retained_bytes.saturating_sub(file.size);
                    retained_count = retained_count.saturating_sub(1);
                }
                Err(error) => warn!("stats: failed to prune {:?}: {}", file.path, error),
            }
        }
    }
    Ok(())
}

pub async fn stats_writer_task(
    stats_dir_fn: StatsDirFn,
    mut receiver: tokio::sync::mpsc::Receiver<LlmCallEvent>,
) {
    let mut state: Option<StatsFileState> = None;

    loop {
        let mut batch = Vec::with_capacity(BATCH_SIZE);

        let first = tokio::time::timeout(
            std::time::Duration::from_millis(BATCH_TIMEOUT_MS),
            receiver.recv(),
        )
        .await;

        match first {
            Ok(Some(event)) => {
                batch.push(event);
                while batch.len() < BATCH_SIZE {
                    match receiver.try_recv() {
                        Ok(e) => batch.push(e),
                        Err(_) => break,
                    }
                }
            }
            Ok(None) => break,
            Err(_) => continue,
        }

        let desired_stats_dir = stats_dir_fn().await;

        let should_open_stats_file = state
            .as_ref()
            .map(|current| current.stats_dir != desired_stats_dir)
            .unwrap_or(true);
        if should_open_stats_file {
            if let Some(new_state) = open_stats_file_state(desired_stats_dir.clone()).await {
                let previous_state = std::mem::replace(&mut state, Some(new_state));
                if let Some(mut previous_state) = previous_state {
                    if previous_state.stats_dir != desired_stats_dir {
                        info!(
                            "stats: switching from {:?} to {:?}",
                            previous_state.stats_dir, desired_stats_dir
                        );
                    }
                    if let Err(e) = previous_state.file.flush().await {
                        warn!("stats: flush failed while switching dirs: {}", e);
                    }
                }
            }
        }

        let Some(state) = state.as_mut() else {
            continue;
        };

        for event in batch {
            let line = match serde_json::to_string(&event) {
                Ok(s) => s,
                Err(e) => {
                    warn!("stats: failed to serialize event: {}", e);
                    continue;
                }
            };

            let bytes = format!("{}\n", line);
            let byte_len = bytes.len() as u64;

            if state.current_size + byte_len > MAX_FILE_SIZE {
                if let Err(e) = rotate_stats_file(state).await {
                    warn!(
                        "stats: failed to rotate file in {:?}: {}",
                        state.stats_dir, e
                    );
                    break;
                }
            }

            if let Err(e) = state.file.write_all(bytes.as_bytes()).await {
                warn!("stats: write failed: {}", e);
                continue;
            }
            state.current_size += byte_len;
        }

        if let Err(e) = state.file.flush().await {
            warn!("stats: flush failed: {}", e);
        }
    }
}

async fn open_append(path: &PathBuf) -> std::io::Result<fs::File> {
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rotation_retention_deletes_two_oldest_files() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("tempdir failed: {error}"));
        for sequence in 1..=5 {
            let path = seq_filename(&dir.path().to_path_buf(), sequence);
            std::fs::write(path, format!("{sequence}\n"))
                .unwrap_or_else(|error| panic!("test file write failed: {error}"));
        }

        prune_stats_files(
            &dir.path().to_path_buf(),
            5,
            Duration::MAX,
            u64::MAX,
            Some(3),
        )
        .unwrap_or_else(|error| panic!("retention failed: {error}"));

        assert!(!seq_filename(&dir.path().to_path_buf(), 1).exists());
        assert!(!seq_filename(&dir.path().to_path_buf(), 2).exists());
        assert!(seq_filename(&dir.path().to_path_buf(), 3).exists());
        assert!(seq_filename(&dir.path().to_path_buf(), 4).exists());
        assert!(seq_filename(&dir.path().to_path_buf(), 5).exists());
    }
}
