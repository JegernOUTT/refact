use super::*;
use crate::app_state::AppState;
use crate::chat::types::TaskMeta as ThreadTaskMeta;
use crate::tasks::types::{TaskMeta, TaskStatus};
use crate::tools::tools_description::Tool;
use serde_json::json;

fn args() -> HashMap<String, Value> {
    HashMap::new()
}

fn task_meta() -> TaskMeta {
    TaskMeta {
        schema_version: 1,
        id: "task-1".to_string(),
        name: "Memory Task".to_string(),
        status: TaskStatus::Active,
        created_at: "2026-05-22T00:00:00Z".to_string(),
        updated_at: "2026-05-22T00:00:00Z".to_string(),
        cards_total: 0,
        cards_done: 0,
        cards_failed: 0,
        agents_active: 0,
        base_branch: None,
        base_commit: None,
        default_agent_model: None,
        is_name_generated: false,
        last_agents_summary_at: None,
        planner_session_state: None,
    }
}

struct Fixture {
    _temp: tempfile::TempDir,
    gcx: Arc<GlobalContext>,
    task_dir: PathBuf,
}

async fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx().await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
    let task_dir = temp.path().join(".refact/tasks/task-1");
    tokio::fs::create_dir_all(&task_dir).await.unwrap();
    storage::save_task_meta(gcx.clone(), "task-1", &task_meta())
        .await
        .unwrap();
    storage::save_board(gcx.clone(), "task-1", &TaskBoard::default())
        .await
        .unwrap();
    Fixture {
        _temp: temp,
        gcx,
        task_dir,
    }
}

async fn planner_ccx(gcx: Arc<GlobalContext>) -> Arc<AMutex<AtCommandsContext>> {
    let app = AppState::from_gcx(gcx).await;
    Arc::new(AMutex::new(
        AtCommandsContext::new_from_app(
            app,
            200_000,
            20,
            false,
            vec![],
            "planner-chat".to_string(),
            None,
            "model".to_string(),
            Some(ThreadTaskMeta {
                task_id: "task-1".to_string(),
                role: "planner".to_string(),
                agent_id: None,
                card_id: None,
                planner_chat_id: None,
            }),
            None,
        )
        .await,
    ))
}

async fn write_memory(path: &Path, frontmatter: &str) {
    tokio::fs::create_dir_all(path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(
        path,
        format!(
            "---\ncreated_at: {}\n{}\n---\n\n# {}\nMemory body",
            Utc::now().to_rfc3339(),
            frontmatter,
            yaml_title(frontmatter)
        ),
    )
    .await
    .unwrap();
}

fn yaml_title(frontmatter: &str) -> &str {
    frontmatter
        .lines()
        .find_map(|line| line.strip_prefix("title: "))
        .unwrap_or("Memory")
}

async fn tool_output(gcx: Arc<GlobalContext>) -> String {
    let ccx = planner_ccx(gcx).await;
    let (_, contexts) = ToolTaskOverview::new()
        .tool_execute(ccx, &"call".to_string(), &args())
        .await
        .unwrap();
    match &contexts[0] {
        ContextEnum::ChatMessage(message) => match &message.content {
            ChatContent::SimpleText(text) => text.clone(),
            _ => panic!("expected text output"),
        },
        _ => panic!("expected chat message"),
    }
}

async fn write_shared_and_private(fixture: &Fixture) {
    let memories = fixture.task_dir.join("memories");
    write_memory(
        &memories.join("shared.md"),
        "title: Shared Note\nkind: decision\ntags: [visible]\nnamespace: task\npinned: true",
    )
    .await;
    write_memory(
        &memories.join("private.md"),
        "title: Agent Secret Title\nkind: risk\ntags: [private-tag]\nnamespace: agent:A-1\npinned: true",
    )
    .await;
}

async fn enable_supervisor_override(project_root: &Path) {
    let refact_dir = project_root.join(".refact");
    tokio::fs::create_dir_all(&refact_dir).await.unwrap();
    tokio::fs::write(
        refact_dir.join("coordination.yaml"),
        "memory_supervisor_override: true\n",
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn planner_memory_pulse_hides_agent_memory_from_all_output_and_counts() {
    let fixture = fixture().await;
    write_shared_and_private(&fixture).await;

    let output = tool_output(fixture.gcx.clone()).await;

    assert!(output.contains("1 memories total (1 pinned, 0 archived, 1 active)"));
    assert!(output.contains("Recent kinds: decision=1"));
    assert!(output.contains("Top tags: visible=1"));
    assert!(output.contains("memory created: Shared Note"));
    assert!(!output.contains("Agent Secret Title"));
    assert!(!output.contains("private-tag"));
    assert!(!output.contains("memory created: Agent Secret Title"));
}

#[tokio::test]
#[serial_test::serial(runtime_settings)]
async fn planner_memory_pulse_supervisor_override_includes_and_audits_agent_memory() {
    crate::runtime_settings::reset_for_test();
    let fixture = fixture().await;
    write_shared_and_private(&fixture).await;
    enable_supervisor_override(fixture._temp.path()).await;

    let output = tool_output(fixture.gcx.clone()).await;

    assert!(output.contains("2 memories total (2 pinned, 0 archived, 2 active)"));
    assert!(output.contains("decision=1, risk=1") || output.contains("risk=1, decision=1"));
    assert!(output.contains("private-tag=1"));
    assert!(output.contains("memory created: Agent Secret Title"));
    let audit = tokio::fs::read_to_string(fixture.task_dir.join(".mem_access_audit.jsonl"))
        .await
        .unwrap();
    let lines = audit.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 1);
    let record: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(record["surface"], json!("task_overview"));
    assert_eq!(record["grant"], json!("supervisor_override"));
    assert_eq!(record["memory"], json!("private.md"));
    crate::runtime_settings::reset_for_test();
}

#[tokio::test]
async fn planner_memory_pulse_hides_archived_agent_memory() {
    let fixture = fixture().await;
    let memories = fixture.task_dir.join("memories");
    write_memory(
        &memories.join("shared.md"),
        "title: Shared Note\nkind: decision\ntags: [visible]\nnamespace: task",
    )
    .await;
    write_memory(
        &memories.join("archived/private.md"),
        "title: Archived Agent Secret\nkind: risk\ntags: [archived-private-tag]\nnamespace: agent:A-1",
    )
    .await;

    let output = tool_output(fixture.gcx).await;

    assert!(output.contains("1 memories total (0 pinned, 0 archived, 1 active)"));
    assert!(!output.contains("Archived Agent Secret"));
    assert!(!output.contains("archived-private-tag"));
}
