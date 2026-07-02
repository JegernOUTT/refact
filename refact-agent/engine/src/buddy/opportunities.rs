use chrono::{DateTime, Duration, Utc};
use std::collections::HashSet;
use uuid::Uuid;

pub use refact_buddy_core::queue::{
    is_terminal_status, OpportunityQueue, DEFAULT_COOLDOWN, DISMISS_MEMORY, MAX_OPPORTUNITIES,
    MAX_UNREAD,
};

use crate::buddy::types::{
    BuddyAction, BuddyFactKind, BuddyOpportunity, BuddyOpportunityKind, BuddyOpportunityLinks,
    BuddyPage, BuddyPriority, BuddyPulse, CustomizationKind, DefaultsKind, InvestigationContext,
    OpportunityStatus, PulseScope,
};

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
        cooldown_secs: 14400,
        build: rules::worktree_cleanup,
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

    fn memory_garden_signal_class(fact_kind: BuddyFactKind) -> &'static str {
        match fact_kind {
            BuddyFactKind::MemoryOrphan => "orphan",
            BuddyFactKind::MemoryStaleConflict => "conflict",
            BuddyFactKind::MemoryRecurringLesson => "review",
            _ => "review",
        }
    }

    fn memory_garden_lifecycle_class(pulse: &BuddyPulse) -> &'static str {
        if pulse.memory.conflict_candidates > 0 {
            "conflict"
        } else if pulse.memory.duplicate_candidates > 0 || pulse.memory.merge_candidates > 0 {
            "merge"
        } else if pulse.memory.review_candidates > 0 {
            "review"
        } else if pulse.memory.archive_candidates > 0 {
            "orphan"
        } else {
            "review"
        }
    }

    fn memory_garden_lifecycle_classes(pulse: &BuddyPulse) -> Vec<&'static str> {
        let mut classes = Vec::new();
        if pulse.memory.archive_candidates > 0 {
            classes.push("orphan");
        }
        if pulse.memory.duplicate_candidates > 0 || pulse.memory.merge_candidates > 0 {
            classes.push("merge");
        }
        if pulse.memory.review_candidates > 0 {
            classes.push("review");
        }
        if pulse.memory.conflict_candidates > 0 {
            classes.push("conflict");
        }
        if classes.is_empty() {
            classes.push(memory_garden_lifecycle_class(pulse));
        }
        classes
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
            "chat_model_2" => Some((
                DefaultsKind::ChatModel2,
                serde_json::json!({ "chat_model_2": { "model": "your-provider/model-name" } }),
            )),
            "task_planner_agent_model" => Some((
                DefaultsKind::TaskPlannerAgentModel,
                serde_json::json!({ "task_planner_agent_model": { "model": "your-provider/model-name" } }),
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
            "chat_model"
                | "chat_model_2"
                | "task_planner_agent_model"
                | "chat_light_model"
                | "chat_thinking_model"
                | "chat_buddy_model"
        )
    }

    fn is_checked_default_field(field: &str) -> bool {
        is_chat_default_field(field) || field == "completion_model"
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
                if !is_checked_default_field(field) {
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
        pulse: &BuddyPulse,
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
        let lifecycle_attention = pulse.memory.duplicate_candidates
            + pulse.memory.merge_candidates
            + pulse.memory.archive_candidates
            + pulse.memory.review_candidates
            + pulse.memory.conflict_candidates;
        if recent.is_empty() && lifecycle_attention == 0 {
            return vec![];
        }
        let mut classes: Vec<&'static str> = recent
            .iter()
            .map(|fact| memory_garden_signal_class(fact.kind))
            .collect();
        if lifecycle_attention > 0 {
            classes.extend(memory_garden_lifecycle_classes(pulse));
        }
        classes.sort_unstable();
        classes.dedup();
        let summary = if lifecycle_attention > 0 {
            format!(
                "Knowledge base needs attention: {} lifecycle candidate(s)",
                lifecycle_attention
            )
        } else {
            "Knowledge base needs attention".to_string()
        };
        classes
            .into_iter()
            .map(|garden_class| {
                let class_facts: Vec<_> = recent
                    .iter()
                    .copied()
                    .filter(|fact| memory_garden_signal_class(fact.kind) == garden_class)
                    .collect();
                let fact_keys: Vec<String> = class_facts.iter().map(|f| f.key.clone()).collect();
                let memory_ids: Vec<String> = class_facts
                    .iter()
                    .flat_map(|f| {
                        let mut ids = payload_strings(f, "memory_ids");
                        ids.extend(payload_strings(f, "doc_ids"));
                        ids
                    })
                    .collect();
                let mut o = opp(
                    BuddyOpportunityKind::MemoryGarden,
                    summary.clone(),
                    BuddyPriority::Normal,
                    0.8,
                    fact_keys,
                    format!("memory:garden:{}", garden_class),
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
                o
            })
            .collect()
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
                let source_bucket = fact
                    .payload
                    .get("source_bucket")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let diagnostic_ids = fact_diagnostic_ids(fact);
                let mut o = opp(
                    BuddyOpportunityKind::DiagnosticInvestigation,
                    format!("Repeated errors: {}", error_type),
                    BuddyPriority::High,
                    fact.confidence,
                    vec![fact.key.clone()],
                    format!("diag:cluster:{}:{}", error_type, source_bucket),
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

    pub fn worktree_cleanup(
        store: &crate::buddy::facts::FactStore,
        _pulse: &BuddyPulse,
        _queue: &OpportunityQueue,
        now: DateTime<Utc>,
    ) -> Vec<BuddyOpportunity> {
        store
            .recent_at(BuddyFactKind::WorktreeHygiene, Duration::hours(4), now)
            .into_iter()
            .filter_map(|fact| {
                let summary = fact.payload.get("summary")?;
                let total = summary.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
                let abandoned = summary
                    .get("abandoned_clean")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                if abandoned == 0 {
                    return None;
                }
                let dirty = summary.get("dirty").and_then(|v| v.as_u64()).unwrap_or(0);
                let stale = summary.get("stale").and_then(|v| v.as_u64()).unwrap_or(0);
                let summary_text = format!(
                    "I found {} worktrees: {} clean abandoned, {} with changes, {} stale. Want to review cleanup candidates?",
                    total, abandoned, dirty, stale
                );
                Some(opp(
                    BuddyOpportunityKind::WorktreeCleanup,
                    summary_text,
                    BuddyPriority::Normal,
                    fact.confidence,
                    vec![fact.key.clone()],
                    "worktrees:cleanup:global",
                    vec![
                        BuddyAction::OpenPage {
                            page: BuddyPage::Worktrees,
                        },
                        BuddyAction::LaunchInvestigationChat {
                            preload: InvestigationContext {
                                fact_keys: vec![fact.key.clone()],
                                diagnostic_ids: vec![],
                                log_excerpt: String::new(),
                                config_summary: String::new(),
                                initial_user_message: "Review worktree cleanup candidates and help me choose safe IDs to clean".to_string(),
                            },
                        },
                        BuddyAction::CreatePulseReport {
                            scope: PulseScope::Worktrees,
                        },
                        BuddyAction::Dismiss,
                    ],
                    now,
                ))
            })
            .take(1)
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
        BuddyOpportunityKind::WorktreeCleanup => BuddyFactKind::WorktreeHygiene,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buddy::facts::FactStore;
    use crate::buddy::types::BuddyFact;

    fn memory_fact(kind: BuddyFactKind, key: &str) -> BuddyFact {
        BuddyFact {
            kind,
            key: key.to_string(),
            source: "test",
            payload: serde_json::json!({ "memory_ids": [key] }),
            seen_at: Utc::now(),
            confidence: 0.9,
        }
    }

    #[test]
    fn memory_garden_uses_lifecycle_pulse_counts_without_recent_facts() {
        let mut pulse = BuddyPulse::default();
        pulse.memory.duplicate_candidates = 1;
        pulse.memory.merge_candidates = 2;
        let queue = OpportunityQueue::new();

        let opps = rules::memory_garden(&FactStore::new(), &pulse, &queue, Utc::now());

        assert_eq!(opps.len(), 1);
        assert!(opps[0].summary.contains("3 lifecycle candidate"));
        assert_eq!(opps[0].kind, BuddyOpportunityKind::MemoryGarden);
    }

    #[test]
    fn distinct_memory_garden_classes_get_distinct_cooldown_keys() {
        let pulse = BuddyPulse::default();
        let queue = OpportunityQueue::new();
        let mut orphan_store = FactStore::new();
        orphan_store.ingest(memory_fact(BuddyFactKind::MemoryOrphan, "orphan"));
        let mut conflict_store = FactStore::new();
        conflict_store.ingest(memory_fact(BuddyFactKind::MemoryStaleConflict, "conflict"));

        let orphan_opps = rules::memory_garden(&orphan_store, &pulse, &queue, Utc::now());
        let conflict_opps = rules::memory_garden(&conflict_store, &pulse, &queue, Utc::now());

        assert_eq!(orphan_opps[0].cooldown_key, "memory:garden:orphan");
        assert_eq!(conflict_opps[0].cooldown_key, "memory:garden:conflict");
        assert_ne!(orphan_opps[0].cooldown_key, conflict_opps[0].cooldown_key);

        let mut lifecycle_pulse = BuddyPulse::default();
        lifecycle_pulse.memory.merge_candidates = 1;
        lifecycle_pulse.memory.review_candidates = 1;
        let lifecycle_keys =
            rules::memory_garden(&FactStore::new(), &lifecycle_pulse, &queue, Utc::now())
                .into_iter()
                .map(|opp| opp.cooldown_key)
                .collect::<HashSet<_>>();

        assert!(lifecycle_keys.contains("memory:garden:merge"));
        assert!(lifecycle_keys.contains("memory:garden:review"));
    }
}
