use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Weak;
use std::process::Stdio;
use async_trait::async_trait;
use tokio::sync::Mutex as AMutex;
use tokio::time::timeout;
use tokio::time::Duration;
use rmcp::serve_client;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::global_context::GlobalContext;
use crate::integrations::integr_abstract::{IntegrationTrait, IntegrationCommon};
use super::session_mcp::{McpClientHandler, McpRunningService, SessionMCP, add_log_entry};
use super::mcp_path_resolution;
use super::integr_mcp_common::{CommonMCPSettings, MCPTransportInitializer, impl_mcp_integration_trait};

#[derive(Deserialize, Serialize, Clone, PartialEq, Default, Debug)]
pub struct SettingsMCPStdio {
    #[serde(rename = "command", default)]
    pub mcp_command: String,
    #[serde(default, rename = "env")]
    pub mcp_env: HashMap<String, String>,
    #[serde(flatten)]
    pub common: CommonMCPSettings,
}

#[derive(Default, Clone)]
pub struct IntegrationMCPStdio {
    pub gcx_option: Option<Weak<GlobalContext>>,
    pub cfg: SettingsMCPStdio,
    pub common: IntegrationCommon,
    pub config_path: String,
}

#[async_trait]
impl MCPTransportInitializer for IntegrationMCPStdio {
    async fn init_mcp_transport(
        &self,
        logs: Arc<AMutex<Vec<String>>>,
        debug_name: String,
        init_timeout: u64,
        _request_timeout: u64,
        session_arc_clone: Arc<AMutex<Box<dyn crate::integrations::sessions::IntegrationSession>>>,
        handler: McpClientHandler,
    ) -> Option<McpRunningService> {
        let log = async |level: tracing::Level, msg: String| {
            match level {
                tracing::Level::ERROR => tracing::error!("{msg} for {debug_name}"),
                tracing::Level::WARN => tracing::warn!("{msg} for {debug_name}"),
                _ => tracing::info!("{msg} for {debug_name}"),
            }
            add_log_entry(logs.clone(), msg).await;
        };

        let command = self.cfg.mcp_command.trim();
        if command.is_empty() {
            log(
                tracing::Level::ERROR,
                "Command is empty for STDIO transport".to_string(),
            )
            .await;
            return None;
        }

        let parsed_args = match shell_words::split(&command) {
            Ok(args) => {
                if args.is_empty() {
                    log(tracing::Level::ERROR, "Empty command".to_string()).await;
                    return None;
                }
                args
            }
            Err(e) => {
                log(
                    tracing::Level::ERROR,
                    format!("Failed to parse command: {}", e),
                )
                .await;
                return None;
            }
        };

        let resolved = match mcp_path_resolution::resolve_command(
            &parsed_args[0],
            command,
            self.cfg.mcp_env.get("PATH").map(|s| s.as_str()),
        ) {
            Ok(r) => r,
            Err(e) => {
                log(tracing::Level::ERROR, e.to_user_message()).await;
                return None;
            }
        };

        let mut command = tokio::process::Command::new(&resolved.program);
        command.args(&parsed_args[1..]);
        command.env("PATH", &resolved.effective_path);
        for (key, value) in &self.cfg.mcp_env {
            command.env(key, value);
        }

        match NamedTempFile::new().map(|f| f.keep()) {
            Ok(Ok((file, path))) => {
                {
                    let mut session_locked = session_arc_clone.lock().await;
                    if let Some(mcp_session) = session_locked
                        .as_any_mut()
                        .downcast_mut::<super::session_mcp::SessionMCP>()
                    {
                        mcp_session.stderr_file_path = Some(path.clone());
                        mcp_session.stderr_cursor = Arc::new(AMutex::new(0));
                    }
                }
                command.stderr(Stdio::from(file));
            }
            Ok(Err(e)) => tracing::error!("Failed to persist stderr file for {debug_name}: {e}"),
            Err(e) => tracing::error!("Failed to create stderr file for {debug_name}: {e}"),
        }

        #[cfg(target_os = "linux")]
        let observation_setup = refact_exec::ObservationSetup::prepare(&mut command);
        let transport = match rmcp::transport::TokioChildProcess::new(command) {
            Ok(t) => t,
            Err(e) => {
                log(
                    tracing::Level::ERROR,
                    format!(
                        "Failed to start MCP server process '{}': {}. Resolved binary: {}",
                        &parsed_args[0],
                        e,
                        resolved.program.display()
                    ),
                )
                .await;
                return None;
            }
        };

        #[cfg(target_os = "linux")]
        {
            let process_id = transport.id();
            let observation = observation_setup.start_with_external_reaper(process_id);
            let mut session_locked = session_arc_clone.lock().await;
            if let Some(session) = session_locked.as_any_mut().downcast_mut::<SessionMCP>() {
                if let Some(process_id) = process_id {
                    session.metrics.lock().await.set_pid(process_id);
                }
                session.observation = Some(observation.reader());
            }
        }

        match timeout(
            Duration::from_secs(init_timeout),
            serve_client(handler, transport),
        )
        .await
        {
            Ok(Ok(client)) => Some(client),
            Ok(Err(e)) => {
                log(
                    tracing::Level::ERROR,
                    format!("Failed to init stdio server: {}", e),
                )
                .await;
                None
            }
            Err(_) => {
                log(
                    tracing::Level::ERROR,
                    format!("Request timed out after {} seconds", init_timeout),
                )
                .await;
                None
            }
        }
    }
}

