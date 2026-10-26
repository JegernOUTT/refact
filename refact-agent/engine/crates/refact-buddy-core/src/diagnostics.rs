use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const DIAGNOSTICS_FRESHNESS_SECS: i64 = 6 * 3600;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrences: Option<u32>,
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
        occurrences: None,
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

pub fn unavailable_model_id(message: &str) -> Option<String> {
    if !message.contains("not found") {
        return None;
    }
    let start = message.find("Model '")? + "Model '".len();
    let end = message[start..].find('\'')? + start;
    Some(message[start..end].to_string())
}

pub fn collapse_diagnostics(items: Vec<DiagnosticContext>) -> Vec<DiagnosticContext> {
    let mut signatures: HashMap<String, (usize, u32)> = HashMap::new();
    for (index, ctx) in items.iter().enumerate() {
        let entry = signatures
            .entry(diagnostic_signature(ctx))
            .or_insert((index, 0));
        entry.0 = index;
        entry.1 += 1;
    }

    items
        .into_iter()
        .enumerate()
        .filter_map(|(index, mut ctx)| {
            let (newest_index, count) = signatures.get(&diagnostic_signature(&ctx))?;
            if index != *newest_index {
                return None;
            }
            ctx.occurrences = (*count > 1).then_some(*count);
            Some(ctx)
        })
        .collect()
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

    fn diagnostic(message: &str, collected_at: &str) -> DiagnosticContext {
        DiagnosticContext {
            error_type: "generic".to_string(),
            error_message: message.to_string(),
            source_file: None,
            tool_name: None,
            chat_id: None,
            model_id: None,
            collected_at: collected_at.to_string(),
            severity: DiagnosticSeverity::High,
            occurrences: None,
        }
    }

    #[test]
    fn collapse_diagnostics_keeps_newest_and_counts_occurrences() {
        let collapsed = collapse_diagnostics(vec![
            diagnostic("same", "first"),
            diagnostic("same", "second"),
            diagnostic("same", "third"),
        ]);
        assert_eq!(collapsed.len(), 1);
        assert_eq!(collapsed[0].collected_at, "third");
        assert_eq!(collapsed[0].occurrences, Some(3));
    }

    #[test]
    fn unavailable_model_id_extracts_only_model_not_found_errors() {
        assert_eq!(
            unavailable_model_id(
                "Model 'claude_code/claude-opus-5' not found. \
                 Server has the following models: []"
            ),
            Some("claude_code/claude-opus-5".to_string())
        );
        assert_eq!(unavailable_model_id("unrelated network error"), None);
    }
}
