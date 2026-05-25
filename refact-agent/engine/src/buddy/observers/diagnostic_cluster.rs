use std::collections::HashMap;
use chrono::{DateTime, Utc};

use crate::buddy::diagnostics::DiagnosticContext;
use crate::buddy::observers::{BuddyObserver, ObserverContext};
use crate::buddy::settings::BuddySettings;
use crate::buddy::types::{BuddyFact, BuddyFactKind};
use crate::app_state::AppState;

pub struct DiagnosticClusterObserver;

pub fn detect_diagnostic_cluster_facts(
    diagnostics: &[DiagnosticContext],
    now: DateTime<Utc>,
) -> Vec<BuddyFact> {
    let mut facts = vec![];
    let window_30min = now - chrono::Duration::minutes(30);
    let window_5min = now - chrono::Duration::minutes(5);

    let mut by_type: HashMap<&str, Vec<&DiagnosticContext>> = HashMap::new();
    let mut frontend_diagnostics: Vec<&DiagnosticContext> = Vec::new();

    for diag in diagnostics {
        let Ok(ts) = chrono::DateTime::parse_from_rfc3339(&diag.collected_at) else {
            continue;
        };
        let ts_utc = ts.with_timezone(&Utc);

        if ts_utc >= window_30min {
            by_type
                .entry(diag.error_type.as_str())
                .or_default()
                .push(diag);
        }

        if ts_utc >= window_5min && diag.tool_name.as_deref() == Some("frontend") {
            frontend_diagnostics.push(diag);
        }
    }

    for (error_type, cluster_diagnostics) in &by_type {
        if cluster_diagnostics.len() >= 3 {
            tracing::debug!(
                "diagnostic_cluster: type={} count={}",
                error_type,
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
            facts.push(BuddyFact {
                kind: BuddyFactKind::DiagnosticCluster,
                key: format!("diag:cluster:{}", error_type),
                source: "diagnostic_cluster",
                payload: serde_json::json!({
                    "error_type": error_type,
                    "count": cluster_diagnostics.len(),
                    "window_seconds": 1800,
                    "diagnostic_ids": diagnostic_ids,
                    "sample_collected_at": sample_collected_at,
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
