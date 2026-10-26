use std::collections::HashMap;
use std::path::Path;
use serde::{Deserialize, Serialize};

use crate::config_dirs::{is_claude_dir, source_for_dir, CommandSource, ExtDirs};

/// Engine-level hooks configuration, loaded from the global `privacy.yaml`.
///
/// Project-local hooks are NOT executed by default: a cloned repository could
/// otherwise ship a `.refact/hooks.yaml` that runs arbitrary shell commands on
/// session start. Global hooks (`~/.config/refact/hooks.yaml`) always run.
///
/// "Trusted" has one precise meaning: the project root of the hook's source
/// directory appears, as a canonicalised path, in the `hooks.trusted_projects`
/// list of the global `privacy.yaml` (`~/.config/refact/privacy.yaml`, or the
/// file named by `--privacy-yaml`). That file lives outside any repository, so
/// a clone can never mark itself trusted, and the list is read once at engine
/// startup into `GlobalContext::hooks_config`. Matching is exact per project
/// root, so trusting one repository never trusts another, and hooks shipped by
/// an installed plugin are never executed at all.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct HooksConfig {
    #[serde(default = "default_hooks_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub trusted_projects: Vec<String>,
    #[serde(
        default = "default_hook_timeout_secs",
        deserialize_with = "deserialize_hook_timeout_secs"
    )]
    pub default_timeout_secs: u64,
    #[serde(
        default = "default_hook_recursion_depth",
        deserialize_with = "deserialize_hook_recursion_depth"
    )]
    pub max_recursion_depth: u32,
    #[serde(
        default = "default_hook_output_bytes",
        deserialize_with = "deserialize_hook_output_bytes"
    )]
    pub max_output_bytes: usize,
}

pub const DEFAULT_HOOK_TIMEOUT_SECS: u64 = 30;
pub const MIN_HOOK_TIMEOUT_SECS: u64 = 1;
pub const MAX_HOOK_TIMEOUT_SECS: u64 = 600;
pub const DEFAULT_HOOK_OUTPUT_BYTES: usize = 10 * 1024;
pub const MIN_HOOK_OUTPUT_BYTES: usize = 1024;
pub const MAX_HOOK_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
pub const DEFAULT_HOOK_RECURSION_DEPTH: u32 = 2;
pub const MIN_HOOK_RECURSION_DEPTH: u32 = 1;
pub const MAX_HOOK_RECURSION_DEPTH: u32 = 16;

fn default_hooks_enabled() -> bool {
    true
}

fn default_hook_timeout_secs() -> u64 {
    DEFAULT_HOOK_TIMEOUT_SECS
}

fn default_hook_recursion_depth() -> u32 {
    DEFAULT_HOOK_RECURSION_DEPTH
}

fn default_hook_output_bytes() -> usize {
    DEFAULT_HOOK_OUTPUT_BYTES
}

pub fn validate_hook_timeout_secs(value: u64) -> Result<u64, String> {
    if (MIN_HOOK_TIMEOUT_SECS..=MAX_HOOK_TIMEOUT_SECS).contains(&value) {
        Ok(value)
    } else {
        Err(format!(
            "hook timeout must be between {MIN_HOOK_TIMEOUT_SECS} and {MAX_HOOK_TIMEOUT_SECS} seconds, got {value}"
        ))
    }
}

pub fn validate_hook_recursion_depth(value: u32) -> Result<u32, String> {
    if (MIN_HOOK_RECURSION_DEPTH..=MAX_HOOK_RECURSION_DEPTH).contains(&value) {
        Ok(value)
    } else {
        Err(format!(
            "hook max_recursion_depth must be between {MIN_HOOK_RECURSION_DEPTH} and {MAX_HOOK_RECURSION_DEPTH}, got {value}"
        ))
    }
}

pub fn validate_hook_output_bytes(value: usize) -> Result<usize, String> {
    if (MIN_HOOK_OUTPUT_BYTES..=MAX_HOOK_OUTPUT_BYTES).contains(&value) {
        Ok(value)
    } else {
        Err(format!(
            "hook max_output_bytes must be between {MIN_HOOK_OUTPUT_BYTES} and {MAX_HOOK_OUTPUT_BYTES}, got {value}"
        ))
    }
}

fn deserialize_hook_timeout_secs<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = u64::deserialize(deserializer)?;
    validate_hook_timeout_secs(value).map_err(serde::de::Error::custom)
}

fn deserialize_hook_recursion_depth<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = u32::deserialize(deserializer)?;
    validate_hook_recursion_depth(value).map_err(serde::de::Error::custom)
}

fn deserialize_hook_output_bytes<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = usize::deserialize(deserializer)?;
    validate_hook_output_bytes(value).map_err(serde::de::Error::custom)
}

