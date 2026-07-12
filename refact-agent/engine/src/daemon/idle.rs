//! Idle-stop policy for daemon-managed workers.
//!
//! The daemon only owns process lifetime. Project indexes, VecDB state, and any other warm-start
//! caches are persisted by the worker engine itself, so stopping an idle worker is expected to trade
//! RAM for a later warm restart without daemon-side state migration.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::task::JoinHandle;

use crate::daemon::config::DaemonConfig;
use crate::daemon::state::{now_ms, DaemonState};
use crate::daemon::supervisor::WorkerState;

pub const STATUS_FRESH_MS: u64 = 60_000;
pub const CRON_SOON_MS: u64 = 10 * 60 * 1000;
const TICK_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerIdleSnapshot {
    pub state: WorkerState,
    pub pinned: bool,
    pub live_proxy_streams: u32,
    pub last_proxy_activity_ms: u64,
    pub lsp_clients: u32,
    pub busy_chats: u32,
    pub exec_running: u32,
    pub last_status_report_ms: u64,
    pub cron_next_fire_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleDecision {
    Keep,
    Stop,
}

pub fn idle_decision(now: u64, worker: &WorkerIdleSnapshot, config: &DaemonConfig) -> IdleDecision {
    if config.idle_timeout_secs == 0 {
        return IdleDecision::Keep;
    }
    if !matches!(worker.state, WorkerState::Ready) {
        return IdleDecision::Keep;
    }
    if worker.pinned {
        return IdleDecision::Keep;
    }
    if worker.live_proxy_streams > 0 {
        return IdleDecision::Keep;
    }
    if worker.lsp_clients > 0 || worker.busy_chats > 0 || worker.exec_running > 0 {
        return IdleDecision::Keep;
    }
    if now.saturating_sub(worker.last_status_report_ms) >= STATUS_FRESH_MS {
        return IdleDecision::Keep;
    }
    if worker
        .cron_next_fire_ms
        .map(|next_fire| cron_pending_blocks_idle_stop(next_fire, now))
        .unwrap_or(false)
    {
        return IdleDecision::Keep;
    }
    let idle_timeout_ms = config.idle_timeout_secs.saturating_mul(1000);
    if now.saturating_sub(worker.last_proxy_activity_ms) < idle_timeout_ms {
        return IdleDecision::Keep;
    }
    IdleDecision::Stop
}

pub fn cron_pending_blocks_idle_stop(next_fire_ms: u64, now: u64) -> bool {
    next_fire_ms <= now.saturating_add(CRON_SOON_MS)
}

/// Parse an idle-check cadence override. Values <= 0 or unparseable fall back to
/// the coarse 60s production default.
fn parse_idle_tick_interval(raw: Option<String>) -> Duration {
    raw.and_then(|raw| raw.parse::<u64>().ok())
        .filter(|ms| *ms > 0)
        .map(Duration::from_millis)
        .unwrap_or(TICK_INTERVAL)
}

/// Idle-check cadence, overridable via `REFACT_DAEMON_IDLE_TICK_MS` for tests so
/// idle-stop is detected quickly and deterministically instead of on the coarse
/// 60s production cadence.
fn idle_tick_interval() -> Duration {
    parse_idle_tick_interval(std::env::var("REFACT_DAEMON_IDLE_TICK_MS").ok())
}

pub fn spawn(state: Arc<DaemonState>) -> JoinHandle<()> {
    let interval = idle_tick_interval();
    tokio::spawn(async move {
        run(state, interval).await;
    })
}

async fn run(state: Arc<DaemonState>, interval_duration: Duration) {
    let mut shutdown_rx = state.shutdown_receiver();
    let mut interval = tokio::time::interval(interval_duration);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => break,
            _ = interval.tick() => tick(&state).await,
        }
    }
}

async fn tick(state: &Arc<DaemonState>) {
    tick_at(state, now_ms()).await;
}

