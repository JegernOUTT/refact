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
