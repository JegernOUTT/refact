use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock as ARwLock;
use tracing::{info, warn};

use crate::global_context::GlobalContext;
use crate::stats::event::LlmCallEvent;

const MAX_FILE_SIZE: u64 = 1024 * 1024;

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

pub async fn stats_writer_task(
    gcx: Arc<ARwLock<GlobalContext>>,
    mut receiver: tokio::sync::mpsc::UnboundedReceiver<LlmCallEvent>,
) {
    let stats_dir = crate::stats::get_stats_dir(gcx.clone()).await;
    if let Err(e) = fs::create_dir_all(&stats_dir).await {
        warn!("stats: failed to create stats dir {:?}: {}", stats_dir, e);
        return;
    }
    info!("stats: writing to {:?}", stats_dir);

    let mut seq = find_current_sequence(&stats_dir).await;
    if seq == 0 {
        seq = 1;
    }

    let mut current_path = seq_filename(&stats_dir, seq);
    let mut file = match open_append(&current_path).await {
        Ok(f) => f,
        Err(e) => {
            warn!("stats: failed to open {:?}: {}", current_path, e);
            return;
        }
    };

    loop {
        let event = match receiver.recv().await {
            Some(e) => e,
            None => break,
        };

        let line = match serde_json::to_string(&event) {
            Ok(s) => s,
            Err(e) => {
                warn!("stats: failed to serialize event: {}", e);
                continue;
            }
        };

        let need_rotation = match file_size(&current_path).await {
            Ok(sz) => sz >= MAX_FILE_SIZE,
            Err(_) => false,
        };

        if need_rotation {
            let _ = file.flush().await;
            drop(file);
            seq += 1;
            current_path = seq_filename(&stats_dir, seq);
            file = match open_append(&current_path).await {
                Ok(f) => f,
                Err(e) => {
                    warn!("stats: failed to open new file {:?}: {}", current_path, e);
                    return;
                }
            };
        }

        let bytes = format!("{}\n", line);
        if let Err(e) = file.write_all(bytes.as_bytes()).await {
            warn!("stats: write failed: {}", e);
            continue;
        }
        if let Err(e) = file.flush().await {
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

async fn file_size(path: &PathBuf) -> std::io::Result<u64> {
    let meta = fs::metadata(path).await?;
    Ok(meta.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::event::LlmCallEvent;
    use tokio::io::AsyncReadExt;

    fn make_event(i: u64) -> LlmCallEvent {
        LlmCallEvent {
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

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<LlmCallEvent>();

        let stats_dir_clone = stats_dir.clone();
        let handle = tokio::spawn(async move {
            let mut seq = find_current_sequence(&stats_dir_clone).await;
            if seq == 0 { seq = 1; }
            let path = seq_filename(&stats_dir_clone, seq);
            let mut file = open_append(&path).await.unwrap();

            let mut receiver = rx;
            while let Some(event) = receiver.recv().await {
                let line = serde_json::to_string(&event).unwrap();
                file.write_all(format!("{}\n", line).as_bytes()).await.unwrap();
                file.flush().await.unwrap();
            }
        });

        tx.send(make_event(1)).unwrap();
        tx.send(make_event(2)).unwrap();
        drop(tx);
        handle.await.unwrap();

        let file_path = seq_filename(&stats_dir, 1);
        assert!(file_path.exists(), "stats file should exist");

        let mut contents = String::new();
        fs::File::open(&file_path).await.unwrap().read_to_string(&mut contents).await.unwrap();
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

        let current_size = file_size(&file1_path).await.unwrap();
        assert!(current_size >= MAX_FILE_SIZE, "file should be at/above limit");

        let seq = find_current_sequence(&stats_dir).await;
        assert_eq!(seq, 1);

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<LlmCallEvent>();
        let stats_dir_clone = stats_dir.clone();

        let handle = tokio::spawn(async move {
            let mut current_seq = find_current_sequence(&stats_dir_clone).await;
            if current_seq == 0 { current_seq = 1; }
            let mut current_path = seq_filename(&stats_dir_clone, current_seq);
            let mut file = open_append(&current_path).await.unwrap();

            let mut receiver = rx;
            while let Some(event) = receiver.recv().await {
                let need_rotation = file_size(&current_path).await.map(|sz| sz >= MAX_FILE_SIZE).unwrap_or(false);
                if need_rotation {
                    file.flush().await.unwrap();
                    drop(file);
                    current_seq += 1;
                    current_path = seq_filename(&stats_dir_clone, current_seq);
                    file = open_append(&current_path).await.unwrap();
                }
                let line = serde_json::to_string(&event).unwrap();
                file.write_all(format!("{}\n", line).as_bytes()).await.unwrap();
                file.flush().await.unwrap();
            }
        });

        tx.send(make_event(42)).unwrap();
        drop(tx);
        handle.await.unwrap();

        let file2_path = seq_filename(&stats_dir, 2);
        assert!(file2_path.exists(), "rotated file 00000002.jsonl should exist");

        let contents = fs::read_to_string(&file2_path).await.unwrap();
        let parsed: LlmCallEvent = serde_json::from_str(contents.trim()).unwrap();
        assert_eq!(parsed.chat_id, "chat-42");
    }
}
