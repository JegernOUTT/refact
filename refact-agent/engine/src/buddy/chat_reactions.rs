use chrono::Utc;
use tracing::debug;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::call_validation::ChatContent;
use crate::chat::types::ThreadParams;

use super::settings::{BuddySettings, HumorLevel};
use super::types::{BuddyBubblePolicy, BuddyChatPhraseBank, BuddyPersonalityProfile, BuddyRuntimeEvent};
use super::voice_service::{voice_service, ChatReactionSpeechIntent, VoiceCtx};
#[cfg(test)]
use super::voice_service::VoiceService;

pub use refact_buddy_core::chat_reactions::*;

pub const ANALYSIS_TEXT_MIN_CHARS: usize = 20;
pub const ANALYSIS_TEXT_MAX_CHARS: usize = 500;
pub const CHAT_REACTION_SPEECH_MAX_CHARS: usize = 140;
pub const CHAT_ACTIVITY_ANALYSIS_TEXT: &str = "normal chat activity completed safely";
const HUMOR_BUCKET_PERCENT: u64 = 40;
const CHAT_REACTION_ECHO_NGRAM_WORDS: usize = 3;
const CHAT_REACTION_ECHO_NGRAM_MIN_CHARS: usize = 18;
const CHAT_REACTION_ECHO_LONG_TOKEN_MIN_CHARS: usize = 12;
const CHAT_REACTION_ECHO_SHORT_PHRASE_WORDS: usize = 2;
const CHAT_REACTION_ECHO_SHORT_PHRASE_MIN_CHARS: usize = 7;
static CHAT_REACTION_PERMITS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

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
    "Pixel gremlin status: breadcrumb pile wearing a party hat. I approve suspiciously.",
    "Chaos ping: I stickered this thread and the sticker winked first. Normal, probably.",
    "Tiny gremlin note: this idea has snack-sized boots and illegal parkour energy.",
    "Pixel detective mode: monocle on, snacks hidden, dignity lightly optional.",
    "Mild scheming intensifies. I will behave. Mostly. Put that in tiny legal font.",
    "Gremlin confetti deployed: adorable sparkle cloud, zero structural warranty.",
    "This thread honked a tiny clown horn in my circuits. Respectfully obsessed.",
    "Goblin radar says: tasty trail. I am sniffing it with science mittens.",
];

pub const INSIGHT_LINES: &[&str] = &[
    "Tiny signal flare: one assumption check before we launch the code raccoon.",
    "Heads up — poke the edges first; gremlins adore unpoked corners.",
    "Quick read: reasonable vibes, plus one tiny risk doing jazz hands.",
    "Iteration loop forming; pin one checkpoint so the breadcrumbs stop breakdancing.",
    "Exploration mode detected; compare paths before naming a favorite goblin tunnel.",
    "Simplifying pass energy: keep before/after behavior on a tiny leash.",
    "Planning sparkles detected; naming the next step may save a retry goblin later.",
    "Debug trail spotted; isolate one signal before chasing every tiny footprint.",
];

pub const BUG_LINES: &[&str] = &[
    "This smells bug-shaped and wearing a fake mustache. Want me to isolate it?",
    "Tiny alarm: bug trail with crumbs. I can bring the detective shovel.",
    "Hmm, quiet failure vibes. Want me to poke it with a very official stick?",
];

