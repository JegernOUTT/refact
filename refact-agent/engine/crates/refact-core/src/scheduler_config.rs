use serde::{Deserialize, Serialize};

pub const DEFAULT_RETRY_MAX_ATTEMPTS: u32 = 3;
pub const DEFAULT_RETRY_BACKOFF_MS: [u64; 3] = [60_000, 120_000, 300_000];
pub const DEFAULT_SCHEDULER_MAX_JOBS: u32 = 50;
pub const DEFAULT_SCHEDULER_MAX_CONCURRENT_RUNS: usize = 8;
pub const DEFAULT_MISSED_GRACE_MIN_MS: u64 = 120 * 1000;
pub const DEFAULT_MISSED_GRACE_MAX_MS: u64 = 2 * 60 * 60 * 1000;
pub const SCHEDULER_DISABLE_ENV: &str = "REFACT_DISABLE_SCHEDULER";
pub const RECENT_RUNS_CAP: usize = 20;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RetryConfig {
    #[serde(default = "default_retry_max_attempts")]
    pub max_attempts: u32,
    #[serde(default = "default_retry_backoff_ms")]
    pub backoff_ms: Vec<u64>,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: DEFAULT_RETRY_MAX_ATTEMPTS,
            backoff_ms: default_retry_backoff_ms(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SchedulerConfig {
    #[serde(default = "default_scheduler_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub disable_durable: bool,
    #[serde(default = "default_scheduler_max_jobs")]
    pub max_jobs: u32,
    #[serde(default = "default_scheduler_max_concurrent_runs")]
    pub max_concurrent_runs: usize,
    #[serde(default = "default_scheduler_recent_runs_cap")]
    pub recent_runs_cap: usize,
    #[serde(default = "default_missed_grace_min_ms")]
    pub missed_grace_min_ms: u64,
    #[serde(default = "default_missed_grace_max_ms")]
    pub missed_grace_max_ms: u64,
    #[serde(default)]
    pub retry: RetryConfig,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            disable_durable: false,
            max_jobs: DEFAULT_SCHEDULER_MAX_JOBS,
            max_concurrent_runs: DEFAULT_SCHEDULER_MAX_CONCURRENT_RUNS,
            recent_runs_cap: RECENT_RUNS_CAP,
            missed_grace_min_ms: DEFAULT_MISSED_GRACE_MIN_MS,
            missed_grace_max_ms: DEFAULT_MISSED_GRACE_MAX_MS,
            retry: RetryConfig::default(),
        }
    }
}

impl SchedulerConfig {
    pub fn with_startup_overrides(mut self, no_scheduler: bool) -> Self {
        if no_scheduler || scheduler_disabled_by_env() {
            self.enabled = false;
        }
        self
    }

    pub fn runner_enabled(&self) -> bool {
        self.enabled
    }
}

pub fn scheduler_disabled_by_env() -> bool {
    std::env::var(SCHEDULER_DISABLE_ENV).ok().as_deref() == Some("1")
}

fn default_retry_max_attempts() -> u32 {
    DEFAULT_RETRY_MAX_ATTEMPTS
}

fn default_retry_backoff_ms() -> Vec<u64> {
    DEFAULT_RETRY_BACKOFF_MS.to_vec()
}

fn default_scheduler_enabled() -> bool {
    true
}

fn default_scheduler_max_jobs() -> u32 {
    DEFAULT_SCHEDULER_MAX_JOBS
}

fn default_scheduler_max_concurrent_runs() -> usize {
    DEFAULT_SCHEDULER_MAX_CONCURRENT_RUNS
}

fn default_scheduler_recent_runs_cap() -> usize {
    RECENT_RUNS_CAP
}

fn default_missed_grace_min_ms() -> u64 {
    DEFAULT_MISSED_GRACE_MIN_MS
}

fn default_missed_grace_max_ms() -> u64 {
    DEFAULT_MISSED_GRACE_MAX_MS
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn scheduler_config_defaults_serde_round_trip() {
        let config: SchedulerConfig = serde_json::from_value(json!({})).unwrap();

        assert_eq!(config, SchedulerConfig::default());

        let serialized = serde_json::to_value(&config).unwrap();
        let round_tripped: SchedulerConfig = serde_json::from_value(serialized).unwrap();

        assert_eq!(round_tripped, config);
        assert_eq!(round_tripped.retry, RetryConfig::default());
    }
}
