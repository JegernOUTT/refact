use serde::Deserialize;
use serde::Serialize;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;

use crate::global_context::GlobalContext;

pub use refact_tool_api::IntegrationConfirmation;

#[async_trait]
pub trait IntegrationTrait: Send + Sync {
    fn integr_schema(&self) -> &str;
    async fn integr_settings_apply(
        &mut self,
        gcx: Arc<GlobalContext>,
        config_path: String,
        value: &serde_json::Value,
    ) -> Result<(), serde_json::Error>;
    fn integr_settings_as_json(&self) -> serde_json::Value;
    fn integr_common(&self) -> IntegrationCommon;
    async fn integr_tools(
        &self,
        integr_name: &str,
    ) -> Vec<Box<dyn crate::tools::tools_description::Tool + Send>>; // integr_name is sometimes different, "cmdline_compile_my_project" != "cmdline"
}

#[derive(Deserialize, Serialize, Clone, Default)]
pub struct IntegrationAvailable {
    #[serde(default = "default_true")]
    pub on_your_laptop: bool,
    #[serde(default = "default_true")]
    pub when_isolated: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize, Serialize, Clone, Default)]
pub struct IntegrationCommon {
    #[serde(default)]
    pub available: IntegrationAvailable,
    #[serde(default)]
    pub confirmation: IntegrationConfirmation,
}
