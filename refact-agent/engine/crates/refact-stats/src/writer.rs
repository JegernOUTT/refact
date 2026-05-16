use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};

use crate::event::LlmCallEvent;

pub type StatsDirFn = Arc<
    dyn Fn() -> Pin<Box<dyn std::future::Future<Output = PathBuf> + Send>> + Send + Sync,
>;

const MAX_FILE_SIZE: u64 = 1024 * 1024;
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
    use crate::event::LlmCallEvent;
    use tokio::io::AsyncReadExt;

    fn make_event(i: u64) -> LlmCallEvent {
        LlmCallEvent {
            id: format!("test-id-{}", i),
            ts_start: "2024-01-01T00:00:00Z".to_string(),
            ts_end: "2024-01-01T00:00:01Z".to_string(),
            duration_ms: i * 100,
            chat_id: format!("chat-{}", i),
            root_chat_id: None,
            mode: "agent".to_string(),
            task_id: None,
            task_role: None,
            agent_id: None,
            card_id: None,
            model_id: "anthropic/claude-3".to_string(),
            provider: "anthropic".to_string(),
            model: "claude-3".to_string(),
            messages_count: 3,
            tools_count: 0,
            max_tokens: 4096,
            temperature: Some(0.0),
            success: true,
            error_message: None,
            finish_reason: Some("stop".to_string()),
            attempt_n: 1,
            retry_reason: None,
            prompt_tokens: 100,
            completion_tokens: 50,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            total_tokens: 150,
            cost_usd: Some(0.001),
        }
    }

    #[tokio::test]
    async fn test_writer_creates_jsonl_file() {
        let dir = tempfile::tempdir().unwrap();
        let stats_dir = dir.path().to_path_buf();

        let (tx, rx) = tokio::sync::mpsc::channel::<LlmCallEvent>(1000);

        let stats_dir_clone = stats_dir.clone();
        let handle = tokio::spawn(async move {
            let mut seq = find_current_sequence(&stats_dir_clone).await;
            if seq == 0 {
                seq = 1;
            }
            let path = seq_filename(&stats_dir_clone, seq);
            let mut file = open_append(&path).await.unwrap();

            let mut receiver = rx;
            while let Some(event) = receiver.recv().await {
                let line = serde_json::to_string(&event).unwrap();
                file.write_all(format!("{}\n", line).as_bytes())
                    .await
                    .unwrap();
                file.flush().await.unwrap();
            }
        });

        tx.send(make_event(1)).await.unwrap();
        tx.send(make_event(2)).await.unwrap();
        drop(tx);
        handle.await.unwrap();

        let file_path = seq_filename(&stats_dir, 1);
        assert!(file_path.exists(), "stats file should exist");

        let mut contents = String::new();
        fs::File::open(&file_path)
            .await
            .unwrap()
            .read_to_string(&mut contents)
            .await
            .unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2, "should have 2 JSONL lines");

        let parsed: LlmCallEvent = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(parsed.chat_id, "chat-1");
    }

    #[tokio::test]
    async fn test_rotation_on_size_limit() {
        let dir = tempfile::tempdir().unwrap();
        let stats_dir = dir.path().to_path_buf();

        let file1_path = seq_filename(&stats_dir, 1);
        let large_content = "x".repeat((MAX_FILE_SIZE) as usize);
        fs::write(&file1_path, &large_content).await.unwrap();

        let initial_size = get_initial_file_size(&file1_path).await;
        assert!(
            initial_size >= MAX_FILE_SIZE,
            "file should be at/above limit"
        );

        let seq = find_current_sequence(&stats_dir).await;
        assert_eq!(seq, 1);

        let (tx, rx) = tokio::sync::mpsc::channel::<LlmCallEvent>(1000);
        let stats_dir_clone = stats_dir.clone();

        let handle = tokio::spawn(async move {
            let mut current_seq = find_current_sequence(&stats_dir_clone).await;
            if current_seq == 0 {
                current_seq = 1;
            }
            let mut current_path = seq_filename(&stats_dir_clone, current_seq);
            let mut file = open_append(&current_path).await.unwrap();
            let mut current_size = get_initial_file_size(&current_path).await;

            let mut receiver = rx;
            while let Some(event) = receiver.recv().await {
                let line = serde_json::to_string(&event).unwrap();
                let bytes = format!("{}\n", line);
                let byte_len = bytes.len() as u64;
                if current_size + byte_len > MAX_FILE_SIZE {
                    file.flush().await.unwrap();
                    drop(file);
                    current_seq += 1;
                    current_path = seq_filename(&stats_dir_clone, current_seq);
                    file = open_append(&current_path).await.unwrap();
                    current_size = 0;
                }
                file.write_all(bytes.as_bytes()).await.unwrap();
                current_size += byte_len;
                file.flush().await.unwrap();
            }
        });

        tx.send(make_event(42)).await.unwrap();
        drop(tx);
        handle.await.unwrap();

        let file2_path = seq_filename(&stats_dir, 2);
        assert!(
            file2_path.exists(),
            "rotated file 00000002.jsonl should exist"
        );

        let contents = fs::read_to_string(&file2_path).await.unwrap();
        let parsed: LlmCallEvent = serde_json::from_str(contents.trim()).unwrap();
        assert_eq!(parsed.chat_id, "chat-42");
    }
}
