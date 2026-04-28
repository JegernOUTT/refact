use std::collections::HashSet;
use std::sync::Arc;
use chrono::{DateTime, Utc};
use tokio::sync::RwLock;

use crate::buddy::observers::{BuddyObserver, Ephemeral, ObserverContext, ObserverCost};
use crate::buddy::settings::BuddySettings;
use crate::buddy::types::{BuddyFact, BuddyFactKind};
use crate::call_validation::{ChatContent, ChatMessage};
use crate::global_context::GlobalContext;

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

const STOP_WORDS: &[&str] = &[
    "the", "a", "and", "of", "to", "is", "it", "for", "be", "in", "on",
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

fn tokenize(s: &str) -> HashSet<String> {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .filter(|w| !STOP_WORDS.contains(w))
        .map(|w| w.to_string())
        .collect()
}

pub fn detect_topic_pivot(messages: &[ChatMessage]) -> bool {
    let user_msgs: Vec<&ChatMessage> = messages.iter().filter(|m| m.role == "user").collect();
    let n = user_msgs.len();
    if n < 2 {
        return false;
    }
    let last_tokens = tokenize(message_text(user_msgs[n - 1]));
    if last_tokens.len() <= 5 {
        return false;
    }
    let start = n.saturating_sub(4);
    let mut prior_tokens: HashSet<String> = HashSet::new();
    for msg in &user_msgs[start..n - 1] {
        prior_tokens.extend(tokenize(message_text(msg)));
    }
    if prior_tokens.is_empty() {
        return false;
    }
    let intersection = last_tokens.intersection(&prior_tokens).count();
    let union_size = last_tokens.union(&prior_tokens).count();
    if union_size == 0 {
        return false;
    }
    (intersection as f32 / union_size as f32) < 0.15
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
    let pivot_detected = detect_topic_pivot(view.as_ref());
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
    if pivot_detected {
        facts.push(BuddyFact {
            kind: BuddyFactKind::ChatTopicPivot,
            key: format!("chat:pivot:{}", chat_id),
            source: "chat_pattern",
            payload: serde_json::json!({
                "chat_id": chat_id,
                "pivot_detected": true,
            }),
            seen_at: now,
            confidence: 0.7,
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

    fn cost_class(&self) -> ObserverCost {
        ObserverCost::Cheap
    }

    fn requires_setting(&self, settings: &BuddySettings) -> bool {
        settings.observers.chat_pattern
            && settings.message_observation_enabled
            && settings.proactive_enabled
    }

    async fn observe(
        &self,
        gcx: Arc<RwLock<GlobalContext>>,
        ctx: &ObserverContext,
    ) -> Vec<BuddyFact> {
        let sessions_map = gcx.read().await.chat_sessions.clone();
        let sessions_read = sessions_map.read().await;
        let mut facts = vec![];
        for (chat_id, session_arc) in sessions_read.iter() {
            if let Ok(session) = session_arc.try_lock() {
                // Run detection on borrowed slice — messages never leave this scope
                facts.extend(detect_chat_pattern_facts(&session.messages, chat_id, ctx.now));
            }
        }
        facts
    }
}

#[cfg(test)]
pub fn run_chat_pattern_observer_sync(messages: &[ChatMessage], chat_id: &str) -> Vec<BuddyFact> {
    detect_chat_pattern_facts(messages, chat_id, chrono::Utc::now())
}
