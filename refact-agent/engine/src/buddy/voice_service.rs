#![allow(dead_code)]

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
#[cfg(test)]
use std::sync::Mutex as StdMutex;
#[cfg(test)]
use std::sync::OnceLock;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
#[cfg(test)]
use tokio::sync::OwnedMutexGuard;
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};
use tracing::debug;
use uuid::Uuid;

use crate::buddy::types::{BuddyPersonalityProfile, BuddySpeechItem};
use crate::call_validation::{ChatContent, ChatMessage, ChatModelType, SubchatParameters};
use crate::global_context::GlobalContext;

const VOICE_TTL: Duration = Duration::from_secs(5 * 60);
const VOICE_TIMEOUT: Duration = Duration::from_secs(8);
const VOICE_MAX_CHARS: usize = 80;
const VOICE_CACHE_MAX_ITEMS: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpeechIntent {
    Humor,
    Suggestion,
    Insight,
    Win,
    ErrorAlert,
    Greeting,
    Tour,
    Milestone,
    MemoryPulseCommentary,
    QuestAccept,
    QuestComplete,
}

impl SpeechIntent {
    fn as_str(self) -> &'static str {
        match self {
            SpeechIntent::Humor => "speech:humor",
            SpeechIntent::Suggestion => "speech:suggestion",
            SpeechIntent::Insight => "speech:insight",
            SpeechIntent::Win => "speech:win",
            SpeechIntent::ErrorAlert => "speech:error_alert",
            SpeechIntent::Greeting => "speech:greeting",
            SpeechIntent::Tour => "speech:tour",
            SpeechIntent::Milestone => "speech:milestone",
            SpeechIntent::MemoryPulseCommentary => "speech:memory_pulse_commentary",
            SpeechIntent::QuestAccept => "speech:quest_accept",
            SpeechIntent::QuestComplete => "speech:quest_complete",
        }
    }

    fn mood(self) -> &'static str {
        match self {
            SpeechIntent::ErrorAlert => "concerned",
            SpeechIntent::Win | SpeechIntent::Milestone | SpeechIntent::QuestComplete => "happy",
            SpeechIntent::Humor => "playful",
            SpeechIntent::Tour | SpeechIntent::Greeting | SpeechIntent::QuestAccept => "excited",
            SpeechIntent::Suggestion
            | SpeechIntent::Insight
            | SpeechIntent::MemoryPulseCommentary => "curious",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VoiceIntent {
    AutonomousReportSaved,
    WorkflowStarted,
    WorkflowCompleted,
    WorkflowFailed,
    ChatTitle,
    ActivityTitle,
}

pub struct VoiceCtx<'a> {
    pub persona: &'a BuddyPersonalityProfile,
    pub identity_name: &'a str,
    pub pulse_one_liner: String,
    pub workflow_id: Option<&'a str>,
    pub workflow_summary: Option<&'a str>,
}

pub struct VoiceService {
    cache: Arc<AMutex<HashMap<u64, (String, Instant)>>>,
    ttl: Duration,
    renderer: Arc<dyn VoiceRenderer>,
}

#[derive(Clone)]
struct VoiceRenderRequest {
    intent_kind: String,
    archetype_id: String,
    archetype_label: String,
    vibe: String,
    summary: String,
    prompt: String,
    identity_name: String,
    pulse_one_liner: String,
    workflow_id: Option<String>,
    workflow_summary: Option<String>,
}

#[async_trait]
trait VoiceRenderer: Send + Sync {
    async fn render_voice(
        &self,
        gcx: Arc<ARwLock<GlobalContext>>,
        request: VoiceRenderRequest,
    ) -> Option<String>;
}

struct SubchatVoiceRenderer;

#[cfg(test)]
pub struct TestVoiceRenderer {
    responses: StdMutex<Vec<Option<String>>>,
    calls: AtomicUsize,
    intent_kinds: StdMutex<Vec<String>>,
}

#[cfg(test)]
impl TestVoiceRenderer {
    pub fn new(responses: Vec<Option<String>>) -> Arc<Self> {
        Arc::new(Self {
            responses: StdMutex::new(responses),
            calls: AtomicUsize::new(0),
            intent_kinds: StdMutex::new(Vec::new()),
        })
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub fn intent_kinds(&self) -> Vec<String> {
        self.intent_kinds.lock().unwrap().clone()
    }
}

#[cfg(test)]
#[async_trait]
impl VoiceRenderer for TestVoiceRenderer {
    async fn render_voice(
        &self,
        _gcx: Arc<ARwLock<GlobalContext>>,
        request: VoiceRenderRequest,
    ) -> Option<String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.intent_kinds.lock().unwrap().push(request.intent_kind);
        let mut responses = self.responses.lock().unwrap();
        if responses.is_empty() {
            None
        } else {
            responses.remove(0)
        }
    }
}

