use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex as AMutex;

use super::actor::BuddyService;
use super::diagnostics::DiagnosticContext;
use super::settings::BuddySettings;
use super::types::{
    BuddyActivity, BuddyFact, BuddyJobState, BuddyOnboarding, BuddyPetState, BuddyPulse,
    BuddyRuntimeEvent, BuddySpeechItem, BuddySuggestion,
};
use crate::global_context::GlobalContext;

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone)]
pub struct BuddyJobContext {
    pub identity_name: String,
    pub onboarding: BuddyOnboarding,
    pub recent_diagnostics: Vec<DiagnosticContext>,
    pub project_root: std::path::PathBuf,
    pub job_state: BuddyJobState,
    pub total_workflow_runs: u64,
    pub suggestion_state: Vec<BuddySuggestion>,
    pub pet: BuddyPetState,
    pub active_quest: Option<super::types::BuddyQuest>,
    pub settings: BuddySettings,
    pub pulse: BuddyPulse,
    pub facts: Vec<BuddyFact>,
}

pub struct BuddyJobResult {
    pub speech: Option<BuddySpeechItem>,
    pub suggestion: Option<BuddySuggestion>,
    pub activity: Option<BuddyActivity>,
    pub runtime_event: Option<BuddyRuntimeEvent>,
    pub last_result: Option<String>,
}

impl Default for BuddyJobResult {
    fn default() -> Self {
        Self {
            speech: None,
            suggestion: None,
            activity: None,
            runtime_event: None,
            last_result: None,
        }
    }
}

impl BuddyJobResult {
    fn has_visible_output(&self) -> bool {
        self.speech.is_some()
            || self.suggestion.is_some()
            || self.activity.is_some()
            || self.runtime_event.is_some()
    }
}

fn next_last_result(existing: Option<&str>, result: Option<&str>) -> Option<String> {
    result.or(existing).map(ToString::to_string)
}

pub(crate) fn result_after_suggestion_policy(
    result: BuddyJobResult,
    settings: &BuddySettings,
    suggestion_state: &[BuddySuggestion],
) -> BuddyJobResult {
    if suggestions_allowed(settings, suggestion_state) {
        return result;
    }
    BuddyJobResult {
        suggestion: None,
        ..result
    }
}

#[async_trait::async_trait]
pub trait BuddyJob: Send + Sync {
    fn id(&self) -> &str;
    fn cooldown_seconds(&self) -> u64;
    fn priority(&self) -> u32;
    fn produces_suggestion(&self) -> bool {
        false
    }
    fn runs_when_suggestions_blocked(&self) -> bool {
        false
    }
    async fn should_run(
        &self,
        gcx: Arc<tokio::sync::RwLock<GlobalContext>>,
        ctx: &BuddyJobContext,
    ) -> bool;
    async fn execute(
        &self,
        gcx: Arc<tokio::sync::RwLock<GlobalContext>>,
        ctx: BuddyJobContext,
    ) -> BuddyJobResult;
}

pub(crate) const MAX_UNREAD_SUGGESTIONS: usize = 3;

pub(crate) fn suggestions_allowed(
    settings: &BuddySettings,
    suggestion_state: &[BuddySuggestion],
) -> bool {
    let unread = suggestion_state
        .iter()
        .filter(|suggestion| !suggestion.dismissed)
        .count();
    settings.proactive_enabled && unread < MAX_UNREAD_SUGGESTIONS
}

pub struct BuddyScheduler {
    jobs: Vec<Box<dyn BuddyJob>>,
}

