use std::collections::HashMap;
use std::path::Path;
use serde::{Deserialize, Serialize};

use crate::ext::config_dirs::{is_claude_dir, source_for_dir, CommandSource, ExtDirs};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum HookEvent {
    PreToolUse,
    PostToolUse,
    UserPromptSubmit,
    SessionStart,
    SessionEnd,
    Stop,
    SubagentStop,
    Notification,
    PreCompact,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookConfig {
    pub event: HookEvent,
    pub matcher: Option<String>,
    pub command: String,
    pub timeout: Option<u64>,
    pub source: CommandSource,
}

fn event_from_str(s: &str) -> Option<HookEvent> {
    match s {
        "PreToolUse" => Some(HookEvent::PreToolUse),
        "PostToolUse" => Some(HookEvent::PostToolUse),
        "UserPromptSubmit" => Some(HookEvent::UserPromptSubmit),
        "SessionStart" => Some(HookEvent::SessionStart),
        "SessionEnd" => Some(HookEvent::SessionEnd),
        "Stop" => Some(HookEvent::Stop),
        "SubagentStop" => Some(HookEvent::SubagentStop),
        "Notification" => Some(HookEvent::Notification),
        "PreCompact" => Some(HookEvent::PreCompact),
        _ => None,
    }
}

#[derive(Deserialize)]
struct HookCommandEntry {
    #[serde(rename = "type", default)]
    hook_type: String,
    #[serde(default)]
    command: String,
    #[serde(default)]
    timeout: Option<u64>,
}

#[derive(Deserialize)]
struct HookMatcherEntry {
    #[serde(default)]
    matcher: Option<String>,
    #[serde(default)]
    hooks: Vec<HookCommandEntry>,
}

#[derive(Deserialize)]
struct HooksFile {
    #[serde(default)]
    hooks: HashMap<String, Vec<HookMatcherEntry>>,
}

fn hooks_from_map(
    hooks_map: HashMap<String, Vec<HookMatcherEntry>>,
    source: CommandSource,
) -> Vec<HookConfig> {
    let mut result = Vec::new();
    for (event_str, entries) in hooks_map {
        let event = match event_from_str(&event_str) {
            Some(e) => e,
            None => {
                tracing::warn!("Unknown hook event: '{}'", event_str);
                continue;
            }
        };
        for entry in entries {
            for hook_cmd in &entry.hooks {
                if hook_cmd.hook_type != "command" && !hook_cmd.hook_type.is_empty() {
                    tracing::warn!("Unsupported hook type '{}', skipping", hook_cmd.hook_type);
                    continue;
                }
                if hook_cmd.command.is_empty() {
                    tracing::warn!("Hook entry has empty command, skipping");
                    continue;
                }
                result.push(HookConfig {
                    event: event.clone(),
                    matcher: entry.matcher.clone(),
                    command: hook_cmd.command.clone(),
                    timeout: hook_cmd.timeout,
                    source: source.clone(),
                });
            }
        }
    }
    result
}

async fn load_hooks_from_refact_yaml(path: &Path, source: CommandSource) -> Vec<HookConfig> {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    match serde_yaml::from_str::<HooksFile>(&content) {
        Ok(file) => hooks_from_map(file.hooks, source),
        Err(e) => {
            tracing::warn!("Failed to parse hooks.yaml {:?}: {}", path, e);
            vec![]
        }
    }
}

async fn load_hooks_from_claude_settings(path: &Path, source: CommandSource) -> Vec<HookConfig> {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    match serde_json::from_str::<HooksFile>(&content) {
        Ok(file) => hooks_from_map(file.hooks, source),
        Err(e) => {
            tracing::warn!("Failed to parse settings.json {:?}: {}", path, e);
            vec![]
        }
    }
}

pub async fn load_hooks(ext_dirs: &ExtDirs) -> Vec<HookConfig> {
    let mut result = Vec::new();
    for dir in ext_dirs.all_dirs_in_order() {
        let source = source_for_dir(dir, &ext_dirs.global_dirs, &ext_dirs.installed_dirs);
        if is_claude_dir(dir) {
            let settings_path = dir.join("settings.json");
            let hooks = load_hooks_from_claude_settings(&settings_path, source.clone()).await;
            result.extend(hooks);
            let local_settings_path = dir.join("settings.local.json");
            let local_hooks = load_hooks_from_claude_settings(&local_settings_path, source).await;
            result.extend(local_hooks);
        } else {
            let hooks_path = dir.join("hooks.yaml");
            let hooks = load_hooks_from_refact_yaml(&hooks_path, source).await;
            result.extend(hooks);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_load_hooks_refact_yaml() {
        let tmp = tempfile::tempdir().unwrap();
        let hooks_yaml = r#"
hooks:
  PreToolUse:
    - matcher: "Bash|Write"
      hooks:
        - type: command
          command: "./check.sh"
          timeout: 30
  PostToolUse:
    - matcher: "Write"
      hooks:
        - type: command
          command: "./format.sh"
"#;
        tokio::fs::write(tmp.path().join("hooks.yaml"), hooks_yaml)
            .await
            .unwrap();

        let ext_dirs = ExtDirs {
            global_dirs: vec![tmp.path().to_path_buf()],
            installed_dirs: vec![],
            project_dirs: vec![],
        };
        let hooks = load_hooks(&ext_dirs).await;
        assert_eq!(hooks.len(), 2);

        let pre = hooks
            .iter()
            .find(|h| h.event == HookEvent::PreToolUse)
            .unwrap();
        assert_eq!(pre.matcher, Some("Bash|Write".to_string()));
        assert_eq!(pre.command, "./check.sh");
        assert_eq!(pre.timeout, Some(30));

        let post = hooks
            .iter()
            .find(|h| h.event == HookEvent::PostToolUse)
            .unwrap();
        assert_eq!(post.matcher, Some("Write".to_string()));
        assert_eq!(post.command, "./format.sh");
    }

    #[tokio::test]
    async fn test_load_hooks_claude_settings_json() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_dir = tmp.path().join(".claude");
        tokio::fs::create_dir_all(&claude_dir).await.unwrap();

        let settings_json = r#"{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "echo session started"
          }
        ]
      }
    ],
    "UserPromptSubmit": [
      {
        "matcher": ".*",
        "hooks": [
          {
            "type": "command",
            "command": "./on_prompt.sh",
            "timeout": 10
          }
        ]
      }
    ]
  }
}"#;
        tokio::fs::write(claude_dir.join("settings.json"), settings_json)
            .await
            .unwrap();

        let ext_dirs = ExtDirs {
            global_dirs: vec![claude_dir.clone()],
            installed_dirs: vec![],
            project_dirs: vec![],
        };
        let hooks = load_hooks(&ext_dirs).await;
        assert_eq!(hooks.len(), 2);

        let session = hooks
            .iter()
            .find(|h| h.event == HookEvent::SessionStart)
            .unwrap();
        assert_eq!(session.command, "echo session started");
        assert!(session.matcher.is_none());

        let prompt = hooks
            .iter()
            .find(|h| h.event == HookEvent::UserPromptSubmit)
            .unwrap();
        assert_eq!(prompt.command, "./on_prompt.sh");
        assert_eq!(prompt.timeout, Some(10));
        assert_eq!(prompt.matcher, Some(".*".to_string()));
    }

    #[tokio::test]
    async fn test_load_hooks_missing_file() {
        let ext_dirs = ExtDirs {
            global_dirs: vec![PathBuf::from("/nonexistent/path")],
            installed_dirs: vec![],
            project_dirs: vec![],
        };
        let hooks = load_hooks(&ext_dirs).await;
        assert!(hooks.is_empty());
    }

    #[tokio::test]
    async fn test_load_hooks_malformed_yaml() {
        let tmp = tempfile::tempdir().unwrap();
        tokio::fs::write(tmp.path().join("hooks.yaml"), "not: valid: yaml: :::")
            .await
            .unwrap();

        let ext_dirs = ExtDirs {
            global_dirs: vec![tmp.path().to_path_buf()],
            installed_dirs: vec![],
            project_dirs: vec![],
        };
        let hooks = load_hooks(&ext_dirs).await;
        assert!(hooks.is_empty());
    }

    #[tokio::test]
    async fn test_load_hooks_malformed_json() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_dir = tmp.path().join(".claude");
        tokio::fs::create_dir_all(&claude_dir).await.unwrap();
        tokio::fs::write(claude_dir.join("settings.json"), "{invalid json}")
            .await
            .unwrap();

        let ext_dirs = ExtDirs {
            global_dirs: vec![claude_dir.clone()],
            installed_dirs: vec![],
            project_dirs: vec![],
        };
        let hooks = load_hooks(&ext_dirs).await;
        assert!(hooks.is_empty());
    }

    #[tokio::test]
    async fn test_load_hooks_unknown_event_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let hooks_yaml = r#"
