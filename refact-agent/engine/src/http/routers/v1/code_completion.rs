use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::Mutex as AMutex;

use axum::extract::State;
use axum::response::Result;
use hyper::{Body, Response, StatusCode};
use refact_core::llm_types::BaseModelRecord;
use sha2::{Digest, Sha256};
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

fn safe_hash(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn completion_cache_salt(
    model: &BaseModelRecord,
    scratchpad: &str,
    scratchpad_patch: &serde_json::Value,
) -> String {
    let mut headers: Vec<_> = model.extra_headers.iter().collect();
    headers.sort_by(|(a, _), (b, _)| a.cmp(b));
    let header_hashes: Vec<_> = headers
        .into_iter()
        .map(|(key, value)| serde_json::json!({"key": key, "value_sha256": safe_hash(value)}))
        .collect();
    serde_json::json!({
        "model_id": model.id,
        "upstream_model": model.name,
        "endpoint": model.endpoint,
        "endpoint_style": model.endpoint_style,
        "completion_endpoint_style": model.completion_endpoint_style,
        "wire_format": model.wire_format,
        "api_key_sha256": safe_hash(&model.api_key),
        "auth_token_sha256": safe_hash(&model.auth_token),
        "tokenizer_api_key_sha256": safe_hash(&model.tokenizer_api_key),
        "extra_headers": header_hashes,
        "scratchpad": scratchpad,
        "scratchpad_patch": scratchpad_patch,
    })
    .to_string()
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
    code_completion_post.cache_generation = gcx.completion_cache_generation.load(Ordering::Relaxed);
    code_completion_post.cache_salt = completion_cache_salt(
        &model_rec.base,
        &model_rec.scratchpad,
        &model_rec.scratchpad_patch,
    );
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
    use refact_core::llm_types::{BaseModelRecord, WireFormat};
    use std::collections::HashMap;

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
            cache_salt: String::new(),
            cache_generation: 0,
        }
    }

    fn model_rec() -> crate::caps::CompletionModelRecord {
        let mut record = crate::caps::CompletionModelRecord::default();
        record.base.id = "provider/model".to_string();
        record
    }

    fn test_model(
        endpoint: &str,
        api_key: &str,
        headers: HashMap<String, String>,
    ) -> BaseModelRecord {
        BaseModelRecord {
            id: "custom/qwen".to_string(),
            name: "qwen".to_string(),
            endpoint: endpoint.to_string(),
            endpoint_style: "openai".to_string(),
            completion_endpoint_style: "openai_completions".to_string(),
            wire_format: WireFormat::OpenaiChatCompletions,
            api_key: api_key.to_string(),
            extra_headers: headers,
            ..Default::default()
        }
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

    #[test]
    fn completion_cache_salt_changes_with_endpoint_and_auth() {
        let first = completion_cache_salt(
            &test_model(
                "https://one.example/v1/completions",
                "sk-one",
                HashMap::new(),
            ),
            "FIM-PSM",
            &serde_json::json!({"context_format": "chat"}),
        );
        let second = completion_cache_salt(
            &test_model(
                "https://two.example/v1/completions",
                "sk-one",
                HashMap::new(),
            ),
            "FIM-PSM",
            &serde_json::json!({"context_format": "chat"}),
        );
        let third = completion_cache_salt(
            &test_model(
                "https://one.example/v1/completions",
                "sk-two",
                HashMap::new(),
            ),
            "FIM-PSM",
            &serde_json::json!({"context_format": "chat"}),
        );

        assert_ne!(first, second);
        assert_ne!(first, third);
        assert!(!third.contains("sk-two"));
    }

    #[test]
    fn completion_cache_salt_hashes_extra_header_values() {
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            "Bearer secret-token".to_string(),
        );
        let salt = completion_cache_salt(
            &test_model("https://one.example/v1/completions", "", headers),
            "FIM-PSM",
            &serde_json::json!({"context_format": "chat"}),
        );

        assert!(salt.contains("Authorization"));
        assert!(!salt.contains("secret-token"));
        assert!(!salt.contains("Bearer secret-token"));
    }

    #[test]
    fn completion_cache_salt_changes_with_scratchpad_patch() {
        let model = test_model("https://one.example/v1/completions", "", HashMap::new());
        let first = completion_cache_salt(
            &model,
            "FIM-PSM",
            &serde_json::json!({"context_format": "chat"}),
        );
        let second = completion_cache_salt(
            &model,
            "FIM-PSM",
            &serde_json::json!({"context_format": "plain"}),
        );

        assert_ne!(first, second);
    }
}