#[async_trait]
impl VoiceRenderer for SubchatVoiceRenderer {
    async fn render_voice(
        &self,
        gcx: Arc<ARwLock<GlobalContext>>,
        request: VoiceRenderRequest,
    ) -> Option<String> {
        render_via_subchat(gcx, request).await
    }
}

static VOICE_SERVICE: tokio::sync::OnceCell<Arc<VoiceService>> = tokio::sync::OnceCell::const_new();
#[cfg(test)]
static TEST_VOICE_SERVICE: OnceLock<StdMutex<Option<Arc<VoiceService>>>> = OnceLock::new();
#[cfg(test)]
static TEST_VOICE_SERVICE_LOCK: tokio::sync::OnceCell<Arc<AMutex<()>>> =
    tokio::sync::OnceCell::const_new();

#[cfg(test)]
pub struct VoiceServiceTestGuard {
    _guard: OwnedMutexGuard<()>,
}

#[cfg(test)]
impl Drop for VoiceServiceTestGuard {
    fn drop(&mut self) {
        if let Some(service) = TEST_VOICE_SERVICE.get() {
            *service.lock().unwrap() = None;
        }
    }
}

#[cfg(test)]
fn test_voice_service_override() -> Option<Arc<VoiceService>> {
    TEST_VOICE_SERVICE
        .get_or_init(|| StdMutex::new(None))
        .lock()
        .unwrap()
        .clone()
}

#[cfg(test)]
pub fn test_voice_service_with_responses(
    responses: Vec<Option<String>>,
) -> (Arc<VoiceService>, Arc<TestVoiceRenderer>) {
    let renderer = TestVoiceRenderer::new(responses);
    (
        Arc::new(VoiceService::new_with_renderer(renderer.clone())),
        renderer,
    )
}

#[cfg(test)]
pub async fn install_test_voice_service(service: Arc<VoiceService>) -> VoiceServiceTestGuard {
    let lock = TEST_VOICE_SERVICE_LOCK
        .get_or_init(|| async { Arc::new(AMutex::new(())) })
        .await
        .clone();
    let guard = lock.lock_owned().await;
    *TEST_VOICE_SERVICE
        .get_or_init(|| StdMutex::new(None))
        .lock()
        .unwrap() = Some(service);
    VoiceServiceTestGuard { _guard: guard }
}

pub async fn voice_service() -> Arc<VoiceService> {
    #[cfg(test)]
    if let Some(service) = test_voice_service_override() {
        return service;
    }

    VOICE_SERVICE
        .get_or_init(|| async { Arc::new(VoiceService::new()) })
        .await
        .clone()
}

impl VoiceIntent {
    fn as_str(self) -> &'static str {
        match self {
            VoiceIntent::AutonomousReportSaved => "voice:autonomous_report_saved",
            VoiceIntent::WorkflowStarted => "voice:workflow_started",
            VoiceIntent::WorkflowCompleted => "voice:workflow_completed",
            VoiceIntent::WorkflowFailed => "voice:workflow_failed",
            VoiceIntent::ChatTitle => "voice:chat_title",
            VoiceIntent::ActivityTitle => "voice:activity_title",
        }
    }
}

