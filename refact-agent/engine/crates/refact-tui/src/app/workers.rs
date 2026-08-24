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
        pid: payload_u32_or_previous(
            &event.payload,
            "pid",
            previous.as_ref().and_then(|worker| worker.pid),
        ),
        rss_bytes: previous.as_ref().and_then(|worker| worker.rss_bytes),
        cpu_percent: previous.as_ref().and_then(|worker| worker.cpu_percent),
        uptime_secs: previous.as_ref().and_then(|worker| worker.uptime_secs),
        http_port: payload_u16_or_previous(
            &event.payload,
            "http_port",
            previous.as_ref().and_then(|worker| worker.http_port),
        ),
        lsp_port: payload_u16_or_previous(
            &event.payload,
            "lsp_port",
            previous.as_ref().and_then(|worker| worker.lsp_port),
        ),
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
        last_error: event_last_error(&event.payload).or_else(|| match state {
            "starting" | "ready" => None,
            _ => previous
                .as_ref()
                .and_then(|worker| worker.last_error.clone()),
        }),
        log_path: previous
            .as_ref()
            .map(|worker| worker.log_path.clone())
            .unwrap_or_default(),
    });
    Some(state)
}

fn payload_u32_or_previous(payload: &Value, name: &str, previous: Option<u32>) -> Option<u32> {
    match payload.get(name) {
        Some(value) => value.as_u64().and_then(|value| u32::try_from(value).ok()),
        None => previous,
    }
}

fn payload_u16_or_previous(payload: &Value, name: &str, previous: Option<u16>) -> Option<u16> {
    match payload.get(name) {
        Some(value) => value.as_u64().and_then(|value| u16::try_from(value).ok()),
        None => previous,
    }
}

fn event_last_error(payload: &Value) -> Option<String> {
    payload
        .get("last_error")
        .or_else(|| payload.get("error"))
        .and_then(Value::as_str)
        .filter(|error| !error.trim().is_empty())
        .map(str::to_string)
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::*;

    fn project() -> OpenProjectResponse {
        OpenProjectResponse {
            project_id: "project".to_string(),
            slug: "fixture".to_string(),
            root: PathBuf::from("/tmp/fixture"),
            pinned: None,
            worker: Some(WorkerInfo {
                project_id: "project".to_string(),
                pid: Some(7),
                http_port: Some(31000),
                lsp_port: Some(31001),
                state: Value::String("ready".to_string()),
                last_error: None,
                ..WorkerInfo::default()
            }),
            cron_pending: None,
        }
    }

    fn event(kind: &str, payload: Value) -> DaemonEventRecord {
        DaemonEventRecord {
            ts_ms: None,
            kind: kind.to_string(),
            project_id: Some("project".to_string()),
            payload,
        }
    }

    #[test]
    fn crashed_worker_error_clears_when_starting_then_ready() {
        let mut project = project();
        update_current_worker_from_event(
            &mut project,
            &event("worker_crashed", json!({"last_error": "worker exited"})),
        );
        assert_eq!(
            project.worker.as_ref().unwrap().last_error.as_deref(),
            Some("worker exited")
        );

        update_current_worker_from_event(&mut project, &event("worker_starting", json!({})));
        assert_eq!(project.worker.as_ref().unwrap().last_error, None);

        update_current_worker_from_event(&mut project, &event("worker_ready", json!({})));
        assert_eq!(project.worker.as_ref().unwrap().last_error, None);
    }

    #[test]
    fn overflow_worker_telemetry_is_dropped_without_wrapping() {
        let mut project = project();
        update_current_worker_from_event(
            &mut project,
            &event(
                "worker_ready",
                json!({
                    "pid": u64::from(u32::MAX) + 1,
                    "http_port": u64::from(u16::MAX) + 1,
                    "lsp_port": u64::from(u16::MAX) + 1,
                }),
            ),
        );

        let worker = project.worker.unwrap();
        assert_eq!(worker.pid, None);
        assert_eq!(worker.http_port, None);
        assert_eq!(worker.lsp_port, None);
    }
}
