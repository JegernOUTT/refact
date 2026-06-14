pub mod cron_expr;
pub mod delivery;
pub mod exec_action;
pub mod jitter;
pub mod runner;
pub mod schedule;
pub mod store;
pub mod types;

pub use cron_expr::{CronSchedule, human_schedule, next_run_ms as cron_next_run_ms, parse_cron};
pub use runner::{runner_change_notify, session_cron_store, CronRunner, spawn, spawn_if_enabled};
pub use schedule::{ScheduleTarget, next_run_ms};
pub use store::{scheduled_tasks_path, CronStore, InMemoryCronStore, JsonFileCronStore};
pub use types::{
    Action, AgentTarget, CommandSpec, CronCreatePolicy, CronRunRecord, Delivery,
    DEFAULT_RECURRING_AUTO_EXPIRE_AFTER_MS, DEFAULT_SCHEDULER_MAX_JOBS, DURABLE_DISABLED_NOTE, Job,
    RECENT_RUNS_CAP, SCHEDULER_DISABLE_ENV, SCHEDULER_DISABLED_ERROR, SchedulerConfig, Trigger,
    cron_create_policy, delivery_from_value,
};

pub fn scheduler_timezone() -> chrono_tz::Tz {
    iana_time_zone::get_timezone()
        .ok()
        .and_then(|value| value.parse::<chrono_tz::Tz>().ok())
        .or_else(|| {
            std::env::var("TZ")
                .ok()
                .and_then(|value| value.trim_start_matches(':').parse::<chrono_tz::Tz>().ok())
        })
        .unwrap_or(chrono_tz::UTC)
}

pub async fn active_durable_cron_store(
    gcx: std::sync::Arc<crate::global_context::GlobalContext>,
) -> Result<Option<std::sync::Arc<dyn CronStore>>, String> {
    match crate::files_correction::get_active_project_path(gcx).await {
        None => Ok(None),
        Some(path) => JsonFileCronStore::new(path)
            .map(|store| Some(std::sync::Arc::new(store) as std::sync::Arc<dyn CronStore>)),
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::sync::Arc;

    use super::*;

    struct EnvGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    fn memory_store() -> Arc<dyn CronStore> {
        Arc::new(InMemoryCronStore::new())
    }

    #[test]
    #[serial_test::serial]
    fn env_disable_skips_runner_spawn() {
        let _guard = EnvGuard::set(SCHEDULER_DISABLE_ENV, "1");
        let config = SchedulerConfig::default().with_startup_overrides(false);

        assert!(!config.enabled);
        assert!(spawn_if_enabled(memory_store(), config).is_none());
        assert_eq!(
            cron_create_policy(&config, false),
            Err(SCHEDULER_DISABLED_ERROR.to_string())
        );
    }

    #[test]
    fn config_disable_skips_runner_spawn() {
        let config = SchedulerConfig {
            enabled: false,
            ..SchedulerConfig::default()
        };

        assert!(spawn_if_enabled(memory_store(), config).is_none());
        assert_eq!(
            cron_create_policy(&config, true),
            Err(SCHEDULER_DISABLED_ERROR.to_string())
        );
    }

    #[test]
    fn cli_disable_skips_runner_spawn() {
        let config = SchedulerConfig::default().with_startup_overrides(true);

        assert!(!config.enabled);
        assert!(spawn_if_enabled(memory_store(), config).is_none());
    }

    #[test]
    fn disable_durable_forces_session_only() {
        let config = SchedulerConfig {
            disable_durable: true,
            ..SchedulerConfig::default()
        };

        let policy = cron_create_policy(&config, true).unwrap();

        assert_eq!(
            policy,
            CronCreatePolicy {
                durable: false,
                note: Some(DURABLE_DISABLED_NOTE.to_string()),
            }
        );
    }
}
