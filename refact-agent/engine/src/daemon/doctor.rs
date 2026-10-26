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
}
