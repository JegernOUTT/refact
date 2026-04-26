use axum::Extension;
use axum::response::Result;
use hyper::{Body, Response, StatusCode};
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;

use crate::custom_error::ScratchError;
use crate::files_correction::get_project_dirs;
use crate::global_context::GlobalContext;
use crate::yaml_configs::customization_registry::load_merged_registry;
use crate::yaml_configs::project_configs_bootstrap::{
    global_configs_try_create_all, project_configs_ensure_dirs,
};

#[derive(Serialize)]
pub struct ChatModesResponse {
    pub modes: Vec<ChatModeInfo>,
    pub errors: Vec<ChatModeError>,
}

#[derive(Serialize)]
pub struct ChatModeInfo {
    pub id: String,
    pub title: String,
    pub description: String,
    pub tools_count: usize,
    pub thread_defaults: ChatModeThreadDefaults,
    pub ui: ChatModeUi,
}

#[derive(Serialize)]
pub struct ChatModeThreadDefaults {
    pub include_project_info: bool,
    pub checkpoints_enabled: bool,
    pub auto_approve_editing_tools: bool,
    pub auto_approve_dangerous_commands: bool,
}

#[derive(Serialize)]
pub struct ChatModeUi {
    pub order: i32,
    pub tags: Vec<String>,
}

#[derive(Serialize)]
pub struct ChatModeError {
    pub file_path: String,
    pub error: String,
}

fn json_response<T: Serialize>(data: &T) -> Result<Response<Body>, ScratchError> {
    let body = serde_json::to_string(data).map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("JSON serialization error: {}", e),
        )
    })?;
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Response build error: {}", e),
            )
        })
}

pub async fn handle_v1_chat_modes(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<Response<Body>, ScratchError> {
    let dirs = get_project_dirs(gcx.clone()).await;
    let project_root = dirs.first().cloned();

    let config_dir = gcx.read().await.config_dir.clone();
    let _ = global_configs_try_create_all(&config_dir).await;
    if let Some(ref root) = project_root {
        let _ = project_configs_ensure_dirs(root).await;
    }
    let registry = load_merged_registry(&config_dir, project_root.as_deref()).await;

    let mut modes: Vec<ChatModeInfo> = registry
        .modes
        .values()
        .filter(|m| !m.specific)
        .map(|m| ChatModeInfo {
            id: m.id.clone(),
            title: if m.title.is_empty() {
                m.id.clone()
            } else {
                m.title.clone()
            },
            description: m.description.clone(),
            tools_count: m.tools.len(),
            thread_defaults: ChatModeThreadDefaults {
                include_project_info: m.thread_defaults.include_project_info.unwrap_or(true),
                checkpoints_enabled: m.thread_defaults.checkpoints_enabled.unwrap_or(true),
                auto_approve_editing_tools: m
                    .thread_defaults
                    .auto_approve_editing_tools
                    .unwrap_or(false),
                auto_approve_dangerous_commands: m
                    .thread_defaults
                    .auto_approve_dangerous_commands
                    .unwrap_or(false),
            },
            ui: ChatModeUi {
                order: m.ui.order.unwrap_or(100),
                tags: m.ui.tags.clone(),
            },
        })
        .collect();

    modes.sort_by(|a, b| a.ui.order.cmp(&b.ui.order).then_with(|| a.id.cmp(&b.id)));

    let response = ChatModesResponse {
        modes,
        errors: registry
            .errors
            .iter()
            .map(|e| ChatModeError {
                file_path: e.file_path.clone(),
                error: e.error.clone(),
            })
            .collect(),
    };

    json_response(&response)
}
