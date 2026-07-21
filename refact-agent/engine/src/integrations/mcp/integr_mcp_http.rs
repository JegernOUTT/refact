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
pub struct SettingsMCPHttp {
    #[serde(default, rename = "url")]
    pub mcp_url: String,
    #[serde(default = "default_http_headers", rename = "headers")]
    pub mcp_headers: HashMap<String, String>,
    #[serde(flatten)]
    pub auth: MCPAuthSettings,
    #[serde(flatten)]
    pub common: CommonMCPSettings,
}

pub fn default_http_headers() -> HashMap<String, String> {
    HashMap::from([
        (
            "User-Agent".to_string(),
            "Refact.ai (+https://github.com/JegernOUTT/refact)".to_string(),
        ),
        (
            "Accept".to_string(),
            "application/json, text/event-stream".to_string(),
        ),
        ("Content-Type".to_string(), "application/json".to_string()),
    ])
}

#[derive(Default, Clone)]
pub struct IntegrationMCPHttp {
    pub gcx_option: Option<Weak<GlobalContext>>,
    pub cfg: SettingsMCPHttp,
    pub common: IntegrationCommon,
    pub config_path: String,
}

#[async_trait]
impl MCPTransportInitializer for IntegrationMCPHttp {
    async fn init_mcp_transport(
        &self,
        logs: Arc<AMutex<Vec<String>>>,
        debug_name: String,
        init_timeout: u64,
        _request_timeout: u64,
        session: Arc<AMutex<Box<dyn crate::integrations::sessions::IntegrationSession>>>,
        handler: McpClientHandler,
    ) -> Option<McpRunningService> {
        // Both attempts share one init_timeout budget so a slow primary attempt
        // cannot double the configured startup deadline.
        let started = tokio::time::Instant::now();
        if let Some(client) = serve_streamable_http_transport(
            self.cfg.mcp_url.clone(),
            self.cfg.mcp_headers.clone(),
            self.cfg.auth.clone(),
            self.config_path.clone(),
            "Streamable HTTP",
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
                "Streamable HTTP connection failed; no time budget left for the legacy SSE fallback"
                    .to_string();
            tracing::warn!("{msg} for {debug_name}");
            add_log_entry(logs, msg).await;
            return None;
        }

        let msg = "Streamable HTTP connection failed; trying legacy SSE fallback".to_string();
        tracing::warn!("{msg} for {debug_name}");
        add_log_entry(logs.clone(), msg).await;
        serve_legacy_sse_transport(
            self.cfg.mcp_url.clone(),
            self.cfg.mcp_headers.clone(),
            self.cfg.auth.clone(),
            self.config_path.clone(),
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

impl_mcp_integration_trait!(IntegrationMCPHttp, "mcp_http_schema.yaml");
