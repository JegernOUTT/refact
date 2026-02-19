pub mod models;
pub mod types;

#[cfg(feature = "voice")]
pub mod audio_decode;
#[cfg(feature = "voice")]
pub mod transcribe;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Duration;
use tokio::sync::{broadcast, watch, RwLock as ARwLock, Mutex as AMutex, mpsc, oneshot};
use tracing::info;

use crate::voice::types::{
    TranscribeRequest, TranscribeResult, VoiceStreamEvent, StreamingTranscriptEvent,
};
use crate::voice::models::WhisperModel;

const DEBOUNCE_MS: u64 = 300;
const LIVE_WINDOW_SAMPLES: usize = 16000 * 20;

pub struct StreamingSession {
    pub audio_buffer: Vec<f32>,
    pub language: Option<String>,
    pub final_requested: bool,
    pub event_tx: broadcast::Sender<VoiceStreamEvent>,
    pub update_tx: watch::Sender<u64>,
    pub update_seq: u64,
}

impl StreamingSession {
    pub fn new(language: Option<String>) -> Self {
        let (event_tx, _) = broadcast::channel(64);
        let (update_tx, _) = watch::channel(0u64);
        Self {
            audio_buffer: Vec::new(),
            language,
            final_requested: false,
            event_tx,
            update_tx,
            update_seq: 0,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<VoiceStreamEvent> {
        self.event_tx.subscribe()
    }

    pub fn subscribe_updates(&self) -> watch::Receiver<u64> {
        self.update_tx.subscribe()
    }

    pub fn emit(&self, event: VoiceStreamEvent) {
        let _ = self.event_tx.send(event);
    }

    pub fn notify_update(&mut self) {
        self.update_seq = self.update_seq.wrapping_add(1);
        let _ = self.update_tx.send(self.update_seq);
    }
}

pub struct VoiceService {
    #[cfg(feature = "voice")]
    ctx: ARwLock<Option<whisper_rs::WhisperContext>>,
    model_name: ARwLock<String>,
    is_downloading: AtomicBool,
    download_progress: AtomicU8,
    queue_tx: mpsc::Sender<QueuedTranscription>,
    streaming_sessions: ARwLock<HashMap<String, Arc<AMutex<StreamingSession>>>>,
}

struct QueuedTranscription {
    request: TranscribeRequest,
    response_tx: oneshot::Sender<Result<TranscribeResult, String>>,
}

impl VoiceService {
    pub fn new() -> Arc<Self> {
        let (queue_tx, queue_rx) = mpsc::channel::<QueuedTranscription>(100);

        let service = Arc::new(Self {
            #[cfg(feature = "voice")]
            ctx: ARwLock::new(None),
            model_name: ARwLock::new("base.en".to_string()),
            is_downloading: AtomicBool::new(false),
            download_progress: AtomicU8::new(0),
            queue_tx,
            streaming_sessions: ARwLock::new(HashMap::new()),
        });

        let service_clone = service.clone();
        tokio::spawn(async move {
            service_clone.process_queue(queue_rx).await;
        });

        service
    }

    pub async fn get_or_create_session(
        self: &Arc<Self>,
        session_id: &str,
        language: Option<String>,
    ) -> Arc<AMutex<StreamingSession>> {
        let mut sessions = self.streaming_sessions.write().await;
        if let Some(session) = sessions.get(session_id) {
            return session.clone();
        }

        let session = Arc::new(AMutex::new(StreamingSession::new(language)));
        sessions.insert(session_id.to_string(), session.clone());

        let session_for_worker = session.clone();
        let service_for_worker = self.clone();
        let session_id_for_worker = session_id.to_string();

        tokio::spawn(async move {
            service_for_worker
                .session_worker(session_id_for_worker, session_for_worker)
                .await;
        });

        session
    }

    pub async fn remove_session(&self, session_id: &str) {
        self.streaming_sessions.write().await.remove(session_id);
    }

    async fn session_worker(
        self: Arc<Self>,
        session_id: String,
        session_arc: Arc<AMutex<StreamingSession>>,
    ) {
        let mut update_rx = {
            let session = session_arc.lock().await;
            session.subscribe_updates()
        };

        loop {
            if update_rx.changed().await.is_err() {
                break;
            }

            tokio::time::sleep(Duration::from_millis(DEBOUNCE_MS)).await;

            let (buffer_snapshot, language, is_final, duration_ms) = {
                let session = session_arc.lock().await;
                let is_final = session.final_requested;
                let duration_ms = (session.audio_buffer.len() as f64 / 16.0) as u64;

                let buffer = if session.audio_buffer.is_empty() {
                    Vec::new()
                } else if is_final {
                    session.audio_buffer.clone()
                } else {
                    let start = session
                        .audio_buffer
                        .len()
                        .saturating_sub(LIVE_WINDOW_SAMPLES);
                    session.audio_buffer[start..].to_vec()
                };

                (buffer, session.language.clone(), is_final, duration_ms)
            };

            if !buffer_snapshot.is_empty() {
                match self
                    .transcribe_buffer(&buffer_snapshot, language.as_deref())
                    .await
                {
                    Ok(text) => {
                        let clean_text = text.replace("[BLANK_AUDIO]", "").trim().to_string();
                        let session = session_arc.lock().await;
                        if !clean_text.is_empty() || is_final {
                            session.emit(VoiceStreamEvent::Transcript(StreamingTranscriptEvent {
                                session_id: session_id.clone(),
                                text: clean_text,
                                is_final,
                                duration_ms,
                            }));
                        }
                    }
                    Err(e) => {
                        if is_final {
                            let session = session_arc.lock().await;
                            session.emit(VoiceStreamEvent::Error { message: e });
                        }
                    }
                }
            } else if is_final {
                let session = session_arc.lock().await;
                session.emit(VoiceStreamEvent::Transcript(StreamingTranscriptEvent {
                    session_id: session_id.clone(),
                    text: String::new(),
                    is_final: true,
                    duration_ms,
                }));
            }

            if is_final {
                let session = session_arc.lock().await;
                session.emit(VoiceStreamEvent::Ended);
                drop(session);
                self.remove_session(&session_id).await;
                break;
            }
        }
    }

    #[cfg(feature = "voice")]
    pub async fn transcribe_buffer(
        &self,
        samples: &[f32],
        language: Option<&str>,
    ) -> Result<String, String> {
        self.ensure_model_loaded().await?;
        let ctx_guard = self.ctx.read().await;
        let ctx = ctx_guard.as_ref().ok_or("Model not loaded")?;
        transcribe::transcribe_pcm(ctx, samples, language)
    }

    #[cfg(feature = "voice")]
    async fn ensure_model_loaded(&self) -> Result<(), String> {
        if self.ctx.read().await.is_some() {
            return Ok(());
        }

        let mut ctx_guard = self.ctx.write().await;
        if ctx_guard.is_some() {
            return Ok(());
        }

        let model_name = self.model_name.read().await.clone();
        let whisper_model = WhisperModel::from_name(&model_name)?;

        if let Some(path) = models::model_exists(whisper_model) {
            info!("Loading model from {:?}", path);
            let ctx = transcribe::load_context(&path)?;
            *ctx_guard = Some(ctx);
            Ok(())
        } else {
            drop(ctx_guard);
            self.download_model(&model_name).await
        }
    }

    #[cfg(not(feature = "voice"))]
    pub async fn transcribe_buffer(
        &self,
        _samples: &[f32],
        _language: Option<&str>,
    ) -> Result<String, String> {
        Err("Voice feature not enabled".to_string())
    }

    async fn process_queue(self: Arc<Self>, mut rx: mpsc::Receiver<QueuedTranscription>) {
        while let Some(item) = rx.recv().await {
            let result = self.do_transcribe(item.request).await;
            let _ = item.response_tx.send(result);
        }
    }

    pub async fn transcribe(&self, request: TranscribeRequest) -> Result<TranscribeResult, String> {
        let (response_tx, response_rx) = oneshot::channel();

        self.queue_tx
            .send(QueuedTranscription {
                request,
                response_tx,
            })
            .await
            .map_err(|_| "Voice service queue full".to_string())?;

        response_rx
            .await
            .map_err(|_| "Transcription cancelled".to_string())?
    }

    #[cfg(feature = "voice")]
    async fn do_transcribe(&self, request: TranscribeRequest) -> Result<TranscribeResult, String> {
        let mut ctx_guard = self.ctx.write().await;

        if ctx_guard.is_none() {
            let model_name = self.model_name.read().await.clone();
            let whisper_model = WhisperModel::from_name(&model_name)?;

            if let Some(path) = models::model_exists(whisper_model) {
                info!("Loading model from {:?}", path);
                let ctx = transcribe::load_context(&path)?;
                *ctx_guard = Some(ctx);
            } else {
                drop(ctx_guard);
                self.download_model(&model_name).await?;
                ctx_guard = self.ctx.write().await;
            }
        }

        let ctx = ctx_guard.as_ref().ok_or("Model not loaded")?;
        transcribe::transcribe(ctx, &request)
    }

    #[cfg(not(feature = "voice"))]
    async fn do_transcribe(&self, _request: TranscribeRequest) -> Result<TranscribeResult, String> {
        Err("Voice feature not enabled. Rebuild with --features voice".to_string())
    }

    #[cfg(feature = "voice")]
    pub async fn download_model(&self, model_name: &str) -> Result<(), String> {
        if self.is_downloading.load(Ordering::SeqCst) {
            return Err("Already downloading".to_string());
        }

        self.is_downloading.store(true, Ordering::SeqCst);
        self.download_progress.store(0, Ordering::SeqCst);

        let whisper_model = WhisperModel::from_name(model_name)?;

        let progress_ref = &self.download_progress;
        let result = models::download_model(whisper_model, |p| {
            progress_ref.store(p, Ordering::SeqCst);
        })
        .await;

        self.is_downloading.store(false, Ordering::SeqCst);

        match result {
            Ok(path) => {
                info!("Model downloaded to {:?}", path);
                let ctx = transcribe::load_context(&path)?;
                *self.ctx.write().await = Some(ctx);
                *self.model_name.write().await = model_name.to_string();
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    #[cfg(not(feature = "voice"))]
    pub async fn download_model(&self, _model_name: &str) -> Result<(), String> {
        Err("Voice feature not enabled".to_string())
    }

    pub fn is_downloading(&self) -> bool {
        self.is_downloading.load(Ordering::SeqCst)
    }

    pub fn download_progress(&self) -> u8 {
        self.download_progress.load(Ordering::SeqCst)
    }

    pub async fn model_name(&self) -> String {
        self.model_name.read().await.clone()
    }

    #[cfg(feature = "voice")]
    pub async fn is_model_loaded(&self) -> bool {
        self.ctx.read().await.is_some()
    }

    #[cfg(not(feature = "voice"))]
    pub async fn is_model_loaded(&self) -> bool {
        false
    }

    pub fn is_enabled() -> bool {
        cfg!(feature = "voice")
    }
}

pub type SharedVoiceService = Arc<VoiceService>;
