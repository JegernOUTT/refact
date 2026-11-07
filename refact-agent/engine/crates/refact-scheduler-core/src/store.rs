use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::{Notify, RwLock};

use super::types::{Job, Trigger};

pub type JobPredicate = Box<dyn FnOnce(&Job) -> bool + Send>;

pub type JobMutation = Box<dyn FnOnce(&mut Job) -> Result<(), String> + Send>;

#[async_trait]
pub trait CronStore: Send + Sync {
    async fn add(&self, job: Job) -> Result<(), String>;
    async fn get(&self, id: &str) -> Option<Job>;
    async fn try_get(&self, id: &str) -> Result<Option<Job>, String> {
        Ok(self.get(id).await)
    }
    async fn try_list(&self) -> Result<Vec<Job>, String> {
        Ok(self.list().await)
    }
    async fn remove_if(&self, id: &str, predicate: JobPredicate) -> Result<bool, String>;
    async fn replace(&self, job: Job) -> Result<bool, String>;
    async fn mutate(&self, id: &str, mutation: JobMutation) -> Result<Option<Job>, String>;
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

    async fn mutate(&self, id: &str, mutation: JobMutation) -> Result<Option<Job>, String> {
        let mut jobs = self.jobs.write().await;
        let Some(mut job) = jobs.get(id).cloned() else {
            return Ok(None);
        };
        mutation(&mut job)?;
        jobs.insert(id.to_string(), job.clone());
        self.change_notify.notify_waiters();
        Ok(Some(job))
    }

    async fn remove_if(&self, id: &str, predicate: JobPredicate) -> Result<bool, String> {
        let mut jobs = self.jobs.write().await;
        let remove = jobs.get(id).is_some_and(predicate);
        if remove {
            jobs.remove(id);
            self.change_notify.notify_waiters();
        }
        Ok(remove)
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
    change_notify: Arc<Notify>,
}

impl JsonFileCronStore {
    pub fn new(project_root: impl AsRef<Path>) -> Result<Self, String> {
        let path = scheduled_tasks_path(project_root.as_ref());
        read_jobs(&path)?;
        Ok(Self {
            path,
            change_notify: Arc::new(Notify::new()),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    async fn transaction<T: Send + 'static>(
        &self,
        write: bool,
        operation: impl FnOnce(&mut Vec<Job>) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let path = self.path.clone();
        let result = tokio::task::spawn_blocking(move || {
            if !write
                && !path
                    .try_exists()
                    .map_err(|error| format!("Failed to inspect scheduler storage: {error}"))?
            {
                return operation(&mut Vec::new());
            }
            let _lock = lock_store(&path)?;
            let mut jobs = read_jobs(&path)?;
            let result = operation(&mut jobs)?;
            if write {
                write_jobs(&path, &jobs)?;
            }
            Ok(result)
        })
        .await
        .map_err(|error| format!("Scheduler storage task failed: {error}"))?;
        if write && result.is_ok() {
            self.change_notify.notify_waiters();
        }
        result
    }
}

struct StoreLock(std::fs::File);

impl Drop for StoreLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

fn lock_store(path: &Path) -> Result<StoreLock, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create scheduler storage directory: {error}"))?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.with_extension("json.lock"))
        .map_err(|error| format!("Failed to open scheduler lock: {error}"))?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => return Ok(StoreLock(file)),
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(format!("Failed to lock scheduler storage: {error}")),
        }
    }
}

#[async_trait]
impl CronStore for JsonFileCronStore {
    async fn add(&self, job: Job) -> Result<(), String> {
        self.transaction(true, move |jobs| {
            jobs.retain(|existing| existing.id != job.id);
            jobs.push(job);
            Ok(())
        })
        .await
    }

    async fn get(&self, id: &str) -> Option<Job> {
        self.try_get(id).await.unwrap_or_else(|error| {
            tracing::warn!("Failed to read scheduler storage: {error}");
            None
        })
    }

    async fn try_get(&self, id: &str) -> Result<Option<Job>, String> {
        let id = id.to_string();
        self.transaction(false, move |jobs| {
            Ok(jobs.iter().find(|job| job.id == id).cloned())
        })
        .await
    }

    async fn remove_if(&self, id: &str, predicate: JobPredicate) -> Result<bool, String> {
        let id = id.to_string();
        self.transaction(true, move |jobs| {
            let Some(index) = jobs.iter().position(|job| job.id == id) else {
                return Ok(false);
            };
            if !predicate(&jobs[index]) {
                return Ok(false);
            }
            jobs.remove(index);
            Ok(true)
        })
        .await
    }

