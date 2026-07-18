use std::collections::HashSet;
use std::future::Future;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::Json;
use futures::future::join_all;
use serde::Serialize;

use crate::daemon::config::DaemonConfig;
use crate::daemon::state::DaemonState;
use crate::daemon::supervisor::{WorkerInfo, WorkerState};

const CHECK_TIMEOUT: Duration = Duration::from_secs(2);
const DISK_WARNING_BYTES: u64 = 10 * 1024 * 1024 * 1024;
const DISK_SCAN_BYTE_CAP: u64 = 20 * 1024 * 1024 * 1024;
const DISK_SCAN_ENTRY_CAP: usize = 200_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingSeverity {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub id: String,
    pub severity: FindingSeverity,
    pub message: String,
    pub detail: Option<String>,
    pub fix_action: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorResponse {
    pub findings: Vec<Finding>,
    pub generated_at_ms: u64,
}

type CheckFuture = Pin<Box<dyn Future<Output = Vec<Finding>> + Send>>;

pub async fn handle(State((state, port)): State<(Arc<DaemonState>, u16)>) -> Json<DoctorResponse> {
    Json(doctor_report(state, port).await)
}

pub async fn doctor_report(state: Arc<DaemonState>, port: u16) -> DoctorResponse {
    doctor_report_with_cache_root(state, port, crate::daemon::paths::cache_root()).await
}

async fn doctor_report_with_cache_root(
    state: Arc<DaemonState>,
    port: u16,
    cache_root: PathBuf,
) -> DoctorResponse {
    let checks: Vec<CheckFuture> = vec![
        Box::pin(timed_check(
            "port_conflict",
            CHECK_TIMEOUT,
            port_conflict_check(state.clone(), port),
        )),
        Box::pin(timed_check(
            "disk_cache_usage",
            CHECK_TIMEOUT,
            disk_cache_usage_check(cache_root),
        )),
        Box::pin(timed_check(
            "orphan_workers",
            CHECK_TIMEOUT,
            orphan_workers_check(state.clone()),
        )),
        Box::pin(timed_check(
            "version_vs_latest",
            CHECK_TIMEOUT,
            version_vs_latest_check(state.clone()),
        )),
        Box::pin(timed_check(
            "lan_without_auth",
            CHECK_TIMEOUT,
            ready(lan_without_auth_check(&state.config)),
        )),
        Box::pin(timed_check(
            "binary_path",
            CHECK_TIMEOUT,
            binary_path_check(),
        )),
    ];
    DoctorResponse {
        findings: join_all(checks).await.into_iter().flatten().collect(),
        generated_at_ms: crate::daemon::state::now_ms(),
    }
}

async fn timed_check<F>(id: &'static str, budget: Duration, check: F) -> Vec<Finding>
where
    F: Future<Output = Option<Finding>>,
{
    match tokio::time::timeout(budget, check).await {
        Ok(Some(finding)) => vec![finding],
        Ok(None) => Vec::new(),
        Err(_) => vec![finding(
            id,
            FindingSeverity::Info,
            "Check timed out",
            Some(format!("The {id} check exceeded {}ms", budget.as_millis())),
            None,
        )],
    }
}

async fn ready(value: Option<Finding>) -> Option<Finding> {
    value
}

async fn port_conflict_check(state: Arc<DaemonState>, daemon_port: u16) -> Option<Finding> {
    let workers = state.supervisor.worker_infos().await;
    let client = match reqwest::Client::builder()
        .connect_timeout(Duration::from_millis(250))
        .timeout(Duration::from_millis(500))
        .no_proxy()
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            return Some(finding(
                "port_conflict",
                FindingSeverity::Warning,
                format!("Daemon control port {daemon_port} is healthy"),
                Some(format!("Worker port checks could not start: {error}")),
                None,
            ));
        }
    };
    let probes = workers.into_iter().map(|worker| {
        let client = client.clone();
        async move {
            let responsive = client
                .get(format!("http://127.0.0.1:{}/v1/ping", worker.http_port))
                .send()
                .await
                .map(|response| response.status().is_success())
                .unwrap_or(false);
            (worker, responsive)
        }
    });
    let mismatches = join_all(probes)
        .await
        .into_iter()
        .filter_map(|(worker, responsive)| {
            let expected = matches!(worker.state, WorkerState::Ready);
            (responsive != expected).then(|| {
                format!(
                    "{}:{} state={:?} responsive={responsive}",
                    worker.project_id, worker.http_port, worker.state
                )
            })
        })
        .collect::<Vec<_>>();
    Some(finding(
        "port_conflict",
        if mismatches.is_empty() {
            FindingSeverity::Info
        } else {
            FindingSeverity::Warning
        },
        if mismatches.is_empty() {
            format!("Daemon control port {daemon_port} and worker ports are healthy")
        } else {
            format!(
                "Daemon control port {daemon_port} is healthy, but {} worker port states mismatch",
                mismatches.len()
            )
        },
        (!mismatches.is_empty()).then(|| mismatches.join("; ")),
        None,
    ))
}

