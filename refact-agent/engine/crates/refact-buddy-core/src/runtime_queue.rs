use std::collections::VecDeque;
use serde::{Deserialize, Serialize};
use crate::types::BuddyRuntimeEvent;

const MAX_QUEUE_SIZE: usize = 100;

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
                existing.priority = event.priority;
                existing.ttl_ms = event.ttl_ms;
                existing.speech_text = event.speech_text;
                existing.scene = event.scene;
                existing.duration_hint = event.duration_hint;
                existing.persistent = event.persistent;
                existing.controls = event.controls;
                existing.chat_id = event.chat_id;
                existing.dismissed = existing.dismissed || event.dismissed;
                return Vec::new();
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
                existing.priority = event.priority;
                existing.ttl_ms = event.ttl_ms;
                existing.speech_text = event.speech_text;
                existing.scene = event.scene;
                existing.duration_hint = event.duration_hint;
                existing.persistent = event.persistent;
                existing.controls = event.controls;
                existing.chat_id = event.chat_id;
                // Sticky dismissal: once the user dismissed an event, any
                // subsequent re-emission with the same dedupe_key (e.g.
                // because the same window error fired again) stays hidden.
                // We OR the flags so an explicit dismiss flag on the new
                // event also takes effect, but a fresh (undismissed)
                // event can never silently un-dismiss the existing one.
                existing.dismissed = existing.dismissed || event.dismissed;
                return Vec::new();
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
        let mut evicted = Vec::new();
        while self.items.len() > MAX_QUEUE_SIZE {
            let dropped = if let Some(pos) = self.items.iter().position(|e| e.priority == "low") {
                self.items.remove(pos)
            } else {
                self.items.pop_back()
            };
            if let Some(ev) = dropped {
                evicted.push(ev.id);
            }
        }
        evicted
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
        }
        if let Some(ref mut np) = self.now_playing {
            if np.dedupe_key.as_deref() == Some(dedupe_key) {
                np.status = status.to_string();
                np.persistent = false;
                np.ttl_ms.get_or_insert(4000);
            }
        }
    }
}
