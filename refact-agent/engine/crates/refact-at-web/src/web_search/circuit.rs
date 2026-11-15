use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use super::SearchResult;
const BACKEND_FAILURE_THRESHOLD: u32 = 3;
const BACKEND_FAILURE_COOLDOWN: Duration = Duration::from_secs(60);
const BACKEND_RATE_LIMIT_COOLDOWN: Duration = Duration::from_secs(300);
#[derive(Debug, Clone, Copy)]
struct BackendFailureState {
    consecutive_failures: u32,
    cooldown_until: Option<Instant>,
    recovery_mode: bool,
    next_attempt: u64,
    latest_completed_attempt: u64,
    probe_attempt: Option<u64>,
}

impl Default for BackendFailureState {
    fn default() -> Self {
        Self {
            consecutive_failures: 0,
            cooldown_until: None,
            recovery_mode: false,
            next_attempt: 1,
            latest_completed_attempt: 0,
            probe_attempt: None,
        }
    }
}

fn backend_failure_states() -> &'static Mutex<HashMap<String, BackendFailureState>> {
    static STATES: OnceLock<Mutex<HashMap<String, BackendFailureState>>> = OnceLock::new();
    STATES.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(test)]
fn backend_cooldown_remaining(backend: &str, now: Instant) -> Option<Duration> {
    let states = backend_failure_states()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let state = states.get(backend).copied()?;
    match state.cooldown_until {
        Some(until) if until > now => Some(until.duration_since(now)),
        Some(_) => None,
        None => None,
    }
}

pub(super) enum BackendAttemptLease {
    Run(u64),
    Skip(String),
}

pub(super) struct BackendAttemptGuard {
    pub(super) backend: &'static str,
    pub(super) attempt: u64,
}

impl Drop for BackendAttemptGuard {
    fn drop(&mut self) {
        let mut states = backend_failure_states()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(state) = states.get_mut(self.backend) {
            if state.probe_attempt == Some(self.attempt) {
                state.probe_attempt = None;
            }
        }
    }
}

pub(super) fn lease_backend_attempt(backend: &str, now: Instant) -> BackendAttemptLease {
    let mut states = backend_failure_states()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let state = states.entry(backend.to_string()).or_default();
    if let Some(until) = state.cooldown_until {
        if until > now {
            return BackendAttemptLease::Skip(format!(
                "temporarily skipped after repeated failures; retry in {}s",
                until.duration_since(now).as_secs().max(1)
            ));
        }
    }
    if state.recovery_mode && state.probe_attempt.is_some() {
        return BackendAttemptLease::Skip(
            "temporarily skipped while a recovery probe is in progress".to_string(),
        );
    }

    let attempt = state.next_attempt;
    state.next_attempt = state.next_attempt.saturating_add(1);
    if state.recovery_mode {
        state.probe_attempt = Some(attempt);
    }
    BackendAttemptLease::Run(attempt)
}

pub(super) fn error_needs_long_cooldown(detail: &str) -> bool {
    let detail = detail.to_ascii_lowercase();
    detail.contains("429")
        || detail.contains("rate limit")
        || detail.contains("too many requests")
        || detail.contains("usage limit")
        || detail.contains("captcha")
        || detail.contains("bot-detection")
        || detail.contains("bot detection")
        || detail.contains("verify you are human")
        || detail.contains("verify you're human")
}

