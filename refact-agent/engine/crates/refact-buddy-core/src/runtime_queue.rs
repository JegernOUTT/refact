use std::collections::VecDeque;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use crate::types::BuddyRuntimeEvent;

const MAX_QUEUE_SIZE: usize = 100;

pub fn runtime_event_expired_at(event: &BuddyRuntimeEvent, now: DateTime<Utc>) -> bool {
    if event.persistent || event.ttl_ms.is_none() {
        return false;
    }
    if event.progress.is_some()
        || matches!(event.status.as_str(), "started" | "progress" | "active")
    {
        return false;
    }
    let Some(ttl_ms) = event.ttl_ms else {
        return false;
    };
    let Ok(created_at) = DateTime::parse_from_rfc3339(&event.created_at) else {
        return false;
    };
    let created_at = created_at.with_timezone(&Utc);
    if created_at > now {
        return false;
    }
    let Ok(ttl) = chrono::Duration::from_std(std::time::Duration::from_millis(ttl_ms)) else {
        return false;
    };
    created_at
        .checked_add_signed(ttl)
        .is_some_and(|expires_at| expires_at <= now)
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
        let insert_front = event.priority == "critical" || event.priority == "high";
        if insert_front {
            self.items.push_front(event);
        } else {
            self.items.push_back(event);
        }

        // Cap queue size, drop oldest low-priority first
        while self.items.len() > MAX_QUEUE_SIZE {
            let dropped = if let Some(pos) = self.items.iter().position(|e| e.priority == "low") {
                self.items.remove(pos)
            } else {
                self.items.pop_back()
            };
            if let Some(ev) = dropped {
                removed.push(ev.id);
            }
        }
        removed
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

    #[allow(dead_code)]
    pub fn update_progress(&mut self, dedupe_key: &str, progress: u8, title: Option<&str>) {
        if let Some(e) = self
            .items
            .iter_mut()
            .find(|e| e.dedupe_key.as_deref() == Some(dedupe_key))
        {
            e.progress = Some(progress);
            if let Some(t) = title {
                e.title = t.to_string();
            }
        }
        if let Some(ref mut np) = self.now_playing {
            if np.dedupe_key.as_deref() == Some(dedupe_key) {
                np.progress = Some(progress);
                if let Some(t) = title {
                    np.title = t.to_string();
                }
            }
        }
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
    use crate::types::BuddyBubblePolicy;

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
            created_at: "2024-01-01T00:00:00Z".to_string(),
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

    #[test]
    fn runtime_queue_prunes_expired_non_persistent_event() {
        let now = DateTime::parse_from_rfc3339("2024-01-01T00:00:10Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut queue = RuntimeQueue::new();
        let mut event = make_event("ev1", "key-1");
        event.status = "completed".to_string();
        event.created_at = "2024-01-01T00:00:00Z".to_string();
        event.ttl_ms = Some(1000);
        queue.items.push_back(event);

        let removed = queue.prune_expired_at(now);

        assert_eq!(removed, vec!["ev1".to_string()]);
        assert!(queue.items.is_empty());
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
    fn runtime_queue_keeps_event_with_invalid_created_at() {
        let now = DateTime::parse_from_rfc3339("2024-01-01T00:00:10Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut queue = RuntimeQueue::new();
        let mut event = make_event("ev1", "key-1");
        event.status = "completed".to_string();
        event.created_at = "goblin-time".to_string();
        event.ttl_ms = Some(1000);
        queue.items.push_back(event);

        let removed = queue.prune_expired_at(now);

        assert!(removed.is_empty());
        assert_eq!(queue.items.len(), 1);
    }

    #[test]
    fn runtime_queue_keeps_event_with_no_ttl() {
        let now = DateTime::parse_from_rfc3339("2024-01-01T00:00:10Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut queue = RuntimeQueue::new();
        let mut event = make_event("ev1", "key-1");
        event.status = "completed".to_string();
        event.created_at = "2024-01-01T00:00:00Z".to_string();
        event.ttl_ms = None;
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
    fn coalesced_items_event_updates_bubble_policy_and_created_at() {
        let mut queue = RuntimeQueue::new();
        queue.enqueue(make_event("ev1", "key-1"));

        let mut ev2 = make_event("ev2", "key-1");
        ev2.bubble_policy = Some(BuddyBubblePolicy::Ambient);
        ev2.created_at = "2024-06-01T00:00:00Z".to_string();
        queue.enqueue(ev2);

        assert_eq!(queue.items.len(), 1);
        assert_eq!(
            queue.items[0].bubble_policy,
            Some(BuddyBubblePolicy::Ambient)
        );
        assert_eq!(queue.items[0].created_at, "2024-06-01T00:00:00Z");
    }

    #[test]
    fn coalesced_now_playing_updates_bubble_policy_and_created_at() {
        let mut queue = RuntimeQueue::new();
        queue.now_playing = Some(make_event("ev1", "np-key"));

        let mut ev2 = make_event("ev2", "np-key");
        ev2.bubble_policy = Some(BuddyBubblePolicy::Durable);
        ev2.created_at = "2024-07-01T00:00:00Z".to_string();
        queue.enqueue(ev2);

        assert!(queue.items.is_empty());
        let np = queue.now_playing.as_ref().unwrap();
        assert_eq!(np.bubble_policy, Some(BuddyBubblePolicy::Durable));
        assert_eq!(np.created_at, "2024-07-01T00:00:00Z");
    }

    #[test]
    fn complete_refreshes_created_at_so_completion_is_fresh() {
        let mut queue = RuntimeQueue::new();
        let mut ev = make_event("ev1", "complete-key");
        ev.persistent = true;
        ev.created_at = "2020-01-01T00:00:00Z".to_string();
        queue.enqueue(ev);

        queue.complete("complete-key", "completed");

        let stored = &queue.items[0];
        assert_eq!(stored.status, "completed");
        assert_ne!(stored.created_at, "2020-01-01T00:00:00Z");
        assert!(chrono::DateTime::parse_from_rfc3339(&stored.created_at).is_ok());
    }

    #[test]
    fn coalesced_event_updates_structured_failure_fields() {
        let mut queue = RuntimeQueue::new();
        queue.enqueue(make_event("ev1", "failure-key"));

        let mut ev2 = make_event("ev2", "failure-key");
        ev2.status = "failed".to_string();
        ev2.failure_category = Some("model_unavailable".to_string());
        ev2.failure_summary = Some("Model unavailable".to_string());
        queue.enqueue(ev2);

        assert_eq!(queue.items.len(), 1);
        assert_eq!(queue.items[0].status, "failed");
        assert_eq!(
            queue.items[0].failure_category.as_deref(),
            Some("model_unavailable")
        );
        assert_eq!(
            queue.items[0].failure_summary.as_deref(),
            Some("Model unavailable")
        );
    }
}
