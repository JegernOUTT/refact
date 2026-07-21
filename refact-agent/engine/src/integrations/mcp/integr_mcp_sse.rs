use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Weak;
use async_trait::async_trait;
use tokio::sync::Mutex as AMutex;
use serde::{Deserialize, Serialize};

use crate::global_context::GlobalContext;
use crate::integrations::integr_abstract::IntegrationCommon;
use super::session_mcp::{McpClientHandler, McpRunningService, add_log_entry};
use super::integr_mcp_common::{
    CommonMCPSettings, MCPTransportInitializer, impl_mcp_integration_trait,
    serve_legacy_sse_transport, serve_streamable_http_transport,
};
use super::mcp_auth::MCPAuthSettings;

#[derive(Deserialize, Serialize, Clone, PartialEq, Default, Debug)]
pub struct SettingsMCPSse {
    #[serde(default, rename = "url")]
    pub mcp_url: String,
    #[serde(default = "default_headers", rename = "headers")]
    pub mcp_headers: HashMap<String, String>,
    #[serde(flatten)]
    pub auth: MCPAuthSettings,
    #[serde(flatten)]
    pub common: CommonMCPSettings,
}

pub fn default_headers() -> HashMap<String, String> {
    HashMap::from([
        (
            "User-Agent".to_string(),
            "Refact.ai (+https://github.com/JegernOUTT/refact)".to_string(),
        ),
        ("Accept".to_string(), "text/event-stream".to_string()),
        ("Content-Type".to_string(), "application/json".to_string()),
    ])
}

#[derive(Default, Clone)]
pub struct IntegrationMCPSse {
    pub gcx_option: Option<Weak<GlobalContext>>,
    pub cfg: SettingsMCPSse,
    pub common: IntegrationCommon,
    pub config_path: String,
}

#[async_trait]
impl MCPTransportInitializer for IntegrationMCPSse {
    async fn init_mcp_transport(
        &self,
        logs: Arc<AMutex<Vec<String>>>,
        debug_name: String,
        init_timeout: u64,
        _request_timeout: u64,
        session: Arc<AMutex<Box<dyn crate::integrations::sessions::IntegrationSession>>>,
        handler: McpClientHandler,
    ) -> Option<McpRunningService> {
        // The mcp_sse_* config type is the legacy HTTP+SSE transport
        // (2024-11-05 protocol revision), so speak real SSE first. Both
        // attempts share one init_timeout budget.
        let started = tokio::time::Instant::now();
        if let Some(client) = serve_legacy_sse_transport(
            self.cfg.mcp_url.clone(),
            self.cfg.mcp_headers.clone(),
            self.cfg.auth.clone(),
            self.config_path.clone(),
            init_timeout,
            logs.clone(),
            debug_name.clone(),
            session.clone(),
            handler.clone(),
        )
        .await
        {
            return Some(client);
        }

        let remaining = init_timeout.saturating_sub(started.elapsed().as_secs());
        if remaining < 5 {
            let msg =
                "Legacy SSE connection failed; no time budget left for the Streamable HTTP fallback"
                    .to_string();
            tracing::warn!("{msg} for {debug_name}");
            add_log_entry(logs, msg).await;
            return None;
        }

        // The server may in fact be a modern Streamable HTTP server that was
        // configured as SSE; fall back before giving up.
        let msg = "Legacy SSE connection failed; trying Streamable HTTP fallback".to_string();
        tracing::warn!("{msg} for {debug_name}");
        add_log_entry(logs.clone(), msg).await;
        serve_streamable_http_transport(
            self.cfg.mcp_url.clone(),
            self.cfg.mcp_headers.clone(),
            self.cfg.auth.clone(),
            self.config_path.clone(),
            "Streamable HTTP",
            remaining,
            logs,
            debug_name,
            session,
            handler,
        )
        .await
    }

    fn remote_probe_info(&self) -> Option<(String, HashMap<String, String>, MCPAuthSettings)> {
        let url = self.cfg.mcp_url.trim().to_string();
        if url.is_empty() {
            return None;
        }
        Some((url, self.cfg.mcp_headers.clone(), self.cfg.auth.clone()))
    }
}

impl_mcp_integration_trait!(IntegrationMCPSse, "mcp_sse_schema.yaml");
