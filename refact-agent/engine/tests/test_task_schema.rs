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

#[test]
fn test_old_yaml_backward_compatibility() {
    let old_yaml = r#"
schema_version: 1
id: task-123
name: Test Task
status: planning
created_at: "2024-01-01T00:00:00Z"
updated_at: "2024-01-01T00:00:00Z"
cards_total: 5
cards_done: 2
cards_failed: 0
agents_active: 1
"#;

    let meta: TaskMeta = serde_yaml::from_str(old_yaml).expect("Failed to parse old YAML");
    assert_eq!(meta.id, "task-123");
    assert_eq!(meta.name, "Test Task");
    assert!(meta.base_branch.is_none());
    assert!(meta.base_commit.is_none());
    assert!(meta.default_agent_model.is_none());
}

#[test]
fn test_new_yaml_with_fields() {
    let new_yaml = r#"
schema_version: 1
id: task-456
name: New Task
status: active
created_at: "2024-01-02T00:00:00Z"
updated_at: "2024-01-02T00:00:00Z"
cards_total: 3
cards_done: 1
cards_failed: 0
agents_active: 2
base_branch: main
base_commit: abc123def456
default_agent_model: gpt-4o
last_agents_summary_at: "2024-01-02T00:00:01Z"
"#;

    let meta: TaskMeta = serde_yaml::from_str(new_yaml).expect("Failed to parse new YAML");
    assert_eq!(meta.id, "task-456");
    assert_eq!(meta.base_branch, Some("main".to_string()));
    assert_eq!(meta.base_commit, Some("abc123def456".to_string()));
    assert_eq!(meta.default_agent_model, Some("gpt-4o".to_string()));
    assert_eq!(
        meta.last_agents_summary_at,
        Some("2024-01-02T00:00:01Z".to_string())
    );
}

#[test]
fn test_serialization_skips_none_fields() {
    let meta = TaskMeta {
        schema_version: 1,
        id: "task-789".to_string(),
        name: "Serialize Test".to_string(),
        status: "planning".to_string(),
        created_at: "2024-01-03T00:00:00Z".to_string(),
        updated_at: "2024-01-03T00:00:00Z".to_string(),
        cards_total: 0,
        cards_done: 0,
        cards_failed: 0,
        agents_active: 0,
        base_branch: None,
        base_commit: None,
        default_agent_model: None,
        last_agents_summary_at: None,
    };

    let yaml = serde_yaml::to_string(&meta).expect("Failed to serialize");
    // Verify None fields are not in output
    assert!(!yaml.contains("base_branch"));
    assert!(!yaml.contains("base_commit"));
    assert!(!yaml.contains("default_agent_model"));
    assert!(!yaml.contains("last_agents_summary_at"));
}

#[test]
fn test_serialization_includes_some_fields() {
    let meta = TaskMeta {
        schema_version: 1,
        id: "task-999".to_string(),
        name: "Full Test".to_string(),
        status: "active".to_string(),
        created_at: "2024-01-04T00:00:00Z".to_string(),
        updated_at: "2024-01-04T00:00:00Z".to_string(),
        cards_total: 5,
        cards_done: 2,
        cards_failed: 0,
        agents_active: 1,
        base_branch: Some("develop".to_string()),
        base_commit: Some("xyz789abc123".to_string()),
        default_agent_model: Some("claude-3-opus".to_string()),
        last_agents_summary_at: Some("2024-01-04T00:00:01Z".to_string()),
    };

    let yaml = serde_yaml::to_string(&meta).expect("Failed to serialize");
    assert!(yaml.contains("base_branch: develop"));
    assert!(yaml.contains("base_commit: xyz789abc123"));
    assert!(yaml.contains("default_agent_model: claude-3-opus"));
    assert!(yaml.contains("last_agents_summary_at: 2024-01-04T00:00:01Z"));
}