impl VoiceService {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(AMutex::new(HashMap::new())),
            ttl: VOICE_TTL,
            renderer: Arc::new(SubchatVoiceRenderer),
        }
    }

    #[cfg(test)]
    fn new_with_renderer(renderer: Arc<dyn VoiceRenderer>) -> Self {
        Self {
            cache: Arc::new(AMutex::new(HashMap::new())),
            ttl: VOICE_TTL,
            renderer,
        }
    }

    pub async fn render_activity_title(
        &self,
        gcx: Arc<ARwLock<GlobalContext>>,
        ctx: VoiceCtx<'_>,
        intent: VoiceIntent,
    ) -> String {
        self.render_line(gcx, &ctx, intent.as_str()).await
    }

    pub async fn render_speech(
        &self,
        gcx: Arc<ARwLock<GlobalContext>>,
        ctx: VoiceCtx<'_>,
        intent: SpeechIntent,
    ) -> BuddySpeechItem {
        let intent_kind = intent.as_str();
        let text = self.render_line(gcx, &ctx, intent_kind).await;
        BuddySpeechItem {
            id: format!("buddy-voice-{}", Uuid::new_v4()),
            text,
            mood: intent.mood().to_string(),
            scope: "global".to_string(),
            persistent: false,
            ttl_seconds: 10,
            dedupe_key: Some(intent_kind.to_string()),
            created_at: chrono::Utc::now().to_rfc3339(),
            controls: vec![],
            chat_id: None,
        }
    }

    pub async fn render_runtime_event(
        &self,
        gcx: Arc<ARwLock<GlobalContext>>,
        ctx: VoiceCtx<'_>,
        status: &str,
    ) -> (String, Option<String>) {
        let intent_kind = format!("runtime:{}", status);
        let title = self.render_line(gcx, &ctx, &intent_kind).await;
        let description = ctx
            .workflow_summary
            .map(normalize_voice_line)
            .filter(|text| !text.is_empty());
        (title, description)
    }

    pub async fn render_chat_title(
        &self,
        gcx: Arc<ARwLock<GlobalContext>>,
        ctx: VoiceCtx<'_>,
    ) -> String {
        self.render_line(gcx, &ctx, VoiceIntent::ChatTitle.as_str())
            .await
    }

    fn fallback_for(&self, intent_kind: &str, ctx: &VoiceCtx<'_>) -> String {
        let phrases = fallback_phrases(intent_kind);
        let idx = fallback_index(intent_kind, &ctx.persona.archetype_id, phrases.len());
        let style = fallback_style(&ctx.persona.archetype_id);
        normalize_voice_line(&format!("{}: {}", style, phrases[idx]))
    }

    async fn render_line(
        &self,
        gcx: Arc<ARwLock<GlobalContext>>,
        ctx: &VoiceCtx<'_>,
        intent_kind: &str,
    ) -> String {
        let key = self.cache_key(intent_kind, ctx);
        if let Some(cached) = self.cache_get(key).await {
            return cached;
        }

        let fallback = self.fallback_for(intent_kind, ctx);
        let request = VoiceRenderRequest::from_ctx(intent_kind, ctx);
        let rendered =
            tokio::time::timeout(VOICE_TIMEOUT, self.renderer.render_voice(gcx, request))
                .await
                .ok()
                .flatten()
                .map(|text| normalize_voice_line(&text))
                .filter(|text| !text.is_empty())
                .unwrap_or(fallback);

        self.cache_insert(key, rendered.clone()).await;
        rendered
    }

    fn cache_key(&self, intent_kind: &str, ctx: &VoiceCtx<'_>) -> u64 {
        let mut hasher = DefaultHasher::new();
        intent_kind.hash(&mut hasher);
        ctx.persona.archetype_id.hash(&mut hasher);
        ctx.pulse_one_liner.hash(&mut hasher);
        ctx.workflow_id.hash(&mut hasher);
        ctx.workflow_summary.hash(&mut hasher);
        hasher.finish()
    }

    async fn cache_get(&self, key: u64) -> Option<String> {
        let now = Instant::now();
        let mut cache = self.cache.lock().await;
        match cache.get_mut(&key) {
            Some((text, seen_at)) if now.saturating_duration_since(*seen_at) < self.ttl => {
                let text = text.clone();
                *seen_at = now;
                Some(text)
            }
            Some(_) => {
                cache.remove(&key);
                None
            }
            None => None,
        }
    }

    async fn cache_insert(&self, key: u64, text: String) {
        let now = Instant::now();
        let mut cache = self.cache.lock().await;
        cache.retain(|_, (_, seen_at)| now.saturating_duration_since(*seen_at) < self.ttl);
        if cache.len() >= VOICE_CACHE_MAX_ITEMS {
            if let Some(oldest_key) = cache
                .iter()
                .min_by_key(|(_, (_, seen_at))| *seen_at)
                .map(|(key, _)| *key)
            {
                cache.remove(&oldest_key);
            }
        }
        cache.insert(key, (text, now));
    }
}

impl Default for VoiceService {
    fn default() -> Self {
        Self::new()
    }
}