#[derive(Debug, Default, PartialEq, Eq)]
struct DiskUsage {
    worktrees: u64,
    shadow_repos: u64,
    logs: u64,
    capped: bool,
}

impl DiskUsage {
    fn total(&self) -> u64 {
        self.worktrees
            .saturating_add(self.shadow_repos)
            .saturating_add(self.logs)
    }
}

async fn disk_cache_usage_check(cache_root: PathBuf) -> Option<Finding> {
    let usage = match tokio::task::spawn_blocking(move || cache_usage(&cache_root)).await {
        Ok(usage) => usage,
        Err(error) => {
            return Some(finding(
                "disk_cache_usage",
                FindingSeverity::Info,
                "Cache usage check failed",
                Some(error.to_string()),
                None,
            ));
        }
    };
    let total = usage.total();
    Some(finding(
        "disk_cache_usage",
        if total > DISK_WARNING_BYTES {
            FindingSeverity::Warning
        } else {
            FindingSeverity::Info
        },
        format!("Refact caches use {} bytes", total),
        Some(format!(
            "worktrees={} shadow_repos={} logs={} capped={}",
            usage.worktrees, usage.shadow_repos, usage.logs, usage.capped
        )),
        (total > DISK_WARNING_BYTES).then(|| "prune_caches".to_string()),
    ))
}

fn cache_usage(cache_root: &Path) -> DiskUsage {
    let mut remaining_entries = DISK_SCAN_ENTRY_CAP;
    let mut remaining_bytes = DISK_SCAN_BYTE_CAP;
    let (worktrees, worktrees_capped) = directory_size(
        &cache_root.join("worktrees"),
        &mut remaining_entries,
        &mut remaining_bytes,
    );
    let (shadow_repos, shadow_capped) = directory_size(
        &cache_root.join("shadow_git"),
        &mut remaining_entries,
        &mut remaining_bytes,
    );
    let (root_logs, root_logs_capped) = directory_size(
        &cache_root.join("logs"),
        &mut remaining_entries,
        &mut remaining_bytes,
    );
    let (daemon_logs, daemon_logs_capped) = directory_size(
        &cache_root.join("daemon").join("logs"),
        &mut remaining_entries,
        &mut remaining_bytes,
    );
    DiskUsage {
        worktrees,
        shadow_repos,
        logs: root_logs.saturating_add(daemon_logs),
        capped: worktrees_capped || shadow_capped || root_logs_capped || daemon_logs_capped,
    }
}

fn directory_size(
    path: &Path,
    remaining_entries: &mut usize,
    remaining_bytes: &mut u64,
) -> (u64, bool) {
    let mut bytes = 0u64;
    if !path.exists() {
        return (bytes, false);
    }
    for entry in walkdir::WalkDir::new(path).follow_links(false) {
        if *remaining_entries == 0 || *remaining_bytes == 0 {
            return (bytes, true);
        }
        *remaining_entries -= 1;
        let Ok(entry) = entry else {
            continue;
        };
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let add = metadata.len().min(*remaining_bytes);
        bytes = bytes.saturating_add(add);
        *remaining_bytes -= add;
        if add < metadata.len() {
            return (bytes, true);
        }
    }
    (bytes, false)
}

async fn orphan_workers_check(state: Arc<DaemonState>) -> Option<Finding> {
    let registered = state
        .projects
        .read()
        .await
        .list()
        .into_iter()
        .map(|project| project.id)
        .collect::<HashSet<_>>();
    let workers = state.supervisor.worker_infos().await;
    orphan_workers_finding(&registered, &workers).await
}