impl_mcp_integration_trait!(IntegrationMCPStdio, "mcp_stdio_schema.yaml");

#[derive(Clone)]
pub enum UnifiedMCPInner {
    Stdio(IntegrationMCPStdio),
    Http(super::integr_mcp_http::IntegrationMCPHttp),
}

impl Default for UnifiedMCPInner {
    fn default() -> Self {
        UnifiedMCPInner::Stdio(IntegrationMCPStdio::default())
    }
}

/// Decides which transport a unified-schema config should use.
/// `url` selects the remote (Streamable HTTP with legacy SSE fallback)
/// transport, `command` selects stdio; setting both is an error.
pub fn unified_transport_choice(value: &serde_json::Value) -> Result<&'static str, String> {
    let url = value
        .get("url")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("");
    let command = value
        .get("command")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("");
    if !url.is_empty() && !command.is_empty() {
        return Err(
            "provide either `command` (local stdio server) or `url` (remote server), not both"
                .to_string(),
        );
    }
    if !url.is_empty() {
        Ok("http")
    } else {
        Ok("stdio")
    }
}

#[derive(Default, Clone)]
pub struct IntegrationMCPUnified {
    pub inner: UnifiedMCPInner,
}

#[async_trait]
impl IntegrationTrait for IntegrationMCPUnified {
    async fn integr_settings_apply(
        &mut self,
        gcx: Arc<GlobalContext>,
        config_path: String,
        value: &serde_json::Value,
    ) -> Result<(), serde_json::Error> {
        use serde::de::Error as _;
        let choice = unified_transport_choice(value).map_err(serde_json::Error::custom)?;
        if choice == "http" {
            let mut http = super::integr_mcp_http::IntegrationMCPHttp::default();
            http.integr_settings_apply(gcx, config_path, value).await?;
            self.inner = UnifiedMCPInner::Http(http);
        } else {
            let mut stdio = IntegrationMCPStdio::default();
            stdio.integr_settings_apply(gcx, config_path, value).await?;
            self.inner = UnifiedMCPInner::Stdio(stdio);
        }
        Ok(())
    }

    fn integr_settings_as_json(&self) -> serde_json::Value {
        match &self.inner {
            UnifiedMCPInner::Stdio(integration) => integration.integr_settings_as_json(),
            UnifiedMCPInner::Http(integration) => integration.integr_settings_as_json(),
        }
    }

    fn integr_common(&self) -> crate::integrations::integr_abstract::IntegrationCommon {
        match &self.inner {
            UnifiedMCPInner::Stdio(integration) => integration.integr_common(),
            UnifiedMCPInner::Http(integration) => integration.integr_common(),
        }
    }

    async fn integr_tools(
        &self,
        integr_name: &str,
    ) -> Vec<Box<dyn crate::tools::tools_description::Tool + Send>> {
        match &self.inner {
            UnifiedMCPInner::Stdio(integration) => integration.integr_tools(integr_name).await,
            UnifiedMCPInner::Http(integration) => integration.integr_tools(integr_name).await,
        }
    }

    fn integr_schema(&self) -> &str {
        include_str!("mcp_unified_schema.yaml")
    }
}

#[cfg(test)]
mod unified_tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn integr_stdio_unavailable_observer_does_not_block_spawn() {
        let mut command = tokio::process::Command::new("sh");
        command.arg("-c").arg("exit 0");
        let setup = refact_exec::ObservationSetup::unavailable("ptrace denied");

        let mut child = command.spawn().unwrap();
        let observation = setup.start_with_external_reaper(child.id());
        let status = child.wait().await.unwrap();

        assert!(status.success());
        assert_eq!(
            observation.reader().status(),
            refact_exec::ObservationStatus::Unavailable("ptrace denied".to_string())
        );
    }

    #[test]
    fn test_unified_transport_choice_url_selects_http() {
        let value = serde_json::json!({"url": "https://mcp.example.com/mcp", "command": ""});
        assert_eq!(unified_transport_choice(&value).unwrap(), "http");
        let value = serde_json::json!({"url": "  https://mcp.example.com/mcp  "});
        assert_eq!(unified_transport_choice(&value).unwrap(), "http");
    }

    #[test]
    fn test_unified_transport_choice_command_selects_stdio() {
        let value = serde_json::json!({"command": "npx -y @org/mcp-server", "url": ""});
        assert_eq!(unified_transport_choice(&value).unwrap(), "stdio");
    }

    #[test]
    fn test_unified_transport_choice_empty_defaults_to_stdio() {
        let value = serde_json::json!({});
        assert_eq!(unified_transport_choice(&value).unwrap(), "stdio");
    }

    #[test]
    fn test_unified_transport_choice_rejects_both() {
        let value = serde_json::json!({
            "command": "npx -y @org/mcp-server",
            "url": "https://mcp.example.com/mcp"
        });
        assert!(unified_transport_choice(&value).is_err());
    }
}
