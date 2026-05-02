#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutonomousWorkflowMeta {
    pub id: &'static str,
    pub title: &'static str,
    pub icon: &'static str,
    pub badge: &'static str,
    pub priority: &'static str,
    pub kind: &'static str,
}

pub const ERROR_DETECTIVE_WORKFLOW_ID: &str = "buddy_error_detective";
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