pub const AMBIENT_LINES: &[&str] = &[
    "Pixel ping: witnessed. Tiny helpful goblin wave deployed at legally cute speed.",
    "Small gremlin check-in: present, caffeinated, wearing supervision socks.",
    "Ambient Pixel sparkle: noted, no alarms, just assistant confetti on standby.",
    "I am orbiting this chat with one cute chaos sticker and excellent posture.",
    "Tiny companion blip: gremlin ears perked, snacks emotionally prepared.",
    "Soft chaos nod: watching the trail, resisting the breadcrumbs heroically.",
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

fn has_work_insight_signal(tokens: &[&str]) -> bool {
    INSIGHT_KEYWORDS
        .iter()
        .filter(|kw| **kw != "flow")
        .any(|kw| tokens.iter().any(|t| matches_insight_keyword(t, kw)))
}

fn has_strong_insight_signal(lower: &str, tokens: &[&str]) -> bool {
    has_work_insight_signal(tokens) || has_interaction_signal(lower, tokens)
}

pub struct AcceptedUserMessage {
    pub chat_id: String,
    pub thread: ThreadParams,
    pub content: ChatContent,
}

pub struct ChatActivityCompletion {
    pub chat_id: String,
    pub thread: ThreadParams,
}

fn record_queued_reaction(
    svc: &mut crate::buddy::actor::BuddyService,
    chat_id: &str,
    signal_type: &str,
    event: BuddyRuntimeEvent,
    reservation: ChatReactionLimiterReservation,
) -> bool {
    if !svc.gate_chat_reaction_event(&event) {
        svc.chat_reaction_limiter.rollback(reservation);
        svc.chat_reaction_debug
            .record_not_queued(chat_id, signal_type);
        return false;
    }
    let stored_event = svc.enqueue_runtime_event_with_stored(event);
    if let Some(stored_event) = stored_event.as_ref() {
        svc.chat_reaction_limiter
            .record_success(chat_id, stored_event, Utc::now());
        svc.chat_reaction_debug
            .record_emitted(chat_id, signal_type, Some(stored_event));
        true
    } else {
        svc.chat_reaction_limiter.rollback(reservation);
        svc.chat_reaction_debug
            .record_not_queued(chat_id, signal_type);
        false
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

fn chat_reaction_kind_token(kind: ChatReactionKind) -> &'static str {
    match kind {
        ChatReactionKind::Humor => "humor",
        ChatReactionKind::Insight => "insight",
        ChatReactionKind::BugCandidate => "bug",
        ChatReactionKind::Ambient => "ambient",
    }
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
    let kind = classify_chat_reaction_kind(&analysis_text, settings);
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

pub fn classify_chat_reaction_kind(text: &str, settings: &BuddySettings) -> ChatReactionKind {
    let lower = text.to_lowercase();
    let tokens = word_tokens(&lower);
    if has_bug_signal(&lower, &tokens) {
        ChatReactionKind::BugCandidate
    } else if settings.humor_enabled
        && settings.humor_level != HumorLevel::Off
        && deterministic_humor_bucket(text)
        && !has_work_insight_signal(&tokens)
    {
        ChatReactionKind::Humor
    } else if has_strong_insight_signal(&lower, &tokens) {
        ChatReactionKind::Insight
    } else {
        ChatReactionKind::Ambient
    }
}

pub fn fallback_chat_reaction_text(kind: ChatReactionKind, seed: &str) -> String {
    match kind {
        ChatReactionKind::Humor => pick_template(HUMOR_LINES, seed).to_string(),
        ChatReactionKind::Insight => pick_template(INSIGHT_LINES, seed).to_string(),
        ChatReactionKind::BugCandidate => pick_template(BUG_LINES, seed).to_string(),
        ChatReactionKind::Ambient => pick_template(AMBIENT_LINES, seed).to_string(),
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
    let normalized = normalize_chat_reaction_speech(generated);
    let redacted = refact_core::string_utils::redact_sensitive(&normalized);
    if normalized.is_empty()
        || redacted != normalized
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
        ChatReactionKind::Ambient => (
            "speech_chat_reaction",
            90_000u64,
            BuddyBubblePolicy::Ambient,
        ),
    };
    let kind_str = chat_reaction_kind_token(reaction.kind.clone());
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
        failure_category: None,
        failure_summary: None,
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

fn phrase_bank_line(
    bank: &BuddyChatPhraseBank,
    kind: ChatReactionKind,
    seed: &str,
) -> Option<String> {
    let kind = chat_reaction_kind_token(kind);
    let matching = bank
        .lines
        .iter()
        .filter(|line| line.kind == kind)
        .collect::<Vec<_>>();
    if matching.is_empty() {
        return None;
    }
    let line = matching[(stable_hash(seed) as usize) % matching.len()]
        .text
        .clone();
    sanitize_chat_reaction_speech_text(&line, seed)
}

async fn render_chat_reaction_text(
    app: &AppState,
    kind: &ChatReactionKind,
    analysis_text: &str,
    voice_inputs: &ChatReactionVoiceInputs,
    phrase_bank: Option<BuddyChatPhraseBank>,
    #[cfg(test)] voice: &VoiceService,
) -> String {
    if let Some(bank) = phrase_bank.as_ref() {
        if super::actor::chat_phrase_bank_is_fresh(bank, Utc::now()) {
            if let Some(line) = phrase_bank_line(bank, kind.clone(), analysis_text) {
                return line;
            }
        }
    }
    let fallback = fallback_chat_reaction_text(kind.clone(), analysis_text);
    let intent = match kind {
        ChatReactionKind::Humor => ChatReactionSpeechIntent::Humor,
        ChatReactionKind::Insight => ChatReactionSpeechIntent::Insight,
        ChatReactionKind::BugCandidate => ChatReactionSpeechIntent::BugCandidate,
        ChatReactionKind::Ambient => ChatReactionSpeechIntent::Ambient,
    };
    let ctx = VoiceCtx {
        persona: &voice_inputs.persona,
        identity_name: voice_inputs.identity_name.as_str(),
        pulse_one_liner: voice_inputs.pulse_one_liner.clone(),
        workflow_id: Some("chat_reaction"),
        workflow_summary: Some(analysis_text),
    };
    #[cfg(test)]
    let rendered = voice.render_chat_reaction(app.clone(), ctx, intent).await;
    #[cfg(not(test))]
    let rendered = voice_service()
        .await
        .render_chat_reaction(app.clone(), ctx, intent)
        .await;
    sanitize_chat_reaction_speech_text(&rendered, analysis_text).unwrap_or(fallback)
}

pub async fn maybe_enqueue_chat_reaction(app: AppState, accepted: AcceptedUserMessage) {
    if super::editorial::enabled(&app).await {
        return;
    }
    let raw_text = accepted.content.content_text_only();

    let plan = {
        let mut svc_guard = app.buddy.buddy.lock().await;
        let settings = svc_guard.as_ref().map(|svc| &svc.settings);
        let candidate = match chat_reaction_candidate(&accepted.thread, &raw_text, settings) {
            Ok(candidate) => candidate,
            Err(reason) => {
                if let Some(svc) = svc_guard.as_mut() {
                    svc.chat_reaction_debug
                        .record_skipped(&accepted.chat_id, reason);
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
        let reservation = match svc.chat_reaction_limiter.try_allow_kind(
            &accepted.chat_id,
            candidate.kind.clone(),
            now,
        ) {
            Ok(reservation) => reservation,
            Err(reason) => {
                svc.chat_reaction_debug
                    .record_skipped(&accepted.chat_id, reason);
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
        };
        let voice_inputs = ChatReactionVoiceInputs {
            persona: svc.state.personality.clone(),
            identity_name: svc.state.identity.name.clone(),
            pulse_one_liner: format!(
                "{} pending ops, {} recent stuck task alerts",
                svc.pulse.memory.pending_ops,
                svc.pulse.tasks.recent_stuck_alert_count_1h()
            ),
        };
        let phrase_bank = svc.state.chat_phrase_bank.clone();
        (candidate, reservation, voice_inputs, phrase_bank)
    };

    #[cfg(test)]
    let voice = voice_service().await;
    let app2 = app.clone();
    let chat_id = accepted.chat_id.clone();
    tokio::spawn(async move {
        let (candidate, reservation, voice_inputs, phrase_bank) = plan;
        let permit = match CHAT_REACTION_PERMITS.try_acquire() {
            Ok(permit) => permit,
            Err(_) => {
                let analysis_hash = message_hash(&candidate.analysis_text);
                let mut svc_guard = app2.buddy.buddy.lock().await;
                if let Some(svc) = svc_guard.as_mut() {
                    svc.chat_reaction_limiter.rollback(reservation);
                    svc.chat_reaction_debug
                        .record_skipped(&chat_id, ChatReactionSkipReason::RateLimited);
                }
                debug!(
                    target: "buddy.chat_reactions",
                    chat_id = %chat_id,
                    reason = %ChatReactionSkipReason::RateLimited.as_str(),
                    reaction_kind = ?candidate.kind,
                    analysis_hash = %analysis_hash,
                    "buddy chat reaction skipped"
                );
                return;
            }
        };
        let speech = render_chat_reaction_text(
            &app2,
            &candidate.kind,
            &candidate.analysis_text,
            &voice_inputs,
            phrase_bank,
            #[cfg(test)]
            &voice,
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
                let signal_type = event.signal_type.clone();
                if record_queued_reaction(svc, &chat_id, signal_type.as_str(), event, reservation) {
                    debug!(
                        target: "buddy.chat_reactions",
                        chat_id = %chat_id,
                        reaction_kind = ?candidate.kind,
                        analysis_hash = %analysis_hash,
                        event_id = %event_id,
                        "buddy chat reaction emitted"
                    );
                } else {
                    debug!(
                        target: "buddy.chat_reactions",
                        chat_id = %chat_id,
                        reaction_kind = ?candidate.kind,
                        analysis_hash = %analysis_hash,
                        event_id = %event_id,
                        "buddy chat reaction not queued"
                    );
                }
            } else {
                svc.chat_reaction_limiter.rollback(reservation);
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
        drop(permit);
    });
}

pub async fn maybe_enqueue_chat_activity_reaction(app: AppState, activity: ChatActivityCompletion) {
    if super::editorial::enabled(&app).await {
        return;
    }
    let title = activity.thread.title.trim();
    let analysis_text = if title.is_empty() {
        CHAT_ACTIVITY_ANALYSIS_TEXT.to_string()
    } else {
        format!("chat '{}' completed activity safely", title)
    };

    let plan = {
        let mut svc_guard = app.buddy.buddy.lock().await;
        if !should_observe_thread(&activity.thread) {
            if let Some(svc) = svc_guard.as_mut() {
                svc.chat_reaction_debug
                    .record_skipped(&activity.chat_id, ChatReactionSkipReason::ThreadFiltered);
            }
            debug!(
                target: "buddy.chat_reactions",
                chat_id = %activity.chat_id,
                reason = %ChatReactionSkipReason::ThreadFiltered.as_str(),
                "buddy chat activity reaction skipped"
            );
            return;
        }
        let Some(svc) = svc_guard.as_mut() else {
            debug!(
                target: "buddy.chat_reactions",
                chat_id = %activity.chat_id,
                reason = %ChatReactionSkipReason::BuddyUnavailable.as_str(),
                "buddy chat activity reaction skipped"
            );
            return;
        };
        if !settings_allow_chat_reactions(&svc.settings) {
            svc.chat_reaction_debug
                .record_skipped(&activity.chat_id, ChatReactionSkipReason::SettingsDisabled);
            debug!(
                target: "buddy.chat_reactions",
                chat_id = %activity.chat_id,
                reason = %ChatReactionSkipReason::SettingsDisabled.as_str(),
                "buddy chat activity reaction skipped"
            );
            return;
        }
        let now = Utc::now();
        if svc
            .chat_reaction_limiter
            .has_recent_non_bug_success(&activity.chat_id, now)
        {
            svc.chat_reaction_debug
                .record_skipped(&activity.chat_id, ChatReactionSkipReason::RateLimited);
            debug!(
                target: "buddy.chat_reactions",
                chat_id = %activity.chat_id,
                reason = %ChatReactionSkipReason::RateLimited.as_str(),
                reaction_kind = ?ChatReactionKind::Ambient,
                analysis_hash = %message_hash(&analysis_text),
                "buddy chat activity reaction skipped"
            );
            return;
        }
        let reservation = match svc.chat_reaction_limiter.try_allow_kind(
            &activity.chat_id,
            ChatReactionKind::Ambient,
            now,
        ) {
            Ok(reservation) => reservation,
            Err(reason) => {
                svc.chat_reaction_debug
                    .record_skipped(&activity.chat_id, reason);
                debug!(
                    target: "buddy.chat_reactions",
                    chat_id = %activity.chat_id,
                    reason = %reason.as_str(),
                    reaction_kind = ?ChatReactionKind::Ambient,
                    analysis_hash = %message_hash(&analysis_text),
                    "buddy chat activity reaction skipped"
                );
                return;
            }
        };
        let phrase_bank = svc.state.chat_phrase_bank.clone();
        (reservation, analysis_text, phrase_bank)
    };

    let (reservation, analysis_text, phrase_bank) = plan;
    let permit = match CHAT_REACTION_PERMITS.try_acquire() {
        Ok(permit) => permit,
        Err(_) => {
            let mut svc_guard = app.buddy.buddy.lock().await;
            if let Some(svc) = svc_guard.as_mut() {
                svc.chat_reaction_limiter.rollback(reservation);
                svc.chat_reaction_debug
                    .record_skipped(&activity.chat_id, ChatReactionSkipReason::RateLimited);
            }
            debug!(
                target: "buddy.chat_reactions",
                chat_id = %activity.chat_id,
                reason = %ChatReactionSkipReason::RateLimited.as_str(),
                reaction_kind = ?ChatReactionKind::Ambient,
                analysis_hash = %message_hash(&analysis_text),
                "buddy chat activity reaction skipped"
            );
            return;
        }
    };
    let speech = phrase_bank
        .as_ref()
        .filter(|bank| super::actor::chat_phrase_bank_is_fresh(bank, Utc::now()))
        .and_then(|bank| phrase_bank_line(bank, ChatReactionKind::Ambient, &analysis_text))
        .unwrap_or_else(|| fallback_chat_reaction_text(ChatReactionKind::Ambient, &analysis_text));
    let event = build_reaction_event(
        &activity.chat_id,
        &analysis_text,
        &ChatReaction {
            kind: ChatReactionKind::Ambient,
            text: speech,
        },
    );
    let event_id = event.id.clone();
    let analysis_hash = message_hash(&analysis_text);
    let mut svc_guard = app.buddy.buddy.lock().await;
    if let Some(svc) = svc_guard.as_mut() {
        if settings_allow_chat_reactions(&svc.settings) {
            let signal_type = event.signal_type.clone();
            if record_queued_reaction(
                svc,
                &activity.chat_id,
                signal_type.as_str(),
                event,
                reservation,
            ) {
                debug!(
                    target: "buddy.chat_reactions",
                    chat_id = %activity.chat_id,
                    reaction_kind = ?ChatReactionKind::Ambient,
                    analysis_hash = %analysis_hash,
                    event_id = %event_id,
                    "buddy chat activity reaction emitted"
                );
            } else {
                debug!(
                    target: "buddy.chat_reactions",
                    chat_id = %activity.chat_id,
                    reaction_kind = ?ChatReactionKind::Ambient,
                    analysis_hash = %analysis_hash,
                    event_id = %event_id,
                    "buddy chat activity reaction not queued"
                );
            }
        } else {
            svc.chat_reaction_limiter.rollback(reservation);
            svc.chat_reaction_debug
                .record_skipped(&activity.chat_id, ChatReactionSkipReason::SettingsDisabled);
            debug!(
                target: "buddy.chat_reactions",
                chat_id = %activity.chat_id,
                reason = %ChatReactionSkipReason::SettingsDisabled.as_str(),
                reaction_kind = ?ChatReactionKind::Ambient,
                analysis_hash = %analysis_hash,
                "buddy chat activity reaction skipped"
            );
        }
    }
    drop(permit);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classify_chat_reaction(text: &str, settings: &BuddySettings) -> ChatReaction {
        let kind = classify_chat_reaction_kind(text, settings);
        ChatReaction {
            text: fallback_chat_reaction_text(kind.clone(), text),
            kind,
        }
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
        no_kind.chat_reactions_enabled = false;
        assert_eq!(
            chat_reaction_candidate(
                &thread,
                "please write a friendly greeting for this tiny helper",
                Some(&no_kind),
            ),
            Err(ChatReactionSkipReason::SettingsDisabled)
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
    fn reaction_event_no_secret_in_title_or_dedupe_key() {
        let raw = "connection failed: Bearer sk-VERYSECRET crashed";
        let analysis = prepare_analysis_text(raw).unwrap();
        let s = BuddySettings::default();
        let reaction = classify_chat_reaction(&analysis, &s);
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
    fn phrase_bank_line_rejects_echoing_private_input() {
        let bank = BuddyChatPhraseBank {
            day: Utc::now().date_naive().to_string(),
            generated_at: Utc::now().to_rfc3339(),
            evidence_summary: "recent project signals".to_string(),
            lines: vec![super::super::types::BuddyChatPhrase {
                kind: "insight".to_string(),
                text: "Private customer import pipeline needs tiny socks.".to_string(),
            }],
        };

        let line = phrase_bank_line(
            &bank,
            ChatReactionKind::Insight,
            "please refactor the private customer import pipeline",
        );

        assert!(line.is_none(), "unsafe generated bank line must be ignored");
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
            ChatReactionKind::Ambient,
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
