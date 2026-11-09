use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use chrono_tz::Tz;
use refact_scheduler_core::automation::{AutomationJournal, WakeSummary};
use serde::Serialize;
use serde_json::json;
use tokio::task::JoinHandle;

use crate::daemon::config::DaemonConfig;
use crate::daemon::projects::ProjectEntry;
use crate::daemon::state::{now_ms, DaemonState};
use crate::daemon::supervisor::WorkerState;
use crate::scheduler::{
    next_run_ms, recurring_missed_grace_state, scheduled_tasks_path, scheduler_timezone, Job,
    MissedRunGraceConfig,
};

pub(crate) const WAKE_LEAD_MS: u64 = 90_000;
pub const CRON_PENDING_HORIZON_MS: u64 = crate::daemon::idle::CRON_SOON_MS;
const TICK_INTERVAL: Duration = Duration::from_secs(30);
const REPARSE_SOON_MS: u64 = 120_000;
const OBSERVER_RETRY_MS: u64 = 5 * 60_000;

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
    observer_retry_ms: u64,
}

#[derive(Debug, Eq, PartialEq)]
struct ProjectScan {
    pending: Option<PendingCron>,
    parsed: bool,
}

pub(crate) struct AutomationObservation {
    pub summary: WakeSummary,
    pub legacy_authority_retired: bool,
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
        enabled: cron_clock_enabled(&state.config),
        jobs: pending.len().min(u32::MAX as usize) as u32,
        next_wake_ms: next_fire_ms.map(|ms| ms.saturating_sub(WAKE_LEAD_MS)),
    }
}