    async fn replace(&self, job: Job) -> Result<bool, String> {
        let id = job.id.clone();
        Ok(self
            .mutate(
                &id,
                Box::new(move |stored| {
                    *stored = job;
                    Ok(())
                }),
            )
            .await?
            .is_some())
    }

    async fn mutate(&self, id: &str, mutation: JobMutation) -> Result<Option<Job>, String> {
        let id = id.to_string();
        self.transaction(true, move |jobs| {
            let Some(job) = jobs.iter_mut().find(|job| job.id == id) else {
                return Ok(None);
            };
            mutation(job)?;
            Ok(Some(job.clone()))
        })
        .await
    }

    async fn remove(&self, id: &str) -> Result<bool, String> {
        let id = id.to_string();
        self.transaction(true, move |jobs| {
            let before = jobs.len();
            jobs.retain(|job| job.id != id);
            Ok(jobs.len() != before)
        })
        .await
    }

    async fn list(&self) -> Vec<Job> {
        self.try_list().await.unwrap_or_else(|error| {
            tracing::warn!("Failed to read scheduler storage: {error}");
            Vec::new()
        })
    }

    async fn try_list(&self) -> Result<Vec<Job>, String> {
        self.transaction(false, |jobs| {
            jobs.sort_by(|left, right| left.id.cmp(&right.id));
            Ok(jobs.clone())
        })
        .await
    }

    async fn update_fired(
        &self,
        id: &str,
        last_fired_at_ms: u64,
        fire_count: u32,
    ) -> Result<(), String> {
        self.mutate(
            id,
            Box::new(move |job| {
                job.last_fired_at_ms = Some(last_fired_at_ms);
                job.fire_count = fire_count;
                Ok(())
            }),
        )
        .await?
        .ok_or_else(|| format!("Scheduled task {id} not found"))?;
        Ok(())
    }

