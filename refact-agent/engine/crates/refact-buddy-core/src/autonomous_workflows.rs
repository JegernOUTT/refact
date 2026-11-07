#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutonomousWorkflowMeta {
    pub id: &'static str,
    pub title: &'static str,
    pub icon: &'static str,
    pub badge: &'static str,
    pub priority: &'static str,
    pub kind: &'static str,
}

pub const ERROR_DETECTIVE_WORKFLOW_ID: &str = "refact_error_detective";
pub const REFACT_SELF_CRITIC_WORKFLOW_ID: &str = "refact_self_critic";
pub const REFACT_COMPILE_SNIFFER_WORKFLOW_ID: &str = "refact_compile_sniffer";
pub const BUDDY_DAILY_DIGEST_WORKFLOW_ID: &str = "buddy_daily_digest";
pub const BUDDY_FRIDAY_RETRO_WORKFLOW_ID: &str = "buddy_friday_retro";
pub const BUDDY_IDLE_SUGGESTER_WORKFLOW_ID: &str = "buddy_idle_suggester";
pub const BUDDY_ONBOARDING_WORKFLOW_ID: &str = "buddy_onboarding";
pub const BUDDY_PR_ISSUE_MATCHMAKER_WORKFLOW_ID: &str = "buddy_pr_issue_matchmaker";
pub const BUDDY_REFACTOR_HUNTER_WORKFLOW_ID: &str = "buddy_refactor_hunter";
pub const BUDDY_SKILL_AUTHOR_WORKFLOW_ID: &str = "buddy_skill_author";
pub const BUDDY_TEST_COVERAGE_WATCHER_WORKFLOW_ID: &str = "buddy_test_coverage_watcher";
pub const SECURITY_WHISPERER_WORKFLOW_ID: &str = "buddy_security_whisperer";
pub const SETUP_COACH_WORKFLOW_ID: &str = "buddy_setup_coach";
pub const DEPENDENCY_RADAR_WORKFLOW_ID: &str = "buddy_dependency_radar";
pub const DOCS_GARDENER_WORKFLOW_ID: &str = "buddy_docs_gardener";
pub const ARCHITECTURE_DRIFT_WORKFLOW_ID: &str = "buddy_architecture_drift_watcher";
pub const MEMORY_GARDENER_WORKFLOW_ID: &str = "buddy_memory_gardener";
pub const KNOWLEDGE_CONFLICT_WORKFLOW_ID: &str = "buddy_knowledge_conflict_resolver";
pub const BEHAVIOR_LEARNER_WORKFLOW_ID: &str = "buddy_behavior_learner";
pub const USER_HABIT_COACH_WORKFLOW_ID: &str = "buddy_user_habit_coach";
pub const MODEL_COST_OPTIMIZER_WORKFLOW_ID: &str = "buddy_model_cost_optimizer";
pub const FINDING_PROPOSING_WORKFLOWS: &[&str] = &[
    REFACT_SELF_CRITIC_WORKFLOW_ID,
    BUDDY_REFACTOR_HUNTER_WORKFLOW_ID,
    DOCS_GARDENER_WORKFLOW_ID,
    BUDDY_TEST_COVERAGE_WATCHER_WORKFLOW_ID,
];

pub fn workflow_proposes_findings(workflow_id: &str) -> bool {
    FINDING_PROPOSING_WORKFLOWS.contains(&workflow_id)
}