impl VoiceRenderRequest {
    fn from_ctx(intent_kind: &str, ctx: &VoiceCtx<'_>) -> Self {
        Self {
            intent_kind: intent_kind.to_string(),
            archetype_id: ctx.persona.archetype_id.clone(),
            archetype_label: ctx.persona.archetype_label.clone(),
            vibe: ctx.persona.vibe.clone(),
            summary: ctx.persona.summary.clone(),
            prompt: ctx.persona.prompt.clone(),
            identity_name: ctx.identity_name.to_string(),
            pulse_one_liner: ctx.pulse_one_liner.clone(),
            workflow_id: ctx.workflow_id.map(str::to_string),
            workflow_summary: ctx.workflow_summary.map(str::to_string),
        }
    }

    fn system_prompt(&self) -> String {
        format!(
            "You write short in-character UI copy for Buddy, a project companion. Persona: {} ({}) with vibe '{}'. Style guide: {}. Return one line under 80 characters, no markdown, no quotes.",
            self.archetype_label, self.archetype_id, self.vibe, self.prompt
        )
    }

    fn user_prompt(&self) -> String {
        format!(
            "Intent: {}\nBuddy name: {}\nPersona summary: {}\nProject pulse: {}\nWorkflow id: {}\nWorkflow summary: {}\nWrite exactly one concise line.",
            self.intent_kind,
            self.identity_name,
            self.summary,
            self.pulse_one_liner,
            self.workflow_id.as_deref().unwrap_or("none"),
            self.workflow_summary.as_deref().unwrap_or("none"),
        )
    }
}

async fn render_via_subchat(
    gcx: Arc<ARwLock<GlobalContext>>,
    request: VoiceRenderRequest,
) -> Option<String> {
    let mut config = match crate::subchat::resolve_subchat_config(
        gcx.clone(),
        "follow_up",
        false,
        Some(format!("buddy-voice-{}", Uuid::new_v4())),
        Some("Buddy Voice".to_string()),
        None,
        None,
        None,
        Some(vec![]),
        1,
        false,
        None,
        "buddy".to_string(),
    )
    .await
    {
        Ok(config) => config,
        Err(e) => {
            debug!("buddy voice: failed to resolve subchat config: {}", e);
            return None;
        }
    };

    let params = SubchatParameters {
        subchat_model_type: ChatModelType::Light,
        subchat_model: String::new(),
        subchat_n_ctx: config.n_ctx,
        subchat_max_new_tokens: VOICE_MAX_CHARS,
        subchat_temperature: Some(0.9),
        subchat_tokens_for_rag: 0,
        subchat_reasoning_effort: None,
    };
    let model = match crate::subchat::resolve_subchat_model(gcx.clone(), &params).await {
        Ok(model) => model,
        Err(e) => {
            debug!("buddy voice: failed to resolve light model: {}", e);
            return None;
        }
    };

    config.tool_name = "buddy_voice".to_string();
    config.tools = crate::subchat::ToolsPolicy::None;
    config.max_steps = 1;
    config.model = model;
    config.max_new_tokens = VOICE_MAX_CHARS;
    config.temperature = Some(0.9);
    config.buddy_meta = Some(crate::buddy::types::BuddyThreadMeta {
        is_buddy_chat: true,
        buddy_chat_kind: "system".to_string(),
        workflow_id: Some("buddy_voice".to_string()),
    });

    let messages = vec![
        ChatMessage::new("system".to_string(), request.system_prompt()),
        ChatMessage::new("user".to_string(), request.user_prompt()),
    ];

    match crate::subchat::run_subchat(gcx, messages, config).await {
        Ok(result) => result
            .messages
            .last()
            .and_then(|message| match &message.content {
                ChatContent::SimpleText(text) => Some(text.clone()),
                _ => None,
            }),
        Err(e) => {
            debug!("buddy voice: subchat failed: {}", e);
            None
        }
    }
}

