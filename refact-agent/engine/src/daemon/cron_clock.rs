use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use chrono_tz::Tz;
use serde::Serialize;
use serde_json::json;
use tokio::task::JoinHandle;

use crate::daemon::projects::ProjectEntry;
use crate::daemon::state::{now_ms, DaemonState};
use crate::daemon::supervisor::WorkerState;
use crate::scheduler::{
    next_run_ms, recurring_missed_grace_state, scheduled_tasks_path, scheduler_timezone, Action,
    AgentTarget, Delivery, Job, MissedRunGraceConfig, Trigger,
};

pub(crate) const WAKE_LEAD_MS: u64 = 90_000;
pub const CRON_PENDING_HORIZON_MS: u64 = crate::daemon::idle::CRON_SOON_MS;
const TICK_INTERVAL: Duration = Duration::from_secs(30);
const REPARSE_SOON_MS: u64 = 120_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PendingCron {
    pub task_id: String,
    pub next_fire_ms: u64,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub(crate) struct CronClockStatus {
    pub enabled: bool,
    pub jobs: u32,
    pub next_wake_ms: Option<u64>,
}

#[derive(Default)]
struct ProjectScheduleCache {
    mtime: Option<SystemTime>,
    pending: Option<PendingCron>,
    parse_failed: bool,
    warned_mtime: Option<SystemTime>,
}

#[derive(Debug, Eq, PartialEq)]
struct ProjectScan {
    pending: Option<PendingCron>,
    parsed: bool,
}

pub fn spawn(state: Arc<DaemonState>) -> JoinHandle<()> {
    tokio::spawn(async move {
        CronClock::new(state).run().await;
    })
}

pub fn cron_pending_blocks_idle_stop(next_fire_ms: u64, now: u64) -> bool {
    crate::daemon::idle::cron_pending_blocks_idle_stop(next_fire_ms, now)
}

pub(crate) async fn status(state: &DaemonState) -> CronClockStatus {
    let pending = state.cron_pending_snapshot().await;
    let next_fire_ms = pending.values().copied().min();
    CronClockStatus {
        enabled: crate::scheduler::runner::scheduler_enabled(),
        jobs: pending.len().min(u32::MAX as usize) as u32,
        next_wake_ms: next_fire_ms.map(|ms| ms.saturating_sub(WAKE_LEAD_MS)),
    }
}

pub(crate) struct CronClock {
    state: Arc<DaemonState>,
    cache: HashMap<String, ProjectScheduleCache>,
}

impl CronClock {
    pub(crate) fn new(state: Arc<DaemonState>) -> Self {
        Self {
            state,
            cache: HashMap::new(),
        }
    }

    async fn run(mut self) {
        let mut shutdown_rx = self.state.shutdown_receiver();
        let mut interval = tokio::time::interval(TICK_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => break,
                _ = interval.tick() => self.tick().await,
            }
        }
    }

    async fn tick(&mut self) {
        self.tick_at(now_ms()).await;
    }

    pub(crate) async fn tick_at(&mut self, now: u64) {
        let projects = {
            let registry = self.state.projects.read().await;
            registry.list()
        };
        let project_ids: HashSet<String> = projects.iter().map(|entry| entry.id.clone()).collect();
        self.cache
            .retain(|project_id, _| project_ids.contains(project_id));
        self.state.retain_cron_pending(&project_ids).await;
        for entry in projects {
            self.tick_project(entry, now).await;
        }
    }

    async fn tick_project(&mut self, entry: ProjectEntry, now: u64) {
        let cache = self.cache.entry(entry.id.clone()).or_default();
        let scan = scan_project_file(&entry, cache, now).await;
        self.state
            .set_cron_pending(
                &entry.id,
                scan.pending.as_ref().map(|pending| pending.next_fire_ms),
            )
            .await;
        if let Some(pending) = scan.pending {
            if pending.next_fire_ms <= now.saturating_add(WAKE_LEAD_MS) {
                self.wake_project(entry, pending).await;
            }
        }
    }

    async fn wake_project(&self, entry: ProjectEntry, pending: PendingCron) {
        let project_id = entry.id.clone();
        let task_id = pending.task_id.clone();
        let next_fire_ms = pending.next_fire_ms;
        let worker_ready_or_starting = self
            .state
            .supervisor
            .worker_info(&project_id)
            .await
            .is_some_and(|info| matches!(info.state, WorkerState::Ready | WorkerState::Starting));
        if worker_ready_or_starting {
            return;
        }
        match self.state.supervisor.ensure_worker(&entry).await {
            Ok(worker) => {
                let worker_state = worker.state.clone();
                let _ = self
                    .state
                    .events
                    .emit(
                        "cron_wake",
                        Some(project_id.clone()),
                        json!({
                            "project_id": project_id.clone(),
                            "task_id": task_id.clone(),
                            "next_fire_ms": next_fire_ms,
                            "worker_state": worker_state,
                        }),
                    )
                    .await;
                if matches!(worker.state, WorkerState::Crashed) {
                    tracing::warn!(
                        "scheduled task {} is pending for crashed project {}",
                        task_id,
                        project_id
                    );
                    let _ = self
                        .state
                        .events
                        .emit(
                            "cron_worker_crashed",
                            Some(project_id.clone()),
                            json!({
                                "task_id": task_id.clone(),
                                "next_fire_ms": next_fire_ms,
                            }),
                        )
                        .await;
                }
            }
            Err(error) => {
                tracing::warn!(
                    "failed to wake worker for scheduled task {} in project {}: {}",
                    task_id,
                    project_id,
                    error
                );
            }
        }
    }
}