async fn orphan_workers_finding(
    registered: &HashSet<String>,
    workers: &[WorkerInfo],
) -> Option<Finding> {
    let unregistered_pids = workers
        .iter()
        .filter(|worker| !registered.contains(&worker.project_id))
        .filter_map(|worker| worker.pid)
        .collect::<Vec<_>>();
    let alive = tokio::task::spawn_blocking(move || {
        crate::daemon::resources::worker_resources(&unregistered_pids)
    })
    .await
    .unwrap_or_default();
    let mut issues = Vec::new();
    let mut first_project_id = None;
    for worker in workers {
        let failed = matches!(
            worker.state,
            WorkerState::Crashed | WorkerState::Failed { .. }
        );
        let unregistered_alive = !registered.contains(&worker.project_id)
            && worker
                .pid
                .map(|pid| alive.contains_key(&pid))
                .unwrap_or(false);
        if failed || unregistered_alive {
            first_project_id.get_or_insert_with(|| worker.project_id.clone());
            issues.push(format!(
                "{} state={:?} pid={}",
                worker.project_id,
                worker.state,
                worker
                    .pid
                    .map(|pid| pid.to_string())
                    .unwrap_or_else(|| "none".to_string())
            ));
        }
    }
    (!issues.is_empty()).then(|| {
        finding(
            "orphan_workers",
            FindingSeverity::Warning,
            format!("{} workers need recovery", issues.len()),
            Some(issues.join("; ")),
            first_project_id.map(|project_id| format!("restart_worker:{project_id}")),
        )
    })
}

async fn version_vs_latest_check(state: Arc<DaemonState>) -> Option<Finding> {
    match crate::daemon::server::cached_update_check(&state, false).await {
        Ok(info) if info.update_available => Some(finding(
            "version_vs_latest",
            FindingSeverity::Warning,
            format!(
                "Version {} is behind {}",
                info.current_version,
                info.latest_version.unwrap_or_else(|| "latest".to_string())
            ),
            None,
            Some("run_update".to_string()),
        )),
        Ok(info) => Some(finding(
            "version_vs_latest",
            FindingSeverity::Info,
            format!("Version {} is current", info.current_version),
            info.latest_version
                .map(|latest| format!("Latest release: {latest}")),
            None,
        )),
        Err(error) => Some(finding(
            "version_vs_latest",
            FindingSeverity::Info,
            "Version check unavailable",
            Some(error),
            None,
        )),
    }
}

fn lan_without_auth_check(config: &DaemonConfig) -> Option<Finding> {
    let lan_enabled = config
        .bind
        .parse::<IpAddr>()
        .map(|ip| !ip.is_loopback())
        .unwrap_or(false);
    (lan_enabled && !config.auth.enabled).then(|| {
        finding(
            "lan_without_auth",
            FindingSeverity::Critical,
            "LAN access is enabled without authentication",
            Some(format!("Daemon bind address: {}", config.bind)),
            Some("open_settings".to_string()),
        )
    })
}

async fn binary_path_check() -> Option<Finding> {
    match std::env::current_exe() {
        Ok(path) if path.is_file() => Some(finding(
            "binary_path",
            FindingSeverity::Info,
            "Daemon binary path is valid",
            Some(path.display().to_string()),
            None,
        )),
        Ok(path) => Some(finding(
            "binary_path",
            FindingSeverity::Warning,
            "Daemon binary path is not a file",
            Some(path.display().to_string()),
            None,
        )),
        Err(error) => Some(finding(
            "binary_path",
            FindingSeverity::Warning,
            "Daemon binary path could not be resolved",
            Some(error.to_string()),
            None,
        )),
    }
}

