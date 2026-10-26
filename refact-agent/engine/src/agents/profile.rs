use std::collections::BTreeSet;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::global_context::GlobalContext;
use crate::subchat::ToolsPolicy;

pub const PROFILE_SCHEMA_VERSION: u32 = 1;
pub const PROFILE_CONFIG_KEY: &str = "agent_profile";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum ProfileError {
    ProfileUnknown {
        name: String,
    },
    ProfileMalformed {
        name: String,
        detail: String,
    },
    ProfileBounds {
        name: String,
        detail: String,
    },
    ProfileRevisionMismatch {
        name: String,
        accepted: u64,
        current: u64,
    },
    ProfileWidened {
        name: String,
        detail: String,
    },
}

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProfileError::ProfileUnknown { name } => write!(
                f,
                "agent profile `{name}` is not defined; define an `agent_profile` block in a subagent config before referencing it"
            ),
            ProfileError::ProfileMalformed { name, detail } => {
                write!(f, "agent profile `{name}` is malformed: {detail}")
            }
            ProfileError::ProfileBounds { name, detail } => {
                write!(f, "agent profile `{name}` exceeds configured bounds: {detail}")
            }
            ProfileError::ProfileRevisionMismatch { name, accepted, current } => write!(
                f,
                "agent profile `{name}` revision {accepted} is pinned but the current definition is revision {current}"
            ),
            ProfileError::ProfileWidened { name, detail } => write!(
                f,
                "run instance cannot widen agent profile `{name}`: {detail}"
            ),
        }
    }
}

impl ProfileError {
    pub fn to_json(&self) -> serde_json::Value {
        let mut value = serde_json::to_value(self).unwrap_or(serde_json::Value::Null);
        if let Some(object) = value.as_object_mut() {
            object.insert("message".into(), serde_json::json!(self.to_string()));
        }
        value
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProfileApproval {
    pub autonomous_no_confirm: Option<bool>,
    pub auto_approve_editing_tools: Option<bool>,
    pub auto_approve_dangerous_commands: Option<bool>,
}

/// Reusable, named definition. Data only: it never launches anything and never
/// reads the parent's authority. A run instance references it by name and may
/// narrow it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentProfileSpec {
    pub model: Option<String>,
    pub model_type: Option<String>,
    pub tools: Option<Vec<String>>,
    pub deny_tools: Vec<String>,
    pub role: Option<String>,
    pub skills: Vec<String>,
    pub reasoning_effort: Option<refact_core::chat_types::ReasoningEffort>,
    pub hooks: Vec<String>,
    pub path_scope: Vec<String>,
    pub approval: ProfileApproval,
    pub budget: Option<refact_chat_api::GoalBudget>,
    pub max_steps: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentProfile {
    pub name: String,
    pub revision: u64,
    pub spec: AgentProfileSpec,
}

/// What a single launch contributes on top of a profile. Every field may only
/// narrow the profile, never widen it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunNarrowing {
    pub tools: Option<Vec<String>>,
    pub model: Option<String>,
    pub model_type: Option<String>,
    pub target_files: Vec<String>,
    pub max_steps: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileProvenance {
    #[serde(default)]
    pub contributed: Vec<String>,
    #[serde(default)]
    pub narrowed: Vec<String>,
    #[serde(default)]
    pub notes: Vec<String>,
}

/// The effective run configuration a profile plus its run narrowing produced.
/// This is the value frozen into the accepted launch bytes; the profile it came
/// from may change afterwards without affecting an accepted or queued launch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedProfile {
    pub schema_version: u32,
    pub name: String,
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_steps: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<refact_core::chat_types::ReasoningEffort>,
    #[serde(default)]
    pub approval: ProfileApproval,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<refact_chat_api::GoalBudget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default)]
    pub provenance: ProfileProvenance,
}

fn content_revision(name: &str, spec: &AgentProfileSpec) -> u64 {
    use sha2::Digest;
    let mut value = serde_json::to_value(spec).unwrap_or(serde_json::Value::Null);
    value.sort_all_objects();
    let mut digest = sha2::Sha256::new();
    digest.update(name.as_bytes());
    digest.update(b"\0");
    digest.update(value.to_string().as_bytes());
    let bytes = digest.finalize();
    let mut head = [0u8; 8];
    head.copy_from_slice(&bytes[..8]);
    u64::from_be_bytes(head) | 1
}

