use chrono::Utc;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticSeverity {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticContext {
    pub error_type: String,
    pub error_message: String,
    pub source_file: Option<String>,
    pub tool_name: Option<String>,
    pub chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    pub collected_at: String,
    pub severity: DiagnosticSeverity,
}

fn classify_diagnostic_severity_with_default(
    error: &str,
    default_severity: DiagnosticSeverity,
) -> DiagnosticSeverity {
    if refact_core::retry_policy::classify_user_error(error)
        == refact_core::retry_policy::UserErrorCategory::StreamCorrupted
    {
        return DiagnosticSeverity::Medium;
    }
    let lower = error.to_lowercase();
    if lower.contains("critical") || lower.contains("panic") {
        DiagnosticSeverity::Critical
    } else if lower.contains("error") {
        DiagnosticSeverity::High
    } else if lower.contains("warn") {
        DiagnosticSeverity::Medium
    } else {
        default_severity
    }
}

pub fn classify_diagnostic_severity(error: &str) -> DiagnosticSeverity {
    classify_diagnostic_severity_with_default(error, DiagnosticSeverity::High)
}

pub fn diagnostic_priority_label(error: &str) -> &'static str {
    if refact_core::retry_policy::classify_user_error(error)
        == refact_core::retry_policy::UserErrorCategory::StreamCorrupted
    {
        "normal"
    } else {
        "high"
    }
}

pub fn collect_diagnostics_from_error(error: &str) -> DiagnosticContext {
    let severity = classify_diagnostic_severity_with_default(error, DiagnosticSeverity::Low);
    DiagnosticContext {
        error_type: classify_error(error),
        error_message: error.to_string(),
        source_file: None,
        tool_name: None,
        chat_id: None,
        model_id: None,
        collected_at: Utc::now().to_rfc3339(),
        severity,
    }
}

pub fn diagnostic_id(ctx: &DiagnosticContext) -> String {
    format!(
        "{}|{}|{}|{}|{}",
        ctx.collected_at,
        ctx.error_type,
        ctx.source_file.as_deref().unwrap_or(""),
        ctx.tool_name.as_deref().unwrap_or(""),
        ctx.chat_id.as_deref().unwrap_or("")
    )
}

pub fn diagnostic_signature(ctx: &DiagnosticContext) -> String {
    format!(
        "{}|{}|{}|{}|{}",
        ctx.error_type,
        ctx.error_message,
        ctx.source_file.as_deref().unwrap_or(""),
        ctx.tool_name.as_deref().unwrap_or(""),
        ctx.model_id.as_deref().unwrap_or("")
    )
}

pub fn classify_error(error: &str) -> String {
    let lower = error.to_lowercase();
    if lower.contains("timeout") {
        "timeout".to_string()
    } else if lower.contains("permission") {
        "permission".to_string()
    } else if lower.contains("network") || lower.contains("connect") {
        "network".to_string()
    } else if lower.contains("parse") {
        "parse".to_string()
    } else {
        "generic".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_corrupted_maps_to_medium_severity_and_normal_priority() {
        let error = "stream ended unexpectedly while decoding response body";
        assert_eq!(
            refact_core::retry_policy::classify_user_error(error),
            refact_core::retry_policy::UserErrorCategory::StreamCorrupted
        );
        assert_eq!(
            classify_diagnostic_severity(error),
            DiagnosticSeverity::Medium
        );
        assert_eq!(
            collect_diagnostics_from_error(error).severity,
            DiagnosticSeverity::Medium
        );
        assert_eq!(diagnostic_priority_label(error), "normal");
    }

    #[test]
    fn generic_error_preserves_old_default_high_heuristic() {
        assert_eq!(
            classify_diagnostic_severity("some error happened"),
            DiagnosticSeverity::High
        );
        assert_eq!(
            classify_diagnostic_severity("critical panic occurred"),
            DiagnosticSeverity::Critical
        );
        assert_eq!(
            classify_diagnostic_severity("just a warn message"),
            DiagnosticSeverity::Medium
        );
        assert_eq!(
            classify_diagnostic_severity("nothing notable"),
            DiagnosticSeverity::High
        );
        assert_eq!(
            collect_diagnostics_from_error("nothing notable").severity,
            DiagnosticSeverity::Low
        );
        assert_eq!(diagnostic_priority_label("some error happened"), "high");
        assert_eq!(diagnostic_priority_label("just a warn message"), "high");
    }
}
