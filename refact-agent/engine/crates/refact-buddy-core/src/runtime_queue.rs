use std::collections::{HashSet, VecDeque};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use crate::types::BuddyRuntimeEvent;

const MAX_QUEUE_SIZE: usize = 100;
const PERSONALITY_RESERVED_SLOTS: usize = 8;
const DISMISSED_NO_TTL_RETENTION_MINUTES: i64 = 15;
const COMPLETED_NO_TTL_RETENTION_HOURS: i64 = 1;
const FAILED_NO_TTL_RETENTION_HOURS: i64 = 24;
const ACTIVE_NO_TTL_RETENTION_HOURS: i64 = 24;

pub fn is_personality_runtime_event(event: &BuddyRuntimeEvent) -> bool {
    event.source == "chat_reactions"
        || matches!(
            event.signal_type.as_str(),
            "speech_humor" | "speech_insight" | "speech_chat_reaction" | "chat_bug_candidate"
        )
}

fn runtime_event_created_at(event: &BuddyRuntimeEvent) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&event.created_at)
        .ok()
        .map(|created_at| created_at.with_timezone(&Utc))
}

fn older_runtime_event_position(
    left: &(usize, &BuddyRuntimeEvent),
    right: &(usize, &BuddyRuntimeEvent),
) -> std::cmp::Ordering {
    match (
        runtime_event_created_at(left.1),
        runtime_event_created_at(right.1),
    ) {
        (Some(left_at), Some(right_at)) => {
            left_at.cmp(&right_at).then_with(|| left.0.cmp(&right.0))
        }
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => left.0.cmp(&right.0),
    }
}

fn newer_runtime_event_position(
    left: &(usize, &BuddyRuntimeEvent),
    right: &(usize, &BuddyRuntimeEvent),
) -> std::cmp::Ordering {
    older_runtime_event_position(left, right).reverse()
}

pub fn runtime_event_expired_at(event: &BuddyRuntimeEvent, now: DateTime<Utc>) -> bool {
    let Ok(created_at) = DateTime::parse_from_rfc3339(&event.created_at) else {
        return false;
    };
    let created_at = created_at.with_timezone(&Utc);
    if created_at > now {
        return false;
    }
    if let Some(ttl_ms) = event.ttl_ms {
        if event.progress.is_some() || runtime_event_is_active(event) {
            return false;
        }
        if event.persistent {
            return false;
        }
        let Ok(ttl) = chrono::Duration::from_std(std::time::Duration::from_millis(ttl_ms)) else {
            return false;
        };
        return created_at
            .checked_add_signed(ttl)
            .is_some_and(|expires_at| expires_at <= now);
    }
    runtime_event_stale_without_ttl_at(event, created_at, now)
}

fn runtime_event_is_active(event: &BuddyRuntimeEvent) -> bool {
    matches!(
        event.status.as_str(),
        "started" | "progress" | "active" | "streaming" | "paused"
    )
}

