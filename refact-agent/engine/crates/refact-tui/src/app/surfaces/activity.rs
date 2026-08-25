use crate::client::{worker_state_label, WorkerInfo};
use crate::protocol::{BackgroundAgentSummary, ProcessCompletedEvent};
use crate::text_safety::sanitize_tool_inline;
use crate::tools::ToolCard;

#[derive(Debug, Clone, Default)]
pub(crate) struct ActivitySurfaceState {
    selected_agent_id: Option<String>,
}

impl ActivitySurfaceState {
    pub(crate) fn new(agents: &[BackgroundAgentSummary]) -> Self {
        Self {
            selected_agent_id: agents.first().map(|agent| agent.agent_id.clone()),
        }
    }

    pub(crate) fn selected_agent<'a>(
        &self,
        agents: &'a [BackgroundAgentSummary],
    ) -> Option<&'a BackgroundAgentSummary> {
        self.selected_agent_id
            .as_deref()
            .and_then(|id| agents.iter().find(|agent| agent.agent_id == id))
            .or_else(|| agents.first())
    }

    pub(crate) fn move_selection(&mut self, agents: &[BackgroundAgentSummary], offset: isize) {
        let Some(current) = self.selected_agent(agents) else {
            self.selected_agent_id = None;
            return;
        };
        let current_index = agents
            .iter()
            .position(|agent| agent.agent_id == current.agent_id)
            .unwrap_or_default();
        let selected_index = if offset.is_negative() {
            current_index.saturating_sub(offset.unsigned_abs())
        } else {
            current_index
                .saturating_add(offset as usize)
                .min(agents.len().saturating_sub(1))
        };
        self.selected_agent_id = agents
            .get(selected_index)
            .map(|agent| agent.agent_id.clone());
    }

    pub(crate) fn selected_agent_id(&self) -> Option<&str> {
        self.selected_agent_id.as_deref()
    }
}

#[derive(Debug, Clone)]
pub(super) struct ActivityOverlay {
    pub(super) lines: Vec<String>,
}

pub(crate) fn surfaces_enabled() -> bool {
    surfaces_enabled_from_value(std::env::var("REFACT_TUI_SURFACES").ok().as_deref())
}

fn surfaces_enabled_from_value(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

pub(super) fn overlay(
    agents: &[BackgroundAgentSummary],
    workers: &[WorkerInfo],
    current_worker: Option<&WorkerInfo>,
    completed_processes: &[ProcessCompletedEvent],
    process_cards: &[&ToolCard],
    selected_agent_id: Option<&str>,
) -> ActivityOverlay {
    let mut lines = vec!["Background agents / delegates".to_string()];
    if agents.is_empty() {
        lines.push("No background agents reported by this chat.".to_string());
    } else {
        for agent in agents {
            push_agent_lines(
                &mut lines,
                agent,
                selected_agent_id.is_some_and(|id| id == agent.agent_id),
            );
        }
    }

    lines.push(String::new());
    lines.push("Worker telemetry".to_string());
    let workers = workers_for_display(workers, current_worker);
    if workers.is_empty() {
        lines.push("No workers reported by the daemon.".to_string());
    } else {
        for worker in workers {
            push_worker_lines(&mut lines, worker);
        }
    }

    lines.push(String::new());
    lines.push("Processes and services".to_string());
    push_process_completion_lines(&mut lines, completed_processes);
    if process_cards.is_empty() {
        lines.push("Exec registry: no process or service tool records.".to_string());
    } else {
        lines.push("Exec registry".to_string());
        for card in process_cards {
            let name = card.name.strip_prefix("t_").unwrap_or(&card.name);
            lines.push(format!("• {} · {}", clean(name), card.status.label()));
            lines.push(format!("  tool ID: {}", unknown(&card.id)));
            lines.push(format!("  details: {}", unknown(&card.args_preview)));
            lines.push(format!("  duration ms: {}", option_value(card.duration_ms)));
        }
    }

    ActivityOverlay { lines }
}

pub(super) fn is_process_registry_card(card: &ToolCard) -> bool {
    let name = card.name.strip_prefix("t_").unwrap_or(&card.name);
    name.starts_with("process_")
        || matches!(
            name,
            "shell" | "shell_service" | "clean_background_processes"
        )
}

fn push_agent_lines(lines: &mut Vec<String>, agent: &BackgroundAgentSummary, selected: bool) {
    let marker = if selected { "›" } else { "•" };
    lines.push(format!(
        "{marker} [{}] {}",
        unknown(&agent.status),
        unknown(&agent.title)
    ));
    lines.push(format!("  agent ID: {}", unknown(&agent.agent_id)));
    lines.push(format!("  kind: {}", unknown(&agent.kind)));
    lines.push(format!(
        "  progress: {}",
        option_text(agent.progress.as_deref())
    ));
    lines.push(format!("  steps: {}", agent.step_count));
    lines.push(format!(
        "  last activity: {}",
        option_text(agent.last_activity.as_deref())
    ));
    lines.push(format!(
        "  edited files: {}",
        list_value(&agent.edited_files)
    ));
    lines.push(format!(
        "  target files: {}",
        list_value(&agent.target_files)
    ));
    lines.push(format!(
        "  diff summary: {}",
        option_text(agent.diff_summary.as_deref())
    ));
    lines.push(format!(
        "  conflict summary: {}",
        option_text(agent.conflict_summary.as_deref())
    ));
    lines.push(format!(
        "  result summary: {}",
        option_text(agent.result_summary.as_deref())
    ));
    lines.push(format!("  error: {}", option_text(agent.error.as_deref())));
    lines.push(format!(
        "  child chat: {}",
        option_text(agent.child_chat_id.as_deref())
    ));
}

fn workers_for_display<'a>(
    workers: &'a [WorkerInfo],
    current_worker: Option<&'a WorkerInfo>,
) -> Vec<&'a WorkerInfo> {
    let mut values = workers.iter().collect::<Vec<_>>();
    if let Some(current) = current_worker {
        let represented = values
            .iter()
            .any(|worker| worker.project_id == current.project_id && worker.slug == current.slug);
        if !represented {
            values.push(current);
        }
    }
    values
}

