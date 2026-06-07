use std::sync::Arc;
use tokio::sync::Mutex as AMutex;

use axum::extract::State;
use axum::response::Result;
use hyper::{Body, Response, StatusCode};
use tracing::info;
use crate::call_validation::{CodeCompletionPost, code_completion_post_validate};
use crate::caps::resolve_completion_model;
use crate::completion_cache;
use crate::app_state::AppState;
use crate::custom_error::ScratchError;
use crate::privacy::{check_file_privacy, load_privacy_if_needed};
use crate::files_correction::canonical_path;
use crate::scratchpads;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::scratchpad_abstract::ScratchpadPromptInput;

const CODE_COMPLETION_TOP_N: usize = 5;

fn normalize_code_completion_post(
    code_completion_post: &mut CodeCompletionPost,
    model_rec: &crate::caps::CompletionModelRecord,
) -> Result<(), ScratchError> {
    if code_completion_post.use_vecdb {
        return Err(ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Code completion use_vecdb is not supported yet; use use_ast for completion RAG"
                .to_string(),
        ));
    }
    if code_completion_post.parameters.max_new_tokens == 0 {
        code_completion_post.parameters.max_new_tokens = 50;
    }
    code_completion_post.model = model_rec.base.id.clone();
    code_completion_post.parameters.temperature =
        Some(code_completion_post.parameters.temperature.unwrap_or(0.2));
    Ok(())
}

pub async fn handle_v1_code_completion(
    app: AppState,
    code_completion_post: &mut CodeCompletionPost,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    code_completion_post_validate(code_completion_post)?;

    let cpath = canonical_path(&code_completion_post.inputs.cursor.file);
    check_file_privacy(
        load_privacy_if_needed(gcx.clone()).await,
        &cpath,
        &crate::privacy::FilePrivacyLevel::OnlySendToServersIControl,
    )
    .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, e))?;

    let caps = crate::global_context::try_load_caps_quickly_if_not_present(gcx.clone(), 0).await?;
    let model_rec = resolve_completion_model(caps, &code_completion_post.model)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
    normalize_code_completion_post(code_completion_post, &model_rec)?;
    info!(
        "chosen completion model: {}, scratchpad: {}",
        code_completion_post.model, model_rec.scratchpad
    );
    let cache_arc = { gcx.completions_cache.clone() };
    if !code_completion_post.no_cache {
        let cache_key = completion_cache::cache_key_from_post(&code_completion_post);
        let cached_maybe = completion_cache::cache_get(cache_arc.clone(), cache_key.clone());
        if let Some(cached_json_value) = cached_maybe {
            // info!("cache hit for key {:?}", cache_key.clone());
            if !code_completion_post.stream {
                return crate::restream::cached_not_stream(&cached_json_value).await;
            } else {
                return crate::restream::cached_stream(&cached_json_value).await;
            }
        }
    }

    let ast_service_opt = gcx.ast_service.lock().unwrap().clone();
    let mut scratchpad = scratchpads::create_code_completion_scratchpad(
        gcx.clone(),
        &model_rec,
        &code_completion_post.clone(),
        cache_arc.clone(),
        ast_service_opt,
    )
    .await
    .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, e))?;
    let ccx = Arc::new(AMutex::new(
        AtCommandsContext::new_from_app(
            app,
            model_rec.base.n_ctx,
            CODE_COMPLETION_TOP_N,
            true,
            vec![],
            "".to_string(),
            None,
            model_rec.base.id.clone(),
            None,
            None,
        )
        .await,
    ));
    if !code_completion_post.stream {
        crate::restream::scratchpad_interaction_not_stream(
            ccx.clone(),
            &mut scratchpad,
            "completion".to_string(),
            &model_rec.base,
            &mut code_completion_post.parameters,
            false,
        )
        .await
    } else {
        crate::restream::scratchpad_interaction_stream(
            ccx.clone(),
            scratchpad,
            "completion-stream".to_string(),
            model_rec.base.clone(),
            code_completion_post.parameters.clone(),
            false,
            None,
        )
        .await
    }
}

pub async fn handle_v1_code_completion_web(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let mut code_completion_post = serde_json::from_slice::<CodeCompletionPost>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e)))?;
    handle_v1_code_completion(app, &mut code_completion_post).await
}

pub async fn handle_v1_code_completion_prompt(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    // Almost the same function, but only returns the prompt (good for generating data)
    let mut post = serde_json::from_slice::<CodeCompletionPost>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e)))?;
    code_completion_post_validate(&post)?;

    let cpath = canonical_path(&post.inputs.cursor.file);
    check_file_privacy(
        load_privacy_if_needed(gcx.clone()).await,
        &cpath,
        &crate::privacy::FilePrivacyLevel::OnlySendToServersIControl,
    )
    .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, e))?;

    let caps = crate::global_context::try_load_caps_quickly_if_not_present(gcx.clone(), 0).await?;
    let model_rec = resolve_completion_model(caps, &post.model)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
    normalize_code_completion_post(&mut post, &model_rec)?;

    // don't need cache, but go along
    let cache_arc = { gcx.completions_cache.clone() };

    let ast_service_opt = gcx.ast_service.lock().unwrap().clone();
    let mut scratchpad = scratchpads::create_code_completion_scratchpad(
        gcx.clone(),
        &model_rec,
        &post,
        cache_arc.clone(),
        ast_service_opt,
    )
    .await
    .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, e))?;

    let ccx = Arc::new(AMutex::new(
        AtCommandsContext::new_from_app(
            app,
            model_rec.base.n_ctx,
            CODE_COMPLETION_TOP_N,
            true,
            vec![],
            "".to_string(),
            None,
            model_rec.base.id.clone(),
            None,
            None,
        )
        .await,
    ));
    let prompt_input = {
        let cgcx = ccx.lock().await;
        ScratchpadPromptInput {
            n_ctx: cgcx.n_ctx,
            postprocess_parameters: cgcx.postprocess_parameters.clone(),
        }
    };
    let prompt = scratchpad
        .prompt(prompt_input, &mut post.parameters)
        .await
        .map_err(|e| {
            ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Prompt: {}", e))
        })?;

    let body = serde_json::json!({"prompt": prompt}).to_string();
    let response = Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .unwrap();
    return Ok(response);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post(model: &str) -> CodeCompletionPost {
        CodeCompletionPost {
            inputs: crate::call_validation::CodeCompletionInputs::default(),
            parameters: crate::call_validation::SamplingParameters::default(),
            model: model.to_string(),
            stream: false,
            no_cache: false,
            use_ast: false,
            use_vecdb: false,
            rag_tokens_n: 0,
        }
    }

    fn model_rec() -> crate::caps::CompletionModelRecord {
        let mut record = crate::caps::CompletionModelRecord::default();
        record.base.id = "provider/model".to_string();
        record
    }

    #[test]
    fn code_completion_prompt_uses_real_completion_normalization() {
        let mut post = post("");
        normalize_code_completion_post(&mut post, &model_rec()).unwrap();

        assert_eq!(post.model, "provider/model");
        assert_eq!(post.parameters.max_new_tokens, 50);
        assert_eq!(post.parameters.temperature, Some(0.2));
    }

    #[test]
    fn code_completion_use_vecdb_is_explicitly_rejected() {
        let mut post = post("");
        post.use_vecdb = true;
        let err = normalize_code_completion_post(&mut post, &model_rec()).unwrap_err();

        assert_eq!(err.status_code, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(err.message.contains("use_vecdb"));
    }
}
