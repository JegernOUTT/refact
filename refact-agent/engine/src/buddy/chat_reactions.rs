use chrono::{DateTime, Utc};
use std::collections::HashMap;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::call_validation::ChatContent;
use crate::chat::types::ThreadParams;

use super::settings::{BuddySettings, HumorLevel};
use super::types::{BuddyBubblePolicy, BuddyRuntimeEvent};

pub const ANALYSIS_TEXT_MIN_CHARS: usize = 20;
pub const ANALYSIS_TEXT_MAX_CHARS: usize = 500;
pub const PER_CHAT_COOLDOWN_SECS: i64 = 300;
pub const GLOBAL_HOURLY_CAP: u32 = 10;

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
    "refactor",
    "performance",
    "security",
    "migrate",
    "migration",
    "design",
    "review",
    "tradeoff",
    "deprecate",
    "deprecated",
];

pub const HUMOR_LINES: &[&str] = &[
    "Tiny gremlin note: that idea has snack-sized boots — I'm watching the edges.",
    "Pixel detective mode engaged. Snacks hidden in the margins.",
    "Chaos ping: good thread, suspiciously cute.",
    "Mild scheming intensifies. I will behave. Mostly.",
];

pub const INSIGHT_LINES: &[&str] = &[
    "Tiny signal: this might want one assumption check before charging in.",
    "Heads up — worth poking the edges before committing.",
    "Quick read: looks reasonable; one small risk worth sanity-checking.",
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

pub struct AcceptedUserMessage {
    pub chat_id: String,
    pub thread: ThreadParams,
    pub content: ChatContent,
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

pub fn settings_allow_chat_reactions(settings: &BuddySettings) -> bool {
    settings.enabled
        && settings.proactive_enabled
        && settings.message_observation_enabled
        && settings.chat_reactions_enabled
        && !settings.quiet_mode
}

pub fn prepare_analysis_text(raw: &str) -> Option<String> {
    let redacted = refact_core::string_utils::redact_sensitive(raw);
    let normalized: String = redacted.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() < ANALYSIS_TEXT_MIN_CHARS {
        return None;
    }
    Some(normalized.chars().take(ANALYSIS_TEXT_MAX_CHARS).collect())
}

pub fn classify_chat_reaction(text: &str, settings: &BuddySettings) -> Option<ChatReaction> {
    let lower = text.to_lowercase();
    let tokens = word_tokens(&lower);
    let kind = if has_bug_signal(&lower, &tokens) {
        ChatReactionKind::BugCandidate
    } else if INSIGHT_KEYWORDS
        .iter()
        .any(|kw| tokens.iter().any(|t| t.starts_with(kw)))
    {
        ChatReactionKind::Insight
    } else if settings.humor_enabled && settings.humor_level != HumorLevel::Off {
        ChatReactionKind::Humor
    } else {
        return None;
    };
    let reaction_text = match kind {
        ChatReactionKind::Humor => pick_template(HUMOR_LINES, text).to_string(),
        ChatReactionKind::Insight => pick_template(INSIGHT_LINES, text).to_string(),
        ChatReactionKind::BugCandidate => pick_template(BUG_LINES, text).to_string(),
    };
    Some(ChatReaction {
        kind,
        text: reaction_text,
    })
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
        speech_text: Some(reaction.text.clone()),
        scene: None,
        duration_hint: None,
        persistent: false,
        controls: vec![],
        chat_id: Some(chat_id.to_string()),
        dismissed: false,
    }
}

pub async fn maybe_enqueue_chat_reaction(app: AppState, accepted: AcceptedUserMessage) {
    if !should_observe_thread(&accepted.thread) {
        return;
    }

    let raw_text = accepted.content.content_text_only();
    let Some(analysis) = prepare_analysis_text(&raw_text) else {
        return;
    };

    let event = {
        let mut svc_guard = app.buddy.buddy.lock().await;
        let Some(svc) = svc_guard.as_mut() else {
            return;
        };
        if !settings_allow_chat_reactions(&svc.settings) {
            return;
        }
        let Some(reaction) = classify_chat_reaction(&analysis, &svc.settings) else {
            return;
        };
        let now = chrono::Utc::now();
        if !svc.chat_reaction_limiter.allow_kind(&accepted.chat_id, reaction.kind.clone(), now) {
            return;
        }
        build_reaction_event(&accepted.chat_id, &analysis, &reaction)
    };

    let mut svc_guard = app.buddy.buddy.lock().await;
    if let Some(svc) = svc_guard.as_mut() {
        svc.enqueue_runtime_event(event);
    }
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
    pub fn allow_kind(&mut self, chat_id: &str, kind: ChatReactionKind, now: DateTime<Utc>) -> bool {
        self.per_chat_kind_last_at
            .retain(|_, last_at| (now - *last_at).num_seconds() < PER_CHAT_COOLDOWN_SECS);
        if (now - self.global_window_start).num_seconds() >= 3600 {
            self.global_hourly_count = 0;
            self.global_window_start = now;
        }
        if self.global_hourly_count >= GLOBAL_HOURLY_CAP {
            return false;
        }
        let key = (chat_id.to_string(), kind);
        if let Some(last) = self.per_chat_kind_last_at.get(&key) {
            if (now - *last).num_seconds() < PER_CHAT_COOLDOWN_SECS {
                return false;
            }
        }
        self.per_chat_kind_last_at.insert(key, now);
        self.global_hourly_count += 1;
        true
    }

    pub fn allow(&mut self, chat_id: &str, now: DateTime<Utc>) -> bool {
        self.allow_kind(chat_id, ChatReactionKind::Humor, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn settings_allow_all_defaults() {
        assert!(settings_allow_chat_reactions(&BuddySettings::default()));
    }

    #[test]
    fn settings_gate_blocks_each_toggle() {
        let mut s = BuddySettings::default();
        s.enabled = false;
        assert!(!settings_allow_chat_reactions(&s));

        let mut s = BuddySettings::default();
        s.proactive_enabled = false;
        assert!(!settings_allow_chat_reactions(&s));

        let mut s = BuddySettings::default();
        s.message_observation_enabled = false;
        assert!(!settings_allow_chat_reactions(&s));

        let mut s = BuddySettings::default();
        s.chat_reactions_enabled = false;
        assert!(!settings_allow_chat_reactions(&s));
    }

    #[test]
    fn settings_gate_blocks_quiet_mode() {
        let mut s = BuddySettings::default();
        s.quiet_mode = true;
        assert!(!settings_allow_chat_reactions(&s));
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
    fn classify_bug_wins_over_insight() {
        let s = BuddySettings::default();
        let reaction =
            classify_chat_reaction("there is an error in the refactor plan", &s).unwrap();
        assert_eq!(reaction.kind, ChatReactionKind::BugCandidate);
    }

    #[test]
    fn classify_insight_wins_over_humor() {
        let s = BuddySettings::default();
        let reaction =
            classify_chat_reaction("let us design a new system architecture", &s).unwrap();
        assert_eq!(reaction.kind, ChatReactionKind::Insight);
    }

    #[test]
    fn classify_humor_fallback() {
        let s = BuddySettings::default();
        let reaction =
            classify_chat_reaction("please write a hello world example for me", &s).unwrap();
        assert_eq!(reaction.kind, ChatReactionKind::Humor);
    }

    #[test]
    fn classify_none_when_humor_off() {
        let mut s = BuddySettings::default();
        s.humor_level = HumorLevel::Off;
        assert!(classify_chat_reaction("please write a hello world example for me", &s).is_none());
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
    fn classify_humor_uses_template_not_echo() {
        let s = BuddySettings::default();
        let input = "make this nicer please give it a better look overall";
        let reaction = classify_chat_reaction(input, &s).unwrap();
        assert_eq!(reaction.kind, ChatReactionKind::Humor);
        assert!(
            HUMOR_LINES.contains(&reaction.text.as_str()),
            "reaction.text must be one of HUMOR_LINES, got: {}",
            reaction.text
        );
        assert!(
            !reaction.text.contains(input),
            "reaction.text must not echo the user message"
        );
    }

    #[test]
    fn classify_insight_uses_template() {
        let s = BuddySettings::default();
        let input = "design a new architecture for the service layer";
        let reaction = classify_chat_reaction(input, &s).unwrap();
        assert_eq!(reaction.kind, ChatReactionKind::Insight);
        assert!(
            INSIGHT_LINES.contains(&reaction.text.as_str()),
            "reaction.text must be one of INSIGHT_LINES, got: {}",
            reaction.text
        );
        assert!(
            !reaction.text.contains(input),
            "reaction.text must not echo the user message"
        );
    }

    #[test]
    fn classify_bug_uses_template() {
        let s = BuddySettings::default();
        let input = "the app crashed on save every time I try";
        let reaction = classify_chat_reaction(input, &s).unwrap();
        assert_eq!(reaction.kind, ChatReactionKind::BugCandidate);
        assert!(
            BUG_LINES.contains(&reaction.text.as_str()),
            "reaction.text must be one of BUG_LINES, got: {}",
            reaction.text
        );
        assert!(
            !reaction.text.contains(input),
            "reaction.text must not echo the user message"
        );
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
}