hooks:
  UnknownEvent:
    - hooks:
        - type: command
          command: "./unknown.sh"
  PreToolUse:
    - hooks:
        - type: command
          command: "./known.sh"
"#;
        tokio::fs::write(tmp.path().join("hooks.yaml"), hooks_yaml)
            .await
            .unwrap();

        let ext_dirs = ExtDirs {
            global_dirs: vec![tmp.path().to_path_buf()],
            installed_dirs: vec![],
            project_dirs: vec![],
        };
        let hooks = load_hooks(&ext_dirs).await;
        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].event, HookEvent::PreToolUse);
    }

    #[tokio::test]
    async fn test_load_hooks_all_events() {
        let tmp = tempfile::tempdir().unwrap();
        let hooks_yaml = r#"
hooks:
  PreToolUse:
    - hooks:
        - type: command
          command: "cmd1"
  PostToolUse:
    - hooks:
        - type: command
          command: "cmd2"
  UserPromptSubmit:
    - hooks:
        - type: command
          command: "cmd3"
  SessionStart:
    - hooks:
        - type: command
          command: "cmd4"
  SessionEnd:
    - hooks:
        - type: command
          command: "cmd5"
  Stop:
    - hooks:
        - type: command
          command: "cmd6"
  SubagentStop:
    - hooks:
        - type: command
          command: "cmd7"
  Notification:
    - hooks:
        - type: command
          command: "cmd8"
  PreCompact:
    - hooks:
        - type: command
          command: "cmd9"
"#;
        tokio::fs::write(tmp.path().join("hooks.yaml"), hooks_yaml)
            .await
            .unwrap();

        let ext_dirs = ExtDirs {
            global_dirs: vec![tmp.path().to_path_buf()],
            installed_dirs: vec![],
            project_dirs: vec![],
        };
        let hooks = load_hooks(&ext_dirs).await;
        assert_eq!(hooks.len(), 9);
    }

    #[tokio::test]
    async fn test_precompact_event_parsing() {
        let tmp = tempfile::tempdir().unwrap();
        let hooks_yaml = r#"
hooks:
  PreCompact:
    - hooks:
        - type: command
          command: "./compact_hook.sh"
          timeout: 60
"#;
        tokio::fs::write(tmp.path().join("hooks.yaml"), hooks_yaml)
            .await
            .unwrap();

        let ext_dirs = ExtDirs {
            global_dirs: vec![tmp.path().to_path_buf()],
            installed_dirs: vec![],
            project_dirs: vec![],
        };
        let hooks = load_hooks(&ext_dirs).await;
        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].event, HookEvent::PreCompact);
        assert_eq!(hooks[0].command, "./compact_hook.sh");
        assert_eq!(hooks[0].timeout, Some(60));
    }

    #[tokio::test]
    async fn test_precompact_event_parsing_json() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_dir = tmp.path().join(".claude");
        tokio::fs::create_dir_all(&claude_dir).await.unwrap();

        let settings_json = r#"{
  "hooks": {
    "PreCompact": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "echo compacting"
          }
        ]
      }
    ]
  }
}"#;
        tokio::fs::write(claude_dir.join("settings.json"), settings_json)
            .await
            .unwrap();

        let ext_dirs = ExtDirs {
            global_dirs: vec![claude_dir.clone()],
            installed_dirs: vec![],
            project_dirs: vec![],
        };
        let hooks = load_hooks(&ext_dirs).await;
        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].event, HookEvent::PreCompact);
        assert_eq!(hooks[0].command, "echo compacting");
    }

    #[tokio::test]
    async fn test_settings_local_json_loaded() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_dir = tmp.path().join(".claude");
        tokio::fs::create_dir_all(&claude_dir).await.unwrap();

        let settings_json = r#"{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "echo from_settings"
          }
        ]
      }
    ]
  }
}"#;
        let local_settings_json = r#"{
  "hooks": {
    "SessionEnd": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "echo from_local_settings"
          }
        ]
      }
    ]
  }
}"#;
        tokio::fs::write(claude_dir.join("settings.json"), settings_json)
            .await
            .unwrap();
        tokio::fs::write(claude_dir.join("settings.local.json"), local_settings_json)
            .await
            .unwrap();

        let ext_dirs = ExtDirs {
            global_dirs: vec![claude_dir.clone()],
            installed_dirs: vec![],
            project_dirs: vec![],
        };
        let hooks = load_hooks(&ext_dirs).await;
        assert_eq!(hooks.len(), 2);
        assert!(hooks
            .iter()
            .any(|h| h.event == HookEvent::SessionStart && h.command == "echo from_settings"));
        assert!(hooks
            .iter()
            .any(|h| h.event == HookEvent::SessionEnd && h.command == "echo from_local_settings"));
    }

    #[tokio::test]
    async fn test_settings_local_overrides_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_dir = tmp.path().join(".claude");
        tokio::fs::create_dir_all(&claude_dir).await.unwrap();

        let settings_json = r#"{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "echo base_command"
          }
        ]
      }
    ]
  }
}"#;
        let local_settings_json = r#"{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "echo local_override"
          }
        ]
      }
    ]
  }
}"#;
        tokio::fs::write(claude_dir.join("settings.json"), settings_json)
            .await
            .unwrap();
        tokio::fs::write(claude_dir.join("settings.local.json"), local_settings_json)
            .await
            .unwrap();

        let ext_dirs = ExtDirs {
            global_dirs: vec![claude_dir.clone()],
            installed_dirs: vec![],
            project_dirs: vec![],
        };
        let hooks = load_hooks(&ext_dirs).await;
        assert_eq!(hooks.len(), 2);
        assert!(hooks.iter().any(|h| h.command == "echo base_command"));
        assert!(hooks.iter().any(|h| h.command == "echo local_override"));
        let local_hook = hooks
            .iter()
            .find(|h| h.command == "echo local_override")
            .unwrap();
        let base_hook = hooks
            .iter()
            .find(|h| h.command == "echo base_command")
            .unwrap();
        let local_pos = hooks
            .iter()
            .position(|h| h.command == "echo local_override")
            .unwrap();
        let base_pos = hooks
            .iter()
            .position(|h| h.command == "echo base_command")
            .unwrap();
        assert!(
            local_pos > base_pos,
            "local settings hooks should come after base settings hooks"
        );
        let _ = (local_hook, base_hook);
    }

    #[tokio::test]
    async fn test_load_hooks_no_matcher_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let hooks_yaml = r#"
