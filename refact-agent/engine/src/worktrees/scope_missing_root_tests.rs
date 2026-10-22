use super::scope::ExecutionScope;
use super::types::WorktreeMeta;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ContextEnum};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::process::Command;
use tokio::sync::Mutex as AMutex;

struct Fixture {
    _temp: tempfile::TempDir,
    source: PathBuf,
    root: PathBuf,
    worktree: WorktreeMeta,
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let root = temp.path().join("worktree");
    fs::create_dir_all(&source).unwrap();
    git(&source, &["init", "-q"]);
    git(&source, &["config", "user.email", "l4@example.invalid"]);
    git(&source, &["config", "user.name", "L4 test"]);
    fs::create_dir_all(source.join("src")).unwrap();
    fs::write(source.join("src/lib.rs"), "source\n").unwrap();
    git(&source, &["add", "src/lib.rs"]);
    git(&source, &["commit", "-q", "-m", "fixture"]);
    git(
        &source,
        &[
            "worktree",
            "add",
            "--detach",
            root.to_str().unwrap(),
            "HEAD",
        ],
    );
    fs::create_dir_all(source.join(".refact/tasks/task/memories")).unwrap();
    let source = fs::canonicalize(source).unwrap();
    let root = fs::canonicalize(root).unwrap();
    Fixture {
        _temp: temp,
        source: source.clone(),
        root: root.clone(),
        worktree: WorktreeMeta {
            id: "wt-l4".to_string(),
            kind: "chat".to_string(),
            root,
            source_workspace_root: source.clone(),
            repo_root: source,
            branch: Some("feature".to_string()),
            base_branch: Some("main".to_string()),
            base_commit: None,
            task_id: None,
            card_id: None,
            agent_id: None,
            enforce: false,
        },
    }
}

async fn ccx(fixture: &Fixture, enforce: bool) -> Arc<AMutex<AtCommandsContext>> {
    let gcx = crate::global_context::tests::make_test_gcx().await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![fixture.source.clone()];
    *gcx.privacy_settings.write().unwrap() = Arc::new(crate::privacy::PrivacySettings {
        privacy_rules: crate::privacy::FilePrivacySettings {
            only_send_to_servers_I_control: Vec::new(),
            blocked: Vec::new(),
        },
        loaded_ts: u64::MAX / 2,
    });
    Arc::new(AMutex::new(
        AtCommandsContext::new_from_app(
            crate::app_state::AppState::from_gcx(gcx).await,
            4096,
            20,
            false,
            Vec::new(),
            "chat-l4".to_string(),
            None,
            "model".to_string(),
            None,
            Some(WorktreeMeta {
                enforce,
                ..fixture.worktree.clone()
            }),
        )
        .await,
    ))
}

async fn execute_tool(
    ccx: Arc<AMutex<AtCommandsContext>>,
    tool_call_id: &str,
    tool_name: &str,
    args: HashMap<String, Value>,
) -> Result<Vec<ContextEnum>, String> {
    let registry = ccx.lock().await.app.tool_registry.clone();
    let result = registry
        .execute_tool(
            &ccx,
            "agent",
            Some("test/model"),
            tool_call_id,
            tool_name,
            args.into_iter().collect(),
        )
        .await?
        .ok_or_else(|| format!("tool '{tool_name}' is not registered"))?;
    Ok(result
        .messages
        .into_iter()
        .map(ContextEnum::ChatMessage)
        .chain(
            result
                .context_files
                .into_iter()
                .map(ContextEnum::ContextFile),
        )
        .collect())
}

