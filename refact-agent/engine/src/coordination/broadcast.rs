//! Broadcast admission: a sliding-window rate limit and the deterministic
//! delivery id that makes the same logical broadcast reach one recipient once.
//!
//! Exceeding the limit is refused before anything is sent, so a bounded
//! broadcast is never silently trimmed down to whatever happened to fit.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct BroadcastLimiter {
    windows: Arc<Mutex<HashMap<String, VecDeque<u64>>>>,
}

impl BroadcastLimiter {
    pub fn admit(&self, key: &str, now_ms: u64) -> Result<(), String> {
        let settings = refact_work_core::coordination_settings();
        settings
            .validate()
            .map_err(|error| format!("coordination settings are invalid: {error}"))?;
        let window_ms = settings.broadcast_rate_window_secs.saturating_mul(1000);
        let floor = now_ms.saturating_sub(window_ms);
        let mut windows = self
            .windows
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for stamps in windows.values_mut() {
            while stamps.front().is_some_and(|stamp| *stamp <= floor) {
                stamps.pop_front();
            }
        }
        let oldest_in_window = {
            let stamps = windows.entry(key.to_string()).or_default();
            if stamps.len() >= settings.broadcast_rate_limit {
                stamps.front().copied()
            } else {
                stamps.push_back(now_ms);
                None
            }
        };
        windows.retain(|_, stamps| !stamps.is_empty());
        let Some(oldest) = oldest_in_window else {
            return Ok(());
        };
        let retry_in_secs = oldest
            .saturating_add(window_ms)
            .saturating_sub(now_ms)
            .saturating_add(999)
            / 1000;
        Err(format!(
            "broadcast rate limit reached: {} broadcasts per {}s are allowed and nothing was sent; retry in {}s",
            settings.broadcast_rate_limit, settings.broadcast_rate_window_secs, retry_in_secs
        ))
    }
}

/// Stable id for one logical broadcast reaching one recipient chat, so a
/// repeated send of the same text is recognised by the delivery layer as a
/// duplicate instead of landing twice.
pub fn broadcast_delivery_id(task_id: &str, card_id: &str, chat_id: &str, message: &str) -> String {
    format!(
        "broadcast:{}",
        crate::file_activity::digest_of(&format!(
            "{task_id}\u{1}{card_id}\u{1}{chat_id}\u{1}{message}"
        ))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_logical_broadcast_maps_to_one_id_per_recipient() {
        let id = broadcast_delivery_id("task", "T-1", "chat-1", "use API Y");
        assert_eq!(
            id,
            broadcast_delivery_id("task", "T-1", "chat-1", "use API Y")
        );
        for other in [
            broadcast_delivery_id("task", "T-1", "chat-2", "use API Y"),
            broadcast_delivery_id("task", "T-2", "chat-1", "use API Y"),
            broadcast_delivery_id("other", "T-1", "chat-1", "use API Y"),
            broadcast_delivery_id("task", "T-1", "chat-1", "use API Z"),
        ] {
            assert_ne!(id, other);
        }
        assert_ne!(
            broadcast_delivery_id("a", "b", "c", "d"),
            broadcast_delivery_id("a", "b", "c\u{1}d", ""),
            "field separator must not be forgeable by concatenation"
        );
    }

    #[test]
    fn the_window_refuses_truthfully_and_reopens_when_it_slides() {
        let limiter = BroadcastLimiter::default();
        let limit = refact_work_core::coordination_settings().broadcast_rate_limit;
        let window_ms = refact_work_core::coordination_settings().broadcast_rate_window_secs * 1000;
        for index in 0..limit {
            limiter.admit("task-1", 1000 + index as u64).unwrap();
        }
        let error = limiter.admit("task-1", 1000 + limit as u64).unwrap_err();
        assert!(error.contains("nothing was sent"), "{error}");
        assert!(error.contains("retry in"), "{error}");
        limiter.admit("task-2", 1000).expect("limits are per key");
        limiter
            .admit("task-1", 1000 + window_ms + 1)
            .expect("a slid window admits again");
    }
}
