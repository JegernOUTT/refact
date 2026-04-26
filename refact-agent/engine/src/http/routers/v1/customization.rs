use axum::response::Result;
use axum::Extension;
use hyper::{Body, Response, StatusCode};
use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::RwLock as ARwLock;
use serde::Serialize;

use crate::global_context::GlobalContext;
use crate::custom_error::ScratchError;
use crate::yaml_configs::customization_registry::get_project_registry;

#[derive(Serialize)]
struct SystemPromptCompat {
    text: String,
    description: String,
    show: String,
}

#[derive(Serialize)]
struct CustomizationCompat {
    system_prompts: HashMap<String, SystemPromptCompat>,
    toolbox_commands: HashMap<String, serde_json::Value>,
    code_lens: HashMap<String, serde_json::Value>,
    error_log: Vec<String>,
}

pub async fn handle_v1_config_path(
    Extension(global_context): Extension<Arc<ARwLock<GlobalContext>>>,
    _body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let config_dir = global_context.read().await.config_dir.clone();
    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(config_dir.to_string_lossy().to_string()))
        .unwrap())
}

pub async fn handle_v1_customization(
    Extension(global_context): Extension<Arc<ARwLock<GlobalContext>>>,
    _body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let registry = get_project_registry(global_context.clone()).await;

    let mut system_prompts: HashMap<String, SystemPromptCompat> = HashMap::new();
    let mut toolbox_commands: HashMap<String, serde_json::Value> = HashMap::new();
    let mut code_lens: HashMap<String, serde_json::Value> = HashMap::new();
    let mut error_log: Vec<String> = Vec::new();

    if let Some(reg) = registry {
        for (id, mode) in &reg.modes {
            system_prompts.insert(
                id.clone(),
                SystemPromptCompat {
                    text: mode.prompt.clone(),
                    description: mode.description.clone(),
                    show: if mode.specific {
                        "never".to_string()
                    } else {
                        "always".to_string()
                    },
                },
            );
        }

        for (id, cmd) in &reg.toolbox_commands {
            toolbox_commands.insert(id.clone(), serde_json::to_value(cmd).unwrap_or_default());
        }

        for (id, lens) in &reg.code_lens {
            code_lens.insert(id.clone(), serde_json::to_value(lens).unwrap_or_default());
        }

        error_log = reg
            .errors
            .iter()
            .map(|e| format!("{}: {}", e.file_path, e.error))
            .collect();
    }

    let response = CustomizationCompat {
        system_prompts,
        toolbox_commands,
        code_lens,
        error_log,
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(serde_json::to_string_pretty(&response).unwrap()))
        .unwrap())
}