pub(crate) async fn tick_at(state: &Arc<DaemonState>, now: u64) {
    let snapshots = state.worker_idle_snapshots().await;
    for (project_id, snapshot) in snapshots {
        if idle_decision(now, &snapshot, &state.config) != IdleDecision::Stop {
            continue;
        }
        let idle_secs = now
            .saturating_sub(snapshot.last_proxy_activity_ms)
            .saturating_div(1000);
        match state.stop_worker_if_idle(&project_id).await {
            Ok(Some(_)) => {
                let _ = state
                    .events
                    .emit(
                        "worker_idle_stopped",
                        Some(project_id),
                        json!({"idle_secs": idle_secs}),
                    )
                    .await;
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!("failed to idle-stop worker {project_id}: {error}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::events::EventBus;
    use crate::daemon_link::WorkerStatusReport;
    use hyper::{Body, Request, StatusCode};
    use serial_test::serial;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};
    use tempfile::{tempdir, TempDir};
    use tower::ServiceExt;

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
            let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
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

    fn config(timeout_secs: u64) -> DaemonConfig {
        DaemonConfig {
            idle_timeout_secs: timeout_secs,
            ..DaemonConfig::default()
        }
    }

    fn stoppable_snapshot(now: u64) -> WorkerIdleSnapshot {
        WorkerIdleSnapshot {
            state: WorkerState::Ready,
            pinned: false,
            live_proxy_streams: 0,
            last_proxy_activity_ms: now - 2_000,
            lsp_clients: 0,
            busy_chats: 0,
            exec_running: 0,
            last_status_report_ms: now - 1_000,
            cron_next_fire_ms: None,
        }
    }

    #[test]
    fn idle_decision_table() {
        let now = 120_000;
        let stop = stoppable_snapshot(now);
        assert_eq!(idle_decision(now, &stop, &config(2)), IdleDecision::Stop);

        let mut exact_timeout = stoppable_snapshot(now);
        exact_timeout.last_proxy_activity_ms = now - 2_000;
        assert_eq!(
            idle_decision(now, &exact_timeout, &config(2)),
            IdleDecision::Stop
        );

        let mut not_ready = stoppable_snapshot(now);
        not_ready.state = WorkerState::Starting;
        assert_eq!(
            idle_decision(now, &not_ready, &config(2)),
            IdleDecision::Keep
        );

        let mut pinned = stoppable_snapshot(now);
        pinned.pinned = true;
        assert_eq!(idle_decision(now, &pinned, &config(2)), IdleDecision::Keep);

        let mut live_stream = stoppable_snapshot(now);
        live_stream.live_proxy_streams = 1;
        assert_eq!(
            idle_decision(now, &live_stream, &config(2)),
            IdleDecision::Keep
        );

        let mut lsp = stoppable_snapshot(now);
        lsp.lsp_clients = 1;
        assert_eq!(idle_decision(now, &lsp, &config(2)), IdleDecision::Keep);

        let mut busy_chat = stoppable_snapshot(now);
        busy_chat.busy_chats = 1;
        assert_eq!(
            idle_decision(now, &busy_chat, &config(2)),
            IdleDecision::Keep
        );

        let mut exec = stoppable_snapshot(now);
        exec.exec_running = 1;
        assert_eq!(idle_decision(now, &exec, &config(2)), IdleDecision::Keep);

        let mut fresh_activity = stoppable_snapshot(now);
        fresh_activity.last_proxy_activity_ms = now - 1_999;
        assert_eq!(
            idle_decision(now, &fresh_activity, &config(2)),
            IdleDecision::Keep
        );

        let mut stale_report = stoppable_snapshot(now);
        stale_report.last_status_report_ms = now - STATUS_FRESH_MS;
        assert_eq!(
            idle_decision(now, &stale_report, &config(2)),
            IdleDecision::Keep
        );

        let mut cron_soon = stoppable_snapshot(now);
        cron_soon.cron_next_fire_ms = Some(now + CRON_SOON_MS);
        assert_eq!(
            idle_decision(now, &cron_soon, &config(2)),
            IdleDecision::Keep
        );

        let disabled = stoppable_snapshot(now);
        assert_eq!(
            idle_decision(now, &disabled, &config(0)),
            IdleDecision::Keep
        );
    }

    #[test]
    fn idle_tick_interval_override_parsing() {
        assert_eq!(
            parse_idle_tick_interval(Some("500".to_string())),
            Duration::from_millis(500)
        );
        assert_eq!(parse_idle_tick_interval(None), TICK_INTERVAL);
        assert_eq!(
            parse_idle_tick_interval(Some("0".to_string())),
            TICK_INTERVAL
        );
        assert_eq!(
            parse_idle_tick_interval(Some("not-a-number".to_string())),
            TICK_INTERVAL
        );
    }

    #[test]
    fn cron_pending_boundary_blocks_only_within_horizon() {
        let now = 5_000;
        assert!(cron_pending_blocks_idle_stop(now, now));
        assert!(cron_pending_blocks_idle_stop(now + CRON_SOON_MS, now));
        assert!(!cron_pending_blocks_idle_stop(now + CRON_SOON_MS + 1, now));
    }

    async fn wait_for_stopped(state: &DaemonState, project_id: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(info) = state.supervisor.worker_info(project_id).await {
                if matches!(info.state, WorkerState::Stopped) {
                    return;
                }
            }
            assert!(Instant::now() < deadline, "worker did not idle-stop");
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    async fn wait_for_idle_stopped_event(state: &DaemonState, project_id: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if state.events.snapshot().await.iter().any(|event| {
                event.kind == "worker_idle_stopped"
                    && event.project_id.as_deref() == Some(project_id)
            }) {
                return;
            }
            assert!(Instant::now() < deadline, "idle-stop event was not emitted");
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    async fn harness(
        timeout_secs: u64,
    ) -> (
        TempDir,
        Arc<DaemonState>,
        crate::daemon::projects::ProjectEntry,
    ) {
        let dir = tempdir().unwrap();
        let project_root = dir.path().join("idle-project");
        std::fs::create_dir_all(&project_root).unwrap();
        let state = DaemonState::new(
            config(timeout_secs),
            EventBus::new(dir.path().join("events.jsonl")),
            None,
        );
        state.load_projects(dir.path().join("projects.json")).await;
        let entry = {
            let mut registry = state.projects.write().await;
            registry.open(project_root).await.unwrap()
        };
        state.sync_project_liveness(&entry).await;
        (dir, state, entry)
    }

    fn idle_report(project_id: &str) -> WorkerStatusReport {
        WorkerStatusReport {
            project_id: project_id.to_string(),
            pid: 7,
            instance_token: "token".to_string(),
            lsp_clients: 0,
            busy_chats: 0,
            exec_running: 0,
            last_activity_ts: 0,
        }
    }

    async fn store_idle_report_for_worker(state: &DaemonState, project_id: &str) {
        let worker = state.supervisor.worker_info(project_id).await.unwrap();
        let mut report = idle_report(project_id);
        report.pid = worker.pid.unwrap();
        report.instance_token = state
            .supervisor
            .test_worker_instance_token(project_id)
            .await
            .unwrap();
        state.store_worker_status(report).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn idle_tick_stops_fake_worker_and_proxy_rewakes() {
        let Some(_env) = EnvGuard::fake_worker() else {
            return;
        };
        let (_dir, state, entry) = harness(2).await;
        let worker = state.supervisor.ensure_worker(&entry).await.unwrap();
        assert_eq!(worker.state, WorkerState::Ready);
        store_idle_report_for_worker(&state, &entry.id).await;

        let idle_task = tokio::spawn(run(state.clone(), Duration::from_millis(25)));
        wait_for_stopped(&state, &entry.id).await;
        wait_for_idle_stopped_event(&state, &entry.id).await;

        let router = crate::daemon::server::make_router(state.clone(), 8488);
        let response = router
            .oneshot(
                Request::builder()
                    .uri(format!("/p/{}/v1/echo", entry.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let _ = hyper::body::to_bytes(response.into_body()).await.unwrap();
        let rewoken = state.supervisor.worker_info(&entry.id).await.unwrap();
        assert_eq!(rewoken.state, WorkerState::Ready);
        assert_ne!(rewoken.pid, worker.pid);
        idle_task.abort();
        state.supervisor.stop_all().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn idle_tick_keeps_pinned_and_cron_pending_workers() {
        let Some(_env) = EnvGuard::fake_worker() else {
            return;
        };
        let (_dir, state, pinned_entry) = harness(1).await;
        let pinned_entry = {
            let mut registry = state.projects.write().await;
            registry
                .set_pinned(&pinned_entry.id, true)
                .await
                .unwrap()
                .unwrap()
        };
        state.sync_project_liveness(&pinned_entry).await;
        state.supervisor.ensure_worker(&pinned_entry).await.unwrap();
        store_idle_report_for_worker(&state, &pinned_entry.id).await;
        tokio::time::sleep(Duration::from_millis(1100)).await;
        tick_at(&state, now_ms()).await;
        assert_eq!(
            state
                .supervisor
                .worker_info(&pinned_entry.id)
                .await
                .unwrap()
                .state,
            WorkerState::Ready
        );
        state.supervisor.stop_all().await;

        let (_dir, state, entry) = harness(1).await;
        state.supervisor.ensure_worker(&entry).await.unwrap();
        store_idle_report_for_worker(&state, &entry.id).await;
        state
            .set_cron_pending(&entry.id, Some(now_ms() + CRON_SOON_MS))
            .await;
        tokio::time::sleep(Duration::from_millis(1100)).await;
        tick_at(&state, now_ms()).await;
        assert_eq!(
            state.supervisor.worker_info(&entry.id).await.unwrap().state,
            WorkerState::Ready
        );
        state.supervisor.stop_all().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn idle_revalidation_keeps_worker_that_becomes_active() {
        let Some(_env) = EnvGuard::fake_worker() else {
            return;
        };
        let (_dir, state, entry) = harness(1).await;
        state.supervisor.ensure_worker(&entry).await.unwrap();
        store_idle_report_for_worker(&state, &entry.id).await;
        tokio::time::sleep(Duration::from_millis(1100)).await;

        let project_id = entry.id.clone();
        let updater = tokio::spawn({
            let state = state.clone();
            async move {
                state.update_proxy_activity(&project_id).await;
            }
        });
        tokio::time::sleep(Duration::from_millis(25)).await;
        tick_at(&state, now_ms()).await;
        updater.await.unwrap();

        assert_eq!(
            state.supervisor.worker_info(&entry.id).await.unwrap().state,
            WorkerState::Ready
        );
        assert!(!state.events.snapshot().await.iter().any(|event| {
            event.kind == "worker_idle_stopped" && event.project_id.as_deref() == Some(&entry.id)
        }));
        state.supervisor.stop_all().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn idle_snapshot_uses_worker_heartbeat_activity() {
        let Some(_env) = EnvGuard::fake_worker() else {
            return;
        };
        let (_dir, state, entry) = harness(1).await;
        state.supervisor.ensure_worker(&entry).await.unwrap();
        tokio::time::sleep(Duration::from_millis(1100)).await;
        let mut report = idle_report(&entry.id);
        report.last_activity_ts = now_ms();
        let worker = state.supervisor.worker_info(&entry.id).await.unwrap();
        report.pid = worker.pid.unwrap();
        report.instance_token = state
            .supervisor
            .test_worker_instance_token(&entry.id)
            .await
            .unwrap();
        state.store_worker_status(report).await;

        tick_at(&state, now_ms()).await;

        assert_eq!(
            state.supervisor.worker_info(&entry.id).await.unwrap().state,
            WorkerState::Ready
        );
        state.supervisor.stop_all().await;
    }
}
