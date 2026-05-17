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
    pub collected_at: String,
    pub severity: DiagnosticSeverity,
}

pub fn collect_diagnostics_from_error(error: &str) -> DiagnosticContext {
    let lower = error.to_lowercase();
    let severity = if lower.contains("critical") || lower.contains("panic") {
        DiagnosticSeverity::Critical
    } else if lower.contains("error") {
        DiagnosticSeverity::High
    } else if lower.contains("warn") {
        DiagnosticSeverity::Medium
    } else {
        DiagnosticSeverity::Low
    };
    DiagnosticContext {
        error_type: classify_error(error),
        error_message: error.to_string(),
        source_file: None,
        tool_name: None,
        chat_id: None,
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
        "{}|{}|{}|{}",
        ctx.error_type,
        ctx.error_message,
        ctx.source_file.as_deref().unwrap_or(""),
        ctx.tool_name.as_deref().unwrap_or("")
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
