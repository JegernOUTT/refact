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
    if msg.role == "plan" {
        return CompressionExemption::Never;
    }
    if msg.role != "event" {
        return CompressionExemption::PreserveAnchor;
    }

    match event_subkind(msg) {
        Some("tick" | "mode_switch") => CompressionExemption::DropOnAge,
        Some("process_completed" | "cron_fire") => CompressionExemption::KeepRecentN,
        Some("tool_decision" | "ide_callback" | "verifier_report") => {
            CompressionExemption::PreserveWindow
        }
        Some("summarization_marker" | "system_notice" | "cancellation_note") => {
            CompressionExemption::PreserveAnchor
        }
        _ => CompressionExemption::PreserveAnchor,
    }
}