pub const AUTONOMOUS_BUDDY_WORKFLOWS: &[AutonomousWorkflowMeta] = &[
    AutonomousWorkflowMeta {
        id: ERROR_DETECTIVE_WORKFLOW_ID,
        title: "Error Detective",
        icon: "🕵️",
        badge: "Error Detective",
        priority: "high",
        kind: "system",
    },
    AutonomousWorkflowMeta {
        id: REFACT_SELF_CRITIC_WORKFLOW_ID,
        title: "Refact Self-Critic",
        icon: "🪞",
        badge: "Self-Critic",
        priority: "normal",
        kind: "system",
    },
    AutonomousWorkflowMeta {
        id: REFACT_COMPILE_SNIFFER_WORKFLOW_ID,
        title: "Refact Compile Sniffer",
        icon: "🧯",
        badge: "Compile Sniffer",
        priority: "high",
        kind: "system",
    },
    AutonomousWorkflowMeta {
        id: BUDDY_DAILY_DIGEST_WORKFLOW_ID,
        title: "Daily Digest",
        icon: "🌇",
        badge: "Daily Digest",
        priority: "normal",
        kind: "system",
    },
    AutonomousWorkflowMeta {
        id: BUDDY_FRIDAY_RETRO_WORKFLOW_ID,
        title: "Friday Retro",
        icon: "🗓️",
        badge: "Friday Retro",
        priority: "normal",
        kind: "system",
    },
    AutonomousWorkflowMeta {
        id: BUDDY_IDLE_SUGGESTER_WORKFLOW_ID,
        title: "Idle Suggester",
        icon: "💡",
        badge: "Idle Suggester",
        priority: "normal",
        kind: "system",
    },
    AutonomousWorkflowMeta {
        id: BUDDY_ONBOARDING_WORKFLOW_ID,
        title: "Onboarding",
        icon: "🧭",
        badge: "Onboarding",
        priority: "normal",
        kind: "system",
    },
    AutonomousWorkflowMeta {
        id: BUDDY_PR_ISSUE_MATCHMAKER_WORKFLOW_ID,
        title: "PR/Issue Matchmaker",
        icon: "🔗",
        badge: "PR Matchmaker",
        priority: "normal",
        kind: "system",
    },
    AutonomousWorkflowMeta {
        id: BUDDY_REFACTOR_HUNTER_WORKFLOW_ID,
        title: "Refactor Hunter",
        icon: "🛠️",
        badge: "Refactor",
        priority: "normal",
        kind: "system",
    },
    AutonomousWorkflowMeta {
        id: BUDDY_SKILL_AUTHOR_WORKFLOW_ID,
        title: "Skill Author",
        icon: "✍️",
        badge: "Skills",
        priority: "normal",
        kind: "system",
    },
    AutonomousWorkflowMeta {
        id: BUDDY_TEST_COVERAGE_WATCHER_WORKFLOW_ID,
        title: "Test Coverage Watcher",
        icon: "🧪",
        badge: "Coverage",
        priority: "normal",
        kind: "system",
    },
    AutonomousWorkflowMeta {
        id: SECURITY_WHISPERER_WORKFLOW_ID,
        title: "Security Whisperer",
        icon: "🛡️",
        badge: "Security",
        priority: "critical",
        kind: "system",
    },
    AutonomousWorkflowMeta {
        id: SETUP_COACH_WORKFLOW_ID,
        title: "Setup Coach",
        icon: "🧰",
        badge: "Setup",
        priority: "normal",
        kind: "system",
    },
    AutonomousWorkflowMeta {
        id: DEPENDENCY_RADAR_WORKFLOW_ID,
        title: "Dependency Radar",
        icon: "📦",
        badge: "Dependencies",
        priority: "normal",
        kind: "system",
    },
    AutonomousWorkflowMeta {
        id: DOCS_GARDENER_WORKFLOW_ID,
        title: "Docs Gardener",
        icon: "📚",
        badge: "Docs",
        priority: "normal",
        kind: "system",
    },
    AutonomousWorkflowMeta {
        id: ARCHITECTURE_DRIFT_WORKFLOW_ID,
        title: "Architecture Drift Watcher",
        icon: "🏗️",
        badge: "Architecture",
        priority: "normal",
        kind: "system",
    },
    AutonomousWorkflowMeta {
        id: MEMORY_GARDENER_WORKFLOW_ID,
        title: "Memory Gardener",
        icon: "🌿",
        badge: "Memory",
        priority: "normal",
        kind: "system",
    },
    AutonomousWorkflowMeta {
        id: KNOWLEDGE_CONFLICT_WORKFLOW_ID,
        title: "Knowledge Conflict Resolver",
        icon: "🧩",
        badge: "Knowledge",
        priority: "normal",
        kind: "system",
    },
    AutonomousWorkflowMeta {
        id: BEHAVIOR_LEARNER_WORKFLOW_ID,
        title: "Behavior Learner",
        icon: "🧭",
        badge: "Preferences",
        priority: "normal",
        kind: "system",
    },
    AutonomousWorkflowMeta {
        id: USER_HABIT_COACH_WORKFLOW_ID,
        title: "User Habit Coach",
        icon: "🏃",
        badge: "Habits",
        priority: "normal",
        kind: "system",
    },
    AutonomousWorkflowMeta {
        id: MODEL_COST_OPTIMIZER_WORKFLOW_ID,
        title: "Model/Cost Optimizer",
        icon: "💸",
        badge: "Model/Cost",
        priority: "normal",
        kind: "system",
    },
];

