use serde_json::Value;

use crate::client::{worker_state_label, OpenProjectResponse, WorkerInfo};
use crate::events_pane::DaemonEventRecord;

pub(super) fn update_current_worker_from_event(
    project: &mut OpenProjectResponse,
    event: &DaemonEventRecord,
) -> Option<&'static str> {
    if event.project_id.as_deref() != Some(project.project_id.as_str()) {
        return None;
    }
    let state = match event.kind.as_str() {
        "worker_starting" => "starting",
        "worker_ready" => "ready",
        "worker_stopped" => "stopped",
        "worker_crashed" => "crashed",
        _ => return None,
    };
    let previous = project.worker.clone();
    project.worker = Some(WorkerInfo {
        project_id: project.project_id.clone(),
        slug: previous
            .as_ref()
            .map(|worker| worker.slug.clone())
            .unwrap_or_else(|| project.slug.clone()),
        root: previous
            .as_ref()
            .map(|worker| worker.root.clone())
            .unwrap_or_else(|| project.root.clone()),
        root_exists: previous.as_ref().and_then(|worker| worker.root_exists),
        pinned: previous
            .as_ref()
            .and_then(|worker| worker.pinned)
            .or(project.pinned),
        last_active_ms: previous.as_ref().and_then(|worker| worker.last_active_ms),
        state: Value::String(state.to_string()),
        pid: event
            .payload
            .get("pid")
            .and_then(Value::as_u64)
            .map(|pid| pid as u32)
            .or_else(|| previous.as_ref().and_then(|worker| worker.pid)),
        rss_bytes: previous.as_ref().and_then(|worker| worker.rss_bytes),
        cpu_percent: previous.as_ref().and_then(|worker| worker.cpu_percent),
        uptime_secs: previous.as_ref().and_then(|worker| worker.uptime_secs),
        http_port: event
            .payload
            .get("http_port")
            .and_then(Value::as_u64)
            .map(|port| port as u16)
            .or_else(|| previous.as_ref().and_then(|worker| worker.http_port)),
        lsp_port: event
            .payload
            .get("lsp_port")
            .and_then(Value::as_u64)
            .map(|port| port as u16)
            .or_else(|| previous.as_ref().and_then(|worker| worker.lsp_port)),
        lsp_clients: previous.as_ref().and_then(|worker| worker.lsp_clients),
        busy_chats: previous.as_ref().and_then(|worker| worker.busy_chats),
        exec_running: previous.as_ref().and_then(|worker| worker.exec_running),
        live_proxy_streams: previous
            .as_ref()
            .and_then(|worker| worker.live_proxy_streams),
        cron_next_fire_ms: previous
            .as_ref()
            .and_then(|worker| worker.cron_next_fire_ms),
        idle_deadline_ms: previous.as_ref().and_then(|worker| worker.idle_deadline_ms),
        last_status_report_ms: previous
            .as_ref()
            .and_then(|worker| worker.last_status_report_ms),
        last_error: event
            .payload
            .get("error")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                previous
                    .as_ref()
                    .and_then(|worker| worker.last_error.clone())
            }),
        log_path: previous
            .as_ref()
            .map(|worker| worker.log_path.clone())
            .unwrap_or_default(),
    });
    Some(state)
}

pub(super) fn worker_status_line(worker: Option<&WorkerInfo>) -> String {
    let Some(worker) = worker else {
        return "unknown".to_string();
    };
    let mut parts = vec![worker_state_label(Some(worker))];
    if let Some(pid) = worker.pid {
        parts.push(format!("pid {pid}"));
    }
    if let Some(http_port) = worker.http_port.filter(|port| *port > 0) {
        parts.push(format!("http {http_port}"));
    }
    if let Some(lsp_port) = worker.lsp_port.filter(|port| *port > 0) {
        parts.push(format!("lsp {lsp_port}"));
    }
    if let Some(error) = worker
        .last_error
        .as_deref()
        .filter(|error| !error.trim().is_empty())
    {
        parts.push(format!("error {error}"));
    }
    parts.join(" · ")
}
