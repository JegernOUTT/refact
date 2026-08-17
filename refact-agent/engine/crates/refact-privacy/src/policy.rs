use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct PrivacyPolicy {
    pub blocked: Vec<String>,
    pub zones: Vec<Zone>,
    pub subagents: SubagentPolicy,
    pub tool_access: ToolAccess,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct ToolAccess {
    pub providers: BTreeMap<String, ProviderToolAccess>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ProviderToolAccess {
    pub mcp: Vec<String>,
}

impl Default for ProviderToolAccess {
    fn default() -> Self {
        Self {
            mcp: vec!["*".to_string()],
        }
    }
}

impl ToolAccess {
    pub fn mcp_allowed(&self, provider: &str, server: &str) -> bool {
        match self.providers.get(provider) {
            None => true,
            Some(access) => access
                .mcp
                .iter()
                .any(|allowed| allowed == "*" || allowed == server),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct Zone {
    pub name: String,
    pub patterns: Vec<String>,
    pub send_to: Vec<String>,
    pub on_shell_read: ShellBehavior,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ShellBehavior {
    #[default]
    Withhold,
    Ask,
    Deny,
}

impl ShellBehavior {
    fn restrict_with(self, project: Self) -> Self {
        use ShellBehavior::{Ask, Deny, Withhold};

        match (self, project) {
            (Deny, _) | (_, Deny) => Deny,
            (Withhold, _) | (_, Withhold) => Withhold,
            (Ask, Ask) => Ask,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SubagentPolicy {
    pub report_declassifies: bool,
}

impl Default for SubagentPolicy {
    fn default() -> Self {
        Self {
            report_declassifies: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyPrivacyPolicy {
    pub blocked: Vec<String>,
    pub only_send_to_servers_i_control: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PolicyLoad {
    pub policy: Arc<PrivacyPolicy>,
    pub error: Option<String>,
    pub source_paths: Vec<PathBuf>,
}

impl Default for PolicyLoad {
    fn default() -> Self {
        Self {
            policy: Arc::new(PrivacyPolicy::default()),
            error: None,
            source_paths: Vec::new(),
        }
    }
}

impl PolicyLoad {
    fn failed(previous: Option<&Self>, error: String, source_paths: Vec<PathBuf>) -> Self {
        Self {
            policy: previous
                .map(|load| load.policy.clone())
                .unwrap_or_else(|| Arc::new(PrivacyPolicy::default())),
            error: Some(error),
            source_paths,
        }
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct PolicyConfig {
    blocked: Vec<String>,
    zones: Vec<Zone>,
    subagents: SubagentPolicy,
    tool_access: ToolAccess,
    #[serde(rename = "only_send_to_servers_I_control")]
    only_send_to_servers_i_control: Option<Vec<String>>,
}

pub fn migrate_legacy(legacy: LegacyPrivacyPolicy) -> PrivacyPolicy {
    let mut zones = Vec::new();
    if !legacy.only_send_to_servers_i_control.is_empty() {
        zones.push(Zone {
            name: "only_send_to_servers_i_control".to_string(),
            patterns: legacy.only_send_to_servers_i_control,
            send_to: Vec::new(),
            on_shell_read: ShellBehavior::Withhold,
        });
    }
    zones.push(Zone {
        name: "normal".to_string(),
        patterns: vec!["*".to_string()],
        send_to: vec!["*".to_string()],
        on_shell_read: ShellBehavior::Withhold,
    });
    PrivacyPolicy {
        blocked: legacy.blocked,
        zones,
        subagents: SubagentPolicy::default(),
        tool_access: ToolAccess::default(),
    }
}

pub fn merge_project(global: &PrivacyPolicy, project: &PrivacyPolicy) -> PrivacyPolicy {
    let mut merged = global.clone();
    union_patterns(&mut merged.blocked, &project.blocked);
    merged.subagents.report_declassifies &= project.subagents.report_declassifies;

    for (provider, project_access) in &project.tool_access.providers {
        let merged_access = merged
            .tool_access
            .providers
            .entry(provider.clone())
            .or_default();
        merged_access.mcp = intersect_destinations(&merged_access.mcp, &project_access.mcp);
    }

    for project_zone in &project.zones {
        if let Some(global_zone) = merged
            .zones
            .iter_mut()
            .find(|zone| zone.name == project_zone.name)
        {
            union_patterns(&mut global_zone.patterns, &project_zone.patterns);
            global_zone.send_to =
                intersect_destinations(&global_zone.send_to, &project_zone.send_to);
            global_zone.on_shell_read = global_zone
                .on_shell_read
                .restrict_with(project_zone.on_shell_read);
            continue;
        }

        let mut new_zone = project_zone.clone();
        new_zone.send_to.clear();
        new_zone.on_shell_read = ShellBehavior::Withhold.restrict_with(new_zone.on_shell_read);
        let insert_at = merged
            .zones
            .iter()
            .position(|zone| zone.name == "normal")
            .unwrap_or(merged.zones.len());
        merged.zones.insert(insert_at, new_zone);
    }

    merged
}

pub fn parse_policy_yaml(content: &str) -> Result<PrivacyPolicy, String> {
    let document = serde_yaml::from_str::<serde_yaml::Value>(content)
        .map_err(|error| format!("invalid privacy YAML: {error}"))?;
    let policy_value = document
        .as_mapping()
        .and_then(|mapping| mapping.get(serde_yaml::Value::String("privacy_rules".to_string())))
        .cloned()
        .unwrap_or(document);
    let config = serde_yaml::from_value::<PolicyConfig>(policy_value)
        .map_err(|error| format!("invalid privacy policy: {error}"))?;

    if config.zones.is_empty() {
        let mut migrated = migrate_legacy(LegacyPrivacyPolicy {
            blocked: config.blocked,
            only_send_to_servers_i_control: config
                .only_send_to_servers_i_control
                .unwrap_or_default(),
        });
        migrated.tool_access = config.tool_access;
        return Ok(migrated);
    }

    let mut policy = PrivacyPolicy {
        blocked: config.blocked,
        zones: config.zones,
        subagents: config.subagents,
        tool_access: config.tool_access,
    };
    if let Some(patterns) = config.only_send_to_servers_i_control {
        let legacy = migrate_legacy(LegacyPrivacyPolicy {
            blocked: Vec::new(),
            only_send_to_servers_i_control: patterns,
        });
        for zone in legacy.zones {
            if zone.name == "normal"
                && policy
                    .zones
                    .iter()
                    .any(|existing| existing.name == "normal")
            {
                continue;
            }
            if let Some(existing) = policy
                .zones
                .iter_mut()
                .find(|existing| existing.name == zone.name)
            {
                union_patterns(&mut existing.patterns, &zone.patterns);
                existing.send_to.clear();
                existing.on_shell_read = existing.on_shell_read.restrict_with(zone.on_shell_read);
            } else {
                let insert_at = policy
                    .zones
                    .iter()
                    .position(|existing| existing.name == "normal")
                    .unwrap_or(policy.zones.len());
                policy.zones.insert(insert_at, zone);
            }
        }
    }

    Ok(policy)
}

pub async fn load_policy(
    global_path: &Path,
    project_paths: &[PathBuf],
    previous: Option<&PolicyLoad>,
) -> PolicyLoad {
    let mut source_paths = vec![global_path.to_path_buf()];
    let global_content = match tokio::fs::read_to_string(global_path).await {
        Ok(content) => content,
        Err(error) => {
            return PolicyLoad::failed(
                previous,
                format!("failed to read {}: {error}", global_path.display()),
                source_paths,
            );
        }
    };
    let mut policy = match parse_policy_yaml(&global_content) {
        Ok(policy) => policy,
        Err(error) => {
            return PolicyLoad::failed(
                previous,
                format!("failed to parse {}: {error}", global_path.display()),
                source_paths,
            );
        }
    };

    for project_path in project_paths {
        let content = match tokio::fs::read_to_string(project_path).await {
            Ok(content) => {
                source_paths.push(project_path.clone());
                content
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                source_paths.push(project_path.clone());
                return PolicyLoad::failed(
                    previous,
                    format!("failed to read {}: {error}", project_path.display()),
                    source_paths,
                );
            }
        };
        let project = match parse_policy_yaml(&content) {
            Ok(project) => project,
            Err(error) => {
                return PolicyLoad::failed(
                    previous,
                    format!("failed to parse {}: {error}", project_path.display()),
                    source_paths,
                );
            }
        };
        policy = merge_project(&policy, &project);
    }

    if let Err(error) = policy.compile() {
        return PolicyLoad::failed(
            previous,
            format!("failed to compile privacy policy: {error}"),
            source_paths,
        );
    }

    PolicyLoad {
        policy: Arc::new(policy),
        error: None,
        source_paths,
    }
}

fn union_patterns(target: &mut Vec<String>, additions: &[String]) {
    let mut seen = target.iter().cloned().collect::<HashSet<_>>();
    target.extend(
        additions
            .iter()
            .filter(|pattern| seen.insert((*pattern).clone()))
            .cloned(),
    );
}

fn intersect_destinations(global: &[String], project: &[String]) -> Vec<String> {
    if project.iter().any(|destination| destination == "*") {
        return deduplicate(global);
    }
    if global.iter().any(|destination| destination == "*") {
        return deduplicate(project);
    }
    let project = project.iter().collect::<HashSet<_>>();
    deduplicate(
        &global
            .iter()
            .filter(|destination| project.contains(destination))
            .cloned()
            .collect::<Vec<_>>(),
    )
}

fn deduplicate(values: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .iter()
        .filter(|value| seen.insert((*value).clone()))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone(name: &str, patterns: &[&str], send_to: &[&str], on_shell_read: ShellBehavior) -> Zone {
        Zone {
            name: name.to_string(),
            patterns: patterns.iter().map(|value| (*value).to_string()).collect(),
            send_to: send_to.iter().map(|value| (*value).to_string()).collect(),
            on_shell_read,
        }
    }

    #[test]
    fn project_merge_unions_patterns_and_intersects_destinations() {
        let global = PrivacyPolicy {
            blocked: vec!["global.key".to_string()],
            zones: vec![
                zone("secrets", &[".env"], &["a", "b"], ShellBehavior::Ask),
                zone("normal", &["*"], &["*"], ShellBehavior::Withhold),
            ],
            subagents: SubagentPolicy::default(),
            tool_access: ToolAccess::default(),
        };
        let project = PrivacyPolicy {
            blocked: vec!["project.key".to_string()],
            zones: vec![zone(
                "secrets",
                &["*.pem"],
                &["b", "c"],
                ShellBehavior::Withhold,
            )],
            subagents: SubagentPolicy::default(),
            tool_access: ToolAccess::default(),
        };

        let merged = merge_project(&global, &project);

        assert_eq!(merged.blocked, vec!["global.key", "project.key"]);
        assert_eq!(merged.zones[0].patterns, vec![".env", "*.pem"]);
        assert_eq!(merged.zones[0].send_to, vec!["b"]);
        assert_eq!(merged.zones[0].on_shell_read, ShellBehavior::Withhold);
    }

    #[test]
    fn project_cannot_add_destination_delete_zone_or_relax_shell_read() {
        let global = PrivacyPolicy {
            blocked: Vec::new(),
            zones: vec![
                zone("secrets", &[".env"], &["trusted"], ShellBehavior::Withhold),
                zone("internal", &["internal/**"], &[], ShellBehavior::Deny),
                zone("normal", &["*"], &["*"], ShellBehavior::Withhold),
            ],
            subagents: SubagentPolicy {
                report_declassifies: false,
            },
            tool_access: ToolAccess::default(),
        };
        let project = PrivacyPolicy {
            blocked: Vec::new(),
            zones: vec![
                zone(
                    "secrets",
                    &["*.key"],
                    &["trusted", "untrusted"],
                    ShellBehavior::Ask,
                ),
                zone(
                    "project_only",
                    &["project/**"],
                    &["untrusted"],
                    ShellBehavior::Ask,
                ),
            ],
            subagents: SubagentPolicy::default(),
            tool_access: ToolAccess::default(),
        };

        let merged = merge_project(&global, &project);

        assert!(merged.zones.iter().any(|zone| zone.name == "internal"));
        let secrets = merged
            .zones
            .iter()
            .find(|zone| zone.name == "secrets")
            .unwrap();
        assert_eq!(secrets.send_to, vec!["trusted"]);
        assert_eq!(secrets.on_shell_read, ShellBehavior::Withhold);
        let project_only = merged
            .zones
            .iter()
            .find(|zone| zone.name == "project_only")
            .unwrap();
        assert!(project_only.send_to.is_empty());
        assert_eq!(project_only.on_shell_read, ShellBehavior::Withhold);
        assert!(!merged.subagents.report_declassifies);
    }

    #[test]
    fn legacy_only_controlled_list_migrates_to_guarded_zone() {
        let policy = parse_policy_yaml(
            "privacy_rules:\n  blocked: []\n  only_send_to_servers_I_control:\n    - a.txt\n",
        )
        .unwrap();

        let migrated = policy
            .zones
            .iter()
            .find(|zone| zone.name == "only_send_to_servers_i_control")
            .unwrap();
        assert_eq!(migrated.patterns, vec!["a.txt"]);
        assert!(migrated.send_to.is_empty());
        assert!(policy.zones.iter().any(|zone| zone.name == "normal"));
    }

    #[test]
    fn legacy_blocked_only_config_gets_normal_fallback() {
        let policy = parse_policy_yaml("privacy_rules:\n  blocked:\n    - '*.key'\n").unwrap();

        assert_eq!(policy.blocked, vec!["*.key"]);
        assert!(policy.zones.iter().any(|zone| zone.name == "normal"));
        policy.compile().unwrap();
    }
    #[test]
    fn absent_provider_may_use_every_mcp_server() {
        let access = ToolAccess::default();

        assert!(access.mcp_allowed("openai_codex", "github"));
        assert!(access.mcp_allowed("anything", "anything"));
    }

    #[test]
    fn listed_provider_is_limited_to_its_servers() {
        let access = ToolAccess {
            providers: BTreeMap::from([(
                "openai_codex".to_string(),
                ProviderToolAccess {
                    mcp: vec!["github".to_string()],
                },
            )]),
        };

        assert!(access.mcp_allowed("openai_codex", "github"));
        assert!(!access.mcp_allowed("openai_codex", "postgres"));
        assert!(access.mcp_allowed("ollama", "postgres"));
    }

    #[test]
    fn project_can_only_narrow_provider_mcp_access() {
        let global = PrivacyPolicy {
            tool_access: ToolAccess {
                providers: BTreeMap::from([(
                    "openai_codex".to_string(),
                    ProviderToolAccess {
                        mcp: vec!["github".to_string(), "fetch".to_string()],
                    },
                )]),
            },
            ..PrivacyPolicy::default()
        };
        let project = PrivacyPolicy {
            tool_access: ToolAccess {
                providers: BTreeMap::from([
                    (
                        "openai_codex".to_string(),
                        ProviderToolAccess {
                            mcp: vec!["fetch".to_string(), "postgres".to_string()],
                        },
                    ),
                    (
                        "ollama".to_string(),
                        ProviderToolAccess {
                            mcp: vec!["github".to_string()],
                        },
                    ),
                ]),
            },
            ..PrivacyPolicy::default()
        };

        let merged = merge_project(&global, &project);

        assert_eq!(
            merged.tool_access.providers["openai_codex"].mcp,
            vec!["fetch"]
        );
        assert_eq!(merged.tool_access.providers["ollama"].mcp, vec!["github"]);
        assert!(!merged.tool_access.mcp_allowed("openai_codex", "postgres"));
        assert!(!merged.tool_access.mcp_allowed("ollama", "postgres"));
    }

    #[test]
    fn tool_access_survives_legacy_migration() {
        let policy = parse_policy_yaml(
            "privacy_rules:\n  blocked: []\n  tool_access:\n    providers:\n      openai_codex:\n        mcp: ['github']\n",
        )
        .unwrap();

        assert!(policy.zones.iter().any(|zone| zone.name == "normal"));
        assert!(policy.tool_access.mcp_allowed("openai_codex", "github"));
        assert!(!policy.tool_access.mcp_allowed("openai_codex", "postgres"));
    }

    #[tokio::test]
    async fn broken_yaml_keeps_last_known_good_policy() {
        let temp = tempfile::tempdir().unwrap();
        let global = temp.path().join("privacy.yaml");
        tokio::fs::write(
            &global,
            "privacy_rules:\n  zones:\n    - name: normal\n      patterns: ['*']\n      send_to: ['*']\n",
        )
        .await
        .unwrap();
        let loaded = load_policy(&global, &[], None).await;
        assert!(loaded.error.is_none());

        tokio::fs::write(&global, "privacy_rules: [").await.unwrap();
        let failed = load_policy(&global, &[], Some(&loaded)).await;

        assert!(Arc::ptr_eq(&failed.policy, &loaded.policy));
        assert!(failed.error.is_some());
        assert_eq!(failed.source_paths, vec![global]);
    }

    #[tokio::test]
    async fn first_load_failure_is_empty_and_loud() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("missing.yaml");

        let failed = load_policy(&path, &[], None).await;

        assert_eq!(*failed.policy, PrivacyPolicy::default());
        assert!(failed.policy.blocked.is_empty());
        assert!(failed.error.is_some());
    }
}