async fn scan_project_file(
    entry: &ProjectEntry,
    cache: &mut ProjectScheduleCache,
    now: u64,
) -> ProjectScan {
    let path = scheduled_tasks_path(&entry.root);
    let metadata = match tokio::fs::metadata(&path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            cache.mtime = None;
            cache.pending = None;
            cache.parse_failed = false;
            return ProjectScan {
                pending: None,
                parsed: false,
            };
        }
        Err(error) => {
            tracing::warn!("failed to stat {}: {}", path.display(), error);
            cache.pending = None;
            return ProjectScan {
                pending: None,
                parsed: false,
            };
        }
    };
    let mtime = metadata.modified().ok();
    if cache.mtime == mtime {
        if cache.parse_failed {
            return ProjectScan {
                pending: None,
                parsed: false,
            };
        }
        match cache.pending.as_ref() {
            Some(pending) if pending.next_fire_ms > now.saturating_add(REPARSE_SOON_MS) => {
                return ProjectScan {
                    pending: cache.pending.clone(),
                    parsed: false,
                };
            }
            None => {
                return ProjectScan {
                    pending: None,
                    parsed: false,
                };
            }
            _ => {}
        }
    }

    let content = match tokio::fs::read_to_string(&path).await {
        Ok(content) => content,
        Err(error) => {
            tracing::warn!("failed to read {}: {}", path.display(), error);
            cache.mtime = mtime;
            cache.pending = None;
            cache.parse_failed = false;
            return ProjectScan {
                pending: None,
                parsed: true,
            };
        }
    };
    let tasks = match serde_json::from_str::<Vec<Job>>(&content) {
        Ok(tasks) => tasks,
        Err(error) => {
            if cache.warned_mtime != mtime || !cache.parse_failed {
                tracing::warn!("failed to parse {}: {}", path.display(), error);
                cache.warned_mtime = mtime;
            }
            cache.mtime = mtime;
            cache.pending = None;
            cache.parse_failed = true;
            return ProjectScan {
                pending: None,
                parsed: true,
            };
        }
    };
    let pending = next_pending_task(&tasks, now, scheduler_timezone());
    cache.mtime = mtime;
    cache.pending = pending.clone();
    cache.parse_failed = false;
    ProjectScan {
        pending,
        parsed: true,
    }
}

fn next_pending_task(tasks: &[Job], now: u64, tz: Tz) -> Option<PendingCron> {
    tasks
        .iter()
        .filter_map(|task| {
            next_fire_for_task(task, now, tz).map(|next_fire_ms| PendingCron {
                task_id: task.id.clone(),
                next_fire_ms,
            })
        })
        .min_by(|left, right| {
            left.next_fire_ms
                .cmp(&right.next_fire_ms)
                .then_with(|| left.task_id.cmp(&right.task_id))
        })
}

