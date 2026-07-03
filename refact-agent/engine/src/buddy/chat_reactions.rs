use chrono::Utc;
use tracing::debug;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::call_validation::ChatContent;
use crate::chat::types::ThreadParams;

use super::settings::{BuddySettings, HumorLevel};
use super::types::{BuddyBubblePolicy, BuddyChatPhraseBank, BuddyPersonalityProfile, BuddyRuntimeEvent};
use super::voice_service::{voice_service, ChatReactionSpeechIntent, VoiceCtx};

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
    use chrono::Duration;

    fn classify_chat_reaction(text: &str, settings: &BuddySettings) -> ChatReaction {
        let kind = classify_chat_reaction_kind(text, settings);
        ChatReaction {
            text: fallback_chat_reaction_text(kind.clone(), text),
            kind,
        }
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
            let reaction = classify_chat_reaction(text, &s);
            assert_eq!(
                reaction.kind,
                ChatReactionKind::BugCandidate,
                "text: {text}"
            );
        }
    }

    #[test]
    fn deterministic_humor_distribution_for_generic_messages() {
        let s = BuddySettings::default();
        let samples = [
            "please write a hello world example for me",
            "please make a small helper that formats the label",
            "show me a short example of this pattern",
            "please draft a friendly greeting for the onboarding screen",
            "what is a simple way to arrange these notes",
            "please explain this part in plain words",
            "please create a tiny example with two inputs",
            "show a compact version of the same idea",
            "please make this text shorter for the tooltip",
            "please give me a starter snippet for this",
            "please outline a lightweight helper for this page",
            "write a concise description for the button",
            "please suggest a name for this option",
            "please sketch a small example without extra details",
            "show me how to phrase this as a checklist",
            "please make the intro paragraph friendlier",
            "please turn this note into a short message",
            "write a quick placeholder for this empty space",
            "please summarize this in one sentence",
            "please give me a tiny reusable example",
        ];
        let humor_bucket_count = samples
            .iter()
            .filter(|text| deterministic_humor_bucket(text))
            .count();
        let kinds: Vec<ChatReactionKind> = samples
            .iter()
            .map(|text| classify_chat_reaction_kind(text, &s))
            .collect();
        let humor_count = kinds
            .iter()
            .filter(|kind| matches!(kind, ChatReactionKind::Humor))
            .count();
        let ambient_count = kinds
            .iter()
            .filter(|kind| matches!(kind, ChatReactionKind::Ambient))
            .count();

        assert_eq!(humor_count, humor_bucket_count);
        assert!(humor_count > 0);
        assert!(ambient_count > 0);
        assert_eq!(humor_count + ambient_count, samples.len());
        assert!(kinds.iter().all(|kind| !matches!(
            kind,
            ChatReactionKind::BugCandidate | ChatReactionKind::Insight
        )));
        assert_eq!(
            kinds,
            samples
                .iter()
                .map(|text| classify_chat_reaction_kind(text, &s))
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
            let reaction = classify_chat_reaction(text, &s);
            assert_ne!(
                reaction.kind,
                ChatReactionKind::BugCandidate,
                "text: {text}"
            );
        }
    }

    #[test]
    fn classify_common_work_messages_as_insight() {
        let s = BuddySettings::default();
        let mut humor = 0;
        let mut insight = 0;
        for text in [
            "please simplify the component state flow for the toolbar",
            "implement the schema cleanup for this api response",
            "rename the cache layer for the caching feature ux",
            "review the ui flow before the migration lands",
        ] {
            let reaction = classify_chat_reaction(text, &s);
            match reaction.kind {
                ChatReactionKind::Humor => humor += 1,
                ChatReactionKind::Insight => insight += 1,
                ChatReactionKind::BugCandidate | ChatReactionKind::Ambient => {
                    panic!("unexpected non-work reaction for: {text}")
                }
            }
        }
        assert_eq!(humor, 0);
        assert_eq!(insight, 4);
    }

    #[test]
    fn classify_default_non_error_falls_back_to_ambient() {
        let s = BuddySettings::default();
        let reaction = classify_chat_reaction("please write a hello world example for me", &s);
        assert_eq!(reaction.kind, ChatReactionKind::Ambient);
    }

    #[test]
    fn classify_humor_disabled_suppresses_humor_bucket() {
        let mut s = BuddySettings::default();
        s.humor_enabled = false;
        s.humor_level = HumorLevel::Normal;
        let text = "please ask about the next small step before we change the helper";

        assert!(deterministic_humor_bucket(text));
        let reaction = classify_chat_reaction(text, &s);
        assert_eq!(reaction.kind, ChatReactionKind::Insight);
    }

    #[test]
    fn classify_bucketed_interaction_as_humor_when_humor_enabled() {
        let s = BuddySettings::default();
        let text = "please ask about the next small step before we change the helper";

        assert!(deterministic_humor_bucket(text));
        let reaction = classify_chat_reaction(text, &s);
        assert_eq!(reaction.kind, ChatReactionKind::Humor);
    }

    #[test]
    fn classify_ambient_when_humor_off_and_no_signal() {
        let mut s = BuddySettings::default();
        s.humor_level = HumorLevel::Off;
        let reaction = classify_chat_reaction("please write a hello world example for me", &s);
        assert_eq!(reaction.kind, ChatReactionKind::Ambient);
    }

    #[test]
    fn classify_interaction_insight_even_when_humor_off() {
        let mut s = BuddySettings::default();
        s.humor_level = HumorLevel::Off;
        let reaction = classify_chat_reaction(
            "can you iterate on this wording and compare the options",
            &s,
        );
        assert_eq!(reaction.kind, ChatReactionKind::Insight);
    }

    #[test]
    fn reaction_event_metadata() {
        let s = BuddySettings::default();
        let analysis_text = "there is a crash in production today";
        let reaction = classify_chat_reaction(analysis_text, &s);
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
    fn phrase_bank_line_prefers_generated_daily_copy() {
        let bank = BuddyChatPhraseBank {
            day: Utc::now().date_naive().to_string(),
            generated_at: Utc::now().to_rfc3339(),
            evidence_summary: "recent project signals".to_string(),
            lines: vec![super::super::types::BuddyChatPhrase {
                kind: "humor".to_string(),
                text: "Today's diff goblin is juggling tidy breadcrumbs.".to_string(),
            }],
        };

        let line = phrase_bank_line(
            &bank,
            ChatReactionKind::Humor,
            "please make this helper nicer without echoing secrets",
        );

        assert_eq!(
            line.as_deref(),
            Some("Today's diff goblin is juggling tidy breadcrumbs.")
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
    fn fallback_ambient_uses_template() {
        let input = "please write a hello world example for me";
        let text = fallback_chat_reaction_text(ChatReactionKind::Ambient, input);
        assert!(
            AMBIENT_LINES.contains(&text.as_str()),
            "fallback must be one of AMBIENT_LINES, got: {}",
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
    fn generated_speech_with_credential_pattern_is_rejected() {
        let analysis = "please debug auth flow without storing secret material";
        let generated = "Bearer sk-VERYSECRET1234567890 while unrelated analysis continues";

        assert!(sanitize_chat_reaction_speech_text(generated, analysis).is_none());
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
    fn keywords_avoid_false_positive_debug_latest_contest() {
        let s = BuddySettings::default();
        let reaction = classify_chat_reaction(
            "please look at the latest debug output and run the contest",
            &s,
        );
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
                ChatReactionKind::Ambient,
                "short insight keyword must not overmatch insight: {text}"
            );
        }
    }

    #[test]
    fn keywords_match_multi_word_not_working_phrases() {
        let s = BuddySettings::default();
        let reaction = classify_chat_reaction("the upload is not working anymore", &s);
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
        let reaction = classify_chat_reaction(input, &s);
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
