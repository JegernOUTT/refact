use chrono::{DateTime, Duration, Utc};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use crate::buddy::types::{
    BuddyAction, BuddyFactKind, BuddyOpportunity, BuddyOpportunityKind, BuddyOpportunityLinks,
    BuddyPage, BuddyPriority, BuddyPulse, CustomizationKind, DefaultsKind, DismissEntry,
    InvestigationContext, OpportunityStatus, PulseScope,
};

pub const MAX_OPPORTUNITIES: usize = 200;
pub const MAX_UNREAD: usize = 3;
pub const DISMISS_MEMORY: Duration = Duration::hours(24);
pub const DEFAULT_COOLDOWN: Duration = Duration::minutes(30);

pub fn is_terminal_status(status: OpportunityStatus) -> bool {
    matches!(
        status,
        OpportunityStatus::Dismissed
            | OpportunityStatus::Accepted
            | OpportunityStatus::Completed
            | OpportunityStatus::Expired
    )
}

/// Priority-ordered queue of `BuddyOpportunity` values with cooldown and dismissal tracking.
pub struct OpportunityQueue {
    pub(crate) items: Vec<BuddyOpportunity>,
    pub(crate) cooldowns: HashMap<String, DateTime<Utc>>,
    pub(crate) dismissed_history: HashMap<String, DateTime<Utc>>,
}

