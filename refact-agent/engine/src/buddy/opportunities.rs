use chrono::{DateTime, Duration, Utc};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use crate::buddy::types::{
    BuddyAction, BuddyFactKind, BuddyOpportunity, BuddyOpportunityKind, BuddyOpportunityLinks,
    BuddyPage, BuddyPriority, BuddyPulse, CustomizationKind, DefaultsKind, InvestigationContext,
    MarketKind, OpportunityStatus, PulseScope,
};

pub const MAX_OPPORTUNITIES: usize = 200;
pub const MAX_UNREAD: usize = 3;
pub const DISMISS_MEMORY: Duration = Duration::hours(24);
pub const DEFAULT_COOLDOWN: Duration = Duration::minutes(30);

/// Priority-ordered queue of `BuddyOpportunity` values with cooldown and dismissal tracking.
pub struct OpportunityQueue {
    items: Vec<BuddyOpportunity>,
    cooldowns: HashMap<String, DateTime<Utc>>,
    dismissed_history: HashMap<String, DateTime<Utc>>,
}

impl OpportunityQueue {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            cooldowns: HashMap::new(),
            dismissed_history: HashMap::new(),
        }
    }

    pub fn from_state(opps: Vec<BuddyOpportunity>) -> Self {
        let mut q = Self::new();
        let now = Utc::now();
        for opp in opps {
            let expires = opp.created_at + DEFAULT_COOLDOWN;
            if expires > now {
                q.cooldowns.insert(opp.cooldown_key.clone(), expires);
            }
            q.items.push(opp);
        }
        q
    }

    pub fn push(&mut self, opp: BuddyOpportunity) {
        let expires = Utc::now() + DEFAULT_COOLDOWN;
        self.cooldowns.insert(opp.cooldown_key.clone(), expires);
        self.items.push(opp);

        if self.items.len() > MAX_OPPORTUNITIES {
            let terminal = [
                OpportunityStatus::Expired,
                OpportunityStatus::Completed,
                OpportunityStatus::Dismissed,
            ];
            if let Some(pos) = self.items.iter().position(|o| terminal.contains(&o.status)) {
                self.items.remove(pos);
            } else if let Some(pos) = self
                .items
                .iter()
                .enumerate()
                .min_by_key(|(_, o)| o.created_at)
                .map(|(i, _)| i)
            {
                self.items.remove(pos);
            }
        }
    }

    pub fn unread_count(&self) -> usize {
        self.items
            .iter()
            .filter(|o| matches!(o.status, OpportunityStatus::New | OpportunityStatus::Shown))
            .count()
    }

    pub fn cooldown_active(&self, key: &str) -> bool {
        self.cooldowns
            .get(key)
            .map(|&exp| exp > Utc::now())
            .unwrap_or(false)
    }

    pub fn recently_dismissed(&self, key: &str, window: Duration) -> bool {
        let cutoff = Utc::now() - window;
        self.dismissed_history
            .get(key)
            .map(|&t| t >= cutoff)
            .unwrap_or(false)
    }

    pub fn mark_status(&mut self, id: &str, status: OpportunityStatus) {
        if let Some(opp) = self.items.iter_mut().find(|o| o.id == id) {
            opp.status = status;
        }
    }

    pub fn dismiss(&mut self, id: &str) {
        if let Some(opp) = self.items.iter_mut().find(|o| o.id == id) {
            opp.status = OpportunityStatus::Dismissed;
            self.dismissed_history
                .insert(opp.cooldown_key.clone(), Utc::now());
        }
    }

    pub fn expire_old(&mut self, now: DateTime<Utc>) {
        let terminal = [
            OpportunityStatus::Expired,
            OpportunityStatus::Completed,
            OpportunityStatus::Dismissed,
        ];
        for opp in self.items.iter_mut() {
            if opp.expires_at <= now && !terminal.contains(&opp.status) {
                opp.status = OpportunityStatus::Expired;
            }
        }
        let cutoff = now - Duration::hours(24);
        self.items
            .retain(|o| !(terminal.contains(&o.status) && o.created_at < cutoff));
    }

    pub fn refresh_cooldowns(&mut self, now: DateTime<Utc>) {
        self.cooldowns.retain(|_, exp| *exp > now);
    }

    pub fn iter(&self) -> impl Iterator<Item = &BuddyOpportunity> {
        self.items.iter()
    }

    pub fn snapshot(&self) -> Vec<BuddyOpportunity> {
        self.items.clone()
    }

    pub fn get(&self, id: &str) -> Option<&BuddyOpportunity> {
        self.items.iter().find(|o| o.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut BuddyOpportunity> {
        self.items.iter_mut().find(|o| o.id == id)
    }
}

impl Default for OpportunityQueue {
    fn default() -> Self {
        Self::new()
    }
}

struct Rule {
    name: &'static str,
    cooldown_secs: u64,
    build: fn(
        &crate::buddy::facts::FactStore,
        &BuddyPulse,
        &OpportunityQueue,
        DateTime<Utc>,
    ) -> Vec<BuddyOpportunity>,
}

const RULES: &[Rule] = &[
    Rule {
        name: "task_stuck",
        cooldown_secs: 3600,
        build: rules::task_stuck,
    },
    Rule {
        name: "task_abandoned",
        cooldown_secs: 21600,
        build: rules::task_abandoned,
    },
    Rule {
        name: "trajectory_clutter",
        cooldown_secs: 43200,
        build: rules::trajectory_cleanup,
    },
    Rule {
        name: "workflow_distill",
        cooldown_secs: 86400,
        build: rules::workflow_distill,
    },
    Rule {
        name: "default_model_missing",
        cooldown_secs: 7200,
        build: rules::provider_tuning_missing,
    },
    Rule {
        name: "broken_model_ref",
        cooldown_secs: 7200,
        build: rules::provider_tuning_broken_ref,
    },
    Rule {
        name: "memory_garden",
        cooldown_secs: 43200,
        build: rules::memory_garden,
    },
    Rule {
        name: "diagnostic_cluster",
        cooldown_secs: 1800,
        build: rules::diagnostic_investigation,
    },
    Rule {
        name: "frontend_error_burst",
        cooldown_secs: 900,
        build: rules::diagnostic_investigation_frontend,
    },
    Rule {
        name: "git_pressure",
        cooldown_secs: 14400,
        build: rules::git_hygiene,
    },
    Rule {
        name: "git_widening",
        cooldown_secs: 7200,
        build: rules::git_hygiene_widening,
    },
    Rule {
        name: "mode_prompt_overlap",
        cooldown_secs: 86400,
        build: rules::config_drift_mode_overlap,
    },
    Rule {
        name: "skill_underused",
        cooldown_secs: 172800,
        build: rules::config_drift_skill_underused,
    },
    Rule {
        name: "skill_trigger_weak",
        cooldown_secs: 172800,
        build: rules::config_drift_skill_trigger,
    },
    Rule {
        name: "agents_md_gap",
        cooldown_secs: 259200,
        build: rules::agents_md_gap,
    },
    Rule {
        name: "mcp_auth_expired",
        cooldown_secs: 7200,
        build: rules::integration_mcp_auth,
    },
    Rule {
        name: "integration_failing",
        cooldown_secs: 7200,
        build: rules::integration_failing,
    },
    Rule {
        name: "smartlink_match",
        cooldown_secs: 3600,
        build: rules::integration_smartlink,
    },
    Rule {
        name: "chat_recap",
        cooldown_secs: 14400,
        build: rules::chat_recap_retry_streak,
    },
];

mod rules {
    use super::*;

    fn opp(
        kind: BuddyOpportunityKind,
        summary: impl Into<String>,
        priority: BuddyPriority,
        confidence: f32,
        fact_keys: Vec<String>,
        cooldown_key: impl Into<String>,
        actions: Vec<BuddyAction>,
        now: DateTime<Utc>,
    ) -> BuddyOpportunity {
        BuddyOpportunity {
            id: Uuid::new_v4().to_string(),
            kind,
            summary: summary.into(),
            priority,
            confidence,
            fact_keys,
            cooldown_key: cooldown_key.into(),
            status: OpportunityStatus::New,
            proposed_actions: actions,
            humor: None,
            humor_allowed: false,
            related: BuddyOpportunityLinks::default(),
            created_at: now,
            expires_at: now + Duration::hours(24),
        }
    }

    pub fn task_stuck(
        store: &crate::buddy::facts::FactStore,
        _pulse: &BuddyPulse,
        _queue: &OpportunityQueue,
        now: DateTime<Utc>,
    ) -> Vec<BuddyOpportunity> {
        store
            .recent(BuddyFactKind::TaskStuck, Duration::hours(2))
            .into_iter()
            .map(|fact| {
                let task_id = fact
                    .payload
                    .get("task_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                opp(
                    BuddyOpportunityKind::TaskHealth,
                    format!("Task stuck: {}", task_id),
                    BuddyPriority::High,
                    fact.confidence,
                    vec![fact.key.clone()],
                    format!("task_health:stuck:{}", task_id),
                    vec![
                        BuddyAction::OpenPage {
                            page: BuddyPage::TaskWorkspace { task_id },
                            params: None,
                        },
                        BuddyAction::Dismiss,
                    ],
                    now,
                )
            })
            .collect()
    }

    pub fn task_abandoned(
        store: &crate::buddy::facts::FactStore,
        _pulse: &BuddyPulse,
        _queue: &OpportunityQueue,
        now: DateTime<Utc>,
    ) -> Vec<BuddyOpportunity> {
        store
            .recent(BuddyFactKind::TaskAbandoned, Duration::days(2))
            .into_iter()
            .map(|fact| {
                let task_id = fact
                    .payload
                    .get("task_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                opp(
                    BuddyOpportunityKind::TaskHealth,
                    "Abandoned task needs review",
                    BuddyPriority::Normal,
                    fact.confidence,
                    vec![fact.key.clone()],
                    format!("task_health:abandoned:{}", task_id),
                    vec![
                        BuddyAction::OpenPage {
                            page: BuddyPage::TasksList,
                            params: None,
                        },
                        BuddyAction::Dismiss,
                    ],
                    now,
                )
            })
            .collect()
    }

    pub fn trajectory_cleanup(
        store: &crate::buddy::facts::FactStore,
        _pulse: &BuddyPulse,
        _queue: &OpportunityQueue,
        now: DateTime<Utc>,
    ) -> Vec<BuddyOpportunity> {
        store
            .recent(BuddyFactKind::TrajectoryClutter, Duration::hours(12))
            .into_iter()
            .map(|fact| {
                opp(
                    BuddyOpportunityKind::TrajectoryCleanup,
                    "Too many chat trajectories",
                    BuddyPriority::Normal,
                    fact.confidence,
                    vec![fact.key.clone()],
                    format!("trajectory:cleanup:{}", &fact.key),
                    vec![
                        BuddyAction::CreatePulseReport {
                            scope: PulseScope::Trajectories,
                        },
                        BuddyAction::Dismiss,
                    ],
                    now,
                )
            })
            .collect()
    }

    pub fn workflow_distill(
        store: &crate::buddy::facts::FactStore,
        _pulse: &BuddyPulse,
        _queue: &OpportunityQueue,
        now: DateTime<Utc>,
    ) -> Vec<BuddyOpportunity> {
        store
            .recent(
                BuddyFactKind::RecurringWorkflowCandidate,
                Duration::hours(24),
            )
            .into_iter()
            .map(|fact| {
                opp(
                    BuddyOpportunityKind::WorkflowDistill,
                    "Recurring pattern could become a workflow",
                    BuddyPriority::Normal,
                    fact.confidence,
                    vec![fact.key.clone()],
                    format!("workflow_distill:{}", &fact.key),
                    vec![
                        BuddyAction::DraftSkill {
                            draft_id: String::new(),
                            label: "Create Skill".to_string(),
                        },
                        BuddyAction::DraftCommand {
                            draft_id: String::new(),
                            label: "Create Command".to_string(),
                        },
                        BuddyAction::DraftSubagent {
                            draft_id: String::new(),
                            label: "Create Subagent".to_string(),
                        },
                        BuddyAction::Dismiss,
                    ],
                    now,
                )
            })
            .collect()
    }

    pub fn provider_tuning_missing(
        store: &crate::buddy::facts::FactStore,
        _pulse: &BuddyPulse,
        _queue: &OpportunityQueue,
        now: DateTime<Utc>,
    ) -> Vec<BuddyOpportunity> {
        store
            .recent(BuddyFactKind::DefaultModelMissing, Duration::hours(6))
            .into_iter()
            .take(1)
            .map(|fact| {
                opp(
                    BuddyOpportunityKind::ProviderTuning,
                    "Default model not configured",
                    BuddyPriority::High,
                    fact.confidence,
                    vec![fact.key.clone()],
                    "provider:default_model_missing",
                    vec![
                        BuddyAction::OpenPage {
                            page: BuddyPage::DefaultModels,
                            params: None,
                        },
                        BuddyAction::DraftDefaultsChange {
                            defaults_kind: DefaultsKind::ChatModel,
                            patch: serde_json::json!({}),
                        },
                        BuddyAction::Dismiss,
                    ],
                    now,
                )
            })
            .collect()
    }

    pub fn provider_tuning_broken_ref(
        store: &crate::buddy::facts::FactStore,
        _pulse: &BuddyPulse,
        _queue: &OpportunityQueue,
        now: DateTime<Utc>,
    ) -> Vec<BuddyOpportunity> {
        store
            .recent(BuddyFactKind::BrokenModelReference, Duration::hours(6))
            .into_iter()
            .take(1)
            .map(|fact| {
                let model = fact
                    .payload
                    .get("model_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                opp(
                    BuddyOpportunityKind::ProviderTuning,
                    format!("Model not available: {}", model),
                    BuddyPriority::High,
                    fact.confidence,
                    vec![fact.key.clone()],
                    format!("provider:broken_ref:{}", model),
                    vec![
                        BuddyAction::OpenPage {
                            page: BuddyPage::DefaultModels,
                            params: None,
                        },
                        BuddyAction::Dismiss,
                    ],
                    now,
                )
            })
            .collect()
    }

    pub fn memory_garden(
        store: &crate::buddy::facts::FactStore,
        _pulse: &BuddyPulse,
        _queue: &OpportunityQueue,
        now: DateTime<Utc>,
    ) -> Vec<BuddyOpportunity> {
        let kinds = [
            BuddyFactKind::MemoryOrphan,
            BuddyFactKind::MemoryStaleConflict,
            BuddyFactKind::MemoryRecurringLesson,
        ];
        let fact_keys: Vec<String> = kinds
            .iter()
            .flat_map(|k| store.recent(*k, Duration::hours(24)))
            .map(|f| f.key.clone())
            .collect();
        if fact_keys.is_empty() {
            return vec![];
        }
        vec![opp(
            BuddyOpportunityKind::MemoryGarden,
            "Knowledge base needs attention",
            BuddyPriority::Normal,
            0.8,
            fact_keys,
            "memory:garden:global",
            vec![
                BuddyAction::OpenPage {
                    page: BuddyPage::KnowledgeGraph,
                    params: None,
                },
                BuddyAction::CreatePulseReport {
                    scope: PulseScope::Memory,
                },
                BuddyAction::Dismiss,
            ],
            now,
        )]
    }

    pub fn diagnostic_investigation(
        store: &crate::buddy::facts::FactStore,
        _pulse: &BuddyPulse,
        _queue: &OpportunityQueue,
        now: DateTime<Utc>,
    ) -> Vec<BuddyOpportunity> {
        store
            .recent(BuddyFactKind::DiagnosticCluster, Duration::hours(1))
            .into_iter()
            .map(|fact| {
                let error_type = fact
                    .payload
                    .get("error_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("error")
                    .to_string();
                opp(
                    BuddyOpportunityKind::DiagnosticInvestigation,
                    format!("Repeated errors: {}", error_type),
                    BuddyPriority::High,
                    fact.confidence,
                    vec![fact.key.clone()],
                    format!("diag:cluster:{}", error_type),
                    vec![
                        BuddyAction::LaunchInvestigationChat {
                            preload: InvestigationContext {
                                fact_keys: vec![fact.key.clone()],
                                diagnostic_ids: vec![],
                                log_excerpt: String::new(),
                                config_summary: String::new(),
                                initial_user_message: format!(
                                    "Investigate repeated {} errors",
                                    error_type
                                ),
                            },
                        },
                        BuddyAction::OpenPage {
                            page: BuddyPage::Buddy,
                            params: None,
                        },
                        BuddyAction::Dismiss,
                    ],
                    now,
                )
            })
            .collect()
    }

    pub fn diagnostic_investigation_frontend(
        store: &crate::buddy::facts::FactStore,
        _pulse: &BuddyPulse,
        _queue: &OpportunityQueue,
        now: DateTime<Utc>,
    ) -> Vec<BuddyOpportunity> {
        store
            .recent(BuddyFactKind::FrontendErrorBurst, Duration::minutes(30))
            .into_iter()
            .take(1)
            .map(|fact| {
                opp(
                    BuddyOpportunityKind::DiagnosticInvestigation,
                    "Frontend error burst detected",
                    BuddyPriority::High,
                    fact.confidence,
                    vec![fact.key.clone()],
                    "diag:fe_burst:global",
                    vec![
                        BuddyAction::LaunchInvestigationChat {
                            preload: InvestigationContext {
                                fact_keys: vec![fact.key.clone()],
                                diagnostic_ids: vec![],
                                log_excerpt: String::new(),
                                config_summary: String::new(),
                                initial_user_message: "Investigate frontend error burst"
                                    .to_string(),
                            },
                        },
                        BuddyAction::OpenPage {
                            page: BuddyPage::Buddy,
                            params: None,
                        },
                        BuddyAction::Dismiss,
                    ],
                    now,
                )
            })
            .collect()
    }

    pub fn git_hygiene(
        store: &crate::buddy::facts::FactStore,
        _pulse: &BuddyPulse,
        _queue: &OpportunityQueue,
        now: DateTime<Utc>,
    ) -> Vec<BuddyOpportunity> {
        store
            .recent(BuddyFactKind::UncommittedPressure, Duration::hours(4))
            .into_iter()
            .take(1)
            .map(|fact| {
                opp(
                    BuddyOpportunityKind::GitHygiene,
                    "Many uncommitted changes",
                    BuddyPriority::Normal,
                    fact.confidence,
                    vec![fact.key.clone()],
                    "git:uncommitted:global",
                    vec![
                        BuddyAction::OpenPage {
                            page: BuddyPage::Stats,
                            params: None,
                        },
                        BuddyAction::Dismiss,
                    ],
                    now,
                )
            })
            .collect()
    }

    pub fn git_hygiene_widening(
        store: &crate::buddy::facts::FactStore,
        _pulse: &BuddyPulse,
        _queue: &OpportunityQueue,
        now: DateTime<Utc>,
    ) -> Vec<BuddyOpportunity> {
        store
            .recent(BuddyFactKind::GitDiffWidening, Duration::hours(4))
            .into_iter()
            .take(1)
            .map(|fact| {
                opp(
                    BuddyOpportunityKind::GitHygiene,
                    "Diff growing fast",
                    BuddyPriority::Normal,
                    fact.confidence,
                    vec![fact.key.clone()],
                    "git:widening:global",
                    vec![
                        BuddyAction::OpenPage {
                            page: BuddyPage::Stats,
                            params: None,
                        },
                        BuddyAction::Dismiss,
                    ],
                    now,
                )
            })
            .collect()
    }

    pub fn config_drift_mode_overlap(
        store: &crate::buddy::facts::FactStore,
        _pulse: &BuddyPulse,
        _queue: &OpportunityQueue,
        now: DateTime<Utc>,
    ) -> Vec<BuddyOpportunity> {
        store
            .recent(BuddyFactKind::ModePromptOverlap, Duration::hours(24))
            .into_iter()
            .take(1)
            .map(|fact| {
                let id = fact
                    .payload
                    .get("mode_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                opp(
                    BuddyOpportunityKind::ConfigDrift,
                    "Mode prompts are overlapping",
                    BuddyPriority::Normal,
                    fact.confidence,
                    vec![fact.key.clone()],
                    format!("config_drift:mode_overlap:{}", id),
                    vec![
                        BuddyAction::OpenPage {
                            page: BuddyPage::Customization,
                            params: None,
                        },
                        BuddyAction::DraftCustomizationChange {
                            customization_kind: CustomizationKind::Mode,
                            id: id.clone(),
                            patch: serde_json::json!({}),
                        },
                        BuddyAction::Dismiss,
                    ],
                    now,
                )
            })
            .collect()
    }

    pub fn config_drift_skill_underused(
        store: &crate::buddy::facts::FactStore,
        _pulse: &BuddyPulse,
        _queue: &OpportunityQueue,
        now: DateTime<Utc>,
    ) -> Vec<BuddyOpportunity> {
        store
            .recent(BuddyFactKind::SkillUnderused, Duration::hours(48))
            .into_iter()
            .take(1)
            .map(|fact| {
                opp(
                    BuddyOpportunityKind::ConfigDrift,
                    "A skill is underused",
                    BuddyPriority::Normal,
                    fact.confidence,
                    vec![fact.key.clone()],
                    format!("config_drift:skill_underused:{}", &fact.key),
                    vec![
                        BuddyAction::OpenPage {
                            page: BuddyPage::Customization,
                            params: None,
                        },
                        BuddyAction::Dismiss,
                    ],
                    now,
                )
            })
            .collect()
    }

    pub fn config_drift_skill_trigger(
        store: &crate::buddy::facts::FactStore,
        _pulse: &BuddyPulse,
        _queue: &OpportunityQueue,
        now: DateTime<Utc>,
    ) -> Vec<BuddyOpportunity> {
        store
            .recent(BuddyFactKind::SkillTriggerWeak, Duration::hours(48))
            .into_iter()
            .take(1)
            .map(|fact| {
                let id = fact
                    .payload
                    .get("skill_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                opp(
                    BuddyOpportunityKind::ConfigDrift,
                    "Skill has weak trigger description",
                    BuddyPriority::Normal,
                    fact.confidence,
                    vec![fact.key.clone()],
                    format!("config_drift:skill_trigger:{}", id),
                    vec![
                        BuddyAction::OpenPage {
                            page: BuddyPage::Customization,
                            params: None,
                        },
                        BuddyAction::DraftCustomizationChange {
                            customization_kind: CustomizationKind::Skill,
                            id,
                            patch: serde_json::json!({}),
                        },
                        BuddyAction::Dismiss,
                    ],
                    now,
                )
            })
            .collect()
    }

    pub fn agents_md_gap(
        store: &crate::buddy::facts::FactStore,
        _pulse: &BuddyPulse,
        _queue: &OpportunityQueue,
        now: DateTime<Utc>,
    ) -> Vec<BuddyOpportunity> {
        store
            .recent(BuddyFactKind::AgentsMdGapDetected, Duration::hours(72))
            .into_iter()
            .take(1)
            .map(|fact| {
                opp(
                    BuddyOpportunityKind::AgentsMdGap,
                    "AGENTS.md missing or outdated",
                    BuddyPriority::Normal,
                    fact.confidence,
                    vec![fact.key.clone()],
                    "agents_md:gap:global",
                    vec![
                        BuddyAction::DraftAgentsMdPatch {
                            diff: String::new(),
                        },
                        BuddyAction::Dismiss,
                    ],
                    now,
                )
            })
            .collect()
    }

    pub fn integration_mcp_auth(
        store: &crate::buddy::facts::FactStore,
        _pulse: &BuddyPulse,
        _queue: &OpportunityQueue,
        now: DateTime<Utc>,
    ) -> Vec<BuddyOpportunity> {
        store
            .recent(BuddyFactKind::McpAuthExpired, Duration::hours(6))
            .into_iter()
            .map(|fact| {
                let id = fact
                    .payload
                    .get("mcp_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                opp(
                    BuddyOpportunityKind::IntegrationFix,
                    format!("MCP auth expiring: {}", id),
                    BuddyPriority::High,
                    fact.confidence,
                    vec![fact.key.clone()],
                    format!("integration:mcp_auth:{}", id),
                    vec![
                        BuddyAction::OpenPage {
                            page: BuddyPage::Integrations,
                            params: None,
                        },
                        BuddyAction::Dismiss,
                    ],
                    now,
                )
            })
            .collect()
    }

    pub fn integration_failing(
        store: &crate::buddy::facts::FactStore,
        _pulse: &BuddyPulse,
        _queue: &OpportunityQueue,
        now: DateTime<Utc>,
    ) -> Vec<BuddyOpportunity> {
        store
            .recent(BuddyFactKind::IntegrationFailing, Duration::hours(4))
            .into_iter()
            .map(|fact| {
                let id = fact
                    .payload
                    .get("mcp_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                opp(
                    BuddyOpportunityKind::IntegrationFix,
                    format!("Integration failing: {}", id),
                    BuddyPriority::Normal,
                    fact.confidence,
                    vec![fact.key.clone()],
                    format!("integration:failing:{}", id),
                    vec![
                        BuddyAction::OpenPage {
                            page: BuddyPage::Integrations,
                            params: None,
                        },
                        BuddyAction::Dismiss,
                    ],
                    now,
                )
            })
            .collect()
    }

    pub fn integration_smartlink(
        store: &crate::buddy::facts::FactStore,
        _pulse: &BuddyPulse,
        _queue: &OpportunityQueue,
        now: DateTime<Utc>,
    ) -> Vec<BuddyOpportunity> {
        store
            .recent(BuddyFactKind::IntegrationSmartlinkMatch, Duration::hours(4))
            .into_iter()
            .map(|fact| {
                let item_id = fact
                    .payload
                    .get("smartlink_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                opp(
                    BuddyOpportunityKind::MarketplaceSuggestion,
                    "Integration available in marketplace",
                    BuddyPriority::Normal,
                    fact.confidence,
                    vec![fact.key.clone()],
                    format!("integration:smartlink:{}", &fact.key),
                    vec![
                        BuddyAction::OfferMarketplaceInstall {
                            market_kind: MarketKind::Mcp,
                            item_id,
                        },
                        BuddyAction::Dismiss,
                    ],
                    now,
                )
            })
            .collect()
    }

    pub fn chat_recap_retry_streak(
        store: &crate::buddy::facts::FactStore,
        _pulse: &BuddyPulse,
        _queue: &OpportunityQueue,
        now: DateTime<Utc>,
    ) -> Vec<BuddyOpportunity> {
        store
            .recent(BuddyFactKind::ChatRetryStreak, Duration::hours(4))
            .into_iter()
            .map(|fact| {
                let chat_id = fact
                    .payload
                    .get("chat_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                opp(
                    BuddyOpportunityKind::ChatRecap,
                    "Chat seems to be going in circles",
                    BuddyPriority::Normal,
                    fact.confidence,
                    vec![fact.key.clone()],
                    format!("chat_recap:retry:{}", chat_id),
                    vec![
                        BuddyAction::LaunchInvestigationChat {
                            preload: InvestigationContext {
                                fact_keys: vec![fact.key.clone()],
                                diagnostic_ids: vec![],
                                log_excerpt: String::new(),
                                config_summary: String::new(),
                                initial_user_message: "Help me break out of this chat loop"
                                    .to_string(),
                            },
                        },
                        BuddyAction::Dismiss,
                    ],
                    now,
                )
            })
            .collect()
    }
}

pub struct OpportunityDetector;

impl OpportunityDetector {
    pub fn new() -> Self {
        Self
    }

    pub fn detect(
        &self,
        fact_store: &crate::buddy::facts::FactStore,
        pulse: &BuddyPulse,
        queue: &OpportunityQueue,
    ) -> Vec<BuddyOpportunity> {
        let now = Utc::now();
        let mut seen: HashSet<String> = HashSet::new();
        let mut result = vec![];

        for rule in RULES {
            let _ = rule.name;
            let _ = rule.cooldown_secs;
            let candidates = (rule.build)(fact_store, pulse, queue, now);
            for opp in candidates {
                if seen.contains(&opp.cooldown_key) {
                    continue;
                }
                if queue.cooldown_active(&opp.cooldown_key) {
                    continue;
                }
                seen.insert(opp.cooldown_key.clone());
                result.push(opp);
            }
        }

        result
    }
}

impl Default for OpportunityDetector {
    fn default() -> Self {
        Self::new()
    }
}

/// Map an opportunity kind to the primary fact kind that drives it (used for humor attachment).
pub fn primary_fact_kind_for_opportunity(opp: &BuddyOpportunity) -> BuddyFactKind {
    match opp.kind {
        BuddyOpportunityKind::TaskHealth => BuddyFactKind::TaskStuck,
        BuddyOpportunityKind::TrajectoryCleanup => BuddyFactKind::TrajectoryClutter,
        BuddyOpportunityKind::ChatRecap => BuddyFactKind::ChatRetryStreak,
        BuddyOpportunityKind::MemoryGarden => BuddyFactKind::MemoryOrphan,
        BuddyOpportunityKind::ConfigDrift => BuddyFactKind::ModePromptOverlap,
        BuddyOpportunityKind::WorkflowDistill => BuddyFactKind::RecurringWorkflowCandidate,
        BuddyOpportunityKind::AgentsMdGap => BuddyFactKind::AgentsMdGapDetected,
        BuddyOpportunityKind::ProviderTuning => BuddyFactKind::DefaultModelMissing,
        BuddyOpportunityKind::IntegrationFix => BuddyFactKind::McpAuthExpired,
        BuddyOpportunityKind::DiagnosticInvestigation => BuddyFactKind::DiagnosticCluster,
        BuddyOpportunityKind::GitHygiene => BuddyFactKind::UncommittedPressure,
        BuddyOpportunityKind::MarketplaceSuggestion => BuddyFactKind::IntegrationSmartlinkMatch,
    }
}
