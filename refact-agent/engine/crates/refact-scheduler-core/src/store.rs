use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

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
    journal: crate::automation::AutomationJournal,
    change_notify: Arc<Notify>,
    mutation: Arc<tokio::sync::Mutex<()>>,
}

impl JsonFileCronStore {
    pub fn new(project_root: impl AsRef<Path>) -> Result<Self, String> {
        let root = project_root.as_ref();
        let journal = crate::automation::AutomationJournal::open(root)?;
        let lease = crate::automation::ProjectAutomationLease::acquire(root)?;
        journal.activate_owner(&lease)?;
        journal.migrate_legacy_jobs(root, &lease)?;
        // Opening the projection performs the single legacy import/catalog reconciliation.
        journal.load_legacy_jobs()?;
        let path = scheduled_tasks_path(root);
        type Shared = (Arc<Notify>, Arc<tokio::sync::Mutex<()>>);
        static STORES: OnceLock<Mutex<HashMap<PathBuf, Shared>>> = OnceLock::new();
        let key = std::fs::canonicalize(root).map_err(|e| e.to_string())?;
        let mut stores = STORES
            .get_or_init(Default::default)
            .lock()
            .map_err(|e| e.to_string())?;
        let (change_notify, mutation) = stores
            .entry(key)
            .or_insert_with(|| {
                (
                    Arc::new(Notify::new()),
                    Arc::new(tokio::sync::Mutex::new(())),
                )
            })
            .clone();
        Ok(Self {
            path,
            journal,
            change_notify,
            mutation,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    async fn transaction<T: Send + 'static>(
        &self,
        write: bool,
        operation: impl FnOnce(&crate::automation::AutomationJournal) -> Result<T, String>
            + Send
            + 'static,
    ) -> Result<T, String> {
        let guard = self.mutation.clone().lock_owned().await;
        let journal = self.journal.clone();
        let notify = self.change_notify.clone();
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            let result = operation(&journal);
            if write && result.is_ok() {
                notify.notify_waiters();
            }
            result
        })
        .await
        .map_err(|error| format!("Scheduler storage task failed: {error}"))?
    }
}

#[async_trait]
impl CronStore for JsonFileCronStore {
    async fn add(&self, job: Job) -> Result<(), String> {
        self.transaction(true, move |journal| journal.put_legacy_job(&job))
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
        self.transaction(false, move |journal| {
            Ok(journal
                .load_legacy_jobs()?
                .into_iter()
                .find(|job| job.id == id))
        })
        .await
    }