impl OpportunityQueue {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            cooldowns: HashMap::new(),
            dismissed_history: HashMap::new(),
        }
    }

    pub fn from_state(opps: Vec<BuddyOpportunity>, dismissed: Vec<DismissEntry>) -> Self {
        let mut q = Self::new();
        let now = Utc::now();
        let dismissed_cutoff = now - DISMISS_MEMORY;
        for entry in dismissed {
            if entry.dismissed_at >= dismissed_cutoff {
                q.dismissed_history
                    .insert(entry.cooldown_key, entry.dismissed_at);
            }
        }
        for opp in opps {
            let expires = opp.created_at + Duration::seconds(opp.cooldown_secs as i64);
            if expires > now {
                q.cooldowns.insert(opp.cooldown_key.clone(), expires);
            }
            q.items.push(opp);
            q.cap_items();
        }
        q
    }

    fn cap_items(&mut self) {
        while self.items.len() > MAX_OPPORTUNITIES {
            if let Some(pos) = self.items.iter().position(|o| is_terminal_status(o.status)) {
                self.items.remove(pos);
            } else if let Some(pos) = self
                .items
                .iter()
                .enumerate()
                .min_by_key(|(_, o)| o.created_at)
                .map(|(i, _)| i)
            {
                self.items.remove(pos);
            } else {
                break;
            }
        }
    }

    pub fn push_with_cooldown(&mut self, mut opp: BuddyOpportunity, cooldown_secs: u64) {
        opp.cooldown_secs = cooldown_secs;
        let expires = Utc::now() + Duration::seconds(cooldown_secs as i64);
        self.cooldowns.insert(opp.cooldown_key.clone(), expires);
        self.items.push(opp);
        self.cap_items();
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

    pub fn mark_status(&mut self, id: &str, status: OpportunityStatus) -> bool {
        let Some(opp) = self.items.iter_mut().find(|o| o.id == id) else {
            return false;
        };
        let mut changed = false;
        if opp.status != status {
            opp.status = status;
            changed = true;
        }
        if is_terminal_status(status) && opp.resolved_at.is_none() {
            opp.resolved_at = Some(Utc::now());
            changed = true;
        }
        changed
    }

    pub fn dismiss(&mut self, id: &str) -> bool {
        let Some(opp) = self.items.iter_mut().find(|o| o.id == id) else {
            return false;
        };
        if opp.status == OpportunityStatus::Dismissed
            && opp.resolved_at.is_some()
            && self.dismissed_history.contains_key(&opp.cooldown_key)
        {
            return false;
        }
        let now = Utc::now();
        let mut changed = false;
        if opp.status != OpportunityStatus::Dismissed {
            opp.status = OpportunityStatus::Dismissed;
            changed = true;
        }
        if opp.resolved_at.is_none() {
            opp.resolved_at = Some(now);
            changed = true;
        }
        if self.dismissed_history.insert(opp.cooldown_key.clone(), now) != Some(now) {
            changed = true;
        }
        changed
    }

    pub fn expire_old(&mut self, now: DateTime<Utc>) -> bool {
        let mut changed = false;
        for opp in self.items.iter_mut() {
            if opp.expires_at <= now && !is_terminal_status(opp.status) {
                opp.status = OpportunityStatus::Expired;
                opp.resolved_at.get_or_insert(now);
                changed = true;
            }
        }
        let cutoff = now - DISMISS_MEMORY;
        let before_items = self.items.len();
        self.items.retain(|o| {
            if !is_terminal_status(o.status) {
                return true;
            }
            let terminal_since = o.resolved_at.unwrap_or(o.created_at);
            terminal_since >= cutoff
        });
        changed |= self.items.len() != before_items;
        let before_history = self.dismissed_history.len();
        self.dismissed_history
            .retain(|_, dismissed_at| *dismissed_at >= cutoff);
        changed |= self.dismissed_history.len() != before_history;
        changed
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

    pub fn dismissed_history_snapshot(&self) -> Vec<DismissEntry> {
        self.dismissed_history
            .iter()
            .map(|(k, v)| DismissEntry {
                cooldown_key: k.clone(),
                dismissed_at: *v,
            })
            .collect()
    }
}

impl Default for OpportunityQueue {
    fn default() -> Self {
        Self::new()
    }
}

struct Rule {
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
        cooldown_secs: 3600,
        build: rules::task_stuck,
    },
    Rule {
        cooldown_secs: 21600,
        build: rules::task_abandoned,
    },
    Rule {
        cooldown_secs: 21600,
        build: rules::task_cluster_duplicate,
    },
    Rule {
        cooldown_secs: 43200,
        build: rules::trajectory_cleanup,
    },
    Rule {
        cooldown_secs: 7200,
        build: rules::provider_tuning_missing,
    },
    Rule {
        cooldown_secs: 7200,
        build: rules::provider_tuning_broken_ref,
    },
    Rule {
        cooldown_secs: 43200,
        build: rules::memory_garden,
    },
    Rule {
        cooldown_secs: 1800,
        build: rules::diagnostic_investigation,
    },
    Rule {
        cooldown_secs: 900,
        build: rules::diagnostic_investigation_frontend,
    },
    Rule {
        cooldown_secs: 14400,
        build: rules::git_hygiene,
    },
    Rule {
        cooldown_secs: 7200,
        build: rules::git_hygiene_widening,
    },
    Rule {
        cooldown_secs: 86400,
        build: rules::config_drift_mode_overlap,
    },
    Rule {
        cooldown_secs: 172800,
        build: rules::config_drift_skill_trigger,
    },
    Rule {
        cooldown_secs: 259200,
        build: rules::agents_md_gap,
    },
    Rule {
        cooldown_secs: 7200,
        build: rules::integration_mcp_auth,
    },
    Rule {
        cooldown_secs: 7200,
        build: rules::integration_failing,
    },
    Rule {
        cooldown_secs: 14400,
        build: rules::chat_recap_retry_streak,
    },
];

mod rules {
    use super::*;

    fn fact_diagnostic_ids(fact: &crate::buddy::types::BuddyFact) -> Vec<String> {
        fact.payload
            .get("diagnostic_ids")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn payload_strings(fact: &crate::buddy::types::BuddyFact, key: &str) -> Vec<String> {
        fact.payload
            .get(key)
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn related_with_config_paths(paths: Vec<String>) -> BuddyOpportunityLinks {
        BuddyOpportunityLinks {
            config_paths: paths,
            ..BuddyOpportunityLinks::default()
        }
    }

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
            cooldown_secs: DEFAULT_COOLDOWN.num_seconds() as u64,
            status: OpportunityStatus::New,
            proposed_actions: actions,
            humor: None,
            humor_allowed: false,
            related: BuddyOpportunityLinks::default(),
            created_at: now,
            expires_at: now + Duration::hours(24),
            resolved_at: None,
        }
    }