hooks:
  SessionStart:
    - hooks:
        - type: command
          command: "./startup.sh"
"#;
        tokio::fs::write(tmp.path().join("hooks.yaml"), hooks_yaml)
            .await
            .unwrap();

        let ext_dirs = ExtDirs {
            global_dirs: vec![tmp.path().to_path_buf()],
            installed_dirs: vec![],
            project_dirs: vec![],
        };
        let hooks = load_hooks(&ext_dirs).await;
        assert_eq!(hooks.len(), 1);
        assert!(hooks[0].matcher.is_none());
        assert!(hooks[0].timeout.is_none());
    }

    #[tokio::test]
    async fn test_load_hooks_combines_global_and_project() {
        let global_tmp = tempfile::tempdir().unwrap();
        let project_tmp = tempfile::tempdir().unwrap();

        tokio::fs::write(
            global_tmp.path().join("hooks.yaml"),
            "hooks:\n  SessionStart:\n    - hooks:\n        - type: command\n          command: global_cmd",
        )
        .await
        .unwrap();

        tokio::fs::write(
            project_tmp.path().join("hooks.yaml"),
            "hooks:\n  SessionEnd:\n    - hooks:\n        - type: command\n          command: project_cmd",
        )
        .await
        .unwrap();

        let ext_dirs = ExtDirs {
            global_dirs: vec![global_tmp.path().to_path_buf()],
            installed_dirs: vec![],
            project_dirs: vec![project_tmp.path().to_path_buf()],
        };
        let hooks = load_hooks(&ext_dirs).await;
        assert_eq!(hooks.len(), 2);
        assert!(hooks.iter().any(|h| h.command == "global_cmd"));
        assert!(hooks.iter().any(|h| h.command == "project_cmd"));
    }

    #[test]
    fn test_event_from_str_all_variants() {
        assert_eq!(event_from_str("PreToolUse"), Some(HookEvent::PreToolUse));
        assert_eq!(event_from_str("PostToolUse"), Some(HookEvent::PostToolUse));
        assert_eq!(
            event_from_str("UserPromptSubmit"),
            Some(HookEvent::UserPromptSubmit)
        );
        assert_eq!(
            event_from_str("SessionStart"),
            Some(HookEvent::SessionStart)
        );
        assert_eq!(event_from_str("SessionEnd"), Some(HookEvent::SessionEnd));
        assert_eq!(event_from_str("Stop"), Some(HookEvent::Stop));
        assert_eq!(
            event_from_str("SubagentStop"),
            Some(HookEvent::SubagentStop)
        );
        assert_eq!(
            event_from_str("Notification"),
            Some(HookEvent::Notification)
        );
        assert_eq!(event_from_str("PreCompact"), Some(HookEvent::PreCompact));
        assert_eq!(event_from_str("Unknown"), None);
        assert_eq!(event_from_str(""), None);
    }
}
