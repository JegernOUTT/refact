use chrono::{DateTime, Utc};
use std::collections::{HashMap, VecDeque};
use tracing::debug;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::call_validation::ChatContent;
use crate::chat::types::ThreadParams;

use super::settings::{BuddySettings, HumorLevel};
use super::snapshot::{ChatReactionAttempt, ChatReactionDebug};
use super::types::{BuddyBubblePolicy, BuddyPersonalityProfile, BuddyRuntimeEvent};
use super::voice_service::{voice_service, ChatReactionSpeechIntent, VoiceCtx};

pub const ANALYSIS_TEXT_MIN_CHARS: usize = 20;
pub const ANALYSIS_TEXT_MAX_CHARS: usize = 500;
pub const CHAT_REACTION_SPEECH_MAX_CHARS: usize = 140;
pub const PER_CHAT_COOLDOWN_SECS: i64 = 300;
pub const GLOBAL_HOURLY_CAP: u32 = 10;
pub const CHAT_REACTION_DEBUG_ATTEMPT_CAP: usize = 50;
const HUMOR_BUCKET_PERCENT: u64 = 40;
const CHAT_REACTION_ECHO_NGRAM_WORDS: usize = 3;
const CHAT_REACTION_ECHO_NGRAM_MIN_CHARS: usize = 18;
const CHAT_REACTION_ECHO_LONG_TOKEN_MIN_CHARS: usize = 12;
const CHAT_REACTION_ECHO_SHORT_PHRASE_WORDS: usize = 2;
const CHAT_REACTION_ECHO_SHORT_PHRASE_MIN_CHARS: usize = 7;

const CHAT_REACTION_ECHO_IDENTIFYING_WORDS: &[&str] = &[
    "account",
    "accounts",
    "billing",
    "client",
    "clients",
    "contract",
    "contracts",
    "customer",
    "customers",
    "import",
    "invoice",
    "invoices",
    "payroll",
    "pipeline",
    "project",
    "roadmap",
    "tenant",
    "tenants",
    "vendor",
    "vendors",
];

const CHAT_REACTION_ECHO_GENERIC_WORDS: &[&str] = &[
    "again",
    "answer",
    "before",
    "better",
    "change",
    "changes",
    "chat",
    "check",
    "checkpoint",
    "choices",
    "comment",
    "compare",
    "debug",
    "edge",
    "fallback",
    "feature",
    "flow",
    "generic",
    "helper",
    "issue",
    "iteration",
    "little",
    "message",
    "naming",
    "option",
    "options",
    "output",
    "phrase",
    "plan",
    "please",
    "quick",
    "reaction",
    "response",
    "retry",
    "review",
    "signal",
    "small",
    "step",
    "task",
    "testing",
    "thread",
    "tiny",
    "update",
    "wording",
    "work",
    "working",
];

// BUG keywords: exact-prefix token match prevents false positives from words like debug, latest,
// contest (e.g. "debug" does not start with "bug"). Removed: fail, failing, broken (too noisy).
// "timeout" stays as a token keyword; multi-word phrase "timed out" is also checked below.
// Additional multi-word phrase triggers: "not working", "doesn't work", "does not work".
const BUG_KEYWORDS: &[&str] = &[
    "bug",
    "error",
    "crash",
    "panic",
    "exception",
    "traceback",
    "regression",
    "deadlock",
    "timeout",
];

// INSIGHT keywords: exact-prefix token match to avoid noise from plan->planning, test->testing,
// perf->performance substring etc. Removed: plan, test, perf, improve, optimize, rewrite.
const INSIGHT_KEYWORDS: &[&str] = &[
    "architecture",
    "api",
    "cache",
    "caching",
    "cleanup",
    "component",
    "refactor",
    "performance",
    "security",
    "migrate",
    "migration",
    "design",
    "feature",
    "flow",
    "implement",
    "review",
    "rename",
    "schema",
    "simplify",
    "state",
    "tradeoff",
    "ui",
    "ux",
    "deprecate",
    "deprecated",
];

pub const HUMOR_LINES: &[&str] = &[
    "Pixel gremlin status: tiny chaos goblin approves this breadcrumb pile.",
    "Chaos ping: I put a sticker on this thread. It wiggled. Suspiciously cute.",
    "Tiny gremlin note: the idea has snack-sized boots and is doing parkour.",
    "Pixel detective mode engaged. Snacks hidden in the margins, naturally.",
    "Mild scheming intensifies. I will behave. Mostly. Probably. Eep.",
    "Gremlin confetti deployed: small sparkle, zero structural warranty.",
    "This thread just made a tiny clown honk in my circuits. Respectfully.",
    "Goblin radar says: interesting trail. I am sniffing it with science mittens.",
];

pub const INSIGHT_LINES: &[&str] = &[
    "Tiny signal: this might want one assumption check before charging in.",
    "Heads up — worth poking the edges before committing.",
    "Quick read: looks reasonable; one small risk worth sanity-checking.",
    "I see an iteration loop forming; one small checkpoint could keep it tidy.",
    "Looks like exploration mode; compare the options before locking the path.",
    "This feels like a simplifying pass; keep the before/after behavior pinned.",
    "Planning energy detected; naming the next step may save a retry later.",
    "Debugging trail spotted; isolate one signal before chasing every footprint.",
];

pub const BUG_LINES: &[&str] = &[
    "This smells bug-shaped. Want me to help isolate it?",
    "Tiny alarm: this looks like a bug trail. I can dig deeper if you want.",
    "Hmm, that pattern usually means something failed quietly. Want a closer look?",
];