    fn change_notify(&self) -> Arc<Notify> {
        self.change_notify.clone()
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
    async fn independent_handles_observe_controls_and_preserve_unrelated_jobs() {
        let temp = tempfile::tempdir().unwrap();
        let runner = JsonFileCronStore::new(temp.path()).unwrap();
        let controls = JsonFileCronStore::new(temp.path()).unwrap();
        assert!(runner.try_list().await.unwrap().is_empty());
        assert!(runner.try_get("first").await.unwrap().is_none());
        assert!(!temp.path().join(".refact").exists());
        controls.add(test_job("first")).await.unwrap();
        assert_eq!(runner.get("first").await, Some(test_job("first")));
        runner.add(test_job("second")).await.unwrap();
        assert!(controls.remove("first").await.unwrap());
        assert!(runner.get("first").await.is_none());
        assert_eq!(controls.list().await, vec![test_job("second")]);
    }

    #[tokio::test]
    async fn atomic_bookkeeping_preserves_controls_and_cannot_resurrect() {
        let temp = tempfile::tempdir().unwrap();
        let runner = JsonFileCronStore::new(temp.path()).unwrap();
        let controls = JsonFileCronStore::new(temp.path()).unwrap();
        runner.add(test_job("job")).await.unwrap();
        controls
            .mutate(
                "job",
                Box::new(|job| {
                    job.enabled = false;
                    job.paused_at_ms = Some(456);
                    if let super::super::types::Action::AgentTurn {
                        prompt, ..
                    } = &mut job.action
                    {
                        *prompt = "edited".to_string();
                    }
                    Ok(())
                }),
            )
            .await
            .unwrap()
            .unwrap();
        runner
            .mutate(
                "job",
                Box::new(|job| {
                    job.last_status = Some("fired".to_string());
                    job.fire_count += 1;
                    Ok(())
                }),
            )
            .await
            .unwrap()
            .unwrap();
        let job = controls.get("job").await.unwrap();
        assert!(!job.enabled);
        assert_eq!(job.paused_at_ms, Some(456));
        assert_eq!(job.prompt(), Some("edited"));
        assert_eq!(job.fire_count, 1);
        controls.remove("job").await.unwrap();
        assert!(runner
            .mutate("job", Box::new(|_| panic!("deleted job mutation ran")))
            .await
            .unwrap()
            .is_none());
        assert!(JsonFileCronStore::new(temp.path())
            .unwrap()
            .list()
            .await
            .is_empty());
    }

    #[tokio::test]
    async fn conditional_removal_preserves_edited_jobs() {
        let temp = tempfile::tempdir().unwrap();
        let runner = JsonFileCronStore::new(temp.path()).unwrap();
        let controls = JsonFileCronStore::new(temp.path()).unwrap();
        runner.add(test_job("conditional")).await.unwrap();
        let selected = runner.get("conditional").await.unwrap();
        controls
            .mutate(
                "conditional",
                Box::new(|job| {
                    job.description = "rescheduled".to_string();
                    Ok(())
                }),
            )
            .await
            .unwrap();
        assert!(!runner
            .remove_if(
                "conditional",
                Box::new(move |job| job.description == selected.description)
            )
            .await
            .unwrap());
        assert_eq!(
            controls.get("conditional").await.unwrap().description,
            "rescheduled"
        );
        assert!(runner
            .remove_if(
                "conditional",
                Box::new(|job| job.description == "rescheduled")
            )
            .await
            .unwrap());
        assert!(controls.try_get("conditional").await.unwrap().is_none());
        std::fs::write(controls.path(), "invalid").unwrap();
        assert!(runner.try_list().await.is_err());
        assert!(runner.try_get("conditional").await.is_err());
    }

    #[tokio::test]
    async fn failed_mutations_leave_authoritative_data_unchanged() {
        let temp = tempfile::tempdir().unwrap();
        let store = JsonFileCronStore::new(temp.path()).unwrap();
        store.add(test_job("original")).await.unwrap();
        assert!(store
            .mutate(
                "original",
                Box::new(|job| {
                    job.enabled = false;
                    Err("invalid edit".to_string())
                })
            )
            .await
            .is_err());
        assert_eq!(store.get("original").await, Some(test_job("original")));
        std::fs::write(store.path(), "broken json").unwrap();
        assert!(store.add(test_job("new")).await.is_err());
        assert_eq!(
            std::fs::read_to_string(store.path()).unwrap(),
            "broken json"
        );
    }

    #[tokio::test]
    async fn concurrent_handles_do_not_clobber_jobs() {
        let temp = tempfile::tempdir().unwrap();
        let mut handles = Vec::new();
        for n in 0..24 {
            let store = JsonFileCronStore::new(temp.path()).unwrap();
            handles.push(tokio::spawn(async move {
                store.add(test_job(&format!("job_{n:02}"))).await.unwrap();
            }));
        }
        for handle in handles {
            handle.await.unwrap();
        }
        let jobs = JsonFileCronStore::new(temp.path()).unwrap().list().await;
        assert_eq!(
            jobs,
            (0..24)
                .map(|n| test_job(&format!("job_{n:02}")))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn scheduler_process_writer() {
        let Some(root) = std::env::var_os("REFACT_SCHEDULER_TEST_ROOT") else {
            return;
        };
        let prefix = std::env::var("REFACT_SCHEDULER_TEST_PREFIX").unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let store = JsonFileCronStore::new(root).unwrap();
            for n in 0..16 {
                store
                    .add(test_job(&format!("{prefix}_{n:02}")))
                    .await
                    .unwrap();
            }
        });
    }

    #[test]
    fn separate_processes_do_not_clobber_jobs() {
        let temp = tempfile::tempdir().unwrap();
        let executable = std::env::current_exe().unwrap();
        let mut children = Vec::new();
        for prefix in ["left", "right"] {
            children.push(
                std::process::Command::new(&executable)
                    .args(["--exact", "store::tests::scheduler_process_writer"])
                    .env("REFACT_SCHEDULER_TEST_ROOT", temp.path())
                    .env("REFACT_SCHEDULER_TEST_PREFIX", prefix)
                    .spawn()
                    .unwrap(),
            );
        }
        for mut child in children {
            assert!(child.wait().unwrap().success());
        }
        let jobs = read_jobs(&scheduled_tasks_path(temp.path())).unwrap();
        let mut ids = jobs.into_iter().map(|job| job.id).collect::<Vec<_>>();
        ids.sort();
        let expected = ["left", "right"]
            .into_iter()
            .flat_map(|prefix| (0..16).map(move |n| format!("{prefix}_{n:02}")))
            .collect::<Vec<_>>();
        assert_eq!(ids, expected);
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
}