pub fn autonomous_workflow_meta(id: &str) -> Option<&'static AutonomousWorkflowMeta> {
    AUTONOMOUS_BUDDY_WORKFLOWS.iter().find(|meta| meta.id == id)
}

pub fn is_autonomous_workflow_id(id: &str) -> bool {
    autonomous_workflow_meta(id).is_some()
}

pub const CHECK_SAVED_EVIDENCE_ACTION_ID: &str = "check_saved_evidence";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowActionFamily {
    MemoryLifecycle,
    DefaultModelSelect,
    BuddySettingsPatch,
    MarkdownProjectApply,
    CustomizationFileApply,
    IsolatedCandidateCheckpoint,
    SavedEvidenceOnly { why: &'static str },
}

impl WorkflowActionFamily {
    pub const fn resource(&self) -> Option<(&'static str, &'static str, &'static str)> {
        match self {
            WorkflowActionFamily::MemoryLifecycle => Some(("memory", "memory_lifecycle", "memory")),
            WorkflowActionFamily::DefaultModelSelect => {
                Some(("default_models", "default_model_select", "default_models"))
            }
            WorkflowActionFamily::BuddySettingsPatch => {
                Some(("buddy_settings", "buddy_settings_patch", "settings"))
            }
            WorkflowActionFamily::MarkdownProjectApply => {
                Some(("extensions", "markdown_project_apply", "native_markdown"))
            }
            WorkflowActionFamily::CustomizationFileApply => Some((
                "project_customization_file",
                "customization_file_apply",
                "project_customization_file",
            )),
            WorkflowActionFamily::IsolatedCandidateCheckpoint => {
                Some(("project_code", "checkpoint_isolated_candidate", "worktree"))
            }
            WorkflowActionFamily::SavedEvidenceOnly {
                ..
            } => None,
        }
    }

    pub const fn action_id(&self) -> Option<&'static str> {
        match self {
            WorkflowActionFamily::MemoryLifecycle => Some("memory:memory_lifecycle"),
            WorkflowActionFamily::DefaultModelSelect => Some("default_models:default_model_select"),
            WorkflowActionFamily::BuddySettingsPatch => Some("buddy_settings:buddy_settings_patch"),
            WorkflowActionFamily::MarkdownProjectApply => Some("extensions:markdown_project_apply"),
            WorkflowActionFamily::CustomizationFileApply => {
                Some("project_customization_file:customization_file_apply")
            }
            WorkflowActionFamily::IsolatedCandidateCheckpoint => {
                Some("project_code:checkpoint_isolated_candidate")
            }
            WorkflowActionFamily::SavedEvidenceOnly {
                ..
            } => None,
        }
    }

    pub fn from_action_id(action_id: &str) -> Option<Self> {
        [
            WorkflowActionFamily::MemoryLifecycle,
            WorkflowActionFamily::DefaultModelSelect,
            WorkflowActionFamily::BuddySettingsPatch,
            WorkflowActionFamily::MarkdownProjectApply,
            WorkflowActionFamily::CustomizationFileApply,
            WorkflowActionFamily::IsolatedCandidateCheckpoint,
        ]
        .into_iter()
        .find(|family| family.action_id() == Some(action_id))
    }
}