fn push_worker_lines(lines: &mut Vec<String>, worker: &WorkerInfo) {
    lines.push(format!(
        "• worker {} · {}",
        unknown(&worker.slug),
        worker_state_label(Some(worker))
    ));
    lines.push(format!("  project ID: {}", unknown(&worker.project_id)));
    lines.push(format!("  slug: {}", unknown(&worker.slug)));
    lines.push(format!("  root: {}", path_value(worker)));
    lines.push(format!(
        "  root exists: {}",
        option_value(worker.root_exists)
    ));
    lines.push(format!("  pinned: {}", option_value(worker.pinned)));
    lines.push(format!(
        "  last active ms: {}",
        option_value(worker.last_active_ms)
    ));
    lines.push(format!("  state: {}", worker_state_label(Some(worker))));
    lines.push(format!("  PID: {}", option_value(worker.pid)));
    lines.push(format!("  RSS bytes: {}", option_value(worker.rss_bytes)));
    lines.push(format!(
        "  CPU percent: {}",
        option_value(worker.cpu_percent)
    ));
    lines.push(format!(
        "  uptime secs: {}",
        option_value(worker.uptime_secs)
    ));
    lines.push(format!("  HTTP port: {}", option_value(worker.http_port)));
    lines.push(format!("  LSP port: {}", option_value(worker.lsp_port)));
    lines.push(format!(
        "  LSP clients: {}",
        option_value(worker.lsp_clients)
    ));
    lines.push(format!("  busy chats: {}", option_value(worker.busy_chats)));
    lines.push(format!(
        "  exec running: {}",
        option_value(worker.exec_running)
    ));
    lines.push(format!(
        "  live proxy streams: {}",
        option_value(worker.live_proxy_streams)
    ));
    lines.push(format!(
        "  cron next fire ms: {}",
        option_value(worker.cron_next_fire_ms)
    ));
    lines.push(format!(
        "  idle deadline ms: {}",
        option_value(worker.idle_deadline_ms)
    ));
    lines.push(format!(
        "  last status report ms: {}",
        option_value(worker.last_status_report_ms)
    ));
    lines.push(format!(
        "  last error: {}",
        option_text(worker.last_error.as_deref())
    ));
    lines.push(format!("  log path: {}", unknown(&worker.log_path)));
}

fn push_process_completion_lines(lines: &mut Vec<String>, processes: &[ProcessCompletedEvent]) {
    if processes.is_empty() {
        lines.push("No process_completed events received.".to_string());
        return;
    }
    for process in processes {
        lines.push(format!(
            "• {} · {} · {}",
            unknown(&process.short_description),
            unknown(&process.status),
            unknown(&process.mode)
        ));
        lines.push(format!("  process ID: {}", unknown(&process.process_id)));
        lines.push(format!("  exit code: {}", option_value(process.exit_code)));
    }
}

fn unknown(value: &str) -> String {
    let value = clean(value);
    if value.trim().is_empty() {
        "unknown".to_string()
    } else {
        value
    }
}

