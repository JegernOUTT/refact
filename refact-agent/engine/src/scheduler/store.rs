use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{Notify, RwLock};

use super::types::{Job, Trigger};

#[async_trait]
pub trait CronStore: Send + Sync {
    async fn add(&self, job: Job) -> Result<(), String>;
    async fn get(&self, id: &str) -> Option<Job>;
    async fn replace(&self, job: Job) -> Result<bool, String>;
    async fn remove(&self, id: &str) -> Result<bool, String>;
    async fn list(&self) -> Vec<Job>;
    async fn jobs_by_hook_id(&self, hook_id: &str) -> Vec<Job> {
        let hook_id = hook_id.trim();
        if hook_id.is_empty() {
            return Vec::new();
        }
        self.list()
            .await
            .into_iter()
            .filter(|job| {
                matches!(&job.trigger, Trigger::Webhook { hook_id: job_hook_id } if job_hook_id == hook_id)
            })
            .collect()
    }
    async fn update_fired(
        &self,
        id: &str,
        last_fired_at_ms: u64,
        fire_count: u32,
    ) -> Result<(), String>;
    fn change_notify(&self) -> Arc<Notify>;
}

#[derive(Clone)]
pub struct InMemoryCronStore {
    jobs: Arc<RwLock<HashMap<String, Job>>>,
    change_notify: Arc<Notify>,
}

impl Default for InMemoryCronStore {
    fn default() -> Self {
        Self {
            jobs: Arc::new(RwLock::new(HashMap::new())),
            change_notify: Arc::new(Notify::new()),
        }
    }
}

impl InMemoryCronStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn from_jobs(jobs: Vec<Job>) -> Self {
        Self {
            jobs: Arc::new(RwLock::new(
                jobs.into_iter().map(|job| (job.id.clone(), job)).collect(),
            )),
            change_notify: Arc::new(Notify::new()),
        }
    }
}

#[async_trait]
impl CronStore for InMemoryCronStore {
    async fn add(&self, job: Job) -> Result<(), String> {
        self.jobs.write().await.insert(job.id.clone(), job);
        self.change_notify.notify_waiters();
        Ok(())
    }

    async fn get(&self, id: &str) -> Option<Job> {
        self.jobs.read().await.get(id).cloned()
    }

    async fn replace(&self, job: Job) -> Result<bool, String> {
        let mut jobs = self.jobs.write().await;
        if !jobs.contains_key(&job.id) {
            return Ok(false);
        }
        jobs.insert(job.id.clone(), job);
        self.change_notify.notify_waiters();
        Ok(true)
    }

    async fn remove(&self, id: &str) -> Result<bool, String> {
        let removed = self.jobs.write().await.remove(id).is_some();
        if removed {
            self.change_notify.notify_waiters();
        }
        Ok(removed)
    }

    async fn list(&self) -> Vec<Job> {
        let mut jobs = self.jobs.read().await.values().cloned().collect::<Vec<_>>();
        jobs.sort_by(|left, right| left.id.cmp(&right.id));
        jobs
    }

    async fn update_fired(
        &self,
        id: &str,
        last_fired_at_ms: u64,
        fire_count: u32,
    ) -> Result<(), String> {
        let mut jobs = self.jobs.write().await;
        let job = jobs
            .get_mut(id)
            .ok_or_else(|| format!("Scheduled task {id} not found"))?;
        job.last_fired_at_ms = Some(last_fired_at_ms);
        job.fire_count = fire_count;
        self.change_notify.notify_waiters();
        Ok(())
    }

    fn change_notify(&self) -> Arc<Notify> {
        self.change_notify.clone()
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
        let jobs = read_jobs(&path)?;
        Ok(Self {
            path,
            cache: InMemoryCronStore::from_jobs(jobs),
        })
    }

    async fn persist(&self) -> Result<(), String> {
        write_jobs(&self.path, &self.cache.list().await)
    }
}

#[async_trait]
impl CronStore for JsonFileCronStore {
    async fn add(&self, job: Job) -> Result<(), String> {
        self.cache.add(job).await?;
        self.persist().await
    }

    async fn get(&self, id: &str) -> Option<Job> {
        self.cache.get(id).await
    }

    async fn replace(&self, job: Job) -> Result<bool, String> {
        let replaced = self.cache.replace(job).await?;
        if replaced {
            self.persist().await?;
        }
        Ok(replaced)
    }

    async fn remove(&self, id: &str) -> Result<bool, String> {
        let removed = self.cache.remove(id).await?;
        if removed {
            self.persist().await?;
        }
        Ok(removed)
    }

    async fn list(&self) -> Vec<Job> {
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

    fn change_notify(&self) -> Arc<Notify> {
        self.cache.change_notify()
    }
}

pub fn scheduled_tasks_path(project_root: &Path) -> PathBuf {
    project_root.join(".refact").join("scheduled_tasks.json")
}

fn read_jobs(path: &Path) -> Result<Vec<Job>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(path)
        .map_err(|error| format!("Failed to read scheduled tasks: {error}"))?;
    serde_json::from_str(&content)
        .map_err(|error| format!("Failed to parse scheduled tasks: {error}"))
}

