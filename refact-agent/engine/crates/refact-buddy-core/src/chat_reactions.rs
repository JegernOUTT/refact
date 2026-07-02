use std::collections::{HashMap, VecDeque};

use chrono::{DateTime, Utc};

use crate::snapshot::{ChatReactionAttempt, ChatReactionDebug};
use crate::types::BuddyRuntimeEvent;

pub const PER_CHAT_COOLDOWN_SECS: i64 = 180;
pub const GLOBAL_HOURLY_CAP: u32 = 20;
pub const CHAT_REACTION_DEBUG_ATTEMPT_CAP: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatReactionSkipReason {
    ThreadFiltered,
    TextTooShort,
    BuddyUnavailable,
    SettingsDisabled,
    RateLimited,
}

impl ChatReactionSkipReason {
    pub fn as_str(self) -> &'static str {
        match self {
            ChatReactionSkipReason::ThreadFiltered => "thread_filtered",
            ChatReactionSkipReason::TextTooShort => "text_too_short",
            ChatReactionSkipReason::BuddyUnavailable => "buddy_unavailable",
            ChatReactionSkipReason::SettingsDisabled => "settings_disabled",
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
            event_id: None,
            ttl_ms: None,
            bubble_policy: None,
            queued: None,
        });
        self.last_skip_reason = Some(skip_reason);
    }

    pub fn record_emitted(
        &mut self,
        chat_id: &str,
        signal_type: &str,
        event: Option<&BuddyRuntimeEvent>,
    ) {
        let attempted_at = Utc::now().to_rfc3339();
        self.record(ChatReactionAttempt {
            attempted_at: attempted_at.clone(),
            chat_id: chat_id.to_string(),
            result: "emitted".to_string(),
            skip_reason: None,
            signal_type: Some(signal_type.to_string()),
            event_id: event.map(|event| event.id.clone()),
            ttl_ms: event.and_then(|event| event.ttl_ms),
            bubble_policy: event.and_then(|event| event.bubble_policy.clone()),
            queued: Some(event.is_some()),
        });
        self.last_emitted_at = Some(attempted_at);
    }

    pub fn record_not_queued(&mut self, chat_id: &str, signal_type: &str) {
        self.record(ChatReactionAttempt {
            attempted_at: Utc::now().to_rfc3339(),
            chat_id: chat_id.to_string(),
            result: "not_queued".to_string(),
            skip_reason: Some("not_queued".to_string()),
            signal_type: Some(signal_type.to_string()),
            event_id: None,
            ttl_ms: None,
            bubble_policy: None,
            queued: Some(false),
        });
        self.last_skip_reason = Some("not_queued".to_string());
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ChatReactionKind {
    Humor,
    Insight,
    BugCandidate,
    Ambient,
}

pub struct ChatReactionLimiter {
    pub(crate) per_chat_kind_last_at: HashMap<(String, ChatReactionKind), DateTime<Utc>>,
    recent_non_bug_success_at: HashMap<String, DateTime<Utc>>,
    global_hourly_count: u32,
    global_window_start: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatReactionLimiterReservation {
    key: (String, ChatReactionKind),
    reserved_at: DateTime<Utc>,
    previous_last_at: Option<DateTime<Utc>>,
    window_start: DateTime<Utc>,
}

impl ChatReactionLimiter {
    pub fn new() -> Self {
        Self {
            per_chat_kind_last_at: HashMap::new(),
            recent_non_bug_success_at: HashMap::new(),
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
    ) -> Result<ChatReactionLimiterReservation, ChatReactionSkipReason> {
        self.per_chat_kind_last_at
            .retain(|_, last_at| (now - *last_at).num_seconds() < PER_CHAT_COOLDOWN_SECS);
        self.recent_non_bug_success_at
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
        let previous_last_at = self.per_chat_kind_last_at.get(&key).cloned();
        self.per_chat_kind_last_at.insert(key.clone(), now);
        self.global_hourly_count += 1;
        Ok(ChatReactionLimiterReservation {
            key,
            reserved_at: now,
            previous_last_at,
            window_start: self.global_window_start,
        })
    }

    pub fn rollback(&mut self, reservation: ChatReactionLimiterReservation) {
        let key = reservation.key;
        if !self
            .per_chat_kind_last_at
            .get(&key)
            .is_some_and(|last_at| *last_at == reservation.reserved_at)
        {
            return;
        }
        if self.global_window_start == reservation.window_start && self.global_hourly_count > 0 {
            self.global_hourly_count -= 1;
        }
        if let Some(previous_last_at) = reservation.previous_last_at {
            self.per_chat_kind_last_at.insert(key, previous_last_at);
        } else {
            self.per_chat_kind_last_at.remove(&key);
        }
    }

    pub fn record_success(&mut self, chat_id: &str, event: &BuddyRuntimeEvent, now: DateTime<Utc>) {
        if event.signal_type != "chat_bug_candidate" {
            self.recent_non_bug_success_at
                .insert(chat_id.to_string(), now);
        }
    }

    pub fn has_recent_non_bug_success(&mut self, chat_id: &str, now: DateTime<Utc>) -> bool {
        self.recent_non_bug_success_at
            .retain(|_, last_at| (now - *last_at).num_seconds() < PER_CHAT_COOLDOWN_SECS);
        self.recent_non_bug_success_at
            .get(chat_id)
            .is_some_and(|last_at| (now - *last_at).num_seconds() < PER_CHAT_COOLDOWN_SECS)
    }

    pub fn allow(&mut self, chat_id: &str, now: DateTime<Utc>) -> bool {
        self.allow_kind(chat_id, ChatReactionKind::Humor, now)
    }

    pub fn reset(&mut self) {
        self.per_chat_kind_last_at.clear();
        self.recent_non_bug_success_at.clear();
        self.global_hourly_count = 0;
        self.global_window_start = Utc::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

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
    fn limiter_rollback_restores_quota_and_cooldown() {
        let mut lim = ChatReactionLimiter::new();
        let now = Utc::now();
        let reservation = lim
            .try_allow_kind("chat-a", ChatReactionKind::Insight, now)
            .unwrap();

        lim.rollback(reservation);

        assert!(lim
            .try_allow_kind(
                "chat-a",
                ChatReactionKind::Insight,
                now + Duration::seconds(10),
            )
            .is_ok());
    }

    #[test]
    fn limiter_rollback_does_not_clobber_newer_reservation() {
        let mut lim = ChatReactionLimiter::new();
        let now = Utc::now();
        let reservation = lim
            .try_allow_kind("chat-a", ChatReactionKind::Insight, now)
            .unwrap();
        lim.per_chat_kind_last_at.insert(
            ("chat-a".to_string(), ChatReactionKind::Insight),
            now + Duration::seconds(PER_CHAT_COOLDOWN_SECS + 1),
        );

        lim.rollback(reservation);

        assert_eq!(
            lim.try_allow_kind(
                "chat-a",
                ChatReactionKind::Insight,
                now + Duration::seconds(PER_CHAT_COOLDOWN_SECS + 2),
            ),
            Err(ChatReactionSkipReason::RateLimited)
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
            assert!(lim.allow(&chat_id, now + Duration::seconds(i64::from(i))));
        }
        let overflow_chat = format!("chat-{}", GLOBAL_HOURLY_CAP);
        assert!(!lim.allow(
            &overflow_chat,
            now + Duration::seconds(i64::from(GLOBAL_HOURLY_CAP))
        ));
    }

    #[test]
    fn limiter_resets_after_hour() {
        let mut lim = ChatReactionLimiter::new();
        let now = Utc::now();
        for i in 0..GLOBAL_HOURLY_CAP {
            let chat_id = format!("chat-reset-{i}");
            lim.allow(&chat_id, now + Duration::seconds(i64::from(i)));
        }
        let after_hour = now + Duration::seconds(3601);
        assert!(lim.allow("chat-fresh", after_hour));
    }

    #[test]
    fn limiter_prunes_stale_chat_ids() {
        let mut lim = ChatReactionLimiter::new();
        let t0 = chrono::Utc::now();
        assert!(lim.allow_kind("chat-1", ChatReactionKind::Humor, t0));

        let t1 = t0 + Duration::seconds(PER_CHAT_COOLDOWN_SECS + 1);
        assert!(lim.allow_kind("chat-2", ChatReactionKind::Humor, t1));

        assert!(
            !lim.per_chat_kind_last_at
                .contains_key(&("chat-1".to_string(), ChatReactionKind::Humor)),
            "stale chat-1 entry must be pruned after second call"
        );
    }
}
