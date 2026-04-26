use std::collections::VecDeque;
use super::types::BuddyRuntimeEvent;

const MAX_QUEUE_SIZE: usize = 100;

pub struct RuntimeQueue {
    pub items: VecDeque<BuddyRuntimeEvent>,
    pub now_playing: Option<BuddyRuntimeEvent>,
}

impl RuntimeQueue {
    pub fn new() -> Self {
        Self {
            items: VecDeque::new(),
            now_playing: None,
        }
    }

    pub fn enqueue(&mut self, event: BuddyRuntimeEvent) {
        // Coalesce by dedupe_key if present
        if let Some(ref key) = event.dedupe_key {
            if let Some(existing) = self.items.iter_mut().find(|e| e.dedupe_key.as_deref() == Some(key)) {
                existing.title = event.title;
                existing.description = event.description;
                existing.progress = event.progress;
                existing.status = event.status;
                return;
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
            if let Some(pos) = self.items.iter().rposition(|e| e.priority == "low") {
                self.items.remove(pos);
            } else {
                self.items.pop_back();
            }
        }
    }

    pub fn update_progress(&mut self, dedupe_key: &str, progress: u8, title: Option<&str>) {
        if let Some(e) = self.items.iter_mut().find(|e| e.dedupe_key.as_deref() == Some(dedupe_key)) {
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
        if let Some(e) = self.items.iter_mut().find(|e| e.dedupe_key.as_deref() == Some(dedupe_key)) {
            e.status = status.to_string();
        }
        if let Some(ref mut np) = self.now_playing {
            if np.dedupe_key.as_deref() == Some(dedupe_key) {
                np.status = status.to_string();
            }
        }
    }
}