impl BuddyScheduler {
    pub fn new() -> Self {
        let mut s = Self { jobs: vec![] };
        s.jobs.push(Box::new(super::jobs::greeting::GreetingJob));
        s.jobs.push(Box::new(super::jobs::tour::TourJob));
        s.jobs
            .push(Box::new(super::jobs::error_triage::ErrorTriageJob));
        s.jobs
            .push(Box::new(super::jobs::config_watcher::ConfigWatcherJob));
        s.jobs
            .push(Box::new(super::jobs::stats_watcher::StatsWatcherJob));
        s.jobs
            .push(Box::new(super::jobs::health_watcher::HealthWatcherJob));
        s.jobs.push(Box::new(
            super::jobs::autonomous_chats::BuddyMemoryGardenerJob,
        ));
        s.jobs.push(Box::new(
            super::jobs::autonomous_chats::BuddyKnowledgeConflictResolverJob,
        ));
        s.jobs.push(Box::new(
            super::jobs::autonomous_chats::BuddyBehaviorLearnerJob,
        ));
        s.jobs.push(Box::new(
            super::jobs::autonomous_chats::BuddyUserHabitCoachJob,
        ));
        s.jobs.push(Box::new(
            super::jobs::autonomous_chats::BuddyModelCostOptimizerJob,
        ));
        s.jobs
            .push(Box::new(super::jobs::quest_prompt::QuestPromptJob));
        s.jobs
            .push(Box::new(super::jobs::autonomous_chats::ErrorDetectiveJob));
        s.jobs.push(Box::new(
            super::jobs::autonomous_chats::SecurityWhispererJob,
        ));
        s.jobs
            .push(Box::new(super::jobs::autonomous_chats::SetupCoachJob));
        s.jobs
            .push(Box::new(super::jobs::autonomous_chats::DependencyRadarJob));
        s.jobs
            .push(Box::new(super::jobs::autonomous_chats::DocsGardenerJob));
        s.jobs.push(Box::new(
            super::jobs::autonomous_chats::ArchitectureDriftWatcherJob,
        ));
        s.jobs.push(Box::new(
            super::jobs::proactive_suggestions::ProactiveSuggestionsJob,
        ));
        s.jobs.sort_by_key(|j| j.priority());
        s
    }

    #[cfg(test)]
    fn job_ids(&self) -> Vec<String> {
        self.jobs.iter().map(|job| job.id().to_string()).collect()
    }

