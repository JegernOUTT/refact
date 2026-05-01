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

#[async_trait::async_trait]
pub trait BuddyJob: Send + Sync {
    fn id(&self) -> &str;
    fn cooldown_seconds(&self) -> u64;
    fn priority(&self) -> u32;
    fn produces_suggestion(&self) -> bool {
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

const MAX_UNREAD_SUGGESTIONS: usize = 3;

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
        s.jobs
            .push(Box::new(super::jobs::quest_prompt::QuestPromptJob));
        s.jobs.push(Box::new(
            super::jobs::proactive_suggestions::ProactiveSuggestionsJob,
        ));
        s.jobs.sort_by_key(|j| j.priority());
        s
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
        let proactive_enabled = settings.proactive_enabled;

        let unread = state
            .suggestion_state
            .iter()
            .filter(|s| !s.dismissed)
            .count();

        for job in &self.jobs {
            if job.produces_suggestion() && (!proactive_enabled || unread >= MAX_UNREAD_SUGGESTIONS)
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
}
