use std::sync::Arc;
use chrono::{DateTime, Utc};
use tokio::sync::RwLock;

use crate::buddy::observers::{BuddyObserver, ObserverContext};
use crate::buddy::settings::BuddySettings;
use crate::buddy::types::{BuddyFact, BuddyFactKind};
use crate::global_context::GlobalContext;
use crate::integrations::mcp::session_mcp::{MCPAuthStatus, SessionMCP};

const FAILURE_THRESHOLD: u64 = 3;

pub struct McpAuthObserver;

pub struct McpSessionSnapshot {
    pub id: String,
    pub auth_status: MCPAuthStatus,
    pub failed_calls: u64,
    pub expires_at_ms: Option<i64>,
}

pub fn detect_mcp_auth_facts(snaps: &[McpSessionSnapshot], now: DateTime<Utc>) -> Vec<BuddyFact> {
    let mut facts = vec![];
    let now_ms = now.timestamp_millis();
    let window_ms = 24 * 3600 * 1000i64;

    for snap in snaps {
        let token_expiring = snap
            .expires_at_ms
            .map(|exp| exp > 0 && now_ms + window_ms >= exp)
            .unwrap_or(false);
        let needs_auth = matches!(
            snap.auth_status,
            MCPAuthStatus::NeedsLogin | MCPAuthStatus::NeedsReauth | MCPAuthStatus::Error(_)
        );
        if token_expiring || needs_auth {
            let expires_iso = snap
                .expires_at_ms
                .and_then(|ms| DateTime::from_timestamp(ms / 1000, 0))
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default();
            facts.push(BuddyFact {
                kind: BuddyFactKind::McpAuthExpired,
                key: format!("mcp:auth_expiring:{}", snap.id),
                source: "mcp_auth",
                payload: serde_json::json!({
                    "mcp_id": snap.id,
                    "expires_at": expires_iso,
                    "failure_count": snap.failed_calls,
                }),
                seen_at: now,
                confidence: 0.95,
            });
        }
        if snap.failed_calls >= FAILURE_THRESHOLD {
            facts.push(BuddyFact {
                kind: BuddyFactKind::IntegrationFailing,
                key: format!("integration:failing:{}", snap.id),
                source: "mcp_auth",
                payload: serde_json::json!({
                    "mcp_id": snap.id,
                    "failure_count": snap.failed_calls,
                }),
                seen_at: now,
                confidence: 0.85,
            });
        }
    }
    facts
}

#[async_trait::async_trait]
impl BuddyObserver for McpAuthObserver {
    fn id(&self) -> &'static str {
        "mcp_auth"
    }

    fn cadence_seconds(&self) -> u64 {
        600
    }

    fn requires_setting(&self, settings: &BuddySettings) -> bool {
        settings.observers.mcp_auth && settings.proactive_enabled
    }

    async fn observe(
        &self,
        gcx: Arc<RwLock<GlobalContext>>,
        _ctx: &ObserverContext,
    ) -> Vec<BuddyFact> {
        let session_entries = {
            let gcx_read = gcx.read().await;
            gcx_read
                .integration_sessions
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect::<Vec<_>>()
        };
        let now = Utc::now();
        let mut snaps = vec![];
        for (key, session_arc) in session_entries {
            let (auth_status, metrics_arc, config_path) = {
                let mut session_locked = session_arc.lock().await;
                let mcp = match session_locked.as_any_mut().downcast_mut::<SessionMCP>() {
                    Some(s) => s,
                    None => continue,
                };
                (
                    mcp.auth_status.clone(),
                    mcp.metrics.clone(),
                    mcp.config_path.clone(),
                )
            };
            let failed_calls = metrics_arc.lock().await.metrics.failed_calls;
            let expires_at_ms =
                crate::integrations::mcp::mcp_auth::load_tokens_from_config(&config_path)
                    .await
                    .filter(|t| t.expires_at > 0)
                    .map(|t| t.expires_at);
            snaps.push(McpSessionSnapshot {
                id: key,
                auth_status,
                failed_calls,
                expires_at_ms,
            });
        }
        detect_mcp_auth_facts(&snaps, now)
    }
}