impl Default for HooksConfig {
    fn default() -> Self {
        Self {
            enabled: default_hooks_enabled(),
            trusted_projects: Vec::new(),
            default_timeout_secs: DEFAULT_HOOK_TIMEOUT_SECS,
            max_recursion_depth: DEFAULT_HOOK_RECURSION_DEPTH,
            max_output_bytes: DEFAULT_HOOK_OUTPUT_BYTES,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum HookEvent {
    PreToolUse,
    PostToolUse,
    UserPromptSubmit,
    PermissionDecision,
    SessionStart,
    SessionEnd,
    Stop,
    SubagentStart,
    SubagentStop,
    Notification,
    PreCompact,
    PostCompact,
    GoalValidation,
}

/// A control hook is waited for and may allow, deny or rewrite. An observation
/// hook is told what happened and can change nothing. The distinction is a
/// property of the event, so a hook definition cannot opt itself into control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookKind {
    Control,
    Observation,
}

impl HookEvent {
    pub fn kind(&self) -> HookKind {
        match self {
            HookEvent::PreToolUse
            | HookEvent::PostToolUse
            | HookEvent::UserPromptSubmit
            | HookEvent::PermissionDecision
            | HookEvent::PreCompact => HookKind::Control,
            HookEvent::SessionStart
            | HookEvent::SessionEnd
            | HookEvent::Stop
            | HookEvent::SubagentStart
            | HookEvent::SubagentStop
            | HookEvent::Notification
            | HookEvent::PostCompact
            | HookEvent::GoalValidation => HookKind::Observation,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            HookEvent::PreToolUse => "PreToolUse",
            HookEvent::PostToolUse => "PostToolUse",
            HookEvent::UserPromptSubmit => "UserPromptSubmit",
            HookEvent::PermissionDecision => "PermissionDecision",
            HookEvent::SessionStart => "SessionStart",
            HookEvent::SessionEnd => "SessionEnd",
            HookEvent::Stop => "Stop",
            HookEvent::SubagentStart => "SubagentStart",
            HookEvent::SubagentStop => "SubagentStop",
            HookEvent::Notification => "Notification",
            HookEvent::PreCompact => "PreCompact",
            HookEvent::PostCompact => "PostCompact",
            HookEvent::GoalValidation => "GoalValidation",
        }
    }
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
        "PermissionDecision" => Some(HookEvent::PermissionDecision),
        "SessionStart" => Some(HookEvent::SessionStart),
        "SessionEnd" => Some(HookEvent::SessionEnd),
        "Stop" => Some(HookEvent::Stop),
        "SubagentStart" => Some(HookEvent::SubagentStart),
        "SubagentStop" => Some(HookEvent::SubagentStop),
        "Notification" => Some(HookEvent::Notification),
        "PreCompact" => Some(HookEvent::PreCompact),
        "PostCompact" => Some(HookEvent::PostCompact),
        "GoalValidation" => Some(HookEvent::GoalValidation),
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
                if let Some(timeout) = hook_cmd.timeout {
                    if let Err(e) = validate_hook_timeout_secs(timeout) {
                        tracing::warn!("Hook '{}' rejected: {}", hook_cmd.command, e);
                        continue;
                    }
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
        assert_eq!(
            event_from_str("PermissionDecision"),
            Some(HookEvent::PermissionDecision)
        );
        assert_eq!(
            event_from_str("SubagentStart"),
            Some(HookEvent::SubagentStart)
        );
        assert_eq!(event_from_str("PostCompact"), Some(HookEvent::PostCompact));
        assert_eq!(
            event_from_str("GoalValidation"),
            Some(HookEvent::GoalValidation)
        );
        assert_eq!(event_from_str("Unknown"), None);
        assert_eq!(event_from_str(""), None);
    }

    #[test]
    fn test_control_and_observation_events_are_partitioned() {
        assert_eq!(HookEvent::PreToolUse.kind(), HookKind::Control);
        assert_eq!(HookEvent::PostToolUse.kind(), HookKind::Control);
        assert_eq!(HookEvent::UserPromptSubmit.kind(), HookKind::Control);
        assert_eq!(HookEvent::PermissionDecision.kind(), HookKind::Control);
        assert_eq!(HookEvent::PreCompact.kind(), HookKind::Control);
        assert_eq!(HookEvent::SessionStart.kind(), HookKind::Observation);
        assert_eq!(HookEvent::SessionEnd.kind(), HookKind::Observation);
        assert_eq!(HookEvent::Stop.kind(), HookKind::Observation);
        assert_eq!(HookEvent::SubagentStart.kind(), HookKind::Observation);
        assert_eq!(HookEvent::SubagentStop.kind(), HookKind::Observation);
        assert_eq!(HookEvent::Notification.kind(), HookKind::Observation);
        assert_eq!(HookEvent::PostCompact.kind(), HookKind::Observation);
        assert_eq!(HookEvent::GoalValidation.kind(), HookKind::Observation);
    }

    #[test]
    fn test_hooks_config_rejects_invalid_values_instead_of_coercing() {
        assert!(serde_yaml::from_str::<HooksConfig>("default_timeout_secs: 0").is_err());
        assert!(serde_yaml::from_str::<HooksConfig>("default_timeout_secs: 100000").is_err());
        assert!(serde_yaml::from_str::<HooksConfig>("max_recursion_depth: 0").is_err());
        assert!(serde_yaml::from_str::<HooksConfig>("max_recursion_depth: 9999").is_err());
        assert!(serde_yaml::from_str::<HooksConfig>("max_output_bytes: 1").is_err());
        assert!(serde_yaml::from_str::<HooksConfig>("max_output_bytes: 999999999").is_err());
    }

    #[test]
    fn test_hooks_config_accepts_valid_values() {
        let config: HooksConfig = serde_yaml::from_str(
            "enabled: false\ndefault_timeout_secs: 5\nmax_recursion_depth: 4\nmax_output_bytes: 2048",
        )
        .unwrap();
        assert!(!config.enabled);
        assert_eq!(config.default_timeout_secs, 5);
        assert_eq!(config.max_recursion_depth, 4);
        assert_eq!(config.max_output_bytes, 2048);
    }

    #[tokio::test]
    async fn test_hook_with_invalid_timeout_is_rejected_not_coerced() {
        let tmp = tempfile::tempdir().unwrap();
        let hooks_yaml = r#"
hooks:
  PreToolUse:
    - hooks:
        - type: command
          command: "./zero_timeout.sh"
          timeout: 0
        - type: command
          command: "./huge_timeout.sh"
          timeout: 100000
        - type: command
          command: "./ok.sh"
          timeout: 30
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
        assert_eq!(hooks[0].command, "./ok.sh");
    }
}