    async fn remove_if(&self, id: &str, predicate: JobPredicate) -> Result<bool, String> {
        let id = id.to_string();
        self.transaction(true, move |journal| {
            let mut removed = false;
            journal.mutate_legacy_job(&id, |job| {
                removed = predicate(job);
                Ok(!removed)
            })?;
            Ok(removed)
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
        self.transaction(true, move |journal| {
            journal.mutate_legacy_job(&id, |job| {
                mutation(job)?;
                Ok(true)
            })
        })
        .await
    }

    async fn remove(&self, id: &str) -> Result<bool, String> {
        self.remove_if(id, Box::new(|_| true)).await
    }

    async fn list(&self) -> Vec<Job> {
        self.try_list().await.unwrap_or_else(|error| {
            tracing::warn!("Failed to read scheduler storage: {error}");
            Vec::new()
        })
    }

    async fn try_list(&self) -> Result<Vec<Job>, String> {
        self.transaction(false, |journal| journal.load_legacy_jobs())
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

/// Observe committed schedules without acquiring ownership, creating storage, or migrating JSON.
pub fn read_durable_jobs(project_root: impl AsRef<Path>) -> Result<Vec<Job>, String> {
    use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
    use std::io::Read;

    let root = project_root.as_ref();
    // Validates every project-root component without following symlinks; does not create storage.
    let journal_path = crate::automation::journal_path(root)?;
    let directory = cap_std::fs::Dir::open_ambient_dir(root, cap_std::ambient_authority())
        .map_err(|e| e.to_string())?;
    match directory.symlink_metadata(".refact") {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
        Ok(_) => {}
    }
    let storage = directory
        .open_dir_nofollow(".refact")
        .map_err(|e| e.to_string())?;
    match std::fs::symlink_metadata(&journal_path) {
        Ok(_) => {
            return crate::automation::AutomationJournal::open_observer(root)?.load_legacy_jobs()
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    // Unmigrated projects remain readable, but reads must never perform the import.
    match storage.symlink_metadata("scheduled_tasks.json") {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
        Ok(_) => {}
    }
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = storage
        .open_with("scheduled_tasks.json", &options)
        .map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err("Scheduled tasks storage must be a regular file".into());
    }
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err("Hard-linked scheduled tasks storage refused".into());
        }
    }
    const LIMIT: u64 = 4 * 1024 * 1024;
    let mut bytes = Vec::new();
    file.take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > LIMIT {
        return Err("Scheduled tasks size limit".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| format!("Failed to parse scheduled tasks: {e}"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn durable_observation_never_creates_or_migrates_storage() {
        let temp = crate::test_support::tempdir();
        assert!(read_durable_jobs(temp.path()).unwrap().is_empty());
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
        let path = scheduled_tasks_path(temp.path());
        std::fs::create_dir(path.parent().unwrap()).unwrap();
        let jobs = vec![test_job("legacy")];
        let bytes = serde_json::to_vec(&jobs).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(read_durable_jobs(temp.path()).unwrap(), jobs);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
        std::fs::write(&path, b"not json").unwrap();
        assert!(read_durable_jobs(temp.path())
            .unwrap_err()
            .contains("Failed to parse scheduled tasks"));
    }

    #[tokio::test]
    async fn durable_observation_reads_committed_journal_and_reports_corruption() {
        let temp = crate::test_support::tempdir();
        let store = JsonFileCronStore::new(temp.path()).unwrap();
        store.add(test_job("committed")).await.unwrap();
        assert_eq!(
            read_durable_jobs(temp.path()).unwrap(),
            vec![test_job("committed")]
        );
        drop(store);
        let path = crate::automation::journal_path(temp.path()).unwrap();
        std::fs::write(path, b"not sqlite").unwrap();
        assert!(read_durable_jobs(temp.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn durable_observation_rejects_symlink_storage() {
        let temp = crate::test_support::tempdir();
        let external = crate::test_support::tempdir();
        std::os::unix::fs::symlink(external.path(), temp.path().join(".refact")).unwrap();
        assert!(read_durable_jobs(temp.path()).is_err());
        std::fs::remove_file(temp.path().join(".refact")).unwrap();
        std::fs::create_dir(temp.path().join(".refact")).unwrap();
        std::fs::write(external.path().join("jobs.json"), b"[]").unwrap();
        std::os::unix::fs::symlink(
            external.path().join("jobs.json"),
            scheduled_tasks_path(temp.path()),
        )
        .unwrap();
        assert!(read_durable_jobs(temp.path()).is_err());
        assert_eq!(std::fs::read_dir(external.path()).unwrap().count(), 1);
    }

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
    async fn durable_handles_share_notifications_and_committed_state() {
        let temp = crate::test_support::tempdir();
        let runner = JsonFileCronStore::new(temp.path()).unwrap();
        let writer = JsonFileCronStore::new(temp.path().join(".")).unwrap();
        let notify = runner.change_notify();
        let notified = notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        writer.add(test_job("cron_shared")).await.unwrap();
        assert_eq!(runner.list().await, vec![test_job("cron_shared")]);
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), notified)
                .await
                .is_ok()
        );
        runner.update_fired("cron_shared", 456, 1).await.unwrap();
        writer.add(test_job("cron_second")).await.unwrap();
        let persisted = crate::automation::AutomationJournal::open(temp.path())
            .unwrap()
            .load_legacy_jobs()
            .unwrap();
        assert_eq!(persisted.len(), 2);
        let fired = persisted
            .iter()
            .find(|job| job.id == "cron_shared")
            .unwrap();
        assert_eq!(fired.last_fired_at_ms, Some(456));
        assert_eq!(fired.fire_count, 1);
    }

    #[tokio::test]
    async fn independent_handles_observe_controls_and_preserve_unrelated_jobs() {
        let temp = crate::test_support::tempdir();
        let runner = JsonFileCronStore::new(temp.path()).unwrap();
        let controls = JsonFileCronStore::new(temp.path()).unwrap();
        assert!(runner.try_list().await.unwrap().is_empty());
        assert!(runner.try_get("first").await.unwrap().is_none());
        assert!(!scheduled_tasks_path(temp.path()).exists());
        controls.add(test_job("first")).await.unwrap();
        assert_eq!(runner.get("first").await, Some(test_job("first")));
        runner.add(test_job("second")).await.unwrap();
        assert!(controls.remove("first").await.unwrap());
        assert!(runner.get("first").await.is_none());
        assert_eq!(controls.list().await, vec![test_job("second")]);
    }

    #[tokio::test]
    async fn atomic_bookkeeping_preserves_controls_and_cannot_resurrect() {
        let temp = crate::test_support::tempdir();
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
        let temp = crate::test_support::tempdir();
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
        std::fs::write(temp.path().join(".refact/automations.json"), "invalid").unwrap();
        assert!(runner.try_list().await.is_err());
        assert!(runner.try_get("conditional").await.is_err());
    }

    #[tokio::test]
    async fn failed_mutations_leave_authoritative_data_unchanged() {
        let temp = crate::test_support::tempdir();
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
        std::fs::write(temp.path().join(".refact/automations.json"), "broken json").unwrap();
        assert!(store.add(test_job("new")).await.is_err());
        assert_eq!(
            std::fs::read_to_string(temp.path().join(".refact/automations.json")).unwrap(),
            "broken json"
        );
    }

    #[tokio::test]
    async fn cancelled_caller_still_notifies_after_committed_blocking_write() {
        let temp = crate::test_support::tempdir();
        let store = Arc::new(JsonFileCronStore::new(temp.path()).unwrap());
        let notify = store.change_notify();
        let notified = notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let writer = store.clone();
        let task = tokio::spawn(async move {
            writer
                .transaction(true, move |journal| {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    journal.put_legacy_job(&test_job("cancelled_writer"))
                })
                .await
        });
        entered_rx.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        release_tx.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), notified)
            .await
            .expect("committed write must notify even after its caller is cancelled");
        assert_eq!(
            store.get("cancelled_writer").await,
            Some(test_job("cancelled_writer"))
        );
    }

    #[tokio::test]
    async fn concurrent_handles_do_not_clobber_jobs() {
        let temp = crate::test_support::tempdir();
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
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let store = loop {
                match JsonFileCronStore::new(&root) {
                    Ok(store) => break store,
                    Err(error)
                        if error.contains("execution owner already active")
                            && std::time::Instant::now() < deadline =>
                    {
                        std::thread::yield_now();
                    }
                    Err(error) => panic!("scheduler writer could not acquire ownership: {error}"),
                }
            };
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
        let temp = crate::test_support::tempdir();
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
        let jobs = read_durable_jobs(temp.path()).unwrap();
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
        let temp = crate::test_support::tempdir();
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
        let temp = crate::test_support::tempdir();
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
        let rewritten: serde_json::Value = serde_json::to_value(
            crate::automation::AutomationCatalog::open(temp.path())
                .unwrap()
                .read()
                .unwrap()
                .legacy_jobs,
        )
        .unwrap();
        assert!(rewritten[0].get("trigger").is_some());
        assert!(rewritten[0].get("action").is_some());
        assert!(rewritten[0].get("delivery").is_some());
        assert!(rewritten[0].get("cron").is_none());
        assert!(rewritten[0].get("prompt").is_none());
    }

    #[tokio::test]
    async fn replace_updates_in_place() {
        let temp = crate::test_support::tempdir();
        let store = JsonFileCronStore::new(temp.path()).unwrap();
        let mut job = test_job("cron_replace");
        store.add(job.clone()).await.unwrap();

        job.description = "Updated description".to_string();
        assert!(store.replace(job.clone()).await.unwrap());
        assert!(!store.replace(test_job("cron_missing")).await.unwrap());

        let listed = store.list().await;
        assert_eq!(listed, vec![job.clone()]);
        let serialized: serde_json::Value = serde_json::to_value(
            crate::automation::AutomationCatalog::open(temp.path())
                .unwrap()
                .read()
                .unwrap()
                .legacy_jobs,
        )
        .unwrap();
        assert_eq!(serialized.as_array().unwrap().len(), 1);
        assert_eq!(serialized[0]["description"], json!(job.description));
    }
}
