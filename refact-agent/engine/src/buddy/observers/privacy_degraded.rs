use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use refact_chat_history::trajectory_snapshot::TrajectorySnapshot;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::buddy::types::BuddyThreadMeta;
use crate::call_validation::ChatMessage;
use crate::global_context::GlobalContext;

static PRIVACY_DEGRADED_WARNING_CREATED: AtomicBool = AtomicBool::new(false);

fn warning_text(reason: &str) -> String {
    let reason = crate::buddy::actor::redact_sensitive(reason);
    let reason = crate::llm::safe_truncate(reason.trim(), 500);
    format!(
        "Privacy file-access observation is unavailable on {}: {}. Shell and process commands remain fail-open. Refact is using heuristic command-path attribution as a best-effort fallback, which may miss indirect file reads.",
        std::env::consts::OS,
        reason
    )
}

async fn create_warning_chat(gcx: Arc<GlobalContext>, reason: &str) -> Result<String, String> {
    let chat_id = Uuid::new_v4().to_string();
    let created_at = chrono::Utc::now().to_rfc3339();
    let model =
        crate::buddy::actor::resolve_buddy_chat_model(AppState::from_gcx(gcx.clone()).await).await;
    let snapshot = TrajectorySnapshot {
        goal: None,
        goal_ledger: Vec::new(),
        goal_verification_blocked_until_ms: None,
        chat_id: chat_id.clone(),
        title: "Privacy observation degraded".to_string(),
        model,
        mode: "buddy".to_string(),
        tool_use: "agent".to_string(),
        messages: vec![ChatMessage::new(
            "assistant".to_string(),
            warning_text(reason),
        )],
        created_at,
        boost_reasoning: false,
        checkpoints_enabled: false,
        context_tokens_cap: None,
        include_project_info: false,
        is_title_generated: true,
        auto_approve_editing_tools: false,
        auto_approve_dangerous_commands: false,
        autonomous_no_confirm: true,
        version: 1,
        task_meta: None,
        worktree: None,
        parent_id: None,
        link_type: None,
        root_chat_id: None,
        reasoning_effort: None,
        thinking_budget: None,
        temperature: None,
        frequency_penalty: None,
        max_tokens: None,
        parallel_tool_calls: None,
        previous_response_id: None,
        active_skill: None,
        auto_enrichment_enabled: None,
        buddy_meta: Some(BuddyThreadMeta {
            is_buddy_chat: true,
            buddy_chat_kind: "system".to_string(),
            workflow_id: Some("privacy_degraded".to_string()),
        }),
        auto_compact_enabled: None,
        frozen_request_prefix: None,
        claude_code_identity: None,
        reactive_compact_attempts: None,
        wake_up_at: None,
        waiting_for_card_ids: Vec::new(),
    };
    crate::chat::trajectories::save_trajectory_snapshot(gcx, snapshot).await?;
    Ok(chat_id)
}

async fn warn_once_with_flag(
    gcx: Arc<GlobalContext>,
    reason: &str,
    created: &AtomicBool,
) -> Option<Result<String, String>> {
    if created
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return None;
    }
    Some(create_warning_chat(gcx, reason).await)
}

pub async fn warn_once(gcx: Arc<GlobalContext>, reason: &str) {
    if let Some(Err(error)) =
        warn_once_with_flag(gcx, reason, &PRIVACY_DEGRADED_WARNING_CREATED).await
    {
        tracing::warn!("failed to create privacy degraded Buddy chat: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn privacy_degraded_warning_creates_one_buddy_chat_per_lifetime() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
        let created = AtomicBool::new(false);

        let first = warn_once_with_flag(gcx.clone(), "ptrace unavailable", &created)
            .await
            .expect("first warning fires")
            .expect("first warning creates a chat");
        let second = warn_once_with_flag(gcx.clone(), "another reason", &created).await;

        assert!(second.is_none());
        let conversations = crate::chat::trajectories::get_buddy_conversations_dir(gcx.clone())
            .await
            .unwrap();
        let warnings = std::fs::read_dir(conversations)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy() == format!("{first}.json"))
            .count();
        assert_eq!(warnings, 1);
        let loaded = crate::chat::trajectories::load_trajectory_for_chat(gcx, &first)
            .await
            .unwrap();
        assert_eq!(
            loaded.thread.buddy_meta.unwrap().workflow_id.as_deref(),
            Some("privacy_degraded")
        );
        let text = loaded.messages[0].content.content_text_only();
        assert!(text.contains(std::env::consts::OS));
        assert!(text.contains("ptrace unavailable"));
        assert!(text.contains("heuristic command-path attribution"));
    }
}