    pub async fn tick(
        &self,
        gcx: Arc<tokio::sync::RwLock<GlobalContext>>,
        buddy_arc: Arc<AMutex<Option<BuddyService>>>,
        project_root: &Path,
    ) {
        let ctx_opt = {
            let buddy = buddy_arc.lock().await;
            buddy.as_ref().map(|svc| {
                (
                    svc.state.clone(),
                    svc.recent_diagnostics.clone(),
                    svc.settings.clone(),
                    svc.pulse.clone(),
                    svc.fact_store.iter().cloned().collect::<Vec<_>>(),
                )
            })
        };
        let (state, diags, settings, pulse, facts) = match ctx_opt {
            Some(x) => x,
            None => return,
        };
        for job in &self.jobs {
            if job.produces_suggestion()
                && !job.runs_when_suggestions_blocked()
                && !suggestions_allowed(&settings, &state.suggestion_state)
            {
                continue;
            }
            let job_state = state
                .job_cooldowns
                .get(job.id())
                .cloned()
                .unwrap_or_default();
            if job_state.dismissed {
                continue;
            }
            let elapsed = job_state
                .last_run
                .as_deref()
                .and_then(|r| chrono::DateTime::parse_from_rfc3339(r).ok())
                .map(|t| {
                    chrono::Utc::now()
                        .signed_duration_since(t)
                        .num_seconds()
                        .max(0) as u64
                })
                .unwrap_or(u64::MAX);
            if elapsed < job.cooldown_seconds() {
                continue;
            }
            let total_workflow_runs = state.workflow_summaries.iter().map(|w| w.run_count).sum();
            let ctx = BuddyJobContext {
                identity_name: state.identity.name.clone(),
                onboarding: state.onboarding.clone(),
                recent_diagnostics: diags.clone(),
                project_root: project_root.to_path_buf(),
                job_state: job_state.clone(),
                total_workflow_runs,
                suggestion_state: state.suggestion_state.clone(),
                pet: state.pet.clone(),
                active_quest: state.active_quest.clone(),
                settings: settings.clone(),
                pulse: pulse.clone(),
                facts: facts.clone(),
            };
            if !job.should_run(gcx.clone(), &ctx).await {
                continue;
            }
            let result = job.execute(gcx.clone(), ctx).await;
            let result = if job.produces_suggestion() {
                result_after_suggestion_policy(result, &settings, &state.suggestion_state)
            } else {
                result
            };
            let has_visible_output = result.has_visible_output();
            let mut buddy = buddy_arc.lock().await;
            if let Some(svc) = buddy.as_mut() {
                let mut js = svc
                    .state
                    .job_cooldowns
                    .entry(job.id().to_string())
                    .or_default()
                    .clone();
                js.last_run = Some(chrono::Utc::now().to_rfc3339());
                js.run_count += 1;
                js.last_result =
                    next_last_result(js.last_result.as_deref(), result.last_result.as_deref());
                svc.state.job_cooldowns.insert(job.id().to_string(), js);
                svc.dirty = true;
                if let Some(suggestion) = result.suggestion {
                    svc.maybe_add_suggestion(suggestion);
                }
                if let Some(activity) = result.activity {
                    svc.add_activity(activity);
                }
                if let Some(speech) = result.speech {
                    svc.update_speech(speech);
                }
                if let Some(event) = result.runtime_event {
                    svc.enqueue_runtime_event(event);
                }
            }
            if has_visible_output {
                break; // max 1 visible job per tick
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buddy::autonomous_workflows::AUTONOMOUS_BUDDY_WORKFLOWS;

    #[test]
    fn next_last_result_preserves_existing_when_job_returns_none() {
        assert_eq!(
            next_last_result(Some("existing-json"), None).as_deref(),
            Some("existing-json")
        );
        assert_eq!(
            next_last_result(Some("existing-json"), Some("new-json")).as_deref(),
            Some("new-json")
        );
        assert_eq!(next_last_result(None, None), None);
    }

    fn active_suggestion(idx: usize) -> BuddySuggestion {
        BuddySuggestion {
            id: format!("suggestion-{idx}"),
            suggestion_type: "test".to_string(),
            title: format!("Suggestion {idx}"),
            description: "Test".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            dismissed: false,
            controls: vec![],
            quest: None,
        }
    }

    #[test]
    fn result_after_suggestion_policy_removes_only_suggestion_output() {
        let mut settings = BuddySettings::default();
        settings.proactive_enabled = false;
        let result = BuddyJobResult {
            suggestion: Some(active_suggestion(1)),
            runtime_event: Some(BuddyRuntimeEvent {
                id: "event".to_string(),
                signal_type: "health".to_string(),
                title: "Health".to_string(),
                description: None,
                source: "test".to_string(),
                status: "failed".to_string(),
                progress: None,
                dedupe_key: None,
                priority: "normal".to_string(),
                created_at: chrono::Utc::now().to_rfc3339(),
                ttl_ms: None,
                speech_text: None,
                scene: None,
                duration_hint: None,
                persistent: false,
                controls: vec![],
                chat_id: None,
                dismissed: false,
            }),
            last_result: Some("unhealthy".to_string()),
            ..Default::default()
        };

        let filtered = result_after_suggestion_policy(result, &settings, &[]);

        assert!(filtered.suggestion.is_none());
        assert!(filtered.runtime_event.is_some());
        assert_eq!(filtered.last_result.as_deref(), Some("unhealthy"));
    }

    #[test]
    fn suggestions_allowed_requires_proactive_and_unread_budget() {
        let mut settings = BuddySettings::default();
        assert!(suggestions_allowed(&settings, &[]));

        settings.proactive_enabled = false;
        assert!(!suggestions_allowed(&settings, &[]));

        settings.proactive_enabled = true;
        let mut suggestions = (0..MAX_UNREAD_SUGGESTIONS)
            .map(active_suggestion)
            .collect::<Vec<_>>();
        assert!(!suggestions_allowed(&settings, &suggestions));
        suggestions[0].dismissed = true;
        assert!(suggestions_allowed(&settings, &suggestions));
    }

    #[test]
    fn mixed_suggestion_watchers_run_when_suggestions_blocked() {
        use crate::buddy::jobs::health_watcher::HealthWatcherJob;
        use crate::buddy::jobs::stats_watcher::StatsWatcherJob;

        assert!(StatsWatcherJob.produces_suggestion());
        assert!(StatsWatcherJob.runs_when_suggestions_blocked());
        assert!(HealthWatcherJob.produces_suggestion());
        assert!(HealthWatcherJob.runs_when_suggestions_blocked());
    }

    #[test]
    fn scheduler_registers_all_autonomous_registry_jobs() {
        let scheduler = BuddyScheduler::new();
        let ids = scheduler.job_ids();

        for meta in AUTONOMOUS_BUDDY_WORKFLOWS {
            assert!(ids.iter().any(|id| id == meta.id), "missing {}", meta.id);
        }
    }
}