fn args(entries: Vec<(&str, Value)>) -> HashMap<String, Value> {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

fn path_value(path: &Path) -> Value {
    json!(path.to_string_lossy().to_string())
}

fn text(messages: &[ContextEnum]) -> String {
    messages
        .iter()
        .filter_map(|item| match item {
            ContextEnum::ChatMessage(message) => match &message.content {
                ChatContent::SimpleText(text) => Some(text.clone()),
                _ => None,
            },
            ContextEnum::ContextFile(_) => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn context_file_names(messages: &[ContextEnum]) -> Vec<String> {
    messages
        .iter()
        .filter_map(|item| match item {
            ContextEnum::ContextFile(file) => Some(file.file_name.clone()),
            ContextEnum::ChatMessage(_) => None,
        })
        .collect()
}

#[tokio::test]
async fn shell_in_chat_with_deleted_worktree_refuses_and_writes_nothing() {
    let f = fixture();
    let ccx = ccx(&f, false).await;
    fs::remove_dir_all(&f.root).unwrap();
    let marker = f.source.join("marker");
    let command = if cfg!(windows) {
        format!("Set-Content -Path \"{}\" -Value marker", marker.display())
    } else {
        format!("touch '{}'", marker.display())
    };
    let err = execute_tool(
        ccx,
        "shell",
        "shell",
        args(vec![
            ("command", json!(command)),
            ("description", json!("Create a test marker")),
        ]),
    )
    .await
    .unwrap_err();
    assert!(err.contains(&f.root.to_string_lossy().to_string()), "{err}");
    assert!(err.contains("Do not continue editing"), "{err}");
    assert!(!marker.exists());
}

#[tokio::test]
async fn file_write_in_chat_with_deleted_worktree_refuses() {
    let f = fixture();
    let ccx = ccx(&f, false).await;
    fs::remove_dir_all(&f.root).unwrap();
    let marker = f.source.join("marker.md");
    let err = execute_tool(
        ccx,
        "create",
        "create_textdoc",
        args(vec![
            ("path", path_value(&marker)),
            ("content", json!("must not be written")),
        ]),
    )
    .await
    .unwrap_err();
    assert!(err.contains(&f.root.to_string_lossy().to_string()), "{err}");
    assert!(!marker.exists());
}

#[tokio::test]
async fn refact_state_paths_are_not_remapped() {
    let f = fixture();
    let ccx = ccx(&f, true).await;
    let path = f.source.join(".refact/tasks/task/report.md");
    execute_tool(
        ccx,
        "create",
        "create_textdoc",
        args(vec![
            ("path", path_value(&path)),
            ("content", json!("report")),
        ]),
    )
    .await
    .unwrap();
    assert!(path.is_file());
    assert!(!f.root.join(".refact/tasks/task/report.md").exists());
}

#[tokio::test]
async fn refact_state_paths_are_read_from_the_literal_source_path() {
    let f = fixture();
    let path = f.source.join(".refact/tasks/task/memories/x.md");
    fs::write(&path, "memory sentinel").unwrap();
    let ccx = ccx(&f, true).await;
    let messages = execute_tool(ccx, "cat", "cat", args(vec![("paths", path_value(&path))]))
        .await
        .unwrap();
    assert!(text(&messages).contains(&path.to_string_lossy().to_string()));
    assert_eq!(
        context_file_names(&messages),
        vec![refact_core::chat_types::normalize_file_name(
            path.to_string_lossy().to_string()
        )]
    );
    assert!(!text(&messages).contains("mapped to active worktree"));
}

#[test]
fn source_paths_are_still_remapped() {
    let f = fixture();
    let source_file = f.source.join("src/lib.rs");
    fs::create_dir_all(source_file.parent().unwrap()).unwrap();
    fs::write(&source_file, "source").unwrap();
    fs::create_dir_all(f.root.join("src")).unwrap();
    fs::write(f.root.join("src/lib.rs"), "worktree").unwrap();
    let scope = ExecutionScope::from_worktree(&WorktreeMeta {
        enforce: true,
        ..f.worktree.clone()
    });
    let resolved = scope.resolve_existing_path(&source_file).unwrap();
    assert_eq!(resolved.path, f.root.join("src/lib.rs"));
    assert_eq!(resolved.remapped_from, Some(source_file));
}