fn normalize_voice_line(raw: &str) -> String {
    let stripped = raw
        .replace(['\r', '\n'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let trimmed = stripped
        .trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`')
        .trim();
    trimmed.chars().take(VOICE_MAX_CHARS).collect()
}

fn fallback_phrases(intent_kind: &str) -> &'static [&'static str] {
    if intent_kind.contains("failed") || intent_kind.contains("error") {
        &[
            "I spotted a snag and kept the trail marked.",
            "Something squeaked, so I saved the clue trail.",
            "I hit a bump, but the breadcrumbs are safe.",
        ]
    } else if intent_kind.contains("completed")
        || intent_kind.contains("saved")
        || intent_kind.contains("win")
        || intent_kind.contains("complete")
        || intent_kind.contains("milestone")
    {
        &[
            "Tiny victory logged and sparkling.",
            "Done, dusted, and neatly tucked away.",
            "That one landed nicely in the win pile.",
        ]
    } else if intent_kind.contains("title") {
        &[
            "Fresh project note",
            "Buddy field report",
            "Quick trail marker",
        ]
    } else if intent_kind.contains("started") || intent_kind.contains("quest_accept") {
        &[
            "I am on the trail now.",
            "Tiny boots on, checking the path.",
            "I will scout this corner for you.",
        ]
    } else {
        &[
            "I found a small signal worth watching.",
            "A tiny project clue just waved at me.",
            "I am keeping an eye on this thread.",
        ]
    }
}

fn fallback_index(intent_kind: &str, archetype_id: &str, len: usize) -> usize {
    let mut hasher = DefaultHasher::new();
    intent_kind.hash(&mut hasher);
    archetype_id.hash(&mut hasher);
    (hasher.finish() as usize) % len
}

fn fallback_style(archetype_id: &str) -> String {
    let style = archetype_id
        .replace(['_', '-'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let style = if style.is_empty() { "buddy" } else { &style };
    style.chars().take(24).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn persona(archetype_id: &str) -> BuddyPersonalityProfile {
        BuddyPersonalityProfile {
            archetype_id: archetype_id.to_string(),
            archetype_label: archetype_id.to_string(),
            vibe: "bright".to_string(),
            summary: "A helpful test buddy.".to_string(),
            prompt: "Helpful and concise".to_string(),
            traits: Default::default(),
        }
    }

    fn voice_ctx<'a>(persona: &'a BuddyPersonalityProfile) -> VoiceCtx<'a> {
        VoiceCtx {
            persona,
            identity_name: "Pixel",
            pulse_one_liner: "Tests are running".to_string(),
            workflow_id: Some("test_workflow"),
            workflow_summary: Some("checking voice service"),
        }
    }

    #[tokio::test]
    async fn voice_returns_fallback_when_renderer_returns_none() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let renderer = TestVoiceRenderer::new(vec![None]);
        let service = VoiceService::new_with_renderer(renderer.clone());
        let persona = persona("helper_sprite");
        let ctx = voice_ctx(&persona);
        let expected = service.fallback_for(VoiceIntent::ChatTitle.as_str(), &ctx);

        let title = service.render_chat_title(gcx, ctx).await;

        assert_eq!(title, expected);
        assert_eq!(renderer.calls(), 1);
    }

    #[tokio::test]
    async fn voice_cache_hits_within_ttl_window() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let renderer = TestVoiceRenderer::new(vec![Some("cached sparkle".to_string())]);
        let service = VoiceService::new_with_renderer(renderer.clone());
        let persona = persona("helper_sprite");

        let first = service
            .render_chat_title(gcx.clone(), voice_ctx(&persona))
            .await;
        let second = service.render_chat_title(gcx, voice_ctx(&persona)).await;

        assert_eq!(first, "cached sparkle");
        assert_eq!(second, "cached sparkle");
        assert_eq!(renderer.calls(), 1);
    }

    #[tokio::test]
    async fn voice_caps_output_at_80_chars() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let renderer = TestVoiceRenderer::new(vec![Some("a".repeat(120))]);
        let service = VoiceService::new_with_renderer(renderer);
        let persona = persona("helper_sprite");

        let title = service.render_chat_title(gcx, voice_ctx(&persona)).await;

        assert_eq!(title.chars().count(), VOICE_MAX_CHARS);
    }

    #[tokio::test]
    async fn voice_strips_newlines() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let renderer = TestVoiceRenderer::new(vec![Some("hello\nbuddy\r\nnow".to_string())]);
        let service = VoiceService::new_with_renderer(renderer);
        let persona = persona("helper_sprite");

        let title = service.render_chat_title(gcx, voice_ctx(&persona)).await;

        assert_eq!(title, "hello buddy now");
        assert!(!title.contains('\n'));
        assert!(!title.contains('\r'));
    }

    #[test]
    fn voice_returns_distinct_fallbacks_per_persona_archetype() {
        let renderer = TestVoiceRenderer::new(vec![]);
        let service = VoiceService::new_with_renderer(renderer);
        let first_persona = persona("helper_sprite");
        let second_persona = persona("quiet_guardian");
        let first = service.fallback_for("speech:insight", &voice_ctx(&first_persona));
        let second = service.fallback_for("speech:insight", &voice_ctx(&second_persona));

        assert_ne!(first, second);
    }
}
