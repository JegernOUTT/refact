use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;

use crate::global_context::GlobalContext;
use crate::stats::event::LlmCallEvent;

pub async fn stats_writer_task(
    gcx: Arc<GlobalContext>,
    receiver: tokio::sync::mpsc::Receiver<LlmCallEvent>,
) {
    let stats_dir_fn = Arc::new(move || {
        let gcx = gcx.clone();
        Box::pin(async move { crate::stats::get_stats_dir(gcx).await })
            as std::pin::Pin<Box<dyn std::future::Future<Output = std::path::PathBuf> + Send>>
    });
    refact_stats::writer::stats_writer_task(stats_dir_fn, receiver).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::global_context::tests::make_test_gcx;

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

    fn seq_filename(stats_dir: &std::path::PathBuf, seq: u32) -> std::path::PathBuf {
        stats_dir.join(format!("{:08}.jsonl", seq))
    }

    #[tokio::test]
    async fn test_writer_uses_workspace_dir_when_workspace_appears_after_startup() {
        let gcx = make_test_gcx().await;
        let config_stats_dir = gcx.config_dir.join("stats");
        let workspace = tempfile::tempdir().unwrap();
        let workspace_stats_dir = workspace.path().join(".refact").join("stats");

        let (tx, rx) = tokio::sync::mpsc::channel::<LlmCallEvent>(1000);
        let handle = tokio::spawn(stats_writer_task(gcx.clone(), rx));

        {
            *gcx.documents_state.workspace_folders.lock().unwrap() =
                vec![workspace.path().to_path_buf()];
        }

        tx.send(make_event(7)).await.unwrap();
        drop(tx);
        handle.await.unwrap();

        let workspace_file_path = seq_filename(&workspace_stats_dir, 1);
        assert!(
            workspace_file_path.exists(),
            "workspace stats file should exist"
        );

        let config_file_path = seq_filename(&config_stats_dir, 1);
        assert!(
            !config_file_path.exists(),
            "config stats file should not be created when workspace becomes available before the first event"
        );

        let contents = tokio::fs::read_to_string(&workspace_file_path).await.unwrap();
        let parsed: LlmCallEvent = serde_json::from_str(contents.trim()).unwrap();
        assert_eq!(parsed.chat_id, "chat-7");
    }
}