pub(super) fn record_backend_result(
    backend: &str,
    attempt: u64,
    result: &Result<Vec<SearchResult>, String>,
) {
    let mut states = backend_failure_states()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let state = states.entry(backend.to_string()).or_default();
    let was_recovery_probe = state.probe_attempt == Some(attempt);
    if was_recovery_probe {
        state.probe_attempt = None;
    }
    if attempt <= state.latest_completed_attempt {
        return;
    }
    state.latest_completed_attempt = attempt;
    match result {
        Ok(_) => {
            state.consecutive_failures = 0;
            state.cooldown_until = None;
            state.recovery_mode = false;
        }
        Err(error) => {
            state.consecutive_failures = state.consecutive_failures.saturating_add(1);
            let cooldown = if error_needs_long_cooldown(error) {
                Some(BACKEND_RATE_LIMIT_COOLDOWN)
            } else if state.consecutive_failures >= BACKEND_FAILURE_THRESHOLD {
                Some(BACKEND_FAILURE_COOLDOWN)
            } else {
                None
            };
            if let Some(cooldown) = cooldown {
                state.cooldown_until = Some(Instant::now() + cooldown);
                state.recovery_mode = true;
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn lease(id: &str, now: Instant) -> u64 {
        match lease_backend_attempt(id, now) {
            BackendAttemptLease::Run(attempt) => attempt,
            BackendAttemptLease::Skip(reason) => panic!("Unexpected skip: {reason}"),
        }
    }

    fn assert_skipped(id: &str, now: Instant, reason: &str) {
        match lease_backend_attempt(id, now) {
            BackendAttemptLease::Skip(detail) => assert!(detail.contains(reason), "{detail}"),
            BackendAttemptLease::Run(_) => panic!("Expected skip for {id}"),
        }
    }

    #[test]
    fn threshold_single_probe_and_cancelled_probe() {
        let id = "circuit-test-threshold-probe";
        let now = Instant::now();
        for count in 1..=BACKEND_FAILURE_THRESHOLD {
            let attempt = lease(id, now);
            record_backend_result(id, attempt, &Err("network error".into()));
            if count < BACKEND_FAILURE_THRESHOLD {
                assert!(backend_cooldown_remaining(id, now).is_none());
            }
        }
        assert!(backend_cooldown_remaining(id, now).unwrap() >= BACKEND_FAILURE_COOLDOWN);
        assert_skipped(id, now, "repeated failures");
        let later = Instant::now() + BACKEND_FAILURE_COOLDOWN + Duration::from_secs(1);
        let probe = lease(id, later);
        let guard = BackendAttemptGuard {
            backend: id,
            attempt: probe,
        };
        assert_skipped(id, later, "recovery probe");
        drop(guard);
        let replacement = lease(id, later);
        assert!(replacement > probe);
        record_backend_result(id, replacement, &Ok(vec![]));
        assert!(backend_cooldown_remaining(id, Instant::now()).is_none());
        let first = lease(id, later);
        let second = lease(id, later);
        assert!(second > first);
    }

    #[test]
    fn stale_completions_cannot_clear_newer_failure_or_override_newer_success() {
        let id = "circuit-test-stale-completion";
        let now = Instant::now();
        let older = lease(id, now);
        let newer = lease(id, now);
        record_backend_result(id, newer, &Err("429".into()));
        record_backend_result(id, older, &Ok(vec![]));
        assert_skipped(id, now, "repeated failures");
        assert!(backend_cooldown_remaining(id, now).unwrap() >= BACKEND_RATE_LIMIT_COOLDOWN);
        let later = Instant::now() + BACKEND_RATE_LIMIT_COOLDOWN + Duration::from_secs(1);
        let probe = lease(id, later);
        record_backend_result(id, probe, &Ok(vec![]));
        record_backend_result(id, newer, &Err("429".into()));
        assert!(backend_cooldown_remaining(id, Instant::now()).is_none());
        let first = lease(id, later);
        let second = lease(id, later);
        record_backend_result(id, second, &Ok(vec![]));
        record_backend_result(id, first, &Err("429".into()));
        assert!(backend_cooldown_remaining(id, Instant::now()).is_none());
        lease(id, later);
    }

    #[test]
    fn failed_recovery_reopens_and_stale_guard_keeps_current_probe() {
        let id = "circuit-test-failed-recovery";
        let now = Instant::now();
        let first = lease(id, now);
        record_backend_result(id, first, &Err("rate limit".into()));
        let later = Instant::now() + BACKEND_RATE_LIMIT_COOLDOWN + Duration::from_secs(1);
        let probe = lease(id, later);
        let guard = BackendAttemptGuard {
            backend: id,
            attempt: probe,
        };
        record_backend_result(id, probe, &Err("network error".into()));
        let replacement = lease(id, later);
        drop(guard);
        assert_skipped(id, later, "recovery probe");
        record_backend_result(id, replacement, &Err("CAPTCHA".into()));
        assert!(backend_cooldown_remaining(id, Instant::now()).is_some());
        assert_skipped(id, Instant::now(), "repeated failures");
    }

    #[test]
    fn rate_limit_detection_is_case_insensitive_and_specific() {
        for detail in [
            "HTTP 429",
            "RATE LIMIT",
            "Too Many Requests",
            "usage limit reached",
            "CAPTCHA",
            "bot-detection",
            "BOT DETECTION",
            "verify you are human",
            "verify you're human",
        ] {
            assert!(error_needs_long_cooldown(detail), "{detail}");
        }
        for detail in [
            "network error",
            "HTTP 500",
            "Provider timed out",
            "invalid response",
        ] {
            assert!(!error_needs_long_cooldown(detail), "{detail}");
        }
    }
}