    pub fn task_stuck(
        store: &crate::buddy::facts::FactStore,
        _pulse: &BuddyPulse,
        _queue: &OpportunityQueue,
        now: DateTime<Utc>,
    ) -> Vec<BuddyOpportunity> {
        store
            .recent_at(BuddyFactKind::TaskStuck, Duration::hours(2), now)
            .into_iter()
            .map(|fact| {
                let task_id = fact
                    .payload
                    .get("task_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let mut o = opp(
                    BuddyOpportunityKind::TaskHealth,
                    format!("Task stuck: {}", task_id),
                    BuddyPriority::High,
                    fact.confidence,
                    vec![fact.key.clone()],
                    format!("task_health:stuck:{}", task_id),
                    vec![
                        BuddyAction::OpenPage {
                            page: BuddyPage::TaskWorkspace {
                                task_id: task_id.clone(),
                            },
                        },
                        BuddyAction::Dismiss,
                    ],
                    now,
                );
                o.related.task_ids = vec![task_id];
                o
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
            .recent_at(BuddyFactKind::TaskAbandoned, Duration::days(2), now)
            .into_iter()
            .map(|fact| {
                let task_id = fact
                    .payload
                    .get("task_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let mut o = opp(
                    BuddyOpportunityKind::TaskHealth,
                    "Abandoned task needs review",
                    BuddyPriority::Normal,
                    fact.confidence,
                    vec![fact.key.clone()],
                    format!("task_health:abandoned:{}", task_id),
                    vec![
                        BuddyAction::OpenPage {
                            page: BuddyPage::TasksList,
                        },
                        BuddyAction::Dismiss,
                    ],
                    now,
                );
                o.related.task_ids = vec![task_id];
                o
            })
            .collect()
    }

    pub fn task_cluster_duplicate(
        store: &crate::buddy::facts::FactStore,
        _pulse: &BuddyPulse,
        _queue: &OpportunityQueue,
        now: DateTime<Utc>,
    ) -> Vec<BuddyOpportunity> {
        store
            .recent_at(
                BuddyFactKind::TaskClusterDuplicate,
                Duration::hours(12),
                now,
            )
            .into_iter()
            .map(|fact| {
                let task_a = fact
                    .payload
                    .get("task_a")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let task_b = fact
                    .payload
                    .get("task_b")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let overlap_count = fact
                    .payload
                    .get("overlap_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let mut o = opp(
                    BuddyOpportunityKind::TaskHealth,
                    format!(
                        "Tasks {} and {} touch {} overlapping files",
                        task_a, task_b, overlap_count
                    ),
                    BuddyPriority::Normal,
                    fact.confidence,
                    vec![fact.key.clone()],
                    format!("task_health:cluster:{}:{}", task_a, task_b),
                    vec![
                        BuddyAction::OpenPage {
                            page: BuddyPage::TasksList,
                        },
                        BuddyAction::Dismiss,
                    ],
                    now,
                );
                o.related.task_ids = vec![task_a, task_b];
                o
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
            .recent_at(BuddyFactKind::TrajectoryClutter, Duration::hours(12), now)
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

    fn provider_defaults_patch(field: &str) -> Option<(DefaultsKind, serde_json::Value)> {
        match field {
            "chat_model" => Some((
                DefaultsKind::ChatModel,
                serde_json::json!({ "chat": { "model": "your-provider/model-name" } }),
            )),
            "chat_light_model" => Some((
                DefaultsKind::ChatLightModel,
                serde_json::json!({ "chat_light": { "model": "your-provider/model-name" } }),
            )),
            "chat_thinking_model" => Some((
                DefaultsKind::ChatThinkingModel,
                serde_json::json!({ "chat_thinking": { "model": "your-provider/model-name" } }),
            )),
            "chat_buddy_model" => Some((
                DefaultsKind::ChatBuddyModel,
                serde_json::json!({ "chat_buddy": { "model": "your-provider/model-name" } }),
            )),
            _ => None,
        }
    }

    fn is_chat_default_field(field: &str) -> bool {
        matches!(
            field,
            "chat_model" | "chat_light_model" | "chat_thinking_model" | "chat_buddy_model"
        )
    }

    pub fn provider_tuning_missing(
        store: &crate::buddy::facts::FactStore,
        _pulse: &BuddyPulse,
        _queue: &OpportunityQueue,
        now: DateTime<Utc>,
    ) -> Vec<BuddyOpportunity> {
        store
            .recent_at(BuddyFactKind::DefaultModelMissing, Duration::hours(6), now)
            .into_iter()
            .filter_map(|fact| {
                let field = fact
                    .payload
                    .get("field")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let Some((defaults_kind, patch)) = provider_defaults_patch(field) else {
                    return None;
                };
                let mut o = opp(
                    BuddyOpportunityKind::ProviderTuning,
                    "Default model not configured",
                    BuddyPriority::High,
                    fact.confidence,
                    vec![fact.key.clone()],
                    format!("provider:default_model_missing:{}", field),
                    vec![
                        BuddyAction::OpenPage {
                            page: BuddyPage::DefaultModels,
                        },
                        BuddyAction::DraftDefaultsChange {
                            defaults_kind,
                            patch,
                        },
                        BuddyAction::Dismiss,
                    ],
                    now,
                );
                o.related = related_with_config_paths(vec!["providers/defaults".to_string()]);
                Some(o)
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
            .recent_at(BuddyFactKind::BrokenModelReference, Duration::hours(6), now)
            .into_iter()
            .filter_map(|fact| {
                let field = fact
                    .payload
                    .get("field")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if !is_chat_default_field(field) {
                    return None;
                }
                let model = fact
                    .payload
                    .get("model_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let mut o = opp(
                    BuddyOpportunityKind::ProviderTuning,
                    format!("Model not available: {}", model),
                    BuddyPriority::High,
                    fact.confidence,
                    vec![fact.key.clone()],
                    format!("provider:broken_ref:{}:{}", field, model),
                    vec![
                        BuddyAction::OpenPage {
                            page: BuddyPage::DefaultModels,
                        },
                        BuddyAction::Dismiss,
                    ],
                    now,
                );
                o.related = related_with_config_paths(vec!["providers/defaults".to_string()]);
                Some(o)
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
        let recent: Vec<_> = kinds
            .iter()
            .flat_map(|k| store.recent_at(*k, Duration::hours(24), now))
            .collect();
        let fact_keys: Vec<String> = recent.iter().map(|f| f.key.clone()).collect();
        let memory_ids: Vec<String> = recent
            .iter()
            .flat_map(|f| {
                let mut ids = payload_strings(f, "memory_ids");
                ids.extend(payload_strings(f, "doc_ids"));
                ids
            })
            .collect();
        if fact_keys.is_empty() {
            return vec![];
        }
        let mut o = opp(
            BuddyOpportunityKind::MemoryGarden,
            "Knowledge base needs attention",
            BuddyPriority::Normal,
            0.8,
            fact_keys,
            "memory:garden:global",
            vec![
                BuddyAction::OpenPage {
                    page: BuddyPage::KnowledgeGraph,
                },
                BuddyAction::CreatePulseReport {
                    scope: PulseScope::Memory,
                },
                BuddyAction::Dismiss,
            ],
            now,
        );
        o.related.memory_ids = memory_ids;
        vec![o]
    }

    pub fn diagnostic_investigation(
        store: &crate::buddy::facts::FactStore,
        _pulse: &BuddyPulse,
        _queue: &OpportunityQueue,
        now: DateTime<Utc>,
    ) -> Vec<BuddyOpportunity> {
        store
            .recent_at(BuddyFactKind::DiagnosticCluster, Duration::hours(1), now)
            .into_iter()
            .map(|fact| {
                let error_type = fact
                    .payload
                    .get("error_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("error")
                    .to_string();
                let diagnostic_ids = fact_diagnostic_ids(fact);
                let mut o = opp(
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
                                diagnostic_ids: diagnostic_ids.clone(),
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
                        },
                        BuddyAction::Dismiss,
                    ],
                    now,
                );
                o.related.chat_ids = payload_strings(fact, "chat_ids");
                o.related.config_paths = payload_strings(fact, "config_paths");
                o
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
            .recent_at(
                BuddyFactKind::FrontendErrorBurst,
                Duration::minutes(30),
                now,
            )
            .into_iter()
            .take(1)
            .map(|fact| {
                let diagnostic_ids = fact_diagnostic_ids(fact);
                let mut o = opp(
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
                                diagnostic_ids: diagnostic_ids.clone(),
                                log_excerpt: String::new(),
                                config_summary: String::new(),
                                initial_user_message: "Investigate frontend error burst"
                                    .to_string(),
                            },
                        },
                        BuddyAction::OpenPage {
                            page: BuddyPage::Buddy,
                        },
                        BuddyAction::Dismiss,
                    ],
                    now,
                );
                o.related.chat_ids = payload_strings(fact, "chat_ids");
                o.related.config_paths = payload_strings(fact, "config_paths");
                o
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
            .recent_at(BuddyFactKind::UncommittedPressure, Duration::hours(4), now)
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
            .recent_at(BuddyFactKind::GitDiffWidening, Duration::hours(4), now)
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
            .recent_at(BuddyFactKind::ModePromptOverlap, Duration::hours(24), now)
            .into_iter()
            .take(1)
            .map(|fact| {
                let id = fact
                    .payload
                    .get("mode_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let mut o = opp(
                    BuddyOpportunityKind::ConfigDrift,
                    "Mode prompts are overlapping",
                    BuddyPriority::Normal,
                    fact.confidence,
                    vec![fact.key.clone()],
                    format!("config_drift:mode_overlap:{}", id),
                    vec![
                        BuddyAction::OpenPage {
                            page: BuddyPage::Customization,
                        },
                        BuddyAction::DraftCustomizationChange {
                            customization_kind: CustomizationKind::Mode,
                            id: id.clone(),
                            patch: serde_json::json!({}),
                        },
                        BuddyAction::Dismiss,
                    ],
                    now,
                );
                o.related = related_with_config_paths(vec![format!("customization/modes/{}", id)]);
                o
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
            .recent_at(BuddyFactKind::SkillTriggerWeak, Duration::hours(48), now)
            .into_iter()
            .take(1)
            .map(|fact| {
                let id = fact
                    .payload
                    .get("skill_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let mut o = opp(
                    BuddyOpportunityKind::ConfigDrift,
                    "Skill has weak trigger description",
                    BuddyPriority::Normal,
                    fact.confidence,
                    vec![fact.key.clone()],
                    format!("config_drift:skill_trigger:{}", id),
                    vec![
                        BuddyAction::OpenPage {
                            page: BuddyPage::Customization,
                        },
                        BuddyAction::DraftCustomizationChange {
                            customization_kind: CustomizationKind::Skill,
                            id: id.clone(),
                            patch: serde_json::json!({}),
                        },
                        BuddyAction::Dismiss,
                    ],
                    now,
                );
                o.related = related_with_config_paths(vec![format!("customization/skills/{}", id)]);
                o
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
            .recent_at(BuddyFactKind::AgentsMdGapDetected, Duration::hours(72), now)
            .into_iter()
            .take(1)
            .map(|fact| {
                let mut o = opp(
                    BuddyOpportunityKind::AgentsMdGap,
                    "AGENTS.md missing or outdated",
                    BuddyPriority::Normal,
                    fact.confidence,
                    vec![fact.key.clone()],
                    "agents_md:gap:global",
                    vec![
                        BuddyAction::DraftAgentsMdPatch {
                            content: String::new(),
                        },
                        BuddyAction::Dismiss,
                    ],
                    now,
                );
                o.related = related_with_config_paths(vec!["AGENTS.md".to_string()]);
                o
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
            .recent_at(BuddyFactKind::McpAuthExpired, Duration::hours(6), now)
            .into_iter()
            .map(|fact| {
                let id = fact
                    .payload
                    .get("mcp_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let mut o = opp(
                    BuddyOpportunityKind::IntegrationFix,
                    format!("MCP auth expiring: {}", id),
                    BuddyPriority::High,
                    fact.confidence,
                    vec![fact.key.clone()],
                    format!("integration:mcp_auth:{}", id),
                    vec![
                        BuddyAction::OpenPage {
                            page: BuddyPage::Integrations,
                        },
                        BuddyAction::Dismiss,
                    ],
                    now,
                );
                o.related = related_with_config_paths(vec![format!("integrations/{}", id)]);
                o
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
            .recent_at(BuddyFactKind::IntegrationFailing, Duration::hours(4), now)
            .into_iter()
            .map(|fact| {
                let id = fact
                    .payload
                    .get("mcp_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let mut o = opp(
                    BuddyOpportunityKind::IntegrationFix,
                    format!("Integration failing: {}", id),
                    BuddyPriority::Normal,
                    fact.confidence,
                    vec![fact.key.clone()],
                    format!("integration:failing:{}", id),
                    vec![
                        BuddyAction::OpenPage {
                            page: BuddyPage::Integrations,
                        },
                        BuddyAction::Dismiss,
                    ],
                    now,
                );
                o.related = related_with_config_paths(vec![format!("integrations/{}", id)]);
                o
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
            .recent_at(BuddyFactKind::ChatRetryStreak, Duration::hours(4), now)
            .into_iter()
            .map(|fact| {
                let chat_id = fact
                    .payload
                    .get("chat_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let mut o = opp(
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
                );
                o.related.chat_ids = vec![chat_id];
                o
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
    ) -> Vec<(BuddyOpportunity, u64)> {
        let now = Utc::now();
        let mut seen: HashSet<String> = HashSet::new();
        let mut result = vec![];

        for rule in RULES {
            let candidates = (rule.build)(fact_store, pulse, queue, now);
            for opp in candidates {
                if seen.contains(&opp.cooldown_key) {
                    continue;
                }
                if queue.cooldown_active(&opp.cooldown_key) {
                    continue;
                }
                seen.insert(opp.cooldown_key.clone());
                result.push((opp, rule.cooldown_secs));
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
pub fn primary_fact_kind_for_opportunity(
    opp: &BuddyOpportunity,
    fact_store: &crate::buddy::facts::FactStore,
) -> BuddyFactKind {
    if let Some(key) = opp.fact_keys.first() {
        if let Some(fact) = fact_store.iter().find(|f| &f.key == key) {
            return fact.kind;
        }
    }

    match opp.kind {
        BuddyOpportunityKind::TaskHealth => BuddyFactKind::TaskStuck,
        BuddyOpportunityKind::TrajectoryCleanup => BuddyFactKind::TrajectoryClutter,
        BuddyOpportunityKind::ChatRecap => BuddyFactKind::ChatRetryStreak,
        BuddyOpportunityKind::MemoryGarden => BuddyFactKind::MemoryOrphan,
        BuddyOpportunityKind::ConfigDrift => BuddyFactKind::ModePromptOverlap,
        BuddyOpportunityKind::AgentsMdGap => BuddyFactKind::AgentsMdGapDetected,
        BuddyOpportunityKind::ProviderTuning => BuddyFactKind::DefaultModelMissing,
        BuddyOpportunityKind::IntegrationFix => BuddyFactKind::McpAuthExpired,
        BuddyOpportunityKind::DiagnosticInvestigation => BuddyFactKind::DiagnosticCluster,
        BuddyOpportunityKind::GitHygiene => BuddyFactKind::UncommittedPressure,
    }
}