fn cron_clock_enabled(config: &DaemonConfig) -> bool {
    crate::scheduler::runner::runner_enabled(&config.scheduler)
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
        if self.state.is_shutting_down() {
            return;
        }
        // Bounded observation only: retained supervision is never cron authority.
        self.state.reconcile_buddy_batch(now).await;
        self.state.tick_buddy_wakes(now).await;
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

    fn wake_enabled(&self) -> bool {
        cron_clock_enabled(&self.state.config)
    }

    async fn tick_project(&mut self, entry: ProjectEntry, now: u64) {
        let cache = self.cache.entry(entry.id.clone()).or_default();
        if now < cache.observer_retry_ms {
            return;
        }
        let scan = match observe_automation(&entry.root, now).await {
            Ok(Some(observation)) => {
                let mut pending = automation_pending(&observation.summary, now);
                if !observation.legacy_authority_retired {
                    let legacy = scan_project_file(&entry, cache, now).await.pending;
                    pending = match (pending, legacy) {
                        (Some(a), Some(b)) => Some(if a.next_fire_ms <= b.next_fire_ms {
                            a
                        } else {
                            b
                        }),
                        (a, b) => a.or(b),
                    };
                }
                ProjectScan {
                    pending,
                    parsed: true,
                }
            }
            Ok(None) => scan_project_file(&entry, cache, now).await,
            Err(error) => {
                cache.observer_retry_ms = now.saturating_add(OBSERVER_RETRY_MS);
                self.state.set_cron_pending(&entry.id, None).await;
                tracing::warn!(project_id = %entry.id, "automation wake observation unavailable: {error}");
                let _ = self
                    .state
                    .events
                    .emit(
                        "automation_observer_error",
                        Some(entry.id),
                        json!({"error":error,"retry_at_ms":cache.observer_retry_ms}),
                    )
                    .await;
                return;
            }
        };
        self.state
            .set_cron_pending(
                &entry.id,
                scan.pending.as_ref().map(|pending| pending.next_fire_ms),
            )
            .await;
        if let Some(pending) = scan.pending {
            if !self.wake_enabled() {
                return;
            }
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
        match self.state.supervisor.ensure_worker_auto(&entry).await {
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

/// Observation never creates a journal, takes the executor lease, or mutates SQL.
/// `None` is the pre-migration legacy project, not a failed/empty journal.
pub(crate) async fn observe_automation(
    root: &std::path::Path,
    now: u64,
) -> Result<Option<AutomationObservation>, String> {
    let root = root.to_path_buf();
    tokio::task::spawn_blocking(move || {
        // The authoritative journal is in the private, canonical-root keyed
        // cache, never in the untrusted project tree. Missing observation does
        // not create storage; observer validates private-path ownership/links.
        let path = refact_scheduler_core::automation::journal_path(&root)?;
        match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
            Ok(_) => {}
        }
        let observer = AutomationJournal::open_observer(&root)?;
        Ok(Some(AutomationObservation {
            summary: observer.wake_summary_at_in_timezone(now, scheduler_timezone())?,
            legacy_authority_retired: observer.legacy_authority_retired()?,
        }))
    })
    .await
    .map_err(|error| format!("automation observer task failed: {error}"))?
}

fn automation_pending(summary: &WakeSummary, now: u64) -> Option<PendingCron> {
    // Only restart-opted-in leases may wake a stopped worker without a timer.
    let next_fire_ms = if summary.background_restart {
        Some(now)
    } else {
        summary.next_wake_ms
    }?;
    Some(PendingCron {
        task_id: "automation".into(),
        next_fire_ms,
    })
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
    if !task.durable || task.is_paused() {
        return None;
    }
    if task.recurring
        && task.auto_expire_after_ms > 0
        && task.created_at_ms.saturating_add(task.auto_expire_after_ms) <= now
    {
        return None;
    }
    if let Some(trigger_at_ms) = task.trigger_at_ms {
        return Some(trigger_at_ms.max(now));
    }
    if task.recurring {
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use serial_test::serial;
    use std::time::Instant;
    use tempfile::tempdir;

    use crate::daemon::config::DaemonConfig;
    use crate::daemon::events::EventBus;
    use crate::daemon::projects::ProjectSettings;
    use crate::scheduler::DEFAULT_RECURRING_AUTO_EXPIRE_AFTER_MS;

    /// Removes only cache storage created for a unique temporary project root.
    struct TestJournalStorage(std::path::PathBuf);
    impl TestJournalStorage {
        fn create(root: &std::path::Path) -> Self {
            let path = refact_scheduler_core::automation::journal_path(root).unwrap();
            assert!(!path.exists(), "test journal must not preexist");
            drop(AutomationJournal::open(root).unwrap());
            Self(path)
        }
    }
    impl Drop for TestJournalStorage {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
        }
    }

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
            Some(Self {
                keys,
            })
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
            automation_suspended: false,
            settings: ProjectSettings::default(),
        }
    }

    async fn wait_for_worker_state(
        state: &DaemonState,
        project_id: &str,
        target: WorkerState,
        timeout: Duration,
    ) {
        let deadline = Instant::now() + timeout;
        loop {
            if state
                .supervisor
                .worker_info(project_id)
                .await
                .map(|info| info.state == target.clone())
                .unwrap_or(false)
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "worker state {:?} was not reached in time",
                target
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
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
    fn automation_wake_summary_does_not_confuse_review_counts_with_authority() {
        let now = 100_000;
        let old_summary: WakeSummary = serde_json::from_value(json!({
            "next_wake_ms": null,
            "background_active": true,
            "pending_delivery": 0,
            "recovery": 0
        }))
        .unwrap();
        assert!(old_summary.background_active);
        assert!(!old_summary.background_restart);
        assert!(automation_pending(&old_summary, now).is_none());
        let mut summary = WakeSummary::default();
        summary.recovery = 5;
        summary.pending_delivery = 5;
        assert!(automation_pending(&summary, now).is_none());
        summary.next_wake_ms = Some(now + WAKE_LEAD_MS);
        assert_eq!(
            automation_pending(&summary, now).unwrap().next_fire_ms,
            now + WAKE_LEAD_MS
        );
        summary.background_active = true;
        summary.next_wake_ms = None;
        assert!(automation_pending(&summary, now).is_none());
        summary.background_restart = true;
        assert_eq!(automation_pending(&summary, now).unwrap().next_fire_ms, now);
    }

    #[test]
    fn legacy_transition_observes_each_meaningful_trigger_and_disable_gate() {
        use crate::scheduler::Trigger;
        let now = utc_ms(2026, 1, 1, 0, 0);
        let cron = task("cron", "*/1 * * * *", now);
        let mut interval = cron.clone();
        interval.id = "interval".into();
        interval.trigger = Trigger::Interval {
            every_ms: 60_000,
        };
        let mut once = cron.clone();
        once.id = "once".into();
        once.recurring = false;
        once.trigger = Trigger::Once {
            at_ms: now + 1_000,
        };
        let mut manual = cron.clone();
        manual.id = "manual".into();
        manual.trigger = Trigger::Manual;
        manual.trigger_at_ms = Some(now);
        let mut retry = cron.clone();
        retry.id = "retry".into();
        retry.trigger_at_ms = Some(now + 5_000);
        retry.retry_attempts = 1;
        for row in [cron, interval, once, manual, retry] {
            assert!(
                next_fire_for_task(&row, now, chrono_tz::UTC).is_some(),
                "{}",
                row.id
            );
            let mut disabled = row;
            disabled.enabled = false;
            assert_eq!(next_fire_for_task(&disabled, now, chrono_tz::UTC), None);
        }
    }

    #[tokio::test]
    async fn observer_missing_project_does_not_create_storage() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("project");
        tokio::fs::create_dir(&root).await.unwrap();
        assert!(observe_automation(&root, 100).await.unwrap().is_none());
        assert!(!root.join(".refact").exists());
    }

    #[tokio::test]
    async fn observer_background_lease_requires_approval_and_expires_at_boundary() {
        use crate::daemon::idle::{idle_decision, IdleDecision, WorkerIdleSnapshot};
        use refact_scheduler_core::automation::{GrantRecord, SourceControl};
        let dir = tempdir().unwrap();
        let _storage = TestJournalStorage::create(dir.path());
        let journal = AutomationJournal::open(dir.path()).unwrap();
        journal
            .reconcile_app_source("app", "stream", "digest", &json!({}))
            .unwrap();
        // Merely declaring an automation is never a keepalive lease.
        assert!(
            !observe_automation(dir.path(), 100)
                .await
                .unwrap()
                .unwrap()
                .summary
                .background_active
        );
        journal
            .grant_source(&GrantRecord {
                app_id: "app".into(),
                source_id: "stream".into(),
                config_digest: "digest".into(),
                endpoint_grant: None,
                credential_refs: vec![],
                granted: true,
            })
            .unwrap();
        journal
            .set_source_control(
                "app",
                "stream",
                &SourceControl {
                    background: true,
                    resume_after_restart: false,
                    expires_at_ms: Some(200),
                    next_wake_ms: Some(150),
                    ..Default::default()
                },
            )
            .unwrap();
        let live = observe_automation(dir.path(), 199)
            .await
            .unwrap()
            .unwrap()
            .summary;
        assert!(live.background_active);
        assert!(!live.background_restart);
        assert!(automation_pending(&live, 199).is_none());
        // Use an otherwise stoppable live snapshot to isolate residency from the
        // independent policy clock/timeout; no process or wall-clock wait needed.
        let policy_now = 120_000;
        let mut snapshot = WorkerIdleSnapshot {
            state: WorkerState::Ready,
            pinned: false,
            live_proxy_streams: 0,
            last_proxy_activity_ms: 0,
            lsp_clients: 0,
            busy_chats: 0,
            exec_running: 0,
            last_status_report_ms: policy_now,
            cron_next_fire_ms: None,
            automation_background_active: Some(live.background_active),
        };
        let config = DaemonConfig {
            idle_timeout_secs: 1,
            ..Default::default()
        };
        assert_eq!(
            idle_decision(policy_now, &snapshot, &config),
            IdleDecision::Keep
        );
        let expired = observe_automation(dir.path(), 200)
            .await
            .unwrap()
            .unwrap()
            .summary;
        assert!(!expired.background_active);
        assert!(!expired.background_restart);
        assert!(automation_pending(&expired, 200).is_none());
        snapshot.automation_background_active = Some(expired.background_active);
        assert_eq!(
            idle_decision(policy_now, &snapshot, &config),
            IdleDecision::Stop
        );
        journal
            .set_source_control(
                "app",
                "stream",
                &SourceControl {
                    background: true,
                    resume_after_restart: true,
                    expires_at_ms: Some(200),
                    ..Default::default()
                },
            )
            .unwrap();
        let resumable = observe_automation(dir.path(), 199)
            .await
            .unwrap()
            .unwrap()
            .summary;
        assert!(resumable.background_active);
        assert!(resumable.background_restart);
        assert_eq!(
            automation_pending(&resumable, 199).unwrap().next_fire_ms,
            199
        );
        assert!(
            !observe_automation(dir.path(), 200)
                .await
                .unwrap()
                .unwrap()
                .summary
                .background_restart
        );
        journal
            .set_source_control(
                "app",
                "stream",
                &SourceControl {
                    background: true,
                    paused: true,
                    resume_after_restart: true,
                    ..Default::default()
                },
            )
            .unwrap();
        let paused = observe_automation(dir.path(), 199)
            .await
            .unwrap()
            .unwrap()
            .summary;
        assert!(!paused.background_active);
        assert!(!paused.background_restart);
        assert!(automation_pending(&paused, 199).is_none());
        journal
            .set_source_control(
                "app",
                "stream",
                &SourceControl {
                    background: true,
                    resume_after_restart: true,
                    ..Default::default()
                },
            )
            .unwrap();
        // Approval is still checked even for both requested lease authorities.
        journal
            .reconcile_app_source("app", "stream", "changed", &json!({}))
            .unwrap();
        let unapproved = observe_automation(dir.path(), 199)
            .await
            .unwrap()
            .unwrap()
            .summary;
        assert!(!unapproved.background_active);
        assert!(!unapproved.background_restart);
        assert!(automation_pending(&unapproved, 199).is_none());
    }

    #[tokio::test]
    async fn corrupt_journal_is_error_not_legacy_or_wake_authority() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("project");
        tokio::fs::create_dir(&root).await.unwrap();
        let storage = TestJournalStorage::create(&root);
        tokio::fs::write(&storage.0, b"corrupt").await.unwrap();
        assert!(observe_automation(&root, 100).await.is_err());
        assert!(!storage.0.with_file_name("runtime.sqlite-wal").exists());
        assert!(!storage.0.with_file_name("runtime.sqlite-shm").exists());

        let binding_root = dir.path().join("binding-project");
        tokio::fs::create_dir(&binding_root).await.unwrap();
        let binding_storage = TestJournalStorage::create(&binding_root);
        {
            let setup = rusqlite::Connection::open(&binding_storage.0).unwrap();
            setup
                .execute_batch(
                    "UPDATE metadata SET value='wrong-project-root' WHERE key='project_root';
                     DROP INDEX runs_status;",
                )
                .unwrap();
        }
        let before = tokio::fs::read(&binding_storage.0).await.unwrap();
        assert_eq!(
            observe_automation(&binding_root, 100).await.err().unwrap(),
            "runtime journal project binding mismatch"
        );
        assert_eq!(tokio::fs::read(&binding_storage.0).await.unwrap(), before);
        assert!(!binding_storage
            .0
            .with_file_name("runtime.sqlite-wal")
            .exists());
        assert!(!binding_storage
            .0
            .with_file_name("runtime.sqlite-shm")
            .exists());
    }

    #[tokio::test]
    async fn corrupt_observer_retries_coarsely_without_starting_worker() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("project");
        tokio::fs::create_dir(&root).await.unwrap();
        let storage = TestJournalStorage::create(&root);
        tokio::fs::write(&storage.0, b"corrupt").await.unwrap();
        let state = DaemonState::new_with_daemon_dir(
            DaemonConfig::default(),
            EventBus::new(dir.path().join("events.jsonl")),
            None,
            dir.path().join("daemon"),
            0,
        );
        let entry = state.projects.write().await.open(root).await.unwrap();
        let mut clock = CronClock::new(state.clone());
        clock.tick_at(100).await;
        clock.tick_at(101).await;
        assert_eq!(state.supervisor.worker_count().await, 0);
        assert_eq!(
            state
                .events
                .snapshot()
                .await
                .iter()
                .filter(|event| event.kind == "automation_observer_error")
                .count(),
            1
        );
        clock.tick_at(100 + OBSERVER_RETRY_MS).await;
        assert_eq!(
            state
                .events
                .snapshot()
                .await
                .iter()
                .filter(|event| event.kind == "automation_observer_error")
                .count(),
            2
        );
        assert_eq!(state.cron_pending(&entry.id).await, None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn observer_ignores_project_local_symlink_and_seeded_database() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("project");
        let outside = dir.path().join("outside");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join(".refact")).unwrap();
        let seeded = outside.join("automation/runtime.sqlite");
        std::fs::create_dir(outside.join("automation")).unwrap();
        std::fs::write(&seeded, b"untrusted project-seeded database").unwrap();
        assert!(observe_automation(&root, 100).await.unwrap().is_none());
        assert_eq!(
            std::fs::read(&seeded).unwrap(),
            b"untrusted project-seeded database"
        );
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
    #[serial(daemon, scheduler_env)]
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
    #[serial(daemon, scheduler_env)]
    async fn tick_does_not_bypass_supervisor_crash_budget_for_due_task() {
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
        write_tasks(&root, &[due]).await;
        let state = DaemonState::new(
            DaemonConfig::default(),
            EventBus::new(dir.path().join("events.jsonl")),
            None,
        );
        state.load_projects(dir.path().join("projects.json")).await;
        let entry = {
            let mut registry = state.projects.write().await;
            registry.open(root.clone()).await.unwrap()
        };
        let project_id = entry.id.clone();
        std::env::set_var("FAKE_WORKER_CRASH", "1");
        let _ = state.supervisor.ensure_worker(&entry).await.unwrap();
        wait_for_worker_state(
            &state,
            &project_id,
            WorkerState::Crashed,
            Duration::from_secs(120),
        )
        .await;
        std::env::remove_var("FAKE_WORKER_CRASH");
        let mut clock = CronClock::new(state.clone());

        clock.tick_at(now).await;

        let worker = state.supervisor.worker_info(&project_id).await.unwrap();
        assert_eq!(worker.state, WorkerState::Crashed);
        assert_eq!(state.cron_pending(&project_id).await, Some(now));
        state.supervisor.stop_all().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial(daemon, scheduler_env)]
    async fn tick_records_pending_but_does_not_wake_when_scheduler_disabled() {
        let Some(_env) = EnvGuard::fake_worker() else {
            return;
        };
        let dir = tempdir().unwrap();
        let root = dir.path().join("project");
        tokio::fs::create_dir_all(&root).await.unwrap();
        let now = now_ms();
        let mut due = task("due-disabled", "0 0 1 1 *", 0);
        due.recurring = false;
        due.auto_expire_after_ms = 0;
        write_tasks(&root, &[due]).await;
        let mut config = DaemonConfig::default();
        config.scheduler.enabled = false;
        let state = DaemonState::new(config, EventBus::new(dir.path().join("events.jsonl")), None);
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

        clock.tick_at(now).await;

        assert_eq!(state.cron_pending(&project_id).await, Some(now));
        assert_eq!(state.supervisor.worker_count().await, 0);
        let status = status(&state).await;
        assert!(!status.enabled);
        assert_eq!(status.jobs, 1);
    }
}
