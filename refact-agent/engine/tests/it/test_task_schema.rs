#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskMeta {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub cards_total: usize,
    #[serde(default)]
    pub cards_done: usize,
    #[serde(default)]
    pub cards_failed: usize,
    #[serde(default)]
    pub agents_active: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_agent_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_agents_summary_at: Option<String>,
}

fn default_schema_version() -> u32 {
    1
}