pub fn workflow_action_family(id: &str) -> WorkflowActionFamily {
    use WorkflowActionFamily::*;
    match id {
        MEMORY_GARDENER_WORKFLOW_ID | KNOWLEDGE_CONFLICT_WORKFLOW_ID => MemoryLifecycle,
        MODEL_COST_OPTIMIZER_WORKFLOW_ID => DefaultModelSelect,
        BEHAVIOR_LEARNER_WORKFLOW_ID | USER_HABIT_COACH_WORKFLOW_ID => BuddySettingsPatch,
        BUDDY_SKILL_AUTHOR_WORKFLOW_ID => MarkdownProjectApply,
        SETUP_COACH_WORKFLOW_ID | BUDDY_ONBOARDING_WORKFLOW_ID => CustomizationFileApply,
        REFACT_SELF_CRITIC_WORKFLOW_ID
        | REFACT_COMPILE_SNIFFER_WORKFLOW_ID
        | BUDDY_REFACTOR_HUNTER_WORKFLOW_ID
        | BUDDY_TEST_COVERAGE_WATCHER_WORKFLOW_ID
        | DOCS_GARDENER_WORKFLOW_ID
        | ARCHITECTURE_DRIFT_WORKFLOW_ID
        | SECURITY_WHISPERER_WORKFLOW_ID
        | DEPENDENCY_RADAR_WORKFLOW_ID => IsolatedCandidateCheckpoint,
        ERROR_DETECTIVE_WORKFLOW_ID => SavedEvidenceOnly {
            why: "a diagnostic cluster names no single resource to change",
        },
        BUDDY_DAILY_DIGEST_WORKFLOW_ID
        | BUDDY_FRIDAY_RETRO_WORKFLOW_ID
        | BUDDY_IDLE_SUGGESTER_WORKFLOW_ID
        | BUDDY_PR_ISSUE_MATCHMAKER_WORKFLOW_ID => SavedEvidenceOnly {
            why: "the deliverable is a summary or candidate list, not a change to a resource",
        },
        _ => SavedEvidenceOnly {
            why: "this workflow is not in the registered deliverable contract",
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkflowDeliverableContract {
    pub kind: &'static str,
    pub local_proposal: &'static str,
    pub coverage_limit: &'static str,
    pub action_ids: &'static [&'static str],
    pub family: WorkflowActionFamily,
}

const fn contract_action_ids(family: WorkflowActionFamily) -> &'static [&'static str] {
    match family {
        WorkflowActionFamily::MemoryLifecycle => {
            &[CHECK_SAVED_EVIDENCE_ACTION_ID, "memory:memory_lifecycle"]
        }
        WorkflowActionFamily::DefaultModelSelect => &[
            CHECK_SAVED_EVIDENCE_ACTION_ID,
            "default_models:default_model_select",
        ],
        WorkflowActionFamily::BuddySettingsPatch => &[
            CHECK_SAVED_EVIDENCE_ACTION_ID,
            "buddy_settings:buddy_settings_patch",
        ],
        WorkflowActionFamily::MarkdownProjectApply => &[
            CHECK_SAVED_EVIDENCE_ACTION_ID,
            "extensions:markdown_project_apply",
        ],
        WorkflowActionFamily::CustomizationFileApply => &[
            CHECK_SAVED_EVIDENCE_ACTION_ID,
            "project_customization_file:customization_file_apply",
        ],
        WorkflowActionFamily::IsolatedCandidateCheckpoint => &[
            CHECK_SAVED_EVIDENCE_ACTION_ID,
            "project_code:checkpoint_isolated_candidate",
        ],
        WorkflowActionFamily::SavedEvidenceOnly {
            ..
        } => &[CHECK_SAVED_EVIDENCE_ACTION_ID],
    }
}

pub fn workflow_deliverable_contract(id: &str) -> Option<WorkflowDeliverableContract> {
    let (kind, local_proposal, coverage_limit) = match id {
        ERROR_DETECTIVE_WORKFLOW_ID => (
            "diagnostic_assessment",
            "Smallest safe diagnostic checks and containment steps",
            "Diagnostic summaries only",
        ),
        REFACT_SELF_CRITIC_WORKFLOW_ID => (
            "quality_proposal",
            "Reproduction, proposed correction, and acceptance checks",
            "Observed traces only",
        ),
        REFACT_COMPILE_SNIFFER_WORKFLOW_ID => (
            "compile_assessment",
            "Failing command, affected scope, and proposed correction",
            "Retained compiler evidence only",
        ),
        BUDDY_DAILY_DIGEST_WORKFLOW_ID => (
            "daily_digest",
            "Decisions and next steps from retained outcomes",
            "Summary of observed activity; not independent verification",
        ),
        BUDDY_FRIDAY_RETRO_WORKFLOW_ID => (
            "retrospective",
            "Evidence-backed lessons and follow-up checklist",
            "Retained week activity only",
        ),
        BUDDY_IDLE_SUGGESTER_WORKFLOW_ID => (
            "research_brief",
            "Candidate next task, rationale, and bounded research plan",
            "Available project signals only; suggestions are not completed tasks",
        ),
        BUDDY_ONBOARDING_WORKFLOW_ID => (
            "onboarding_checklist",
            "Missing setup steps and local configuration proposal",
            "Observed setup only",
        ),
        BUDDY_PR_ISSUE_MATCHMAKER_WORKFLOW_ID => (
            "issue_match_proposal",
            "Candidate links with rationale; retain local draft when issue publication is disabled",
            "Observed issue and PR metadata only; no automatic publication",
        ),
        BUDDY_REFACTOR_HUNTER_WORKFLOW_ID => (
            "refactor_proposal",
            "Target symbols, behavior-preserving edit plan, and regression checks",
            "Observed code signals only; not an applied or tested refactor",
        ),
        BUDDY_SKILL_AUTHOR_WORKFLOW_ID => (
            "skill_draft",
            "Local skill content, trigger examples, and evaluation cases",
            "Draft only; not installed or evaluated",
        ),
        BUDDY_TEST_COVERAGE_WATCHER_WORKFLOW_ID => (
            "test_proposal",
            "Uncovered behavior, proposed test cases, and commands",
            "Available coverage evidence only; not newly executed tests",
        ),
        SECURITY_WHISPERER_WORKFLOW_ID => (
            "security_assessment",
            "Redacted affected paths, containment checklist, and safe remediation proposal",
            "Bounded local findings only; not a clean security audit",
        ),
        SETUP_COACH_WORKFLOW_ID => (
            "setup_proposal",
            "Local setup checklist and next configuration steps",
            "Presence checks only",
        ),
        DEPENDENCY_RADAR_WORKFLOW_ID => (
            "dependency_proposal",
            "Changed manifests, coordination checklist, and proposed compatibility checks",
            "Local manifest activity only; no registry freshness or vulnerability claim",
        ),
        DOCS_GARDENER_WORKFLOW_ID => (
            "documentation_proposal",
            "Affected documentation, suggested content, and source cross-checks",
            "Changed path and documentation signals only",
        ),
        ARCHITECTURE_DRIFT_WORKFLOW_ID => (
            "architecture_assessment",
            "Affected subsystems and proposed guardrails",
            "Path groups and diff statistics only; not semantic architecture verification",
        ),
        MEMORY_GARDENER_WORKFLOW_ID => (
            "memory_proposal",
            "Candidate memory IDs and proposed consolidation or cleanup",
            "Memory metadata only",
        ),
        KNOWLEDGE_CONFLICT_WORKFLOW_ID => (
            "knowledge_resolution_proposal",
            "Conflicting IDs, source questions, and proposed resolution",
            "Conflict summaries only",
        ),
        BEHAVIOR_LEARNER_WORKFLOW_ID => (
            "preference_proposal",
            "Directly supported preferences and review candidates",
            "User-authored snippets only; inferred preferences are not accepted",
        ),
        USER_HABIT_COACH_WORKFLOW_ID => (
            "habit_brief",
            "Observed friction and optional next-step checklist",
            "Aggregate metadata only; not a claim about user intent",
        ),
        MODEL_COST_OPTIMIZER_WORKFLOW_ID => (
            "model_cost_proposal",
            "Aggregate tradeoffs and proposed configuration comparison",
            "Aggregate usage only",
        ),
        _ => return None,
    };
    let family = workflow_action_family(id);
    Some(WorkflowDeliverableContract {
        kind,
        local_proposal,
        coverage_limit,
        action_ids: contract_action_ids(family),
        family,
    })
}