impl AgentProfile {
    pub fn parse(
        name: &str,
        declaration: &serde_yaml::Value,
        limits: &refact_work_core::CoordinationSettings,
    ) -> Result<AgentProfile, ProfileError> {
        let bytes = serde_yaml::to_string(declaration)
            .map_err(|error| ProfileError::ProfileMalformed {
                name: name.to_string(),
                detail: error.to_string(),
            })?
            .len();
        if bytes > limits.agent_profile_max_bytes {
            return Err(ProfileError::ProfileBounds {
                name: name.to_string(),
                detail: format!(
                    "definition is {bytes} bytes, agent_profile_max_bytes is {}",
                    limits.agent_profile_max_bytes
                ),
            });
        }
        let spec: AgentProfileSpec =
            serde_yaml::from_value(declaration.clone()).map_err(|error| {
                ProfileError::ProfileMalformed {
                    name: name.to_string(),
                    detail: error.to_string(),
                }
            })?;
        if spec
            .tools
            .as_ref()
            .is_some_and(|tools| tools.iter().any(|tool| tool.trim().is_empty()))
            || spec.deny_tools.iter().any(|tool| tool.trim().is_empty())
        {
            return Err(ProfileError::ProfileMalformed {
                name: name.to_string(),
                detail: "tool names must not be empty".into(),
            });
        }
        if spec.max_steps == Some(0) {
            return Err(ProfileError::ProfileMalformed {
                name: name.to_string(),
                detail: "max_steps must be a positive integer".into(),
            });
        }
        let revision = content_revision(name, &spec);
        Ok(AgentProfile {
            name: name.to_string(),
            revision,
            spec,
        })
    }

    fn allowed_tools(&self) -> Option<Vec<String>> {
        let denied = self
            .spec
            .deny_tools
            .iter()
            .map(|tool| super::tool_authority::canonical(tool))
            .collect::<BTreeSet<_>>();
        self.spec.tools.as_ref().map(|tools| {
            tools
                .iter()
                .map(|tool| super::tool_authority::canonical(tool))
                .filter(|tool| !denied.contains(tool))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        })
    }

    fn step_cap(&self) -> Option<usize> {
        let budget_cap = self
            .spec
            .budget
            .as_ref()
            .and_then(|budget| budget.max_turns)
            .filter(|turns| *turns > 0)
            .map(|turns| turns as usize);
        match (self.spec.max_steps, budget_cap) {
            (Some(steps), Some(turns)) => Some(steps.min(turns)),
            (steps, None) => steps,
            (None, turns) => turns,
        }
    }

