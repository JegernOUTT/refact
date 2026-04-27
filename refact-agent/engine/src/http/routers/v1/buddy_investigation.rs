use axum::Extension;
use axum::response::Result;
use hyper::StatusCode;
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;
use uuid::Uuid;

use crate::buddy::types::InvestigationContext;
use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;

pub async fn handle_v1_buddy_investigations(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    axum::Json(ctx): axum::Json<InvestigationContext>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let chat_id = create_investigation_chat(gcx, &ctx).await?;
    Ok(axum::Json(serde_json::json!({ "chat_id": chat_id })))
}

async fn create_investigation_chat(
    gcx: Arc<ARwLock<GlobalContext>>,
    ctx: &InvestigationContext,
) -> Result<String, ScratchError> {
    let chat_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let system_msg = build_system_message(ctx);

    let snapshot = crate::chat::trajectories::TrajectorySnapshot {
        chat_id: chat_id.clone(),
        title: "Investigation".to_string(),
        model: String::new(),
        mode: "buddy".to_string(),
        tool_use: "agent".to_string(),
        messages: vec![
            crate::call_validation::ChatMessage {
                role: "system".to_string(),
                content: crate::call_validation::ChatContent::SimpleText(system_msg),
                ..Default::default()
            },
            crate::call_validation::ChatMessage {
                role: "user".to_string(),
                content: crate::call_validation::ChatContent::SimpleText(
                    ctx.initial_user_message.clone(),
                ),
                ..Default::default()
            },
        ],
        created_at: now,
        boost_reasoning: false,
        checkpoints_enabled: false,
        context_tokens_cap: None,
        include_project_info: true,
        is_title_generated: false,
        auto_approve_editing_tools: false,
        auto_approve_dangerous_commands: false,
        version: 1,
        task_meta: None,
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
        auto_enrichment_enabled: Some(true),
        buddy_meta: Some(crate::buddy::types::BuddyThreadMeta {
            is_buddy_chat: true,
            buddy_chat_kind: "investigation".to_string(),
            workflow_id: None,
        }),
    };

    crate::chat::trajectories::save_trajectory_snapshot(gcx, snapshot)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(chat_id)
}

fn build_system_message(ctx: &InvestigationContext) -> String {
    let mut parts = vec!["You are investigating a technical issue. Here is the context:\n".to_string()];
    if !ctx.fact_keys.is_empty() {
        parts.push(format!("Fact keys: {}", ctx.fact_keys.join(", ")));
    }
    if !ctx.diagnostic_ids.is_empty() {
        parts.push(format!("Diagnostic IDs: {}", ctx.diagnostic_ids.join(", ")));
    }
    if !ctx.log_excerpt.is_empty() {
        parts.push(format!("Log excerpt:\n{}", ctx.log_excerpt));
    }
    if !ctx.config_summary.is_empty() {
        parts.push(format!("Config summary:\n{}", ctx.config_summary));
    }
    parts.join("\n")
}