fn next_fire_for_task(task: &Job, now: u64, tz: Tz) -> Option<u64> {
    if !task.durable {
        return None;
    }
    if !is_supported_chat_job(task) {
        tracing::warn!(
            "skipping unsupported scheduled task {} in daemon cron clock",
            task.id
        );
        return None;
    }
    if task.recurring {
        if task.auto_expire_after_ms > 0
            && task.created_at_ms.saturating_add(task.auto_expire_after_ms) <= now
        {
            return None;
        }
        let from_ms = task.last_fired_at_ms.unwrap_or(task.created_at_ms);
        let state =
            recurring_missed_grace_state(task, from_ms, now, tz, MissedRunGraceConfig::default())?;
        if state.due_ms.is_some() && state.should_fire {
            return Some(now);
        }
        return Some(state.next_future_ms);
    }
    if task.fire_count != 0 {
        return None;
    }
    let scheduled_ms = next_run_ms(task, task.created_at_ms, tz)?;
    Some(if scheduled_ms <= now {
        now
    } else {
        scheduled_ms
    })
}

fn is_supported_chat_job(task: &Job) -> bool {
    matches!(task.trigger, Trigger::Cron { .. })
        && matches!(
            (&task.action, &task.delivery),
            (
                Action::AgentTurn {
                    target: AgentTarget::ExistingChat { .. },
                    ..
                },
                Delivery::Chat,
            )
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use serial_test::serial;
    use tempfile::tempdir;

    use crate::daemon::config::DaemonConfig;
    use crate::daemon::events::EventBus;
    use crate::daemon::projects::ProjectSettings;
    use crate::scheduler::DEFAULT_RECURRING_AUTO_EXPIRE_AFTER_MS;

    struct EnvGuard {
        keys: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn fake_worker() -> Option<Self> {
            let python = std::env::var("PYTHON3").unwrap_or_else(|_| "python3".to_string());
            if std::process::Command::new(&python)
                .arg("--version")
                .output()
                .is_err()
            {
                return None;
            }
            let script = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("fake_worker.py");
            let keys = vec![
                (
                    "REFACT_DAEMON_WORKER_CMD",
                    std::env::var("REFACT_DAEMON_WORKER_CMD").ok(),
                ),
                (
                    "REFACT_DAEMON_SUPERVISOR_BACKOFF_MS",
                    std::env::var("REFACT_DAEMON_SUPERVISOR_BACKOFF_MS").ok(),
                ),
                ("FAKE_WORKER_CRASH", std::env::var("FAKE_WORKER_CRASH").ok()),
            ];
            std::env::set_var(
                "REFACT_DAEMON_WORKER_CMD",
                shell_words::join([python.as_str(), script.to_string_lossy().as_ref()]),
            );
            std::env::set_var("REFACT_DAEMON_SUPERVISOR_BACKOFF_MS", "1");
            std::env::remove_var("FAKE_WORKER_CRASH");
            Some(Self { keys })
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in self.keys.drain(..) {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    fn utc_ms(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> u64 {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
            .single()
            .unwrap()
            .timestamp_millis() as u64
    }

    fn task(id: &str, cron: &str, created_at_ms: u64) -> Job {
        let mut task = Job::new_cron_agent_chat(
            cron.to_string(),
            "wake up".to_string(),
            "wake".to_string(),
            true,
            true,
            created_at_ms,
        );
        task.id = id.to_string();
        task.set_existing_chat(Some("chat".to_string()));
        task.set_mode(Some("agent".to_string()));
        task.auto_expire_after_ms = DEFAULT_RECURRING_AUTO_EXPIRE_AFTER_MS;
        task
    }

    fn project_entry(root: std::path::PathBuf, id: &str) -> ProjectEntry {
        ProjectEntry {
            id: id.to_string(),
            slug: id.to_string(),
            root,
            pinned: false,
            last_active_ms: 0,
            settings: ProjectSettings::default(),
        }
    }

    async fn write_tasks(root: &std::path::Path, tasks: &[Job]) -> Vec<u8> {
        let path = scheduled_tasks_path(root);
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        let content = serde_json::to_vec_pretty(tasks).unwrap();
        tokio::fs::write(&path, &content).await.unwrap();
        content
    }

    #[test]
    fn filters_fixture_tasks_to_next_durable_active_fire() {
        let now = utc_ms(2026, 1, 1, 0, 0);
        let mut session_task = task("session", "*/1 * * * *", now);
        session_task.durable = false;
        let mut expired = task("expired", "*/1 * * * *", now - 10_000);
        expired.auto_expire_after_ms = 1;
        let mut fired_one_shot = task("fired-one-shot", "*/1 * * * *", now);
        fired_one_shot.recurring = false;
        fired_one_shot.fire_count = 1;
        fired_one_shot.auto_expire_after_ms = 0;
        let active = task("active", "*/5 * * * *", now);

        let pending = next_pending_task(
            &[session_task, expired, fired_one_shot, active],
            now,
            chrono_tz::UTC,
        )
        .unwrap();

        assert_eq!(pending.task_id, "active");
        assert_eq!(pending.next_fire_ms, utc_ms(2026, 1, 1, 0, 5));
    }

    #[test]
    fn next_fire_matches_scheduler_cron_expr_for_recurring_tasks() {
        let now = utc_ms(2026, 1, 1, 0, 0);
        for expr in ["*/5 * * * *", "17 * * * *", "0 9 * * 1-5"] {
            let task = task(expr, expr, now);
            assert_eq!(
                next_fire_for_task(&task, now, chrono_tz::UTC),
                next_run_ms(expr, now, chrono_tz::UTC)
            );
        }
    }

    #[test]
    fn overdue_recurring_fast_forward_matches_runner_fixture() {
        let now = utc_ms(2026, 1, 1, 0, 10);
        let mut task = task("overdue", "*/1 * * * *", utc_ms(2026, 1, 1, 0, 0));
        task.last_fired_at_ms = Some(utc_ms(2026, 1, 1, 0, 0));
        let mut jitter_cfg = crate::scheduler::jitter::JitterConfig::default();
        jitter_cfg.recurring_frac = 0.0;

        let clock_next = next_fire_for_task(&task, now, chrono_tz::UTC).unwrap();
        let runner_next = crate::scheduler::runner::scheduled_fire_at_ms(
            &task,
            now,
            &jitter_cfg,
            MissedRunGraceConfig::default(),
        )
        .unwrap();

        assert_eq!(clock_next, utc_ms(2026, 1, 1, 0, 11));
        assert_eq!(runner_next, clock_next);
    }

    #[test]
    fn one_shot_past_due_is_due_now() {
        let now = utc_ms(2026, 1, 1, 0, 10);
        let mut task = task("one-shot", "*/5 * * * *", utc_ms(2026, 1, 1, 0, 0));
        task.recurring = false;
        task.auto_expire_after_ms = 0;

        assert_eq!(next_fire_for_task(&task, now, chrono_tz::UTC), Some(now));
    }

    #[test]
    fn due_within_wake_lead_boundary_is_inclusive() {
        let now = utc_ms(2026, 1, 1, 0, 0);
        let inside = PendingCron {
            task_id: "inside".to_string(),
            next_fire_ms: now + WAKE_LEAD_MS,
        };
        let outside = PendingCron {
            task_id: "outside".to_string(),
            next_fire_ms: now + WAKE_LEAD_MS + 1,
        };

        assert!(inside.next_fire_ms <= now + WAKE_LEAD_MS);
        assert!(outside.next_fire_ms > now + WAKE_LEAD_MS);
    }

    #[test]
    fn cron_pending_idle_stop_horizon_is_ten_minutes() {
        let now = utc_ms(2026, 1, 1, 0, 0);

        assert!(cron_pending_blocks_idle_stop(
            now + CRON_PENDING_HORIZON_MS,
            now
        ));
        assert!(!cron_pending_blocks_idle_stop(
            now + CRON_PENDING_HORIZON_MS + 1,
            now
        ));
    }

    #[tokio::test]
    async fn mtime_cache_skips_distant_unchanged_file_then_reparses_near_fire() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("project");
        tokio::fs::create_dir_all(&root).await.unwrap();
        let now = utc_ms(2026, 1, 1, 0, 0);
        let entry = project_entry(root.clone(), "project");
        let task = task("future", "*/5 * * * *", now);
        write_tasks(&root, &[task]).await;
        let mut cache = ProjectScheduleCache::default();

        let first = scan_project_file(&entry, &mut cache, now).await;
        let second = scan_project_file(&entry, &mut cache, now + 1_000).await;
        let near = scan_project_file(&entry, &mut cache, utc_ms(2026, 1, 1, 0, 4)).await;

        assert!(first.parsed);
        assert!(!second.parsed);
        assert!(near.parsed);
        assert_eq!(first.pending, second.pending);
    }

    #[tokio::test]
    async fn corrupt_json_is_skipped_without_repeated_parse_on_same_mtime() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("project");
        let path = scheduled_tasks_path(&root);
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, b"not json").await.unwrap();
        let entry = project_entry(root, "project");
        let mut cache = ProjectScheduleCache::default();
        let now = utc_ms(2026, 1, 1, 0, 0);

        let first = scan_project_file(&entry, &mut cache, now).await;
        let second = scan_project_file(&entry, &mut cache, now + 1_000).await;

        assert!(first.parsed);
        assert!(!second.parsed);
        assert_eq!(first.pending, None);
        assert_eq!(second.pending, None);
    }

    #[tokio::test]
    async fn corrupt_json_warns_again_when_file_changes() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("project");
        let path = scheduled_tasks_path(&root);
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, b"not json").await.unwrap();
        let entry = project_entry(root, "project");
        let mut cache = ProjectScheduleCache::default();
        let now = utc_ms(2026, 1, 1, 0, 0);

        let first = scan_project_file(&entry, &mut cache, now).await;
        tokio::fs::write(&path, b"still not json").await.unwrap();
        filetime::set_file_mtime(&path, filetime::FileTime::from_unix_time(2_000_000_000, 0))
            .unwrap();
        let second = scan_project_file(&entry, &mut cache, now + 1_000).await;

        assert!(first.parsed);
        assert!(second.parsed);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn tick_wakes_fake_worker_for_due_durable_task() {
        let Some(_env) = EnvGuard::fake_worker() else {
            return;
        };
        let dir = tempdir().unwrap();
        let root = dir.path().join("project");
        tokio::fs::create_dir_all(&root).await.unwrap();
        let now = now_ms();
        let mut due = task("due-now", "0 0 1 1 *", 0);
        due.recurring = false;
        due.auto_expire_after_ms = 0;
        let original_content = write_tasks(&root, &[due]).await;
        let events = EventBus::new(dir.path().join("events.jsonl"));
        let state = DaemonState::new(DaemonConfig::default(), events.clone(), None);
        state.load_projects(dir.path().join("projects.json")).await;
        {
            let mut registry = state.projects.write().await;
            registry.open(root.clone()).await.unwrap();
        }
        let project_id = {
            let registry = state.projects.read().await;
            registry.list()[0].id.clone()
        };
        let mut clock = CronClock::new(state.clone());

        clock.tick_at(now).await;

        let worker = state.supervisor.worker_info(&project_id).await.unwrap();
        assert_eq!(worker.state, WorkerState::Ready);
        assert_eq!(state.cron_pending(&project_id).await, Some(now));
        assert_eq!(
            tokio::fs::read(scheduled_tasks_path(&root)).await.unwrap(),
            original_content
        );
        assert!(events
            .snapshot()
            .await
            .iter()
            .any(|event| event.kind == "cron_wake"
                && event.project_id.as_deref() == Some(&project_id)));
        state.supervisor.stop_all().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn tick_ignores_project_without_durable_tasks() {
        let Some(_env) = EnvGuard::fake_worker() else {
            return;
        };
        let dir = tempdir().unwrap();
        let root = dir.path().join("project");
        tokio::fs::create_dir_all(&root).await.unwrap();
        let mut session_task = task("session", "0 0 1 1 *", 0);
        session_task.durable = false;
        session_task.recurring = false;
        session_task.auto_expire_after_ms = 0;
        write_tasks(&root, &[session_task]).await;
        let state = DaemonState::new(
            DaemonConfig::default(),
            EventBus::new(dir.path().join("events.jsonl")),
            None,
        );
        state.load_projects(dir.path().join("projects.json")).await;
        {
            let mut registry = state.projects.write().await;
            registry.open(root).await.unwrap();
        }
        let project_id = {
            let registry = state.projects.read().await;
            registry.list()[0].id.clone()
        };
        let mut clock = CronClock::new(state.clone());

        clock.tick_at(now_ms()).await;

        assert_eq!(state.supervisor.worker_count().await, 0);
        assert_eq!(state.cron_pending(&project_id).await, None);
    }
}