fn option_text(value: Option<&str>) -> String {
    value.map_or_else(|| "unknown".to_string(), unknown)
}

fn option_value<T: std::fmt::Display>(value: Option<T>) -> String {
    value.map_or_else(|| "unknown".to_string(), |value| value.to_string())
}

fn list_value(values: &[String]) -> String {
    if values.is_empty() {
        "unknown".to_string()
    } else {
        values
            .iter()
            .map(|value| unknown(value))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn path_value(worker: &WorkerInfo) -> String {
    let path = worker.root.to_string_lossy();
    unknown(&path)
}

fn clean(value: &str) -> String {
    sanitize_tool_inline(value)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::*;

    fn agent() -> BackgroundAgentSummary {
        BackgroundAgentSummary {
            agent_id: "agent-1".to_string(),
            child_chat_id: Some("child-1".to_string()),
            kind: "task".to_string(),
            status: "running".to_string(),
            title: "Implement activity".to_string(),
            progress: Some("Writing tests".to_string()),
            step_count: 3,
            last_activity: Some("now".to_string()),
            target_files: vec!["src/app.rs".to_string()],
            edited_files: vec!["src/activity.rs".to_string()],
            diff_summary: Some("one file changed".to_string()),
            conflict_summary: Some("none".to_string()),
            result_summary: Some("partial".to_string()),
            error: None,
            ..BackgroundAgentSummary::default()
        }
    }

    #[test]
    fn overlay_renders_agent_status_progress_and_process_completion() {
        let process = ProcessCompletedEvent {
            process_id: "exec-1".to_string(),
            status: "exited".to_string(),
            exit_code: Some(0),
            short_description: "cargo test".to_string(),
            mode: "service".to_string(),
        };
        let card = ToolCard::from_tool_call(&json!({
            "id": "tool-1",
            "function": {"name": "process_start", "arguments": "{\"service_name\":\"demo\"}"},
            "status": "running"
        }));

        let overlay = overlay(&[agent()], &[], None, &[process], &[&card], Some("agent-1"));
        let text = overlay.lines.join("\n");

        assert!(text.contains("[running] Implement activity"));
        assert!(text.contains("progress: Writing tests"));
        assert!(text.contains("edited files: src/activity.rs"));
        assert!(text.contains("diff summary: one file changed"));
        assert!(text.contains("conflict summary: none"));
        assert!(text.contains("result summary: partial"));
        assert!(text.contains("cargo test · exited · service"));
        assert!(text.contains("process_start · running"));
    }

    #[test]
    fn worker_telemetry_renders_every_field_and_unknowns() {
        let worker = WorkerInfo {
            project_id: "project-1".to_string(),
            slug: "demo".to_string(),
            root: PathBuf::from("/tmp/demo"),
            root_exists: Some(true),
            pinned: Some(false),
            last_active_ms: Some(1),
            state: json!("ready"),
            pid: Some(2),
            rss_bytes: Some(3),
            cpu_percent: Some(4.5),
            uptime_secs: Some(5),
            http_port: Some(6),
            lsp_port: Some(7),
            lsp_clients: Some(8),
            busy_chats: Some(9),
            exec_running: Some(10),
            live_proxy_streams: Some(11),
            cron_next_fire_ms: Some(12),
            idle_deadline_ms: Some(13),
            last_status_report_ms: Some(14),
            last_error: Some("none".to_string()),
            log_path: "/tmp/demo.log".to_string(),
        };
        let text = overlay(&[], &[worker], None, &[], &[], None)
            .lines
            .join("\n");

        for label in [
            "project ID",
            "slug",
            "root",
            "root exists",
            "pinned",
            "last active ms",
            "state",
            "PID",
            "RSS bytes",
            "CPU percent",
            "uptime secs",
            "HTTP port",
            "LSP port",
            "LSP clients",
            "busy chats",
            "exec running",
            "live proxy streams",
            "cron next fire ms",
            "idle deadline ms",
            "last status report ms",
            "last error",
            "log path",
        ] {
            assert!(text.contains(label), "missing {label}");
        }

        let unknown = overlay(&[], &[WorkerInfo::default()], None, &[], &[], None)
            .lines
            .join("\n");
        assert!(unknown.contains("RSS bytes: unknown"));
        assert!(unknown.contains("log path: unknown"));
    }

    #[test]
    fn activity_flag_accepts_only_truthy_values() {
        assert!(surfaces_enabled_from_value(Some("1")));
        assert!(surfaces_enabled_from_value(Some("on")));
        assert!(!surfaces_enabled_from_value(Some("0")));
        assert!(!surfaces_enabled_from_value(None));
    }
}
