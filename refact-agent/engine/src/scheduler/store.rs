use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use super::types::ScheduledTask;

#[async_trait]
pub trait CronStore: Send + Sync {
    async fn add(&self, task: ScheduledTask) -> Result<(), String>;
    async fn remove(&self, id: &str) -> Result<bool, String>;
    async fn list(&self) -> Vec<ScheduledTask>;
    async fn update_fired(
        &self,
        id: &str,
        last_fired_at_ms: u64,
        fire_count: u32,
    ) -> Result<(), String>;
}

#[derive(Clone, Default)]
pub struct InMemoryCronStore {
    tasks: Arc<RwLock<HashMap<String, ScheduledTask>>>,
}

impl InMemoryCronStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn from_tasks(tasks: Vec<ScheduledTask>) -> Self {
        Self {
            tasks: Arc::new(RwLock::new(
                tasks
                    .into_iter()
                    .map(|task| (task.id.clone(), task))
                    .collect(),
            )),
        }
    }
}

#[async_trait]
impl CronStore for InMemoryCronStore {
    async fn add(&self, task: ScheduledTask) -> Result<(), String> {
        self.tasks.write().await.insert(task.id.clone(), task);
        Ok(())
    }

    async fn remove(&self, id: &str) -> Result<bool, String> {
        Ok(self.tasks.write().await.remove(id).is_some())
    }

    async fn list(&self) -> Vec<ScheduledTask> {
        let mut tasks = self
            .tasks
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        tasks.sort_by(|left, right| left.id.cmp(&right.id));
        tasks
    }

    async fn update_fired(
        &self,
        id: &str,
        last_fired_at_ms: u64,
        fire_count: u32,
    ) -> Result<(), String> {
        let mut tasks = self.tasks.write().await;
        let task = tasks
            .get_mut(id)
            .ok_or_else(|| format!("Scheduled task {id} not found"))?;
        task.last_fired_at_ms = Some(last_fired_at_ms);
        task.fire_count = fire_count;
        Ok(())
    }
}

pub struct JsonFileCronStore {
    path: PathBuf,
    cache: InMemoryCronStore,
}

impl JsonFileCronStore {
    pub fn new(project_root: impl AsRef<Path>) -> Result<Self, String> {
        let path = scheduled_tasks_path(project_root.as_ref());
        Self::from_scheduled_tasks_path(path)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn from_scheduled_tasks_path(path: PathBuf) -> Result<Self, String> {
        let tasks = read_tasks(&path)?;
        Ok(Self {
            path,
            cache: InMemoryCronStore::from_tasks(tasks),
        })
    }

    async fn persist(&self) -> Result<(), String> {
        write_tasks(&self.path, &self.cache.list().await)
    }
}

#[async_trait]
impl CronStore for JsonFileCronStore {
    async fn add(&self, task: ScheduledTask) -> Result<(), String> {
        self.cache.add(task).await?;
        self.persist().await
    }

    async fn remove(&self, id: &str) -> Result<bool, String> {
        let removed = self.cache.remove(id).await?;
        if removed {
            self.persist().await?;
        }
        Ok(removed)
    }

    async fn list(&self) -> Vec<ScheduledTask> {
        self.cache.list().await
    }

    async fn update_fired(
        &self,
        id: &str,
        last_fired_at_ms: u64,
        fire_count: u32,
    ) -> Result<(), String> {
        self.cache
            .update_fired(id, last_fired_at_ms, fire_count)
            .await?;
        self.persist().await
    }
}

pub fn scheduled_tasks_path(project_root: &Path) -> PathBuf {
    project_root.join(".refact").join("scheduled_tasks.json")
}

fn read_tasks(path: &Path) -> Result<Vec<ScheduledTask>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(path)
        .map_err(|error| format!("Failed to read scheduled tasks: {error}"))?;
    serde_json::from_str(&content)
        .map_err(|error| format!("Failed to parse scheduled tasks: {error}"))
}

fn write_tasks(path: &Path, tasks: &[ScheduledTask]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create scheduler storage directory: {error}"))?;
    }
    let content = serde_json::to_string_pretty(tasks)
        .map_err(|error| format!("Failed to serialize scheduled tasks: {error}"))?;
    let tmp_path = path.with_extension("json.tmp");
    std::fs::write(&tmp_path, content)
        .map_err(|error| format!("Failed to write scheduled tasks: {error}"))?;
    std::fs::rename(&tmp_path, path)
        .map_err(|error| format!("Failed to persist scheduled tasks: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_task(id: &str) -> ScheduledTask {
        ScheduledTask {
            id: id.to_string(),
            cron: "*/5 * * * *".to_string(),
            prompt: "Check the build".to_string(),
            description: "Check build".to_string(),
            recurring: true,
            durable: true,
            created_at_ms: 123,
            chat_id: Some("chat-1".to_string()),
            mode: Some("agent".to_string()),
            last_fired_at_ms: None,
            fire_count: 0,
            auto_expire_after_ms: super::super::types::DEFAULT_RECURRING_AUTO_EXPIRE_AFTER_MS,
        }
    }

    #[tokio::test]
    async fn in_memory_add_list_remove() {
        let store = InMemoryCronStore::new();
        let task = test_task("cron_1");

        store.add(task.clone()).await.unwrap();
        assert_eq!(store.list().await, vec![task]);
        assert!(store.remove("cron_1").await.unwrap());
        assert!(store.list().await.is_empty());
        assert!(!store.remove("cron_1").await.unwrap());
    }

    #[tokio::test]
    async fn json_file_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let task = test_task("cron_1");

        {
            let store = JsonFileCronStore::new(temp.path()).unwrap();
            assert_eq!(store.path(), scheduled_tasks_path(temp.path()));
            store.add(task.clone()).await.unwrap();
        }

        let store = JsonFileCronStore::new(temp.path()).unwrap();
        assert_eq!(store.list().await, vec![task]);
    }
}
