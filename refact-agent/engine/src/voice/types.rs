use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct TranscribeRequest {
    pub audio_data: String,
    #[serde(default = "default_mime")]
    pub mime_type: String,
    pub language: Option<String>,
}

fn default_mime() -> String {
    "audio/webm".to_string()
}

#[derive(Debug, Deserialize)]
pub struct StreamingChunkRequest {
    pub audio_data: String,
    #[serde(default)]
    pub is_final: bool,
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StreamingTranscriptEvent {
    pub session_id: String,
    pub text: String,
    pub is_final: bool,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum VoiceStreamEvent {
    #[serde(rename = "transcript")]
    Transcript(StreamingTranscriptEvent),
    #[serde(rename = "error")]
    Error { message: String },
    #[serde(rename = "ended")]
    Ended,
}

#[derive(Debug, Serialize)]
pub struct TranscribeResponse {
    pub text: String,
    pub language: String,
    pub duration_ms: u64,
}

#[derive(Debug)]
pub struct TranscribeResult {
    pub text: String,
    pub language: String,
    pub duration_ms: u64,
}

#[derive(Debug, Deserialize)]
pub struct DownloadModelRequest {
    #[serde(default = "default_model")]
    pub model: String,
}

fn default_model() -> String {
    "base.en".to_string()
}

#[derive(Debug, Serialize)]
pub struct VoiceStatusResponse {
    pub enabled: bool,
    pub model_loaded: bool,
    pub model_name: String,
    pub is_downloading: bool,
    pub download_progress: u8,
}

#[derive(Debug, Serialize)]
pub struct DownloadModelResponse {
    pub success: bool,
    pub message: String,
}
