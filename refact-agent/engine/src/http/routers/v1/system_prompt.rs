use axum::http::{Response, StatusCode};
use axum::extract::State;
use hyper::Body;
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::call_validation::{ChatContent, ChatMessage, ChatMeta, validate_mode_for_request};
use crate::chat::get_or_create_session_with_trajectory;
use crate::chat::prepare::build_canonical_openai_tools;
use crate::chat::trajectories::{
    ensure_frozen_prefix, maybe_save_trajectory, new_frozen_request_prefix,
};
use crate::custom_error::ScratchError;
use crate::indexing_utils::wait_for_indexing_if_needed;
use crate::scratchpads::chat_utils_prompts::prepend_the_right_system_prompt_and_maybe_more_initial_messages;
use crate::scratchpads::scratchpad_utils::HasRagResults;
use crate::tools::tools_list::{apply_mcp_lazy_filter, get_tools_for_mode};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PrependSystemPromptPost {
    pub messages: Vec<ChatMessage>,
    pub chat_meta: ChatMeta,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::get_or_create_session_with_trajectory;
    use crate::chat::types::{ChatSession, TaskMeta};
    use refact_chat_api::FrozenRequestPrefix;
    use serde_json::json;
    use std::sync::Arc;

    async fn make_app_with_workspace(
        root: &std::path::Path,
    ) -> (Arc<crate::global_context::GlobalContext>, AppState) {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        *app.workspace
            .documents_state
            .workspace_folders
            .lock()
            .unwrap() = vec![root.to_path_buf()];
        (gcx, app)
    }

    fn prefix(system_prompt: &str) -> FrozenRequestPrefix {
        new_frozen_request_prefix(
            Some(system_prompt.to_string()),
            json!([{"type":"function","function":{"name":"cat"}}]),
        )
    }

    #[tokio::test]
    async fn frozen_prefix_init_persists_active_normal_session_immediately() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "init-freeze-normal";
        let session_arc =
            get_or_create_session_with_trajectory(app.clone(), &app.chat.sessions, chat_id).await;
        {
            let mut session = session_arc.lock().await;
            session.add_message(ChatMessage::new("user".to_string(), "hello".to_string()));
        }

        persist_init_frozen_prefix(app, chat_id, prefix("frozen system")).await;

        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        let raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert_eq!(
            raw["frozen_request_prefix"]["system_prompt"],
            "frozen system"
        );
    }

    #[tokio::test]
    async fn frozen_prefix_init_uses_task_session_path_not_generic_path() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let task_id = "task-freeze-init";
        let agent_id = "agent-1";
        let chat_id = "task-agent-freeze-init";
        tokio::fs::create_dir_all(dir.path().join(".refact").join("tasks").join(task_id))
            .await
            .unwrap();
        let session_arc = Arc::new(tokio::sync::Mutex::new(ChatSession::new(
            chat_id.to_string(),
        )));
        {
            let mut session = session_arc.lock().await;
            session.thread.task_meta = Some(TaskMeta {
                task_id: task_id.to_string(),
                role: "agents".to_string(),
                agent_id: Some(agent_id.to_string()),
                card_id: Some("T-1".to_string()),
                planner_chat_id: None,
            });
            session.add_message(ChatMessage::new("user".to_string(), "hello".to_string()));
        }
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc);

        persist_init_frozen_prefix(app, chat_id, prefix("task frozen")).await;

        let generic_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        let task_path = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join(task_id)
            .join("trajectories")
            .join("agents")
            .join(agent_id)
            .join(format!("{chat_id}.json"));
        assert!(!tokio::fs::try_exists(generic_path).await.unwrap());
        let raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(task_path).await.unwrap()).unwrap();
        assert_eq!(raw["frozen_request_prefix"]["system_prompt"], "task frozen");
    }

    #[tokio::test]
    async fn frozen_prefix_init_preserves_existing_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "init-freeze-existing";
        let session_arc =
            get_or_create_session_with_trajectory(app.clone(), &app.chat.sessions, chat_id).await;
        {
            let mut session = session_arc.lock().await;
            session.thread.frozen_request_prefix = Some(prefix("original frozen"));
            session.add_message(ChatMessage::new("user".to_string(), "hello".to_string()));
            session.increment_version();
        }

        persist_init_frozen_prefix(app.clone(), chat_id, prefix("replacement frozen")).await;
        crate::chat::trajectories::maybe_save_trajectory(app, session_arc).await;

        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        let raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert_eq!(
            raw["frozen_request_prefix"]["system_prompt"],
            "original frozen"
        );
    }

    #[tokio::test]
    async fn frozen_prefix_legacy_buddy_migration_does_not_create_generic_copy() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "legacy-buddy-freeze";
        let buddy_dir = dir
            .path()
            .join(".refact")
            .join("buddy")
            .join("chats")
            .join("conversations");
        tokio::fs::create_dir_all(&buddy_dir).await.unwrap();
        tokio::fs::write(
            buddy_dir.join(format!("{chat_id}.json")),
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "Buddy Legacy",
                "model": "model",
                "mode": "buddy",
                "tool_use": "agent",
                "messages": [
                    {"role":"system","content":"buddy system"},
                    {"role":"user","content":"hello buddy"}
                ],
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true,
                "buddy_meta": {"is_buddy_chat": true, "buddy_chat_kind": "investigation"}
            }))
            .unwrap(),
        )
        .await
        .unwrap();
        let session_arc =
            get_or_create_session_with_trajectory(app.clone(), &app.chat.sessions, chat_id).await;
        {
            let session = session_arc.lock().await;
            assert!(session.thread.buddy_meta.is_some());
            let prefix = session.thread.frozen_request_prefix.as_ref().unwrap();
            assert_eq!(prefix.system_prompt.as_deref(), Some("buddy system"));
            assert!(prefix.tools_canonical.is_none());
        }

        let generic_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        let raw: serde_json::Value = serde_json::from_str(
            &tokio::fs::read_to_string(buddy_dir.join(format!("{chat_id}.json")))
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(!tokio::fs::try_exists(generic_path).await.unwrap());
        assert_eq!(
            raw["frozen_request_prefix"]["system_prompt"],
            "buddy system"
        );
        assert!(raw["frozen_request_prefix"]["tools_canonical"].is_null());
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PrependSystemPromptResponse {
    pub messages: Vec<ChatMessage>,
    pub messages_to_stream_back: Vec<serde_json::Value>,
}

async fn persist_init_frozen_prefix(
    app: AppState,
    chat_id: &str,
    frozen_prefix: refact_chat_api::FrozenRequestPrefix,
) {
    let session_arc =
        get_or_create_session_with_trajectory(app.clone(), &app.chat.sessions, chat_id).await;
    let installed = {
        let mut session = session_arc.lock().await;
        ensure_frozen_prefix(
            &mut session,
            frozen_prefix.system_prompt.clone(),
            frozen_prefix.tools_canonical.clone(),
        )
        .is_some()
    };
    if installed {
        maybe_save_trajectory(app, session_arc).await;
    }
}

pub async fn handle_v1_prepend_system_prompt_and_maybe_more_initial_messages(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    wait_for_indexing_if_needed(gcx.clone()).await;

    let post = serde_json::from_slice::<PrependSystemPromptPost>(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;
    let mut has_rag_results = HasRagResults::new();

    let mode_id = validate_mode_for_request(gcx.clone(), &post.chat_meta.chat_mode)
        .await
        .map_err(|e| {
            ScratchError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("Invalid chat mode: {}", e),
            )
        })?;

    let mode_tools = apply_mcp_lazy_filter(get_tools_for_mode(gcx.clone(), &mode_id, None).await);
    let tool_descs: Vec<_> = mode_tools
        .tools
        .into_iter()
        .map(|tool| tool.tool_description())
        .collect();
    let prompt_tool_names = tool_descs.iter().map(|t| t.name.clone()).collect();

    let (messages, _) = prepend_the_right_system_prompt_and_maybe_more_initial_messages(
        crate::app_state::AppState::from_gcx(gcx.clone()).await,
        post.messages,
        &post.chat_meta,
        &None,
        &mut has_rag_results,
        prompt_tool_names,
        &mode_id,
        "",
    )
    .await;

    let system_prompt = messages.iter().find_map(|message| {
        if message.role == "system" {
            match &message.content {
                ChatContent::SimpleText(text) => Some(text.clone()),
                _ => None,
            }
        } else {
            None
        }
    });
    let canonical_tools = build_canonical_openai_tools(gcx.clone(), &tool_descs, false, true).await;
    let frozen_prefix = new_frozen_request_prefix(
        system_prompt,
        serde_json::Value::Array(canonical_tools.tools),
    );
    persist_init_frozen_prefix(app.clone(), &post.chat_meta.chat_id, frozen_prefix).await;
    let messages_to_stream_back = has_rag_results.in_json;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(
            serde_json::to_string(&PrependSystemPromptResponse {
                messages,
                messages_to_stream_back,
            })
            .unwrap(),
        ))
        .unwrap())
}
