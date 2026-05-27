use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const DEFAULT_RECURRING_AUTO_EXPIRE_AFTER_MS: u64 = 30 * 24 * 60 * 60 * 1000;
pub const DEFAULT_SCHEDULER_MAX_JOBS: u32 = 50;
pub const DURABLE_DISABLED_NOTE: &str = "durable schedules disabled by config";
pub const SCHEDULER_DISABLED_ERROR: &str = "scheduler is disabled";
pub const SCHEDULER_DISABLE_ENV: &str = "REFACT_DISABLE_SCHEDULER";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SchedulerConfig {
    #[serde(default = "default_scheduler_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub disable_durable: bool,
    #[serde(default = "default_scheduler_max_jobs")]
    pub max_jobs: u32,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            disable_durable: false,
            max_jobs: DEFAULT_SCHEDULER_MAX_JOBS,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CronCreatePolicy {
    pub durable: bool,
    pub note: Option<String>,
}

pub fn cron_create_policy(
    config: &SchedulerConfig,
    requested_durable: bool,
) -> Result<CronCreatePolicy, String> {
    if !config.enabled {
        return Err(SCHEDULER_DISABLED_ERROR.to_string());
    }
    if requested_durable && config.disable_durable {
        return Ok(CronCreatePolicy {
            durable: false,
            note: Some(DURABLE_DISABLED_NOTE.to_string()),
        });
    }
    Ok(CronCreatePolicy {
        durable: requested_durable,
        note: None,
    })
}

pub fn scheduler_disabled_by_env() -> bool {
    std::env::var(SCHEDULER_DISABLE_ENV).ok().as_deref() == Some("1")
}

fn default_scheduler_enabled() -> bool {
    true
}

fn default_scheduler_max_jobs() -> u32 {
    DEFAULT_SCHEDULER_MAX_JOBS
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ScheduledTask {
    pub id: String,
    pub cron: String,
    pub prompt: String,
    pub description: String,
    pub recurring: bool,
    pub durable: bool,
    pub created_at_ms: u64,
    pub chat_id: Option<String>,
    pub mode: Option<String>,
    pub last_fired_at_ms: Option<u64>,
    pub fire_count: u32,
    pub auto_expire_after_ms: u64,
}

impl ScheduledTask {
    pub fn new(
        cron: String,
        prompt: String,
        description: String,
        recurring: bool,
        durable: bool,
        created_at_ms: u64,
    ) -> Self {
        Self {
            id: format!("cron_{}", Uuid::now_v7()),
            cron,
            prompt,
            description,
            recurring,
            durable,
            created_at_ms,
            chat_id: None,
            mode: None,
            last_fired_at_ms: None,
            fire_count: 0,
            auto_expire_after_ms: if recurring {
                DEFAULT_RECURRING_AUTO_EXPIRE_AFTER_MS
            } else {
                0
            },
        }
    }
}