fn runtime_event_stale_without_ttl_at(
    event: &BuddyRuntimeEvent,
    created_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> bool {
    if event.dismissed {
        return created_at
            .checked_add_signed(chrono::Duration::minutes(
                DISMISSED_NO_TTL_RETENTION_MINUTES,
            ))
            .is_some_and(|expires_at| expires_at <= now);
    }
    if event.persistent {
        return false;
    }
    if event.progress.is_some() || runtime_event_is_active(event) {
        return created_at
            .checked_add_signed(chrono::Duration::hours(ACTIVE_NO_TTL_RETENTION_HOURS))
            .is_some_and(|expires_at| expires_at <= now);
    }
    let retention = if runtime_event_is_failed(event) {
        chrono::Duration::hours(FAILED_NO_TTL_RETENTION_HOURS)
    } else if runtime_event_is_completed(event) {
        chrono::Duration::hours(COMPLETED_NO_TTL_RETENTION_HOURS)
    } else {
        return false;
    };
    created_at
        .checked_add_signed(retention)
        .is_some_and(|expires_at| expires_at <= now)
}

fn runtime_event_is_completed(event: &BuddyRuntimeEvent) -> bool {
    matches!(
        event.status.as_str(),
        "completed" | "complete" | "done" | "success" | "succeeded"
    ) || matches!(
        event.signal_type.as_str(),
        "chat_completed"
            | "checkpoint_saved"
            | "task_completed"
            | "git_commit"
            | "connection_restored"
    )
}

fn runtime_event_is_failed(event: &BuddyRuntimeEvent) -> bool {
    matches!(
        event.status.as_str(),
        "failed" | "error" | "errored" | "failure"
    ) || event.signal_type.contains("error")
        || event.signal_type.ends_with("_failed")
        || event.failure_category.is_some()
        || event.failure_summary.is_some()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuntimeQueue {
    #[serde(default)]
    pub items: VecDeque<BuddyRuntimeEvent>,
    #[serde(default)]
    pub now_playing: Option<BuddyRuntimeEvent>,
}

impl RuntimeQueue {
    pub fn new() -> Self {
        Self {
            items: VecDeque::new(),
            now_playing: None,
        }
    }

    /// Insert or coalesce an event. Returns the list of ids that were evicted
    /// to keep the queue under `MAX_QUEUE_SIZE`. Callers persist tombstones
    /// for those ids so the on-disk JSONL log replays to the same state.
    pub fn enqueue(&mut self, event: BuddyRuntimeEvent) -> Vec<String> {
        let mut removed = self.prune_expired_at(Utc::now());

        // Coalesce by dedupe_key if present
        if let Some(ref key) = event.dedupe_key {
            if let Some(existing) = self
                .now_playing
                .as_mut()
                .filter(|e| e.dedupe_key.as_deref() == Some(key))
            {
                existing.signal_type = event.signal_type;
                existing.title = event.title;
                existing.description = event.description;
                existing.source = event.source;
                existing.progress = event.progress;
                existing.status = event.status;
                existing.failure_category = event.failure_category;
                existing.failure_summary = event.failure_summary;
                existing.priority = event.priority;
                existing.ttl_ms = event.ttl_ms;
                existing.speech_text = event.speech_text;
                existing.scene = event.scene;
                existing.duration_hint = event.duration_hint;
                existing.persistent = event.persistent;
                existing.controls = event.controls;
                existing.chat_id = event.chat_id;
                existing.created_at = event.created_at;
                existing.bubble_policy = event.bubble_policy;
                existing.dismissed = existing.dismissed || event.dismissed;
                return removed;
            }
            if let Some(existing) = self
                .items
                .iter_mut()
                .find(|e| e.dedupe_key.as_deref() == Some(key))
            {
                existing.signal_type = event.signal_type;
                existing.title = event.title;
                existing.description = event.description;
                existing.source = event.source;
                existing.progress = event.progress;
                existing.status = event.status;
                existing.failure_category = event.failure_category;
                existing.failure_summary = event.failure_summary;
                existing.priority = event.priority;
                existing.ttl_ms = event.ttl_ms;
                existing.speech_text = event.speech_text;
                existing.scene = event.scene;
                existing.duration_hint = event.duration_hint;
                existing.persistent = event.persistent;
                existing.controls = event.controls;
                existing.chat_id = event.chat_id;
                existing.created_at = event.created_at;
                existing.bubble_policy = event.bubble_policy;
                // Sticky dismissal: once the user dismissed an event, any
                // subsequent re-emission with the same dedupe_key (e.g.
                // because the same window error fired again) stays hidden.
                // We OR the flags so an explicit dismiss flag on the new
                // event also takes effect, but a fresh (undismissed)
                // event can never silently un-dismiss the existing one.
                existing.dismissed = existing.dismissed || event.dismissed;
                return removed;
            }
        }

        // Priority insertion: critical/high go to front
        let input_id = event.id.clone();
        let insert_front = event.priority == "critical" || event.priority == "high";
        if insert_front {
            self.items.push_front(event);
        } else {
            self.items.push_back(event);
        }

        while self.items.len() > MAX_QUEUE_SIZE {
            let protected = self.protected_eviction_ids(&input_id);
            let incoming_protected = HashSet::from([input_id.clone()]);
            let dropped = self
                .eviction_victim_position(&protected, false)
                .or_else(|| self.eviction_victim_position(&incoming_protected, true))
                .and_then(|pos| self.items.remove(pos));
            if let Some(ev) = dropped {
                removed.push(ev.id);
            } else {
                break;
            }
        }
        removed
    }

    fn protected_eviction_ids(&self, incoming_id: &str) -> HashSet<String> {
        let mut protected = HashSet::from([incoming_id.to_string()]);
        let mut personality_events = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, event)| is_personality_runtime_event(event))
            .collect::<Vec<_>>();
        personality_events.sort_by(newer_runtime_event_position);
        for (_, event) in personality_events
            .into_iter()
            .take(PERSONALITY_RESERVED_SLOTS.min(MAX_QUEUE_SIZE))
        {
            protected.insert(event.id.clone());
        }
        protected
    }

    fn eviction_victim_position(
        &self,
        protected: &HashSet<String>,
        allow_protected_personality_transient: bool,
    ) -> Option<usize> {
        if allow_protected_personality_transient {
            return self
                .oldest_position(|event| !event.persistent && !protected.contains(&event.id));
        }
        self.oldest_position(|event| event.priority == "low" && !protected.contains(&event.id))
            .or_else(|| {
                self.oldest_position(|event| event.dismissed && !protected.contains(&event.id))
            })
            .or_else(|| {
                self.oldest_position(|event| {
                    !protected.contains(&event.id)
                        && !is_personality_runtime_event(event)
                        && !event.persistent
                })
            })
            .or_else(|| self.oldest_position(|event| !protected.contains(&event.id)))
    }

    fn oldest_position(
        &self,
        mut predicate: impl FnMut(&BuddyRuntimeEvent) -> bool,
    ) -> Option<usize> {
        self.items
            .iter()
            .enumerate()
            .filter(|(_, event)| predicate(event))
            .min_by(older_runtime_event_position)
            .map(|(pos, _)| pos)
    }

    pub fn prune_expired_at(&mut self, now: DateTime<Utc>) -> Vec<String> {
        let mut removed = Vec::new();
        if self
            .now_playing
            .as_ref()
            .is_some_and(|event| runtime_event_expired_at(event, now))
        {
            if let Some(event) = self.now_playing.take() {
                removed.push(event.id);
            }
        }
        let mut retained = VecDeque::with_capacity(self.items.len());
        while let Some(event) = self.items.pop_front() {
            if runtime_event_expired_at(&event, now) {
                removed.push(event.id);
            } else {
                retained.push_back(event);
            }
        }
        self.items = retained;
        removed
    }

    pub fn complete(&mut self, dedupe_key: &str, status: &str) {
        if let Some(e) = self
            .items
            .iter_mut()
            .find(|e| e.dedupe_key.as_deref() == Some(dedupe_key))
        {
            e.status = status.to_string();
            e.persistent = false;
            e.ttl_ms.get_or_insert(4000);
            e.created_at = Utc::now().to_rfc3339();
        }
        if let Some(ref mut np) = self.now_playing {
            if np.dedupe_key.as_deref() == Some(dedupe_key) {
                np.status = status.to_string();
                np.persistent = false;
                np.ttl_ms.get_or_insert(4000);
                np.created_at = Utc::now().to_rfc3339();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_event(id: &str, dedupe_key: &str) -> BuddyRuntimeEvent {
        BuddyRuntimeEvent {
            id: id.to_string(),
            signal_type: "streaming".to_string(),
            title: "Test".to_string(),
            description: None,
            source: "chat".to_string(),
            status: "started".to_string(),
            failure_category: None,
            failure_summary: None,
            progress: None,
            dedupe_key: Some(dedupe_key.to_string()),
            priority: "normal".to_string(),
            created_at: Utc::now().to_rfc3339(),
            ttl_ms: None,
            bubble_policy: None,
            speech_text: None,
            scene: None,
            duration_hint: None,
            persistent: false,
            controls: vec![],
            chat_id: None,
            dismissed: false,
        }
    }

    fn make_error_event(id: &str, index: usize) -> BuddyRuntimeEvent {
        let mut event = make_event(id, &format!("error-key-{index}"));
        event.signal_type = "error".to_string();
        event.source = "frontend".to_string();
        event.status = "failed".to_string();
        event.priority = if index % 2 == 0 { "high" } else { "critical" }.to_string();
        event.persistent = true;
        event.created_at = format!("2024-01-01T00:{:02}:{:02}Z", index / 60, index % 60);
        event
    }

    fn make_personality_event(id: &str, index: usize) -> BuddyRuntimeEvent {
        let mut event = make_event(id, &format!("reaction-key-{index}"));
        event.signal_type = "speech_chat_reaction".to_string();
        event.source = "chat_reactions".to_string();
        event.status = "info".to_string();
        event.priority = "normal".to_string();
        event.created_at = format!("2099-01-01T01:{:02}:{:02}Z", index / 60, index % 60);
        event.ttl_ms = Some(90_000);
        event
    }

    fn has_event(queue: &RuntimeQueue, id: &str) -> bool {
        queue.items.iter().any(|event| event.id == id)
    }

    #[test]
    fn runtime_queue_keeps_persistent_active_event_even_if_ttl_elapsed() {
        let now = DateTime::parse_from_rfc3339("2024-01-01T00:00:10Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut queue = RuntimeQueue::new();
        let mut event = make_event("ev1", "key-1");
        event.persistent = true;
        event.status = "progress".to_string();
        event.progress = Some(50);
        event.created_at = "2024-01-01T00:00:00Z".to_string();
        event.ttl_ms = Some(1000);
        queue.items.push_back(event);

        let removed = queue.prune_expired_at(now);

        assert!(removed.is_empty());
        assert_eq!(queue.items.len(), 1);
    }

    #[test]
    fn runtime_queue_enqueue_returns_pruned_expired_ids_before_cap() {
        let mut queue = RuntimeQueue::new();
        let mut expired = make_event("expired", "expired-key");
        expired.status = "completed".to_string();
        expired.created_at = "2024-01-01T00:00:00Z".to_string();
        expired.ttl_ms = Some(1);
        queue.items.push_back(expired);
        let incoming = make_event("incoming", "incoming-key");

        let removed = queue.enqueue(incoming);

        assert_eq!(removed, vec!["expired".to_string()]);
        assert_eq!(queue.items.len(), 1);
        assert_eq!(queue.items[0].id, "incoming");
    }

    #[test]
    fn chat_reaction_survives_full_high_error_queue() {
        let mut queue = RuntimeQueue::new();
        for i in 0..MAX_QUEUE_SIZE {
            queue.enqueue(make_error_event(&format!("error-{i}"), i));
        }

        let removed = queue.enqueue(make_personality_event("reaction-fresh", 0));

        assert_eq!(queue.items.len(), MAX_QUEUE_SIZE);
        assert!(has_event(&queue, "reaction-fresh"));
        assert_eq!(removed, vec!["error-0".to_string()]);
        assert!(!has_event(&queue, "error-0"));
    }

    #[test]
    fn coalesced_event_does_not_trigger_eviction() {
        let mut queue = RuntimeQueue::new();
        for i in 0..MAX_QUEUE_SIZE {
            queue.enqueue(make_error_event(&format!("error-{i}"), i));
        }
        let mut coalesced = make_error_event("error-replacement", 0);
        coalesced.title = "Updated".to_string();

        let removed = queue.enqueue(coalesced);

        assert!(removed.is_empty());
        assert_eq!(queue.items.len(), MAX_QUEUE_SIZE);
        assert!(has_event(&queue, "error-0"));
        assert!(!has_event(&queue, "error-replacement"));
        assert_eq!(
            queue
                .items
                .iter()
                .find(|event| event.dedupe_key.as_deref() == Some("error-key-0"))
                .map(|event| event.title.as_str()),
            Some("Updated")
        );
    }
}
