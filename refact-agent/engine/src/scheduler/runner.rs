use std::sync::Arc;

use tokio::task::JoinHandle;

use super::store::CronStore;
use super::types::SchedulerConfig;

pub struct CronRunner {
    store: Arc<dyn CronStore>,
    config: SchedulerConfig,
}

impl CronRunner {
    pub fn new(store: Arc<dyn CronStore>) -> Self {
        Self::with_config(store, SchedulerConfig::default())
    }

    pub fn with_config(store: Arc<dyn CronStore>, config: SchedulerConfig) -> Self {
        Self { store, config }
    }

    pub fn store(&self) -> Arc<dyn CronStore> {
        self.store.clone()
    }

    pub fn config(&self) -> SchedulerConfig {
        self.config
    }
}

pub fn spawn(store: Arc<dyn CronStore>) -> JoinHandle<()> {
    spawn_enabled(store, SchedulerConfig::default())
}

pub fn spawn_if_enabled(
    store: Arc<dyn CronStore>,
    config: SchedulerConfig,
) -> Option<JoinHandle<()>> {
    if !config.runner_enabled() {
        return None;
    }
    Some(spawn_enabled(store, config))
}

fn spawn_enabled(store: Arc<dyn CronStore>, config: SchedulerConfig) -> JoinHandle<()> {
    tokio::spawn(async move {
        let runner = CronRunner::with_config(store, config);
        drop(runner);
    })
}
