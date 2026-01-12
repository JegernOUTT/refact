use std::path::PathBuf;
use std::io::Write;
use tracing::info;
use futures::StreamExt;

#[derive(Debug, Clone, Copy)]
pub enum WhisperModel {
    TinyEn,
    Tiny,
    BaseEn,
    Base,
    SmallEn,
    Small,
    MediumEn,
    Medium,
    LargeV3,
}

impl WhisperModel {
    pub fn from_name(name: &str) -> Result<Self, String> {
        match name {
            "tiny.en" => Ok(Self::TinyEn),
            "tiny" => Ok(Self::Tiny),
            "base.en" => Ok(Self::BaseEn),
            "base" => Ok(Self::Base),
            "small.en" => Ok(Self::SmallEn),
            "small" => Ok(Self::Small),
            "medium.en" => Ok(Self::MediumEn),
            "medium" => Ok(Self::Medium),
            "large-v3" => Ok(Self::LargeV3),
            _ => Err(format!("Unknown model: {}. Use: tiny.en, base.en, small.en, medium.en, large-v3", name)),
        }
    }

    pub fn filename(&self) -> &'static str {
        match self {
            Self::TinyEn => "ggml-tiny.en.bin",
            Self::Tiny => "ggml-tiny.bin",
            Self::BaseEn => "ggml-base.en.bin",
            Self::Base => "ggml-base.bin",
            Self::SmallEn => "ggml-small.en.bin",
            Self::Small => "ggml-small.bin",
            Self::MediumEn => "ggml-medium.en.bin",
            Self::Medium => "ggml-medium.bin",
            Self::LargeV3 => "ggml-large-v3.bin",
        }
    }

    pub fn download_url(&self) -> String {
        format!(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
            self.filename()
        )
    }

    pub fn size_mb(&self) -> u64 {
        match self {
            Self::TinyEn | Self::Tiny => 39,
            Self::BaseEn | Self::Base => 142,
            Self::SmallEn | Self::Small => 466,
            Self::MediumEn | Self::Medium => 1500,
            Self::LargeV3 => 3100,
        }
    }
}

pub fn models_cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("refact")
        .join("voice-models")
}

pub fn model_exists(model: WhisperModel) -> Option<PathBuf> {
    let path = models_cache_dir().join(model.filename());
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

pub async fn download_model(
    model: WhisperModel,
    on_progress: impl Fn(u8),
) -> Result<PathBuf, String> {
    let cache_dir = models_cache_dir();
    std::fs::create_dir_all(&cache_dir)
        .map_err(|e| format!("Failed to create cache dir: {}", e))?;

    let dest_path = cache_dir.join(model.filename());

    if dest_path.exists() {
        info!("Model {} already exists", model.filename());
        return Ok(dest_path);
    }

    info!("Downloading {} ({} MB)...", model.filename(), model.size_mb());

    let client = reqwest::Client::new();
    let response = client
        .get(model.download_url())
        .send()
        .await
        .map_err(|e| format!("Download failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Download failed with status: {}", response.status()));
    }

    let total_size = response.content_length().unwrap_or(0);
    let mut downloaded: u64 = 0;

    let temp_path = dest_path.with_extension("bin.tmp");
    let mut file = std::fs::File::create(&temp_path)
        .map_err(|e| format!("Failed to create file: {}", e))?;

    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Stream error: {}", e))?;
        file.write_all(&chunk)
            .map_err(|e| format!("Write error: {}", e))?;

        downloaded += chunk.len() as u64;
        if total_size > 0 {
            let progress = ((downloaded * 100) / total_size) as u8;
            on_progress(progress);
        }
    }

    std::fs::rename(&temp_path, &dest_path)
        .map_err(|e| format!("Failed to finalize download: {}", e))?;

    info!("Downloaded {} successfully", model.filename());
    Ok(dest_path)
}