fn write_jobs(path: &Path, jobs: &[Job]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create scheduler storage directory: {error}"))?;
    }
    let content = serde_json::to_string_pretty(jobs)
        .map_err(|error| format!("Failed to serialize scheduled tasks: {error}"))?;
    let tmp_path = path.with_extension("json.tmp");
    std::fs::write(&tmp_path, content)
        .map_err(|error| format!("Failed to write scheduled tasks: {error}"))?;
    std::fs::rename(&tmp_path, path)
        .map_err(|error| format!("Failed to persist scheduled tasks: {error}"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn test_job(id: &str) -> Job {
        let mut job = Job::new_cron_agent_chat(
            "*/5 * * * *".to_string(),
            "Check the build".to_string(),
            "Check build".to_string(),
            true,
            true,
            123,
        );
        job.id = id.to_string();
        job.set_existing_chat(Some("chat-1".to_string()));
        job.set_mode(Some("agent".to_string()));
        job
    }

    #[tokio::test]
    async fn in_memory_add_get_list_replace_remove() {
        let store = InMemoryCronStore::new();
        let mut job = test_job("cron_1");

        store.add(job.clone()).await.unwrap();
        assert_eq!(store.get("cron_1").await, Some(job.clone()));
        assert_eq!(store.list().await, vec![job.clone()]);

        job.description = "Updated".to_string();
        assert!(store.replace(job.clone()).await.unwrap());
        assert_eq!(store.get("cron_1").await, Some(job));
        assert!(!store.replace(test_job("missing")).await.unwrap());

        assert!(store.remove("cron_1").await.unwrap());
        assert!(store.list().await.is_empty());
        assert!(!store.remove("cron_1").await.unwrap());
    }

    #[tokio::test]
    async fn json_file_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let job = test_job("cron_1");

        {
            let store = JsonFileCronStore::new(temp.path()).unwrap();
            assert_eq!(store.path(), scheduled_tasks_path(temp.path()));
            store.add(job.clone()).await.unwrap();
        }

        let store = JsonFileCronStore::new(temp.path()).unwrap();
        assert_eq!(store.list().await, vec![job]);
    }

    #[tokio::test]
    async fn legacy_file_loads_and_rewrites_nested() {
        let temp = tempfile::tempdir().unwrap();
        let path = scheduled_tasks_path(temp.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            json!([{
                "id": "cron_legacy",
                "cron": "7 * * * *",
                "prompt": "Check the frogs",
                "description": "Hourly frog check",
                "recurring": false,
                "durable": true,
                "created_at_ms": 123,
                "chat_id": "chat-1",
                "mode": "agent",
                "last_fired_at_ms": null,
                "fire_count": 0,
                "auto_expire_after_ms": 0
            }])
            .to_string(),
        )
        .unwrap();

        let store = JsonFileCronStore::new(temp.path()).unwrap();
        let loaded = store.list().await;
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "cron_legacy");
        assert_eq!(loaded[0].cron_expr(), Some("7 * * * *"));
        assert_eq!(loaded[0].prompt(), Some("Check the frogs"));

        store.add(test_job("cron_new")).await.unwrap();
        let rewritten: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(rewritten[0].get("trigger").is_some());
        assert!(rewritten[0].get("action").is_some());
        assert!(rewritten[0].get("delivery").is_some());
        assert!(rewritten[0].get("cron").is_none());
        assert!(rewritten[0].get("prompt").is_none());
    }

    #[tokio::test]
    async fn json_file_jobs_by_hook_id_returns_matching_webhook_jobs_only() {
        let temp = tempfile::tempdir().unwrap();
        let store = JsonFileCronStore::new(temp.path()).unwrap();
        let mut matching = test_job("cron_matching_file");
        matching.trigger = Trigger::Webhook {
            hook_id: "deploy".to_string(),
        };
        let mut nonmatching = test_job("cron_other_file");
        nonmatching.trigger = Trigger::Webhook {
            hook_id: "other".to_string(),
        };
        store.add(matching).await.unwrap();
        store.add(nonmatching).await.unwrap();

        let jobs = store.jobs_by_hook_id("deploy").await;

        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].id, "cron_matching_file");
    }

    #[tokio::test]
    async fn replace_updates_in_place() {
        let temp = tempfile::tempdir().unwrap();
        let store = JsonFileCronStore::new(temp.path()).unwrap();
        let mut job = test_job("cron_replace");
        store.add(job.clone()).await.unwrap();

        job.description = "Updated description".to_string();
        assert!(store.replace(job.clone()).await.unwrap());
        assert!(!store.replace(test_job("cron_missing")).await.unwrap());

        let listed = store.list().await;
        assert_eq!(listed, vec![job.clone()]);
        let serialized: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(store.path()).unwrap()).unwrap();
        assert_eq!(serialized.as_array().unwrap().len(), 1);
        assert_eq!(serialized[0]["description"], json!(job.description));
    }

    #[tokio::test]
    async fn jobs_by_hook_id_returns_matching_webhook_jobs_only() {
        let store = InMemoryCronStore::new();
        let mut matching = test_job("cron_matching");
        matching.trigger = Trigger::Webhook {
            hook_id: "deploy".to_string(),
        };
        let mut nonmatching = test_job("cron_other_hook");
        nonmatching.trigger = Trigger::Webhook {
            hook_id: "other".to_string(),
        };
        let timed = test_job("cron_timed");
        store.add(matching).await.unwrap();
        store.add(nonmatching).await.unwrap();
        store.add(timed).await.unwrap();

        let jobs = store.jobs_by_hook_id("deploy").await;

        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].id, "cron_matching");
        assert!(store.jobs_by_hook_id("   ").await.is_empty());
    }
}
