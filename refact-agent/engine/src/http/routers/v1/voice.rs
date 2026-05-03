use std::collections::HashMap;
use std::sync::Arc;
use axum::extract::Path;
use axum::Extension;
use axum::response::Response;
use base64::Engine;
use hyper::{Body, StatusCode};
use tokio::sync::{broadcast, RwLock as ARwLock};

use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
#[cfg(feature = "voice")]
use crate::voice::models::WhisperModel;
use crate::voice::types::*;

pub async fn handle_v1_voice_transcribe(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let req: TranscribeRequest = serde_json::from_slice(&body)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("Invalid JSON: {}", e)))?;

    let gcx_locked = gcx.read().await;
    let voice_service = gcx_locked.voice_service.clone();
    drop(gcx_locked);

    let result = voice_service
        .transcribe(req)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    if result.duration_ms >= 1000 && result.text.len() >= 5 {
        let duration_secs = result.duration_ms / 1000;
        crate::buddy::actor::buddy_apply(
            gcx.clone(),
            crate::buddy::actor::BuddyMutation {
                xp: 2,
                activity: Some(crate::buddy::types::BuddyActivity {
                    icon: "🎤".to_string(),
                    title: "Voice input transcribed".to_string(),
                    description: format!(
                        "{}s of audio → {} chars",
                        duration_secs,
                        result.text.len()
                    ),
                    timestamp: chrono::Utc::now().to_rfc3339(),
                    activity_type: "voice_transcribed".to_string(),
                    chat_id: None,
                }),
                ..Default::default()
            },
        )
        .await;
    }

    let response = TranscribeResponse {
        text: result.text,
        language: result.language,
        duration_ms: result.duration_ms,
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(&response).unwrap()))
        .unwrap())
}

pub async fn handle_v1_voice_download(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let req: DownloadModelRequest = serde_json::from_slice(&body).unwrap_or(DownloadModelRequest {
        model: "base.en".to_string(),
    });

    #[cfg(not(feature = "voice"))]
    {
        let _ = gcx;
        Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!(
                "Voice feature not enabled. Cannot download model: {}",
                req.model
            ),
        ))
    }

    #[cfg(feature = "voice")]
    {
        WhisperModel::from_name(&req.model)
            .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, e))?;

        let gcx_locked = gcx.read().await;
        let voice_service = gcx_locked.voice_service.clone();
        drop(gcx_locked);

        let voice_service_clone = voice_service.clone();
        let model_name = req.model.clone();
        tokio::spawn(async move {
            let _ = voice_service_clone.download_model(&model_name).await;
        });

        let response = DownloadModelResponse {
            success: true,
            message: format!("Download started for model: {}", req.model),
        };

        Ok(Response::builder()
            .status(StatusCode::ACCEPTED)
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_string(&response).unwrap()))
            .unwrap())
    }
}

pub async fn handle_v1_voice_status(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<Response<Body>, ScratchError> {
    let gcx_locked = gcx.read().await;
    let voice_service = gcx_locked.voice_service.clone();
    drop(gcx_locked);

    let response = VoiceStatusResponse {
        enabled: crate::voice::VoiceService::is_enabled(),
        model_loaded: voice_service.is_model_loaded().await,
        model_name: voice_service.model_name().await,
        is_downloading: voice_service.is_downloading(),
        download_progress: voice_service.download_progress(),
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(&response).unwrap()))
        .unwrap())
}

pub async fn handle_v1_voice_stream_subscribe(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(session_id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Result<Response<Body>, ScratchError> {
    let language = params.get("language").cloned();

    let gcx_locked = gcx.read().await;
    let voice_service = gcx_locked.voice_service.clone();
    drop(gcx_locked);

    let session_arc = voice_service
        .get_or_create_session(&session_id, language)
        .await;
    let session = session_arc.lock().await;
    let mut rx = session.subscribe();
    drop(session);

    let stream = async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let json = serde_json::to_string(&event).unwrap_or_default();
                    yield Ok::<_, std::convert::Infallible>(format!("data: {}\n\n", json));
                    if matches!(event, VoiceStreamEvent::Ended) {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
            }
        }
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .body(Body::wrap_stream(stream))
        .unwrap())
}

pub async fn handle_v1_voice_stream_chunk(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(session_id): Path<String>,
    body: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let req: StreamingChunkRequest = serde_json::from_slice(&body)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("Invalid JSON: {}", e)))?;

    let gcx_locked = gcx.read().await;
    let voice_service = gcx_locked.voice_service.clone();
    drop(gcx_locked);

    let session_arc = voice_service
        .get_or_create_session(&session_id, req.language.clone())
        .await;
    let mut session = session_arc.lock().await;

    if !req.audio_data.is_empty() {
        let audio_bytes = decode_base64_audio(&req.audio_data)?;
        let samples = decode_pcm_s16le(&audio_bytes);
        session.audio_buffer.extend(&samples);
    }

    if req.is_final {
        session.final_requested = true;
    }

    session.notify_update();

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"status":"ok"}"#))
        .unwrap())
}

fn decode_base64_audio(data: &str) -> Result<Vec<u8>, ScratchError> {
    let b64_data = if data.starts_with("data:") {
        data.splitn(2, ',').nth(1).unwrap_or(data)
    } else {
        data
    };
    base64::engine::general_purpose::STANDARD
        .decode(b64_data)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("Invalid base64: {}", e)))
}

fn decode_pcm_s16le(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(2)
        .map(|chunk| {
            let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
            sample as f32 / 32768.0
        })
        .collect()
}
