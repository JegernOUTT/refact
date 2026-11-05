pub use refact_scheduler_core::types::*;

#[cfg(test)]
pub fn test_scheduler_config_with<F>(configure: F) -> SchedulerConfig
where
    F: FnOnce(&mut SchedulerConfig),
{
    let mut config = SchedulerConfig::default();
    configure(&mut config);
    config
}