fn stable_hash(text: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for byte in text.bytes() {
        h ^= u64::from(byte);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn pick_template(lines: &[&'static str], seed: &str) -> &'static str {
    lines[(stable_hash(seed) as usize) % lines.len()]
}

pub fn deterministic_humor_bucket(text: &str) -> bool {
    stable_hash(&format!("chat_reaction:{text}")) % 100 < HUMOR_BUCKET_PERCENT
}

fn word_tokens(lower: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start: Option<usize> = None;
    for (i, ch) in lower.char_indices() {
        let is_token = ch.is_ascii_alphanumeric() || ch == '_';
        match (is_token, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                result.push(&lower[s..i]);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        result.push(&lower[s..]);
    }
    result
}

fn has_bug_signal(lower: &str, tokens: &[&str]) -> bool {
    if BUG_KEYWORDS
        .iter()
        .any(|kw| tokens.iter().any(|t| t.starts_with(kw)))
    {
        return true;
    }
    lower.contains("not working")
        || lower.contains("doesn't work")
        || lower.contains("does not work")
        || lower.contains("timed out")
}

fn matches_insight_keyword(token: &str, keyword: &str) -> bool {
    match keyword {
        "api" => token == "api" || token == "apis",
        "cache" => token == "cache" || token == "caches" || token == "cached",
        "flow" => token == "flow" || token == "flows",
        "state" => token == "state" || token == "states" || token == "stateful",
        "ui" => token == "ui" || token == "uis",
        "ux" => token == "ux",
        _ => token.starts_with(keyword),
    }
}

fn has_interaction_signal(lower: &str, tokens: &[&str]) -> bool {
    const INTERACTION_KEYWORDS: &[&str] = &[
        "ask",
        "asking",
        "compare",
        "comparing",
        "debugging",
        "explore",
        "exploring",
        "iterate",
        "iterating",
        "plan",
        "planning",
        "retry",
        "retrying",
        "simplify",
        "simplifying",
        "tweak",
        "tweaking",
    ];
    if INTERACTION_KEYWORDS
        .iter()
        .any(|kw| tokens.iter().any(|t| t == kw || t.starts_with(kw)))
    {
        return true;
    }
    lower.contains("try again")
        || lower.contains("what if")
        || lower.contains("can you")
        || lower.contains("let's")
        || lower.contains("lets")
        || lower.contains("step by step")
}

pub struct AcceptedUserMessage {
    pub chat_id: String,
    pub thread: ThreadParams,
    pub content: ChatContent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatReactionSkipReason {
    ThreadFiltered,
    TextTooShort,
    BuddyUnavailable,
    SettingsDisabled,
    NoReactionKind,
    RateLimited,
}

impl ChatReactionSkipReason {
    pub fn as_str(self) -> &'static str {
        match self {
            ChatReactionSkipReason::ThreadFiltered => "thread_filtered",
            ChatReactionSkipReason::TextTooShort => "text_too_short",
            ChatReactionSkipReason::BuddyUnavailable => "buddy_unavailable",
            ChatReactionSkipReason::SettingsDisabled => "settings_disabled",
            ChatReactionSkipReason::NoReactionKind => "no_reaction_kind",
            ChatReactionSkipReason::RateLimited => "rate_limited",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChatReactionDebugState {
    recent_attempts: VecDeque<ChatReactionAttempt>,
    counts_by_result: HashMap<String, u64>,
    last_skip_reason: Option<String>,
    last_emitted_at: Option<String>,
}

impl ChatReactionDebugState {
    pub fn new() -> Self {
        Self {
            recent_attempts: VecDeque::new(),
            counts_by_result: HashMap::new(),
            last_skip_reason: None,
            last_emitted_at: None,
        }
    }

    pub fn record_skipped(&mut self, chat_id: &str, reason: ChatReactionSkipReason) {
        let skip_reason = reason.as_str().to_string();
        self.record(ChatReactionAttempt {
            attempted_at: Utc::now().to_rfc3339(),
            chat_id: chat_id.to_string(),
            result: "skipped".to_string(),
            skip_reason: Some(skip_reason.clone()),
            signal_type: None,
        });
        self.last_skip_reason = Some(skip_reason);
    }

    pub fn record_emitted(&mut self, chat_id: &str, signal_type: &str) {
        let attempted_at = Utc::now().to_rfc3339();
        self.record(ChatReactionAttempt {
            attempted_at: attempted_at.clone(),
            chat_id: chat_id.to_string(),
            result: "emitted".to_string(),
            skip_reason: None,
            signal_type: Some(signal_type.to_string()),
        });
        self.last_emitted_at = Some(attempted_at);
    }

    pub fn snapshot(&self) -> ChatReactionDebug {
        ChatReactionDebug {
            recent_attempts: self.recent_attempts.iter().cloned().collect(),
            counts_by_result: self.counts_by_result.clone(),
            last_skip_reason: self.last_skip_reason.clone(),
            last_emitted_at: self.last_emitted_at.clone(),
        }
    }

    fn record(&mut self, attempt: ChatReactionAttempt) {
        *self
            .counts_by_result
            .entry(attempt.result.clone())
            .or_insert(0) += 1;
        self.recent_attempts.push_back(attempt);
        while self.recent_attempts.len() > CHAT_REACTION_DEBUG_ATTEMPT_CAP {
            self.recent_attempts.pop_front();
        }
    }
}

impl Default for ChatReactionDebugState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn should_observe_thread(thread: &ThreadParams) -> bool {
    if let Some(meta) = thread.buddy_meta.as_ref() {
        if meta.is_buddy_chat {
            return false;
        }
    }
    if thread.task_meta.is_some() {
        return false;
    }
    let mode = thread.mode.as_str();
    if matches!(mode, "buddy" | "setup" | "task_agent" | "task_planner") {
        return false;
    }
    if mode.starts_with("setup_") {
        return false;
    }
    if thread.parent_id.is_some() && thread.link_type.as_deref() != Some("branch") {
        return false;
    }
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ChatReactionKind {
    Humor,
    Insight,
    BugCandidate,
}

#[derive(Debug, Clone)]
pub struct ChatReaction {
    pub kind: ChatReactionKind,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatReactionCandidate {
    pub analysis_text: String,
    pub kind: ChatReactionKind,
}

#[derive(Clone)]
struct ChatReactionVoiceInputs {
    persona: BuddyPersonalityProfile,
    identity_name: String,
    pulse_one_liner: String,
}

/// Chat reactions are independent of `proactive_enabled` and `message_observation_enabled`,
/// which gate proactive suggestions and the periodic ChatPatternObserver respectively.
pub fn settings_allow_chat_reactions(settings: &BuddySettings) -> bool {
    settings.enabled && settings.chat_reactions_enabled && !settings.quiet_mode
}

pub fn chat_reaction_candidate(
    thread: &ThreadParams,
    raw_text: &str,
    settings: Option<&BuddySettings>,
) -> Result<ChatReactionCandidate, ChatReactionSkipReason> {
    if !should_observe_thread(thread) {
        return Err(ChatReactionSkipReason::ThreadFiltered);
    }
    let analysis_text =
        prepare_analysis_text(raw_text).ok_or(ChatReactionSkipReason::TextTooShort)?;
    let settings = settings.ok_or(ChatReactionSkipReason::BuddyUnavailable)?;
    if !settings_allow_chat_reactions(settings) {
        return Err(ChatReactionSkipReason::SettingsDisabled);
    }
    let kind = classify_chat_reaction_kind(&analysis_text, settings)
        .ok_or(ChatReactionSkipReason::NoReactionKind)?;
    Ok(ChatReactionCandidate {
        analysis_text,
        kind,
    })
}

pub fn prepare_analysis_text(raw: &str) -> Option<String> {
    let redacted = refact_core::string_utils::redact_sensitive(raw);
    let normalized: String = redacted.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() < ANALYSIS_TEXT_MIN_CHARS {
        return None;
    }
    Some(normalized.chars().take(ANALYSIS_TEXT_MAX_CHARS).collect())
}

pub fn classify_chat_reaction_kind(
    text: &str,
    settings: &BuddySettings,
) -> Option<ChatReactionKind> {
    let lower = text.to_lowercase();
    let tokens = word_tokens(&lower);
    if has_bug_signal(&lower, &tokens) {
        Some(ChatReactionKind::BugCandidate)
    } else if settings.humor_enabled
        && settings.humor_level != HumorLevel::Off
        && deterministic_humor_bucket(text)
    {
        Some(ChatReactionKind::Humor)
    } else if INSIGHT_KEYWORDS
        .iter()
        .any(|kw| tokens.iter().any(|t| matches_insight_keyword(t, kw)))
        || has_interaction_signal(&lower, &tokens)
    {
        Some(ChatReactionKind::Insight)
    } else if settings.humor_enabled && settings.humor_level != HumorLevel::Off {
        Some(ChatReactionKind::Insight)
    } else {
        None
    }
}

pub fn fallback_chat_reaction_text(kind: ChatReactionKind, seed: &str) -> String {
    match kind {
        ChatReactionKind::Humor => pick_template(HUMOR_LINES, seed).to_string(),
        ChatReactionKind::Insight => pick_template(INSIGHT_LINES, seed).to_string(),
        ChatReactionKind::BugCandidate => pick_template(BUG_LINES, seed).to_string(),
    }
}

fn normalize_chat_reaction_speech(raw: &str) -> String {
    let stripped = raw
        .replace(['\r', '\n'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    stripped
        .trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`')
        .trim()
        .to_string()
}

#[derive(Debug, Clone)]
struct EchoAnalysisToken {
    text: String,
    identifying: bool,
}

fn echo_raw_tokens(raw: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    for ch in raw.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            current.push(ch);
        } else if !current.is_empty() {
            result.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}

fn normalize_echo_token(token: &str) -> String {
    token.chars().flat_map(char::to_lowercase).collect()
}

fn is_redacted_echo_token(token: &str) -> bool {
    token == "redacted"
}

fn is_possessive_echo_token(token: &str) -> bool {
    token == "s"
}

fn is_generic_echo_token(token: &str) -> bool {
    token.chars().count() <= 3 || CHAT_REACTION_ECHO_GENERIC_WORDS.contains(&token)
}

fn is_identifying_echo_token(token: &str) -> bool {
    if CHAT_REACTION_ECHO_IDENTIFYING_WORDS.contains(&token) {
        return true;
    }
    token.chars().any(|ch| !ch.is_ascii()) || token.chars().count() >= 6
}

fn is_private_code_echo_token(token: &str) -> bool {
    token.chars().any(|ch| ch.is_ascii_digit()) && token.chars().count() >= 2
}

fn is_proper_echo_token(raw_token: &str, normalized: &str) -> bool {
    !is_generic_echo_token(normalized)
        && raw_token.chars().next().is_some_and(|ch| ch.is_uppercase())
}

fn echo_word_tokens(raw: &str) -> Vec<String> {
    echo_raw_tokens(raw)
        .into_iter()
        .map(|token| normalize_echo_token(&token))
        .filter(|token| !is_redacted_echo_token(token) && !is_possessive_echo_token(token))
        .collect()
}

fn echo_analysis_tokens(raw: &str) -> Vec<EchoAnalysisToken> {
    echo_raw_tokens(raw)
        .into_iter()
        .filter_map(|raw_token| {
            let normalized = normalize_echo_token(&raw_token);
            if is_redacted_echo_token(&normalized) || is_possessive_echo_token(&normalized) {
                return None;
            }
            let identifying = is_identifying_echo_token(&normalized)
                || is_private_code_echo_token(&normalized)
                || is_proper_echo_token(&raw_token, &normalized);
            Some(EchoAnalysisToken {
                text: normalized,
                identifying,
            })
        })
        .collect()
}

fn is_identifying_echo_phrase(tokens: &[EchoAnalysisToken]) -> bool {
    tokens.iter().any(|token| token.identifying)
        && !tokens
            .iter()
            .all(|token| is_generic_echo_token(&token.text))
}

fn echo_phrase_chars(tokens: &[EchoAnalysisToken]) -> usize {
    tokens
        .iter()
        .map(|token| token.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .count()
}

fn generated_contains_echo_phrase(
    generated_tokens: &[String],
    phrase_tokens: &[EchoAnalysisToken],
) -> bool {
    generated_tokens.windows(phrase_tokens.len()).any(|window| {
        window
            .iter()
            .zip(phrase_tokens)
            .all(|(left, right)| left == &right.text)
    })
}

fn contains_redaction_marker(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("[redacted") || lower.contains("<redacted") || lower.contains("&lt;redacted")
}

fn contains_analysis_echo(generated: &str, analysis_text: &str) -> bool {
    let generated_tokens = echo_word_tokens(generated);
    let tokens = echo_analysis_tokens(analysis_text);

    for token in &tokens {
        if (token.text.chars().count() >= CHAT_REACTION_ECHO_LONG_TOKEN_MIN_CHARS
            || (token.identifying && token.text.chars().any(|ch| !ch.is_ascii())))
            && generated_tokens.contains(&token.text)
        {
            return true;
        }
    }

    if tokens.len() >= CHAT_REACTION_ECHO_SHORT_PHRASE_WORDS
        && tokens
            .windows(CHAT_REACTION_ECHO_SHORT_PHRASE_WORDS)
            .any(|window| {
                is_identifying_echo_phrase(window)
                    && echo_phrase_chars(window) >= CHAT_REACTION_ECHO_SHORT_PHRASE_MIN_CHARS
                    && generated_contains_echo_phrase(&generated_tokens, window)
            })
    {
        return true;
    }

    if tokens.len() < CHAT_REACTION_ECHO_NGRAM_WORDS {
        return false;
    }
    tokens
        .windows(CHAT_REACTION_ECHO_NGRAM_WORDS)
        .any(|window| {
            echo_phrase_chars(window) >= CHAT_REACTION_ECHO_NGRAM_MIN_CHARS
                && generated_contains_echo_phrase(&generated_tokens, window)
        })
}

pub fn sanitize_chat_reaction_speech_text(generated: &str, analysis_text: &str) -> Option<String> {
    let redacted = refact_core::string_utils::redact_sensitive(generated);
    let normalized = normalize_chat_reaction_speech(&redacted);
    if normalized.is_empty()
        || contains_redaction_marker(&normalized)
        || contains_analysis_echo(&normalized, analysis_text)
    {
        return None;
    }
    Some(
        normalized
            .chars()
            .take(CHAT_REACTION_SPEECH_MAX_CHARS)
            .collect(),
    )
}

pub fn safe_chat_reaction_speech_text(
    kind: ChatReactionKind,
    analysis_text: &str,
    generated: &str,
) -> String {
    sanitize_chat_reaction_speech_text(generated, analysis_text)
        .unwrap_or_else(|| fallback_chat_reaction_text(kind, analysis_text))
}

/// Produces a dedupe fingerprint for a chat reaction event.
///
/// The `text` argument must already be redacted via
/// [`refact_core::string_utils::redact_sensitive`] before being passed here.
/// The resulting value is a non-cryptographic FNV-1a digest used only as an
/// in-memory and queue dedupe key — raw user text is never stored.
fn message_hash(text: &str) -> String {
    format!("{:016x}", stable_hash(text))
}

fn signal_type_for_kind(kind: &ChatReactionKind) -> &'static str {
    match kind {
        ChatReactionKind::Humor => "speech_humor",
        ChatReactionKind::Insight => "speech_insight",
        ChatReactionKind::BugCandidate => "chat_bug_candidate",
    }
}

pub fn build_reaction_event(
    chat_id: &str,
    analysis_text: &str,
    reaction: &ChatReaction,
) -> BuddyRuntimeEvent {
    let (signal_type, ttl_ms, bubble_policy) = match reaction.kind {
        ChatReactionKind::Humor => ("speech_humor", 90_000u64, BuddyBubblePolicy::Ambient),
        ChatReactionKind::Insight => ("speech_insight", 90_000u64, BuddyBubblePolicy::Ambient),
        ChatReactionKind::BugCandidate => (
            "chat_bug_candidate",
            120_000u64,
            BuddyBubblePolicy::EventOnce,
        ),
    };
    let kind_str = match reaction.kind {
        ChatReactionKind::Humor => "humor",
        ChatReactionKind::Insight => "insight",
        ChatReactionKind::BugCandidate => "bug",
    };
    let dedupe_key = format!(
        "chat_reaction:{chat_id}:{kind_str}:{}",
        message_hash(analysis_text)
    );
    let speech_text = safe_chat_reaction_speech_text(
        reaction.kind.clone(),
        analysis_text,
        reaction.text.as_str(),
    );
    BuddyRuntimeEvent {
        id: Uuid::new_v4().to_string(),
        signal_type: signal_type.to_string(),
        title: format!("Chat: {kind_str}"),
        description: None,
        source: "chat_reactions".to_string(),
        status: "info".to_string(),
        progress: None,
        dedupe_key: Some(dedupe_key),
        priority: "normal".to_string(),
        created_at: Utc::now().to_rfc3339(),
        ttl_ms: Some(ttl_ms),
        bubble_policy: Some(bubble_policy),
        speech_text: Some(speech_text),
        scene: None,
        duration_hint: None,
        persistent: false,
        controls: vec![],
        chat_id: Some(chat_id.to_string()),
        dismissed: false,
    }
}

async fn render_chat_reaction_text(
    app: &AppState,
    kind: &ChatReactionKind,
    analysis_text: &str,
    voice_inputs: &ChatReactionVoiceInputs,
) -> String {
    let fallback = fallback_chat_reaction_text(kind.clone(), analysis_text);
    let intent = match kind {
        ChatReactionKind::Humor => ChatReactionSpeechIntent::Humor,
        ChatReactionKind::Insight => ChatReactionSpeechIntent::Insight,
        ChatReactionKind::BugCandidate => ChatReactionSpeechIntent::BugCandidate,
    };
    let ctx = VoiceCtx {
        persona: &voice_inputs.persona,
        identity_name: voice_inputs.identity_name.as_str(),
        pulse_one_liner: voice_inputs.pulse_one_liner.clone(),
        workflow_id: Some("chat_reaction"),
        workflow_summary: Some(analysis_text),
    };
    let rendered = voice_service()
        .await
        .render_chat_reaction(app.clone(), ctx, intent)
        .await;
    sanitize_chat_reaction_speech_text(&rendered, analysis_text).unwrap_or(fallback)
}

pub async fn maybe_enqueue_chat_reaction(app: AppState, accepted: AcceptedUserMessage) {
    let raw_text = accepted.content.content_text_only();

    let plan = {
        let mut svc_guard = app.buddy.buddy.lock().await;
        let settings = svc_guard.as_ref().map(|svc| &svc.settings);
        let candidate = match chat_reaction_candidate(&accepted.thread, &raw_text, settings) {
            Ok(candidate) => candidate,
            Err(reason) => {
                if let Some(svc) = svc_guard.as_mut() {
                    svc.chat_reaction_debug.record_skipped(&accepted.chat_id, reason);
                }
                debug!(
                    target: "buddy.chat_reactions",
                    chat_id = %accepted.chat_id,
                    reason = %reason.as_str(),
                    message_chars = raw_text.chars().count(),
                    "buddy chat reaction skipped"
                );
                return;
            }
        };
        let Some(svc) = svc_guard.as_mut() else {
            return;
        };
        let now = chrono::Utc::now();
        if let Err(reason) =
            svc.chat_reaction_limiter
                .try_allow_kind(&accepted.chat_id, candidate.kind.clone(), now)
        {
            svc.chat_reaction_debug.record_skipped(&accepted.chat_id, reason);
            debug!(
                target: "buddy.chat_reactions",
                chat_id = %accepted.chat_id,
                reason = %reason.as_str(),
                reaction_kind = ?candidate.kind,
                analysis_hash = %message_hash(&candidate.analysis_text),
                "buddy chat reaction skipped"
            );
            return;
        }
        let voice_inputs = ChatReactionVoiceInputs {
            persona: svc.state.personality.clone(),
            identity_name: svc.state.identity.name.clone(),
            pulse_one_liner: format!(
                "{} pending ops, {} stuck tasks",
                svc.pulse.memory.pending_ops, svc.pulse.tasks.stuck
            ),
        };
        (candidate, voice_inputs)
    };

    let app2 = app.clone();
    let chat_id = accepted.chat_id.clone();
    tokio::spawn(async move {
        let (candidate, voice_inputs) = plan;
        let signal_type = signal_type_for_kind(&candidate.kind).to_string();
        let speech = render_chat_reaction_text(
            &app2,
            &candidate.kind,
            &candidate.analysis_text,
            &voice_inputs,
        )
        .await;
        let event = build_reaction_event(
            &chat_id,
            &candidate.analysis_text,
            &ChatReaction {
                kind: candidate.kind.clone(),
                text: speech,
            },
        );
        let event_id = event.id.clone();
        let analysis_hash = message_hash(&candidate.analysis_text);
        let mut svc_guard = app2.buddy.buddy.lock().await;
        if let Some(svc) = svc_guard.as_mut() {
            if settings_allow_chat_reactions(&svc.settings) {
                svc.enqueue_runtime_event(event);
                svc.chat_reaction_debug
                    .record_emitted(&chat_id, signal_type.as_str());
                debug!(
                    target: "buddy.chat_reactions",
                    chat_id = %chat_id,
                    reaction_kind = ?candidate.kind,
                    analysis_hash = %analysis_hash,
                    event_id = %event_id,
                    "buddy chat reaction emitted"
                );
            } else {
                svc.chat_reaction_debug
                    .record_skipped(&chat_id, ChatReactionSkipReason::SettingsDisabled);
                debug!(
                    target: "buddy.chat_reactions",
                    chat_id = %chat_id,
                    reason = %ChatReactionSkipReason::SettingsDisabled.as_str(),
                    reaction_kind = ?candidate.kind,
                    analysis_hash = %analysis_hash,
                    "buddy chat reaction skipped"
                );
            }
        } else {
            debug!(
                target: "buddy.chat_reactions",
                chat_id = %chat_id,
                reason = %ChatReactionSkipReason::BuddyUnavailable.as_str(),
                reaction_kind = ?candidate.kind,
                analysis_hash = %analysis_hash,
                "buddy chat reaction skipped"
            );
        }
    });
}

pub struct ChatReactionLimiter {
    pub(crate) per_chat_kind_last_at: HashMap<(String, ChatReactionKind), DateTime<Utc>>,
    global_hourly_count: u32,
    global_window_start: DateTime<Utc>,
}

impl ChatReactionLimiter {
    pub fn new() -> Self {
        Self {
            per_chat_kind_last_at: HashMap::new(),
            global_hourly_count: 0,
            global_window_start: Utc::now(),
        }
    }

    /// Per-kind cooldown prevents low-signal humor reactions from suppressing high-signal bug candidates.
    pub fn allow_kind(
        &mut self,
        chat_id: &str,
        kind: ChatReactionKind,
        now: DateTime<Utc>,
    ) -> bool {
        self.try_allow_kind(chat_id, kind, now).is_ok()
    }

    pub fn try_allow_kind(
        &mut self,
        chat_id: &str,
        kind: ChatReactionKind,
        now: DateTime<Utc>,
    ) -> Result<(), ChatReactionSkipReason> {
        self.per_chat_kind_last_at
            .retain(|_, last_at| (now - *last_at).num_seconds() < PER_CHAT_COOLDOWN_SECS);
        if (now - self.global_window_start).num_seconds() >= 3600 {
            self.global_hourly_count = 0;
            self.global_window_start = now;
        }
        if self.global_hourly_count >= GLOBAL_HOURLY_CAP {
            return Err(ChatReactionSkipReason::RateLimited);
        }
        let key = (chat_id.to_string(), kind);
        if let Some(last) = self.per_chat_kind_last_at.get(&key) {
            if (now - *last).num_seconds() < PER_CHAT_COOLDOWN_SECS {
                return Err(ChatReactionSkipReason::RateLimited);
            }
        }
        self.per_chat_kind_last_at.insert(key, now);
        self.global_hourly_count += 1;
        Ok(())
    }

    pub fn allow(&mut self, chat_id: &str, now: DateTime<Utc>) -> bool {
        self.allow_kind(chat_id, ChatReactionKind::Humor, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn classify_chat_reaction(text: &str, settings: &BuddySettings) -> Option<ChatReaction> {
        classify_chat_reaction_kind(text, settings).map(|kind| ChatReaction {
            text: fallback_chat_reaction_text(kind.clone(), text),
            kind,
        })
    }

    #[test]
    fn settings_allow_all_defaults() {
        assert!(settings_allow_chat_reactions(&BuddySettings::default()));
    }

    #[test]
    fn settings_gate_blocks_hard_toggles_only() {
        let mut s = BuddySettings::default();
        s.enabled = false;
        assert!(!settings_allow_chat_reactions(&s));

        let mut s = BuddySettings::default();
        s.proactive_enabled = false;
        assert!(
            settings_allow_chat_reactions(&s),
            "live chat reactions must not require proactive_enabled"
        );

        let mut s = BuddySettings::default();
        s.chat_reactions_enabled = false;
        assert!(!settings_allow_chat_reactions(&s));
    }

    #[test]
    fn chat_reactions_independent_from_proactive() {
        let mut s = BuddySettings::default();
        s.proactive_enabled = false;
        assert!(settings_allow_chat_reactions(&s));
    }

    #[test]
    fn chat_reactions_independent_from_message_observation() {
        let mut s = BuddySettings::default();
        s.message_observation_enabled = false;
        s.chat_reactions_enabled = true;
        assert!(
            settings_allow_chat_reactions(&s),
            "chat reactions must work even when message_observation_enabled is false"
        );
    }

    #[test]
    fn settings_gate_blocks_quiet_mode() {
        let mut s = BuddySettings::default();
        s.quiet_mode = true;
        assert!(!settings_allow_chat_reactions(&s));
    }

    #[test]
    fn chat_reaction_candidate_reports_skip_reasons() {
        let settings = BuddySettings::default();
        let thread = ThreadParams::default();
        let normal_text = "please design a new component state flow now";

        let mut filtered = ThreadParams::default();
        filtered.mode = "task_agent".to_string();
        assert_eq!(
            chat_reaction_candidate(&filtered, normal_text, Some(&settings)),
            Err(ChatReactionSkipReason::ThreadFiltered)
        );

        assert_eq!(
            chat_reaction_candidate(&thread, "too short", Some(&settings)),
            Err(ChatReactionSkipReason::TextTooShort)
        );

        assert_eq!(
            chat_reaction_candidate(&thread, normal_text, None),
            Err(ChatReactionSkipReason::BuddyUnavailable)
        );

        let mut disabled = BuddySettings::default();
        disabled.chat_reactions_enabled = false;
        assert_eq!(
            chat_reaction_candidate(&thread, normal_text, Some(&disabled)),
            Err(ChatReactionSkipReason::SettingsDisabled)
        );

        let mut disabled = BuddySettings::default();
        disabled.enabled = false;
        assert_eq!(
            chat_reaction_candidate(&thread, normal_text, Some(&disabled)),
            Err(ChatReactionSkipReason::SettingsDisabled)
        );

        let mut disabled = BuddySettings::default();
        disabled.quiet_mode = true;
        assert_eq!(
            chat_reaction_candidate(&thread, normal_text, Some(&disabled)),
            Err(ChatReactionSkipReason::SettingsDisabled)
        );

        let mut no_kind = BuddySettings::default();
        no_kind.humor_enabled = false;
        no_kind.humor_level = HumorLevel::Off;
        assert_eq!(
            chat_reaction_candidate(
                &thread,
                "please write a friendly greeting for this tiny helper",
                Some(&no_kind),
            ),
            Err(ChatReactionSkipReason::NoReactionKind)
        );
    }

    #[test]
    fn prepare_analysis_text_redacts_secret() {
        let raw = "use Bearer sk-MYSECRET123 for auth in the function call";
        let result = prepare_analysis_text(raw).unwrap();
        assert!(
            !result.contains("sk-MYSECRET123"),
            "raw secret must not appear"
        );
        assert!(
            !result.contains("MYSECRET123"),
            "partial secret must not appear"
        );
    }

    #[test]
    fn prepare_analysis_text_rejects_short() {
        assert!(prepare_analysis_text("too short").is_none());
    }

    #[test]
    fn prepare_analysis_text_truncates_long_input() {
        let raw = "word ".repeat(300);
        let result = prepare_analysis_text(&raw).unwrap();
        assert!(result.chars().count() <= ANALYSIS_TEXT_MAX_CHARS);
    }

    #[test]
    fn classify_bug_wins_over_humor_and_insight() {
        let s = BuddySettings::default();
        for text in [
            "there is an error in the refactor plan",
            "please plan around the crash while we iterate again",
        ] {
            let reaction = classify_chat_reaction(text, &s).unwrap();
            assert_eq!(
                reaction.kind,
                ChatReactionKind::BugCandidate,
                "text: {text}"
            );
        }
    }

    #[test]
    fn deterministic_humor_distribution_for_normal_interactions() {
        let s = BuddySettings::default();
        let samples = [
            "please ask about the next small step before we change the helper",
            "can you iterate on this wording and make the flow gentler",
            "let us compare these two approaches for the sidebar behavior",
            "please tweak the naming so the option feels clearer",
            "explore a smaller version of the settings panel idea",
            "plan the next checkpoint for this cleanup work",
            "try again with a simpler explanation of the button states",
            "walk through the tradeoff before we pick the path",
            "please summarize what changed and what to verify next",
            "can you make the response shorter and more direct",
            "let us outline a tiny migration path for this feature",
            "compare the current behavior with the expected interaction",
            "please simplify the setup sequence for a new user",
            "think through how the retry loop should feel in chat",
            "explore whether this should be a toggle or a command",
            "can you revise the plan after this feedback",
            "please make the naming friendlier without changing behavior",
            "iterate on the copy so it feels less formal",
            "what if we split this into two smaller steps",
            "please check the assumptions before we proceed",
        ];
        assert!(deterministic_humor_bucket(samples[0]));
        let kinds: Vec<ChatReactionKind> = samples
            .iter()
            .map(|text| classify_chat_reaction_kind(text, &s).unwrap())
            .collect();
        let humor_count = kinds
            .iter()
            .filter(|kind| matches!(kind, ChatReactionKind::Humor))
            .count();
        let insight_count = kinds
            .iter()
            .filter(|kind| matches!(kind, ChatReactionKind::Insight))
            .count();

        assert_eq!(humor_count, 8);
        assert_eq!(insight_count, 12);
        assert!(kinds
            .iter()
            .all(|kind| !matches!(kind, ChatReactionKind::BugCandidate)));
        assert_eq!(
            kinds,
            samples
                .iter()
                .map(|text| classify_chat_reaction_kind(text, &s).unwrap())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn classify_interaction_patterns_as_visible_non_error_reactions() {
        let s = BuddySettings::default();
        for text in [
            "can you iterate on this wording and make the answer gentler",
            "let us compare these two options before choosing one",
            "please tweak the response after this feedback",
            "what if we split this into two smaller steps",
            "try again with a simpler explanation of the sequence",
        ] {
            let reaction = classify_chat_reaction(text, &s).unwrap_or_else(|| {
                panic!("expected visible reaction for: {text}");
            });
            assert_ne!(
                reaction.kind,
                ChatReactionKind::BugCandidate,
                "text: {text}"
            );
        }
    }

    #[test]
    fn classify_common_work_messages_mix_humor_and_insight() {
        let s = BuddySettings::default();
        let mut humor = 0;
        let mut insight = 0;
        for text in [
            "please simplify the component state flow for the toolbar",
            "implement the schema cleanup for this api response",
            "rename the cache layer for the caching feature ux",
            "review the ui flow before the migration lands",
        ] {
            let reaction = classify_chat_reaction(text, &s).unwrap_or_else(|| {
                panic!("expected reaction for: {text}");
            });
            match reaction.kind {
                ChatReactionKind::Humor => humor += 1,
                ChatReactionKind::Insight => insight += 1,
                ChatReactionKind::BugCandidate => panic!("unexpected bug reaction for: {text}"),
            }
        }
        assert!(humor > 0);
        assert!(insight > 0);
    }

    #[test]
    fn classify_default_non_error_falls_back_to_insight_or_humor() {
        let s = BuddySettings::default();
        let reaction =
            classify_chat_reaction("please write a hello world example for me", &s).unwrap();
        assert!(matches!(
            reaction.kind,
            ChatReactionKind::Humor | ChatReactionKind::Insight
        ));
    }

    #[test]
    fn classify_humor_disabled_suppresses_humor_bucket() {
        let mut s = BuddySettings::default();
        s.humor_enabled = false;
        s.humor_level = HumorLevel::Normal;
        let text = "please ask about the next small step before we change the helper";

        assert!(deterministic_humor_bucket(text));
        let reaction = classify_chat_reaction(text, &s).unwrap();
        assert_eq!(reaction.kind, ChatReactionKind::Insight);
    }

    #[test]
    fn classify_none_when_humor_off_and_no_signal() {
        let mut s = BuddySettings::default();
        s.humor_level = HumorLevel::Off;
        assert!(classify_chat_reaction("please write a hello world example for me", &s).is_none());
    }

    #[test]
    fn classify_interaction_insight_even_when_humor_off() {
        let mut s = BuddySettings::default();
        s.humor_level = HumorLevel::Off;
        let reaction = classify_chat_reaction(
            "can you iterate on this wording and compare the options",
            &s,
        )
        .unwrap();
        assert_eq!(reaction.kind, ChatReactionKind::Insight);
    }

    #[test]
    fn limiter_reports_rate_limited_reason() {
        let mut lim = ChatReactionLimiter::new();
        let now = Utc::now();
        assert!(lim
            .try_allow_kind("chat-a", ChatReactionKind::Insight, now)
            .is_ok());
        assert_eq!(
            lim.try_allow_kind(
                "chat-a",
                ChatReactionKind::Insight,
                now + Duration::seconds(10),
            ),
            Err(ChatReactionSkipReason::RateLimited)
        );
    }

    #[test]
    fn reaction_event_metadata() {
        let s = BuddySettings::default();
        let analysis_text = "there is a crash in production today";
        let reaction = classify_chat_reaction(analysis_text, &s).unwrap();
        assert_eq!(reaction.kind, ChatReactionKind::BugCandidate);

        let ev = build_reaction_event("chat-1", analysis_text, &reaction);

        assert_eq!(ev.chat_id.as_deref(), Some("chat-1"));
        assert!(ev.speech_text.is_some());
        assert!(ev.ttl_ms.is_some());
        assert!(ev.ttl_ms.unwrap() > 0);
        assert_eq!(ev.bubble_policy, Some(BuddyBubblePolicy::EventOnce));
        assert_eq!(ev.signal_type, "chat_bug_candidate");

        let dedupe = ev.dedupe_key.unwrap();
        assert!(dedupe.contains("chat-1"));
        assert!(
            !dedupe.contains("crash"),
            "raw content must not appear in dedupe key"
        );
    }

    #[test]
    fn reaction_event_no_secret_in_title_or_dedupe_key() {
        let raw = "connection failed: Bearer sk-VERYSECRET crashed";
        let analysis = prepare_analysis_text(raw).unwrap();
        let s = BuddySettings::default();
        let reaction = classify_chat_reaction(&analysis, &s).unwrap();
        let ev = build_reaction_event("chat-sec", &analysis, &reaction);

        assert!(
            !ev.title.contains("sk-VERYSECRET"),
            "secret must not appear in title"
        );
        assert!(
            !ev.title.contains("VERYSECRET"),
            "secret must not appear in title"
        );
        let dedupe = ev.dedupe_key.unwrap();
        assert!(
            !dedupe.contains("sk-VERYSECRET"),
            "secret must not appear in dedupe key"
        );
        assert!(
            !dedupe.contains("VERYSECRET"),
            "secret must not appear in dedupe key"
        );
    }

    #[test]
    fn limiter_per_chat_cooldown() {
        let mut lim = ChatReactionLimiter::new();
        let now = Utc::now();
        assert!(lim.allow("chat-a", now));
        assert!(!lim.allow("chat-a", now + Duration::seconds(10)));
        assert!(lim.allow(
            "chat-a",
            now + Duration::seconds(PER_CHAT_COOLDOWN_SECS + 1)
        ));
    }

    #[test]
    fn limiter_global_hourly_cap() {
        let mut lim = ChatReactionLimiter::new();
        let now = Utc::now();
        for i in 0..GLOBAL_HOURLY_CAP {
            let chat_id = format!("chat-{i}");
            assert!(lim.allow(&chat_id, now + Duration::seconds(i64::from(i) * 300)));
        }
        let overflow_chat = format!("chat-{}", GLOBAL_HOURLY_CAP);
        assert!(!lim.allow(
            &overflow_chat,
            now + Duration::seconds(i64::from(GLOBAL_HOURLY_CAP) * 300)
        ));
    }

    #[test]
    fn limiter_resets_after_hour() {
        let mut lim = ChatReactionLimiter::new();
        let now = Utc::now();
        for i in 0..GLOBAL_HOURLY_CAP {
            let chat_id = format!("chat-reset-{i}");
            lim.allow(&chat_id, now + Duration::seconds(i64::from(i) * 300));
        }
        let after_hour = now + Duration::seconds(3601);
        assert!(lim.allow("chat-fresh", after_hour));
    }

    #[test]
    fn fallback_humor_uses_template_not_echo() {
        let input = "make this nicer please give it a better look overall";
        let text = fallback_chat_reaction_text(ChatReactionKind::Humor, input);
        assert!(
            HUMOR_LINES.contains(&text.as_str()),
            "fallback must be one of HUMOR_LINES, got: {}",
            text
        );
        assert!(
            !text.contains(input),
            "fallback must not echo the user message"
        );
    }

    #[test]
    fn humor_fallbacks_are_chaotic_gremlin_style_and_short() {
        assert!(
            HUMOR_LINES.len() >= 6,
            "humor fallbacks need enough Pixel gremlin variety"
        );
        assert!(HUMOR_LINES.iter().any(|line| line.contains("gremlin")));
        assert!(HUMOR_LINES.iter().any(|line| line.contains("Chaos")));
        assert!(HUMOR_LINES.iter().any(|line| line.contains("Goblin")));
        for line in HUMOR_LINES {
            assert!(line.chars().count() <= 120, "line too long: {line}");
        }
    }

    #[test]
    fn fallback_insight_uses_template() {
        let input = "design a new architecture for the service layer";
        let text = fallback_chat_reaction_text(ChatReactionKind::Insight, input);
        assert!(
            INSIGHT_LINES.contains(&text.as_str()),
            "fallback must be one of INSIGHT_LINES, got: {}",
            text
        );
        assert!(
            !text.contains(input),
            "fallback must not echo the user message"
        );
    }

    #[test]
    fn fallback_bug_uses_template() {
        let input = "the app crashed on save every time I try";
        let text = fallback_chat_reaction_text(ChatReactionKind::BugCandidate, input);
        assert!(
            BUG_LINES.contains(&text.as_str()),
            "fallback must be one of BUG_LINES, got: {}",
            text
        );
        assert!(
            !text.contains(input),
            "fallback must not echo the user message"
        );
    }

    #[test]
    fn generated_speech_echoing_long_user_phrase_falls_back() {
        let analysis =
            "please refactor the private customer import pipeline for northwind accounts";
        let generated = "Tiny note: private customer import pipeline may need one checkpoint.";
        let text = safe_chat_reaction_speech_text(ChatReactionKind::Insight, analysis, generated);

        assert!(INSIGHT_LINES.contains(&text.as_str()));
        assert!(!text
            .to_lowercase()
            .contains("private customer import pipeline"));
        assert!(text.chars().count() <= CHAT_REACTION_SPEECH_MAX_CHARS);
    }

    #[test]
    fn generated_speech_echoing_two_word_private_phrase_falls_back() {
        for (analysis, generated, echoed) in [
            (
                "please keep the Acme roadmap details private while planning",
                "Tiny signal: Acme roadmap deserves a checkpoint.",
                "acme roadmap",
            ),
            (
                "please keep the Acme beta details private while planning",
                "Acme beta needs tiny gremlin gloves.",
                "acme beta",
            ),
            (
                "please keep the Acme Q4 details private while planning",
                "Acme Q4 needs tiny gremlin gloves.",
                "acme q4",
            ),
            (
                "please keep the Nova plan details private while planning",
                "Nova plan deserves a checkpoint.",
                "nova plan",
            ),
            (
                "please keep Acme’s roadmap details private while planning",
                "Acme roadmap deserves a checkpoint.",
                "acme roadmap",
            ),
            (
                "review the Northwind accounts import without leaking names",
                "Northwind accounts look like they need tiny gremlin gloves.",
                "northwind accounts",
            ),
            (
                "compare the customer import behavior before we pick a fix",
                "Customer import might want one assumption check.",
                "customer import",
            ),
        ] {
            let text =
                safe_chat_reaction_speech_text(ChatReactionKind::Insight, analysis, generated);

            assert!(
                INSIGHT_LINES.contains(&text.as_str()),
                "expected fallback for {echoed}, got: {text}"
            );
            assert!(!text.to_lowercase().contains(echoed));
            assert!(text.chars().count() <= CHAT_REACTION_SPEECH_MAX_CHARS);
        }
    }

    #[test]
    fn generated_speech_echoing_non_ascii_private_phrase_falls_back() {
        for (analysis, generated, echoed) in [
            (
                "please summarize the 東京 roadmap without repeating private names",
                "Tiny signal: 東京 roadmap needs snack-sized caution.",
                "東京 roadmap",
            ),
            (
                "please keep 山田 private while planning the helper response",
                "Tiny signal: 山田 needs snack-sized caution.",
                "山田",
            ),
        ] {
            let text =
                safe_chat_reaction_speech_text(ChatReactionKind::Insight, analysis, generated);

            assert!(
                INSIGHT_LINES.contains(&text.as_str()),
                "expected fallback for {echoed}, got: {text}"
            );
            assert!(!text.to_lowercase().contains(echoed));
            assert!(text.chars().count() <= CHAT_REACTION_SPEECH_MAX_CHARS);
        }
    }

    #[test]
    fn generated_speech_echoing_mixed_unicode_ascii_identifier_falls_back() {
        let analysis = "please debug ProjectΔ import while keeping the identifier private";
        let generated = "Tiny alarm: ProjectΔ import is doing suspicious parkour.";
        let text =
            safe_chat_reaction_speech_text(ChatReactionKind::BugCandidate, analysis, generated);

        assert!(BUG_LINES.contains(&text.as_str()));
        assert!(!text.to_lowercase().contains("projectδ import"));
        assert!(text.chars().count() <= CHAT_REACTION_SPEECH_MAX_CHARS);
    }

    #[test]
    fn generic_buddy_phrasing_with_common_words_does_not_fallback() {
        let analysis = "please compare the options and keep the next step tidy";
        let generated = "Tiny signal: compare options before picking one small step.";
        let text = sanitize_chat_reaction_speech_text(generated, analysis).unwrap();

        assert_eq!(text, generated);
        assert!(text.chars().count() <= CHAT_REACTION_SPEECH_MAX_CHARS);
    }

    #[test]
    fn generated_speech_is_capped() {
        let analysis = "please compare the options and keep the next step tidy";
        let generated = "a".repeat(CHAT_REACTION_SPEECH_MAX_CHARS + 80);
        let text = sanitize_chat_reaction_speech_text(&generated, analysis).unwrap();

        assert_eq!(text.chars().count(), CHAT_REACTION_SPEECH_MAX_CHARS);
        assert!(!text.contains('\n'));
        assert!(!text.contains('\r'));
    }

    #[test]
    fn generated_speech_multiline_becomes_one_line() {
        let analysis = "please iterate on the sidebar interaction and compare the choices";
        let text = sanitize_chat_reaction_speech_text(
            "Tiny signal:\ncompare gently\r\nthen pick one step.",
            analysis,
        )
        .unwrap();

        assert_eq!(text, "Tiny signal: compare gently then pick one step.");
        assert!(!text.contains('\n'));
        assert!(!text.contains('\r'));
        assert!(text.chars().count() <= CHAT_REACTION_SPEECH_MAX_CHARS);
    }

    #[test]
    fn generated_speech_with_redacted_marker_falls_back() {
        let analysis = "please debug auth flow without storing secret material";
        for generated in [
            "Tiny alarm: token=[REDACTED] is doing suspicious parkour.",
            "Tiny alarm: token=[redacted] is doing suspicious parkour.",
            "Tiny alarm: token=[Redacted] is doing suspicious parkour.",
            "Tiny alarm: token=<redacted> is doing suspicious parkour.",
            "Tiny alarm: token=&lt;redacted&gt; is doing suspicious parkour.",
        ] {
            let text =
                safe_chat_reaction_speech_text(ChatReactionKind::BugCandidate, analysis, generated);

            assert!(
                BUG_LINES.contains(&text.as_str()),
                "expected fallback for {generated}, got: {text}"
            );
            assert!(!text.to_lowercase().contains("redacted"));
            assert!(text.chars().count() <= CHAT_REACTION_SPEECH_MAX_CHARS);
        }
    }

    #[test]
    fn generated_speech_with_secret_redaction_falls_back() {
        let analysis = "please debug auth flow without storing secret material";
        let generated = "Tiny alarm: Bearer sk-VERYSECRET1234567890 is wobbling.";
        let text =
            safe_chat_reaction_speech_text(ChatReactionKind::BugCandidate, analysis, generated);

        assert!(BUG_LINES.contains(&text.as_str()));
        assert!(!text.contains("VERYSECRET"));
        assert!(!text.contains("[REDACTED"));
        assert!(text.chars().count() <= CHAT_REACTION_SPEECH_MAX_CHARS);
    }

    #[test]
    fn fallback_text_under_max_and_does_not_echo_input() {
        let input = "customer acme roadmap migration plan should stay private";
        for kind in [
            ChatReactionKind::Humor,
            ChatReactionKind::Insight,
            ChatReactionKind::BugCandidate,
        ] {
            let text = fallback_chat_reaction_text(kind, input);
            assert!(text.chars().count() <= CHAT_REACTION_SPEECH_MAX_CHARS);
            assert!(!text.to_lowercase().contains("customer acme"));
            assert!(!text.to_lowercase().contains("roadmap migration"));
            assert!(!text.contains('\n'));
            assert!(!text.contains('\r'));
        }
    }

    #[test]
    fn keywords_avoid_false_positive_debug_latest_contest() {
        let s = BuddySettings::default();
        let reaction = classify_chat_reaction(
            "please look at the latest debug output and run the contest",
            &s,
        )
        .unwrap();
        assert_ne!(
            reaction.kind,
            ChatReactionKind::BugCandidate,
            "debug/latest/contest must not trigger BugCandidate"
        );
    }

    #[test]
    fn insight_short_keywords_require_exact_tokens() {
        let mut s = BuddySettings::default();
        s.humor_level = HumorLevel::Off;
        for text in [
            "please write a quick guide about apical history",
            "please decide how to handle fluid layouts later",
            "please make a useful utility helper for the sidebar",
            "please think about the next xenon parser experiment",
        ] {
            assert_eq!(
                classify_chat_reaction_kind(text, &s),
                None,
                "short insight keyword must not overmatch: {text}"
            );
        }
    }

    #[test]
    fn keywords_match_multi_word_not_working_phrases() {
        let s = BuddySettings::default();
        let reaction = classify_chat_reaction("the upload is not working anymore", &s).unwrap();
        assert_eq!(
            reaction.kind,
            ChatReactionKind::BugCandidate,
            "'not working' must classify as BugCandidate"
        );
    }

    #[test]
    fn reaction_event_speech_text_is_buddy_template() {
        let s = BuddySettings::default();
        let input = "design a new architecture for the service layer";
        let reaction = classify_chat_reaction(input, &s).unwrap();
        assert_eq!(reaction.kind, ChatReactionKind::Insight);
        let ev = build_reaction_event("chat-template", input, &reaction);
        let speech = ev.speech_text.expect("speech_text must be set");
        assert!(
            INSIGHT_LINES.contains(&speech.as_str()),
            "speech_text must be one of INSIGHT_LINES, got: {}",
            speech
        );
        assert!(
            !speech.contains(input),
            "speech_text must not contain raw analysis text"
        );
        assert_eq!(
            speech, reaction.text,
            "speech_text must equal reaction.text"
        );
    }

    #[test]
    fn reaction_event_sanitizes_unsafe_speech_text() {
        let analysis = "please refactor private customer import pipeline for northwind accounts";
        let reaction = ChatReaction {
            kind: ChatReactionKind::Insight,
            text: "private customer import pipeline needs a checkpoint".to_string(),
        };
        let ev = build_reaction_event("chat-safe", analysis, &reaction);
        let speech = ev.speech_text.expect("speech_text must be set");

        assert!(INSIGHT_LINES.contains(&speech.as_str()));
        assert!(!speech.to_lowercase().contains("private customer"));
        assert_eq!(ev.source, "chat_reactions");
        assert_eq!(ev.chat_id.as_deref(), Some("chat-safe"));
        assert!(ev.bubble_policy.is_some());
    }
}
