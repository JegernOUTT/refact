use std::sync::Arc;

use tokio::task::JoinHandle;

use super::store::CronStore;

pub struct CronRunner {
    store: Arc<dyn CronStore>,
}

impl CronRunner {
    pub fn new(store: Arc<dyn CronStore>) -> Self {
        Self { store }
    }

    pub fn store(&self) -> Arc<dyn CronStore> {
        self.store.clone()
    }
}

pub fn spawn(store: Arc<dyn CronStore>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let runner = CronRunner::new(store);
        drop(runner);
    })
}
