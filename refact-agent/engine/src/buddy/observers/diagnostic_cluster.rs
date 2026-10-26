use chrono::{DateTime, Utc};
use std::collections::HashMap;

use crate::buddy::diagnostics::DiagnosticContext;
use crate::buddy::observers::{BuddyObserver, ObserverContext};
use crate::buddy::settings::BuddySettings;
use crate::buddy::types::{BuddyFact, BuddyFactKind};
use crate::app_state::AppState;

pub struct DiagnosticClusterObserver;

fn stable_bucket_part(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        "unknown".to_string()
    } else {
        trimmed.chars().take(80).collect()
    }
}

pub(crate) fn diagnostic_source_bucket(diag: &DiagnosticContext) -> String {
    let mut bucket = diag
        .tool_name
        .as_deref()
        .or(diag.source_file.as_deref())
        .map(stable_bucket_part)
        .unwrap_or_else(|| "unknown_source".to_string());
    if let Some(model_id) = diag.model_id.as_deref() {
        bucket.push(':');
        bucket.push_str(&stable_bucket_part(model_id));
    }
    bucket
}

fn is_frontend_diagnostic(ctx: &DiagnosticContext) -> bool {
    if ctx.tool_name.as_deref() == Some("frontend") {
        return true;
    }
    if matches!(
        ctx.error_type.as_str(),
        "window_error"
            | "ui_error_state"
            | "react_error_boundary"
            | "react_root_render"
            | "react_recoverable"
            | "artifact_iframe"
            | "mermaid_render"
            | "unhandledrejection"
            | "possible_renderer_crash"
            | "console_error"
    ) {
        return true;
    }
    ctx.source_file
        .as_deref()
        .is_some_and(|source_file| source_file.starts_with("frontend/"))
}

pub fn detect_diagnostic_cluster_facts(
    diagnostics: &[DiagnosticContext],
    now: DateTime<Utc>,
) -> Vec<BuddyFact> {
    let mut facts = vec![];
    let window_30min = now - chrono::Duration::minutes(30);
    let window_5min = now - chrono::Duration::minutes(5);

    let mut by_type_and_source: HashMap<(String, String), Vec<&DiagnosticContext>> = HashMap::new();
    let mut frontend_diagnostics: Vec<&DiagnosticContext> = Vec::new();

    for diag in diagnostics {
        let Ok(ts) = chrono::DateTime::parse_from_rfc3339(&diag.collected_at) else {
            continue;
        };
        let ts_utc = ts.with_timezone(&Utc);

        if ts_utc >= window_30min {
            by_type_and_source
                .entry((diag.error_type.clone(), diagnostic_source_bucket(diag)))
                .or_default()
                .push(diag);
        }

        if ts_utc >= window_5min && is_frontend_diagnostic(diag) {
            frontend_diagnostics.push(diag);
        }
    }

    for ((error_type, source_bucket), cluster_diagnostics) in &by_type_and_source {
        if cluster_diagnostics.len() >= 3 {
            tracing::debug!(
                "diagnostic_cluster: type={} source={} count={}",
                error_type,
                source_bucket,
                cluster_diagnostics.len()
            );
            let diagnostic_ids: Vec<String> = cluster_diagnostics
                .iter()
                .map(|diag| crate::buddy::diagnostics::diagnostic_id(diag))
                .collect();
            let sample_collected_at = cluster_diagnostics
                .first()
                .map(|diag| diag.collected_at.clone())
                .unwrap_or_default();
            let model_id = cluster_diagnostics
                .iter()
                .find_map(|diag| diag.model_id.clone());
            facts.push(BuddyFact {
                kind: BuddyFactKind::DiagnosticCluster,
                key: format!("diag:cluster:{}:{}", error_type, source_bucket),
                source: "diagnostic_cluster",
                payload: serde_json::json!({
                    "error_type": error_type,
                    "source_bucket": source_bucket,
                    "count": cluster_diagnostics.len(),
                    "window_seconds": 1800,
                    "diagnostic_ids": diagnostic_ids,
                    "sample_collected_at": sample_collected_at,
                    "model_id": model_id,
                }),
                seen_at: now,
                confidence: 0.9,
            });
        }
    }

    if frontend_diagnostics.len() >= 5 {
        tracing::debug!(
            "diagnostic_cluster: frontend burst count={}",
            frontend_diagnostics.len()
        );
        let diagnostic_ids: Vec<String> = frontend_diagnostics
            .iter()
            .map(|diag| crate::buddy::diagnostics::diagnostic_id(diag))
            .collect();
        let sample_collected_at = frontend_diagnostics
            .first()
            .map(|diag| diag.collected_at.clone())
            .unwrap_or_default();
        let model_id = frontend_diagnostics
            .iter()
            .find_map(|diag| diag.model_id.clone());
        facts.push(BuddyFact {
            kind: BuddyFactKind::FrontendErrorBurst,
            key: "diag:fe_burst:global".to_string(),
            source: "diagnostic_cluster",
            payload: serde_json::json!({
                "error_type": "frontend",
                "count": frontend_diagnostics.len(),
                "window_seconds": 300,
                "diagnostic_ids": diagnostic_ids,
                "sample_collected_at": sample_collected_at,
                "model_id": model_id,
            }),
            seen_at: now,
            confidence: 0.95,
        });
    }

    facts
}

#[async_trait::async_trait]
impl BuddyObserver for DiagnosticClusterObserver {
    fn id(&self) -> &'static str {
        "diagnostic_cluster"
    }

    fn cadence_seconds(&self) -> u64 {
        60
    }

    fn emission_refresh_ttl_seconds(&self) -> u64 {
        1800
    }

    fn requires_setting(&self, settings: &BuddySettings) -> bool {
        settings.observers.diagnostic_cluster
    }

    async fn observe(&self, gcx: AppState, ctx: &ObserverContext) -> Vec<BuddyFact> {
        let buddy_arc = gcx.buddy.buddy.clone();
        let lock = buddy_arc.lock().await;
        let diagnostics = match lock.as_ref() {
            Some(svc) => svc.recent_diagnostics.clone(),
            None => return vec![],
        };
        drop(lock);
        detect_diagnostic_cluster_facts(&diagnostics, ctx.now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
}
