use refact_core::chat_types::ChatMessage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionExemption {
    Never,
    PreserveAnchor,
    KeepRecentN,
    PreserveWindow,
    DropOnAge,
}

pub fn event_subkind(msg: &ChatMessage) -> Option<&str> {
    msg.extra
        .get("event")
        .and_then(|event| event.get("subkind"))
        .and_then(|subkind| subkind.as_str())
}

pub fn event_source(msg: &ChatMessage) -> &str {
    msg.extra
        .get("event")
        .and_then(|event| event.get("source"))
        .and_then(|source| source.as_str())
        .unwrap_or("unknown")
}

pub fn exemption_for(msg: &ChatMessage) -> CompressionExemption {
    if matches!(msg.role.as_str(), "plan" | "goal") {
        return CompressionExemption::Never;
    }
    if msg.role != "event" {
        return CompressionExemption::PreserveAnchor;
    }

    match event_subkind(msg) {
        Some("plan_delta" | "goal_delta") => CompressionExemption::Never,
        Some("tick" | "mode_switch") => CompressionExemption::DropOnAge,
        Some("process_completed" | "cron_fire") => CompressionExemption::KeepRecentN,
        Some("tool_decision" | "ide_callback" | "verifier_report") => {
            CompressionExemption::PreserveWindow
        }
        Some("summarization_marker" | "system_notice" | "cancellation_note" | "goal_pursuit") => {
            CompressionExemption::PreserveAnchor
        }
        _ => CompressionExemption::PreserveAnchor,
    }
}
