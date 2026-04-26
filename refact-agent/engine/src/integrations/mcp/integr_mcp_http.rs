use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Weak;
use async_trait::async_trait;
use tokio::sync::RwLock as ARwLock;
use tokio::sync::Mutex as AMutex;
use tokio::time::Duration;
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
};
use rmcp::transport::common::client_side_sse::ExponentialBackoff;
use rmcp::serve_client;
use serde::{Deserialize, Serialize};

use crate::global_context::GlobalContext;
use crate::integrations::integr_abstract::IntegrationCommon;
use super::session_mcp::{McpClientHandler, McpRunningService};
use super::integr_mcp_common::{
    CommonMCPSettings, MCPTransportInitializer, build_reqwest_client_for_mcp,
    build_auth_client_for_mcp, serve_client_with_timeout, impl_mcp_integration_trait,
};
use super::mcp_auth::{MCPAuthSettings, AuthType};

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
            "Refact.ai (+https://github.com/smallcloudai/refact)".to_string(),
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
    pub gcx_option: Option<Weak<ARwLock<GlobalContext>>>,
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
        let mut retry_config = ExponentialBackoff::default();
        retry_config.max_times = Some(3);
        retry_config.base_duration = Duration::from_millis(500);
        let mut config = StreamableHttpClientTransportConfig::with_uri(self.cfg.mcp_url.trim());
        config.retry_config = Arc::new(retry_config);

        if self.cfg.auth.auth_type == AuthType::Oauth2Pkce {
            let auth_client = build_auth_client_for_mcp(
                self.cfg.mcp_url.trim(),
                &self.cfg.mcp_headers,
                &self.config_path,
                "Streamable HTTP",
                logs.clone(),
                &debug_name,
                session,
            )
            .await?;
            let transport = StreamableHttpClientTransport::with_client(auth_client, config);
            serve_client_with_timeout(
                serve_client(handler, transport),
                init_timeout,
                "Streamable HTTP",
                logs,
                &debug_name,
            )
            .await
        } else {
            let client = build_reqwest_client_for_mcp(
                self.cfg.mcp_url.trim(),
                &self.cfg.mcp_headers,
                &self.cfg.auth,
                "Streamable HTTP",
                logs.clone(),
                &debug_name,
            )
            .await?;
            let transport = StreamableHttpClientTransport::with_client(client, config);
            serve_client_with_timeout(
                serve_client(handler, transport),
                init_timeout,
                "Streamable HTTP",
                logs,
                &debug_name,
            )
            .await
        }
    }
}

impl_mcp_integration_trait!(IntegrationMCPHttp, "mcp_http_schema.yaml");
