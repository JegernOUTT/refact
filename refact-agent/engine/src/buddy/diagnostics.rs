use std::sync::Arc;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock as ARwLock;

use crate::global_context::GlobalContext;

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

pub async fn collect_diagnostics(
    _gcx: Arc<ARwLock<GlobalContext>>,
    error: &str,
) -> DiagnosticContext {
    let severity = if error.contains("critical") || error.contains("panic") {
        DiagnosticSeverity::Critical
    } else if error.contains("error") || error.contains("Error") {
        DiagnosticSeverity::High
    } else if error.contains("warn") || error.contains("Warn") {
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

fn classify_error(error: &str) -> String {
    if error.contains("timeout") || error.contains("Timeout") {
        "timeout".to_string()
    } else if error.contains("permission") || error.contains("Permission") {
        "permission".to_string()
    } else if error.contains("network") || error.contains("connect") {
        "network".to_string()
    } else if error.contains("parse") || error.contains("Parse") {
        "parse".to_string()
    } else {
        "generic".to_string()
    }
}
