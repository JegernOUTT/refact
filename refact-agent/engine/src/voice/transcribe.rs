use std::path::Path;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};
use base64::Engine;
use tracing::info;

use crate::voice::types::{TranscribeRequest, TranscribeResult};
use crate::voice::audio_decode::decode_audio;

pub fn load_context(model_path: &Path) -> Result<WhisperContext, String> {
    info!("Loading Whisper model from {:?}", model_path);

    WhisperContext::new_with_params(
        model_path.to_str().ok_or("Invalid model path")?,
        WhisperContextParameters::default(),
    )
    .map_err(|e| format!("Failed to load model: {:?}", e))
}

pub fn transcribe(
    ctx: &WhisperContext,
    request: &TranscribeRequest,
) -> Result<TranscribeResult, String> {
    let audio_bytes = decode_base64(&request.audio_data)?;
    let pcm = decode_audio(&audio_bytes, &request.mime_type)?;
    let text = transcribe_pcm(ctx, &pcm, request.language.as_deref())?;
    let duration_ms = (pcm.len() as f64 / 16.0) as u64;

    Ok(TranscribeResult {
        text,
        language: request.language.clone().unwrap_or_else(|| "en".to_string()),
        duration_ms,
    })
}

pub fn transcribe_pcm(
    ctx: &WhisperContext,
    pcm: &[f32],
    language: Option<&str>,
) -> Result<String, String> {
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_language(Some(language.unwrap_or("en")));

    let mut state = ctx
        .create_state()
        .map_err(|e| format!("Failed to create state: {:?}", e))?;

    state
        .full(params, pcm)
        .map_err(|e| format!("Transcription failed: {:?}", e))?;

    let num_segments = state
        .full_n_segments()
        .map_err(|e| format!("Failed to get segments: {:?}", e))?;

    let mut text = String::new();
    for i in 0..num_segments {
        if let Ok(segment) = state.full_get_segment_text(i) {
            text.push_str(&segment);
        }
    }

    Ok(text.trim().to_string())
}

fn decode_base64(data: &str) -> Result<Vec<u8>, String> {
    let b64_data = if data.starts_with("data:") {
        data.splitn(2, ',').nth(1).ok_or("Invalid data URL")?
    } else {
        data
    };

    base64::engine::general_purpose::STANDARD
        .decode(b64_data)
        .map_err(|e| format!("Invalid base64: {}", e))
}
