use chrono::{DateTime, Utc};

use crate::buddy::observers::{BuddyObserver, Ephemeral, ObserverContext};
use crate::buddy::settings::BuddySettings;
use crate::buddy::types::{BuddyFact, BuddyFactKind};
use crate::call_validation::{ChatContent, ChatMessage};
use crate::app_state::AppState;

pub struct ChatPatternObserver;

const RETRY_KEYWORDS: &[&str] = &[
    "actually",
    "wait",
    "no,",
    "sorry",
    "not what i meant",
    "try again",
    "undo",
    "revert",
];

fn message_text(msg: &ChatMessage) -> &str {
    match &msg.content {
        ChatContent::SimpleText(s) => s.as_str(),
        _ => "",
    }
}

pub fn count_retry_streak(messages: &[ChatMessage]) -> u32 {
    let mut streak = 0u32;
    for msg in messages.iter().rev() {
        if msg.role != "user" {
            continue;
        }
        let text = message_text(msg).to_lowercase();
        if text.is_empty() {
            break;
        }
        if RETRY_KEYWORDS.iter().any(|k| text.contains(k)) {
            streak += 1;
        } else {
            break;
        }
    }
    streak
}

fn detect_chat_pattern_facts(
    messages: &[ChatMessage],
    chat_id: &str,
    now: DateTime<Utc>,
) -> Vec<BuddyFact> {
    let mut facts = vec![];
    // SECURITY: do not copy message content
    let view = Ephemeral::new(messages);
    let retry_streak = count_retry_streak(view.as_ref());
    // view drops here — no further access
    drop(view);

    if retry_streak >= 3 {
        facts.push(BuddyFact {
            kind: BuddyFactKind::ChatRetryStreak,
            key: format!("chat:retry_streak:{}", chat_id),
            source: "chat_pattern",
            payload: serde_json::json!({
                "chat_id": chat_id,
                "retry_streak": retry_streak,
            }),
            seen_at: now,
            confidence: 0.85,
        });
    }
    facts
}

#[async_trait::async_trait]
impl BuddyObserver for ChatPatternObserver {
    fn id(&self) -> &'static str {
        "chat_pattern"
    }

    fn cadence_seconds(&self) -> u64 {
        60
    }

    fn requires_setting(&self, settings: &BuddySettings) -> bool {
        settings.observers.chat_pattern
            && settings.message_observation_enabled
            && settings.proactive_enabled
    }

    async fn observe(&self, gcx: AppState, ctx: &ObserverContext) -> Vec<BuddyFact> {
        let sessions_map = gcx.chat.sessions.clone();
        let sessions_read = sessions_map.read().await;
        let mut facts = vec![];
        for (chat_id, session_arc) in sessions_read.iter() {
            if let Ok(session) = session_arc.try_lock() {
                // Run detection on borrowed slice — messages never leave this scope
                facts.extend(detect_chat_pattern_facts(
                    &session.messages,
                    chat_id,
                    ctx.now,
                ));
            }
        }
        facts
    }
}

#[cfg(test)]
pub fn run_chat_pattern_observer_sync(messages: &[ChatMessage], chat_id: &str) -> Vec<BuddyFact> {
    detect_chat_pattern_facts(messages, chat_id, chrono::Utc::now())
}