fn finding(
    id: impl Into<String>,
    severity: FindingSeverity,
    message: impl Into<String>,
    detail: Option<String>,
    fix_action: Option<String>,
) -> Finding {
    Finding {
        id: id.into(),
        severity,
        message: message.into(),
        detail,
        fix_action,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::config::AuthConfig;
    use crate::daemon::events::EventBus;
    use crate::daemon::projects::ProjectRegistry;

    #[test]
    fn lan_without_auth_is_critical() {
        let config = DaemonConfig {
            bind: "0.0.0.0".to_string(),
            auth: AuthConfig {
                enabled: false,
                ..Default::default()
            },
            ..Default::default()
        };
        let finding = lan_without_auth_check(&config).unwrap();
        assert_eq!(finding.severity, FindingSeverity::Critical);
        assert_eq!(finding.fix_action.as_deref(), Some("open_settings"));
    }

    #[tokio::test]
    async fn orphan_worker_state_requests_restart() {
        let worker = WorkerInfo {
            project_id: "project-a".to_string(),
            pid: None,
            http_port: 8001,
            lsp_port: 9001,
            state: WorkerState::Crashed,
            last_error: Some("boom".to_string()),
        };
        let finding = orphan_workers_finding(&HashSet::from(["project-a".to_string()]), &[worker])
            .await
            .unwrap();
        assert_eq!(finding.severity, FindingSeverity::Warning);
        assert_eq!(
            finding.fix_action.as_deref(),
            Some("restart_worker:project-a")
        );
    }

    #[test]
    fn disk_walker_reports_cache_subdirectories() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("worktrees/a")).unwrap();
        std::fs::create_dir_all(dir.path().join("shadow_git/b")).unwrap();
        std::fs::create_dir_all(dir.path().join("logs")).unwrap();
        std::fs::create_dir_all(dir.path().join("daemon/logs")).unwrap();
        std::fs::write(dir.path().join("worktrees/a/file"), vec![0; 3]).unwrap();
        std::fs::write(dir.path().join("shadow_git/b/file"), vec![0; 5]).unwrap();
        std::fs::write(dir.path().join("logs/file"), vec![0; 7]).unwrap();
        std::fs::write(dir.path().join("daemon/logs/file"), vec![0; 11]).unwrap();
        assert_eq!(
            cache_usage(dir.path()),
            DiskUsage {
                worktrees: 3,
                shadow_repos: 5,
                logs: 18,
                capped: false,
            }
        );
    }

    #[tokio::test]
    async fn timed_check_returns_info_finding_on_timeout() {
        let findings = timed_check("slow", Duration::from_millis(1), async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            None
        })
        .await;
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].id, "slow");
        assert_eq!(findings[0].severity, FindingSeverity::Info);
        assert_eq!(findings[0].message, "Check timed out");
    }

    #[test]
    fn worker_resources_are_present_for_current_pid_and_absent_for_dead_pid() {
        let samples = crate::daemon::resources::worker_resources(&[std::process::id(), u32::MAX]);
        let current = samples.get(&std::process::id()).unwrap();
        assert!(current.rss_bytes > 0);
        assert!(current.cpu_percent >= 0.0);
        assert!(!samples.contains_key(&u32::MAX));
    }

    #[tokio::test]
    async fn worker_rows_include_live_resources_and_null_dead_resources() {
        let dir = tempfile::tempdir().unwrap();
        let state = DaemonState::new(
            DaemonConfig::default(),
            EventBus::new(dir.path().join("events.jsonl")),
            None,
        );
        *state.projects.write().await = ProjectRegistry::empty(dir.path().join("projects.json"));
        let live_root = dir.path().join("live");
        let dead_root = dir.path().join("dead");
        std::fs::create_dir_all(&live_root).unwrap();
        std::fs::create_dir_all(&dead_root).unwrap();
        let live = state.projects.write().await.open(live_root).await.unwrap();
        let dead = state.projects.write().await.open(dead_root).await.unwrap();
        state
            .supervisor
            .set_test_worker_info(&live.id, std::process::id(), WorkerState::Ready, "live")
            .await;
        state
            .supervisor
            .set_test_worker_info(&dead.id, u32::MAX, WorkerState::Ready, "dead")
            .await;

        let rows = state.worker_rows().await;
        let live_row = rows.iter().find(|row| row.project_id == live.id).unwrap();
        assert!(live_row.rss_bytes.is_some());
        assert!(live_row.cpu_percent.is_some());
        assert!(live_row.uptime_secs.is_some());
        let dead_row = rows.iter().find(|row| row.project_id == dead.id).unwrap();
        assert_eq!(dead_row.rss_bytes, None);
        assert_eq!(dead_row.cpu_percent, None);
        assert_eq!(dead_row.uptime_secs, None);
    }

    #[tokio::test]
    async fn doctor_report_has_machine_readable_findings_under_three_seconds() {
        let dir = tempfile::tempdir().unwrap();
        let state = DaemonState::new_with_daemon_dir(
            DaemonConfig::default(),
            EventBus::new(dir.path().join("events.jsonl")),
            None,
            dir.path().join("daemon"),
            8488,
        );
        {
            let mut cache = state.update_check_cache.lock().await;
            *cache = Some((
                std::time::Instant::now(),
                refact_self_update::UpdateCheckInfo {
                    current_version: env!("CARGO_PKG_VERSION").to_string(),
                    latest_version: Some(env!("CARGO_PKG_VERSION").to_string()),
                    update_available: false,
                    releases: Vec::new(),
                },
            ));
        }
        let started = std::time::Instant::now();
        let report = doctor_report_with_cache_root(state, 8488, dir.path().to_path_buf()).await;
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(report.generated_at_ms > 0);
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.id == "port_conflict"));
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.id == "disk_cache_usage"));
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.id == "version_vs_latest"));
    }
}