    fn render_instructions(&self) -> Option<String> {
        let mut sections = Vec::new();
        if let Some(role) = self
            .spec
            .role
            .as_deref()
            .map(str::trim)
            .filter(|role| !role.is_empty())
        {
            sections.push(format!("## Role\n{role}"));
        }
        if !self.spec.skills.is_empty() {
            sections.push(format!(
                "## Skills\n{}",
                self.spec
                    .skills
                    .iter()
                    .map(|skill| format!("- {skill}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        if !self.spec.hooks.is_empty() {
            sections.push(format!(
                "## Hooks\n{}",
                self.spec
                    .hooks
                    .iter()
                    .map(|hook| format!("- {hook}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        if !self.spec.path_scope.is_empty() {
            sections.push(format!(
                "## Path scope\nDeclared intent, not enforcement: these paths are the ones this profile expects to touch. Built-in mutations are only restricted to them when the run's scope guard is strict.\n{}",
                self.spec
                    .path_scope
                    .iter()
                    .map(|path| format!("- {path}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        if sections.is_empty() {
            return None;
        }
        Some(format!(
            "# Agent Profile: {} (revision {})\n{}",
            self.name,
            self.revision,
            sections.join("\n\n")
        ))
    }
}

/// profile defaults -> explicit per-run narrowing -> effective config.
///
/// A profile can never widen authority: the tools it contributes are intersected
/// with the caller's concrete ceiling and every dropped tool is reported as a
/// note. A run instance that asks for something the profile does not grant is a
/// structured refusal, never a silent escalation.
pub fn resolve_profile_instance(
    profile: &AgentProfile,
    pinned_revision: Option<u64>,
    narrowing: &RunNarrowing,
    ceiling: &ToolsPolicy,
) -> Result<ResolvedProfile, ProfileError> {
    if let Some(pinned) = pinned_revision {
        if pinned != profile.revision {
            return Err(ProfileError::ProfileRevisionMismatch {
                name: profile.name.clone(),
                accepted: pinned,
                current: profile.revision,
            });
        }
    }
    let mut provenance = ProfileProvenance::default();
    let allowed = profile.allowed_tools();
    let denied = profile
        .spec
        .deny_tools
        .iter()
        .map(|tool| super::tool_authority::canonical(tool))
        .collect::<BTreeSet<_>>();
    let tools = match (&allowed, &narrowing.tools) {
        (_, Some(requested)) => {
            let requested = requested
                .iter()
                .map(|tool| super::tool_authority::canonical(tool))
                .collect::<Vec<_>>();
            let outside = requested
                .iter()
                .filter(|tool| {
                    denied.contains(tool.as_str())
                        || allowed
                            .as_ref()
                            .is_some_and(|allowed| !allowed.iter().any(|granted| granted == *tool))
                })
                .cloned()
                .collect::<Vec<_>>();
            if !outside.is_empty() {
                return Err(ProfileError::ProfileWidened {
                    name: profile.name.clone(),
                    detail: format!("the profile does not grant {}", outside.join(", ")),
                });
            }
            provenance.narrowed.push("tools".into());
            Some(requested)
        }
        (Some(allowed), None) => {
            let admitted = allowed
                .iter()
                .filter(|tool| ceiling.allows_tool(tool.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            let dropped = allowed
                .iter()
                .filter(|tool| !ceiling.allows_tool(tool.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            if !dropped.is_empty() {
                provenance.notes.push(format!(
                    "profile tools outside the caller authority were narrowed away, not granted: {}",
                    dropped.join(", ")
                ));
            }
            provenance.contributed.push("tools".into());
            Some(admitted)
        }
        (None, None) => None,
    };
    let target_files = if narrowing.target_files.is_empty() {
        if profile.spec.path_scope.is_empty() {
            Vec::new()
        } else {
            provenance.contributed.push("path_scope".into());
            profile.spec.path_scope.clone()
        }
    } else {
        if !profile.spec.path_scope.is_empty() {
            let scope = profile
                .spec
                .path_scope
                .iter()
                .map(|path| crate::agents::registry::normalize_path_for_overlap(path))
                .collect::<BTreeSet<_>>();
            let outside = narrowing
                .target_files
                .iter()
                .filter(|path| {
                    !scope.contains(&crate::agents::registry::normalize_path_for_overlap(
                        path.as_str(),
                    ))
                })
                .cloned()
                .collect::<Vec<_>>();
            if !outside.is_empty() {
                return Err(ProfileError::ProfileWidened {
                    name: profile.name.clone(),
                    detail: format!(
                        "these target files are outside the profile path scope: {}",
                        outside.join(", ")
                    ),
                });
            }
            provenance.narrowed.push("path_scope".into());
        }
        narrowing.target_files.clone()
    };
    let model = match (&narrowing.model, &profile.spec.model) {
        (Some(model), _) => {
            provenance.narrowed.push("model".into());
            Some(model.clone())
        }
        (None, Some(model)) => {
            provenance.contributed.push("model".into());
            Some(model.clone())
        }
        (None, None) => None,
    };
    let model_type = match (&narrowing.model_type, &profile.spec.model_type) {
        (Some(model_type), _) => Some(model_type.clone()),
        (None, Some(model_type)) => {
            provenance.contributed.push("model_type".into());
            Some(model_type.clone())
        }
        (None, None) => None,
    };
    let max_steps = match (profile.step_cap(), narrowing.max_steps) {
        (Some(profile_cap), Some(run_cap)) => {
            provenance.contributed.push("max_steps".into());
            if run_cap < profile_cap {
                provenance.narrowed.push("max_steps".into());
            }
            Some(profile_cap.min(run_cap))
        }
        (Some(profile_cap), None) => {
            provenance.contributed.push("max_steps".into());
            Some(profile_cap)
        }
        (None, run_cap) => run_cap,
    };
    if profile.spec.reasoning_effort.is_some() {
        provenance.contributed.push("reasoning_effort".into());
    }
    if profile.spec.budget.is_some() {
        provenance.contributed.push("budget".into());
    }
    let instructions = profile.render_instructions();
    if instructions.is_some() {
        provenance.contributed.push("instructions".into());
    }
    if profile.spec.approval != ProfileApproval::default() {
        provenance.contributed.push("approval".into());
    }
    Ok(ResolvedProfile {
        schema_version: PROFILE_SCHEMA_VERSION,
        name: profile.name.clone(),
        revision: profile.revision,
        model,
        model_type,
        tools,
        target_files,
        max_steps,
        reasoning_effort: profile.spec.reasoning_effort.clone(),
        approval: profile.spec.approval.clone(),
        budget: profile.spec.budget.clone(),
        instructions,
        provenance,
    })
}

/// Loads a named profile from the existing subagent configuration registry. An
/// unknown or malformed reference is a structured error before anything is
/// created, never a silent fallback to a permissive default.
pub async fn load_profile(
    gcx: Arc<GlobalContext>,
    name: &str,
    limits: &refact_work_core::CoordinationSettings,
) -> Result<AgentProfile, ProfileError> {
    let unknown = || ProfileError::ProfileUnknown {
        name: name.to_string(),
    };
    let registry = crate::yaml_configs::customization_registry::get_project_registry(gcx)
        .await
        .ok_or_else(unknown)?;
    let declared = registry
        .subagents
        .values()
        .filter(|config| config.extra.contains_key(PROFILE_CONFIG_KEY))
        .count();
    if declared > limits.agent_profile_max_count {
        return Err(ProfileError::ProfileBounds {
            name: name.to_string(),
            detail: format!(
                "{declared} profiles are defined, agent_profile_max_count is {}",
                limits.agent_profile_max_count
            ),
        });
    }
    let declaration = registry
        .subagents
        .get(name)
        .and_then(|config| config.extra.get(PROFILE_CONFIG_KEY))
        .ok_or_else(unknown)?;
    AgentProfile::parse(name, declaration, limits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec_value(yaml: &str) -> serde_yaml::Value {
        serde_yaml::from_str(yaml).unwrap()
    }

    fn profile(yaml: &str) -> AgentProfile {
        AgentProfile::parse(
            "reviewer",
            &spec_value(yaml),
            &refact_work_core::CoordinationSettings::default(),
        )
        .unwrap()
    }

    fn ceiling(tools: &[&str]) -> ToolsPolicy {
        ToolsPolicy::Only(tools.iter().map(|tool| tool.to_string()).collect())
    }

    #[test]
    fn profile_defaults_then_run_narrowing_produce_the_effective_config() {
        let profile = profile(
            "model: profile/model\ntools: [cat, shell]\nmax_steps: 30\nrole: Careful reviewer\n",
        );
        let effective = resolve_profile_instance(
            &profile,
            None,
            &RunNarrowing::default(),
            &ceiling(&["cat", "shell"]),
        )
        .unwrap();
        assert_eq!(effective.model.as_deref(), Some("profile/model"));
        assert_eq!(
            effective.tools,
            Some(vec!["cat".to_string(), "shell".to_string()])
        );
        assert_eq!(effective.max_steps, Some(30));
        assert!(effective.instructions.unwrap().contains("Careful reviewer"));

        let narrowed = resolve_profile_instance(
            &profile,
            None,
            &RunNarrowing {
                tools: Some(vec!["cat".into()]),
                model: Some("run/model".into()),
                max_steps: Some(10),
                ..Default::default()
            },
            &ceiling(&["cat", "shell"]),
        )
        .unwrap();
        assert_eq!(narrowed.tools, Some(vec!["cat".to_string()]));
        assert_eq!(narrowed.model.as_deref(), Some("run/model"));
        assert_eq!(narrowed.max_steps, Some(10));
        assert!(narrowed.provenance.narrowed.contains(&"tools".to_string()));
        assert!(narrowed.provenance.narrowed.contains(&"model".to_string()));
    }

    #[test]
    fn run_instance_widening_the_profile_is_a_structured_refusal() {
        let profile = profile("tools: [cat]\ndeny_tools: [apply]\n");
        let error = resolve_profile_instance(
            &profile,
            None,
            &RunNarrowing {
                tools: Some(vec!["cat".into(), "shell".into()]),
                ..Default::default()
            },
            &ceiling(&["cat", "shell"]),
        )
        .unwrap_err();
        assert!(matches!(error, ProfileError::ProfileWidened { .. }));
        assert!(error.to_string().contains("shell"));
        assert_eq!(error.to_json()["code"], "profile_widened");

        let denied = resolve_profile_instance(
            &profile,
            None,
            &RunNarrowing {
                tools: Some(vec!["t_apply".into()]),
                ..Default::default()
            },
            &ceiling(&["cat", "apply"]),
        )
        .unwrap_err();
        assert!(matches!(denied, ProfileError::ProfileWidened { .. }));
    }

    #[test]
    fn profile_tool_outside_the_caller_ceiling_is_narrowed_with_a_note_not_escalated() {
        let profile = profile("tools: [cat, shell]\n");
        let effective =
            resolve_profile_instance(&profile, None, &RunNarrowing::default(), &ceiling(&["cat"]))
                .unwrap();
        assert_eq!(effective.tools, Some(vec!["cat".to_string()]));
        assert_eq!(effective.provenance.notes.len(), 1);
        assert!(effective.provenance.notes[0].contains("shell"));
        assert!(effective.provenance.notes[0].contains("not granted"));

        let empty =
            resolve_profile_instance(&profile, None, &RunNarrowing::default(), &ToolsPolicy::None)
                .unwrap();
        assert_eq!(empty.tools, Some(Vec::new()));
    }

    #[test]
    fn a_pinned_revision_refuses_a_profile_that_changed_after_acceptance() {
        let first = profile("tools: [cat]\n");
        let edited = profile("tools: [cat, shell]\n");
        assert_ne!(first.revision, edited.revision);
        assert!(resolve_profile_instance(
            &first,
            Some(first.revision),
            &RunNarrowing::default(),
            &ceiling(&["cat"])
        )
        .is_ok());
        let error = resolve_profile_instance(
            &edited,
            Some(first.revision),
            &RunNarrowing::default(),
            &ceiling(&["cat"]),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ProfileError::ProfileRevisionMismatch { .. }
        ));
        assert_eq!(error.to_json()["code"], "profile_revision_mismatch");
    }

    #[test]
    fn malformed_and_oversized_declarations_are_structured_errors() {
        let limits = refact_work_core::CoordinationSettings::default();
        let unknown_field =
            AgentProfile::parse("bad", &spec_value("mdoel: typo\n"), &limits).unwrap_err();
        assert!(matches!(
            unknown_field,
            ProfileError::ProfileMalformed { .. }
        ));
        assert_eq!(unknown_field.to_json()["code"], "profile_malformed");
        assert!(AgentProfile::parse("bad", &spec_value("tools: [\"\"]\n"), &limits).is_err());
        assert!(AgentProfile::parse("bad", &spec_value("max_steps: 0\n"), &limits).is_err());
        let bounded = refact_work_core::CoordinationSettings {
            agent_profile_max_bytes: 4,
            ..Default::default()
        };
        let error =
            AgentProfile::parse("bad", &spec_value("tools: [cat]\n"), &bounded).unwrap_err();
        assert!(matches!(error, ProfileError::ProfileBounds { .. }));
    }

    #[test]
    fn path_scope_seeds_target_files_and_refuses_out_of_scope_runs() {
        let profile = profile("path_scope: [src/a.rs, src/b.rs]\n");
        let seeded =
            resolve_profile_instance(&profile, None, &RunNarrowing::default(), &ToolsPolicy::All)
                .unwrap();
        assert_eq!(seeded.target_files, vec!["src/a.rs", "src/b.rs"]);
        let narrowed = resolve_profile_instance(
            &profile,
            None,
            &RunNarrowing {
                target_files: vec!["src/a.rs".into()],
                ..Default::default()
            },
            &ToolsPolicy::All,
        )
        .unwrap();
        assert_eq!(narrowed.target_files, vec!["src/a.rs"]);
        let outside = resolve_profile_instance(
            &profile,
            None,
            &RunNarrowing {
                target_files: vec!["src/c.rs".into()],
                ..Default::default()
            },
            &ToolsPolicy::All,
        )
        .unwrap_err();
        assert!(matches!(outside, ProfileError::ProfileWidened { .. }));
    }
}
