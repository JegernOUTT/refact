use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex as AMutex;

use super::actor::BuddyService;
use super::diagnostics::DiagnosticContext;
use super::settings::BuddySettings;
use super::types::{
    BuddyActivity, BuddyFact, BuddyJobState, BuddyOnboarding, BuddyPersonalityProfile,
    BuddyPetState, BuddyPulse, BuddyRuntimeEvent, BuddySpeechItem, BuddySuggestion,
    BuddyWorkflowSummary,
};
use super::voice_service::SpeechIntent;
use crate::global_context::GlobalContext;

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone)]
pub struct BuddyJobContext {
    pub identity_name: String,
    pub personality: BuddyPersonalityProfile,
    pub onboarding: BuddyOnboarding,
    pub recent_diagnostics: Vec<DiagnosticContext>,
    pub project_root: std::path::PathBuf,
    pub job_state: BuddyJobState,
    pub workflow_summaries: Vec<BuddyWorkflowSummary>,
    pub total_workflow_runs: u64,
    pub suggestion_state: Vec<BuddySuggestion>,
    pub pet: BuddyPetState,
    pub active_quest: Option<super::types::BuddyQuest>,
    pub settings: BuddySettings,
    pub pulse: BuddyPulse,
    pub facts: Vec<BuddyFact>,
}

pub struct BuddyJobResult {
    pub speech_intent: Option<SpeechIntent>,
    pub speech: Option<BuddySpeechItem>,
    pub suggestion: Option<BuddySuggestion>,
    pub activity: Option<BuddyActivity>,
    pub runtime_event: Option<BuddyRuntimeEvent>,
    pub last_result: Option<String>,
}

impl Default for BuddyJobResult {
    fn default() -> Self {
        Self {
            speech_intent: None,
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

fn should_record_job_result(result: &BuddyJobResult, records_empty_result: bool) -> bool {
    records_empty_result || result.has_visible_output()
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
    fn records_empty_result(&self) -> bool {
        true
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
            super::jobs::refact_compile_sniffer::RefactCompileSnifferJob,
        ));
        s.jobs.push(Box::new(
            super::jobs::refact_self_critic::RefactSelfCriticJob,
        ));
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
        s.jobs
            .push(Box::new(super::jobs::speaker_insight::SpeakerInsightJob));
        s.jobs
            .push(Box::new(super::jobs::speaker_win::SpeakerWinJob));
        s.jobs.push(Box::new(
            super::jobs::speaker_memory_pulse_commentary::SpeakerMemoryPulseCommentaryJob,
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
        let mut ready_results: Vec<(&str, BuddyJobResult, bool)> = vec![];
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
                personality: state.personality.clone(),
                onboarding: state.onboarding.clone(),
                recent_diagnostics: diags.clone(),
                project_root: project_root.to_path_buf(),
                job_state: job_state.clone(),
                workflow_summaries: state.workflow_summaries.clone(),
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
            let records_empty_result = job.records_empty_result();
            ready_results.push((job.id(), result, records_empty_result));
        }

        let now = chrono::Utc::now();
        let winner = {
            let buddy = buddy_arc.lock().await;
            let Some(svc) = buddy.as_ref() else {
                return;
            };
            let speech_result_indexes = ready_results
                .iter()
                .enumerate()
                .filter_map(|(idx, (_, result, _))| {
                    result
                        .speech_intent
                        .zip(result.speech.as_ref())
                        .map(|_| idx)
                })
                .collect::<Vec<_>>();
            let candidates = speech_result_indexes
                .iter()
                .filter_map(|idx| {
                    let (_, result, _) = &ready_results[*idx];
                    result.speech_intent.zip(result.speech.clone())
                })
                .collect::<Vec<_>>();
            let candidate_winner = super::speech_policy::pick_speech_intent(
                &candidates,
                &svc.state.speech_rotation,
                now,
            );
            candidate_winner.and_then(|winner_idx| speech_result_indexes.get(winner_idx).copied())
        };

        for (idx, (job_id, mut result, records_empty_result)) in
            ready_results.into_iter().enumerate()
        {
            if result.speech.is_some() && Some(idx) != winner {
                result.speech = None;
                result.speech_intent = None;
            }
            if should_record_job_result(&result, records_empty_result) {
                let mut buddy = buddy_arc.lock().await;
                if let Some(svc) = buddy.as_mut() {
                    let mut js = svc
                        .state
                        .job_cooldowns
                        .entry(job_id.to_string())
                        .or_default()
                        .clone();
                    js.last_run = Some(chrono::Utc::now().to_rfc3339());
                    js.run_count += 1;
                    js.last_result =
                        next_last_result(js.last_result.as_deref(), result.last_result.as_deref());
                    svc.state.job_cooldowns.insert(job_id.to_string(), js);
                    svc.dirty = true;
                    if let Some(suggestion) = result.suggestion {
                        svc.maybe_add_suggestion(suggestion);
                    }
                    if let Some(activity) = result.activity {
                        svc.add_activity(activity);
                    }
                    if let (Some(intent), Some(speech)) = (result.speech_intent, result.speech) {
                        svc.update_speech(speech);
                        super::speech_policy::record_emission(
                            &mut svc.state.speech_rotation,
                            intent,
                            now,
                        );
                        svc.dirty = true;
                    }
                    if let Some(event) = result.runtime_event {
                        svc.enqueue_runtime_event(event);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buddy::autonomous_workflows::AUTONOMOUS_BUDDY_WORKFLOWS;

    struct NoOutputUnrecordedJob;

    #[async_trait::async_trait]
    impl BuddyJob for NoOutputUnrecordedJob {
        fn id(&self) -> &str {
            "no_output_unrecorded"
        }

        fn cooldown_seconds(&self) -> u64 {
            0
        }

        fn priority(&self) -> u32 {
            0
        }

        fn records_empty_result(&self) -> bool {
            false
        }

        async fn should_run(
            &self,
            _gcx: Arc<tokio::sync::RwLock<GlobalContext>>,
            _ctx: &BuddyJobContext,
        ) -> bool {
            true
        }

        async fn execute(
            &self,
            _gcx: Arc<tokio::sync::RwLock<GlobalContext>>,
            _ctx: BuddyJobContext,
        ) -> BuddyJobResult {
            BuddyJobResult::default()
        }
    }

    struct SpeechJob {
        id: String,
        priority: u32,
        intent: SpeechIntent,
        activity_title: String,
    }

    #[async_trait::async_trait]
    impl BuddyJob for SpeechJob {
        fn id(&self) -> &str {
            &self.id
        }

        fn cooldown_seconds(&self) -> u64 {
            0
        }

        fn priority(&self) -> u32 {
            self.priority
        }

        async fn should_run(
            &self,
            _gcx: Arc<tokio::sync::RwLock<GlobalContext>>,
            _ctx: &BuddyJobContext,
        ) -> bool {
            true
        }

        async fn execute(
            &self,
            _gcx: Arc<tokio::sync::RwLock<GlobalContext>>,
            _ctx: BuddyJobContext,
        ) -> BuddyJobResult {
            BuddyJobResult {
                speech_intent: Some(self.intent),
                speech: Some(BuddySpeechItem {
                    id: format!("speech-{}", self.id),
                    text: format!("speech {}", self.id),
                    mood: "happy".to_string(),
                    scope: "global".to_string(),
                    persistent: false,
                    ttl_seconds: 10,
                    dedupe_key: Some(format!("speech-{}", self.id)),
                    created_at: chrono::Utc::now().to_rfc3339(),
                    controls: vec![],
                    chat_id: None,
                }),
                activity: Some(BuddyActivity {
                    icon: "•".to_string(),
                    title: self.activity_title.to_string(),
                    description: self.activity_title.to_string(),
                    timestamp: chrono::Utc::now().to_rfc3339(),
                    activity_type: "test".to_string(),
                    chat_id: None,
                }),
                ..Default::default()
            }
        }
    }

    async fn test_service(
        dir: &tempfile::TempDir,
        state: crate::buddy::types::BuddyState,
    ) -> Arc<AMutex<Option<BuddyService>>> {
        let (tx, _) = tokio::sync::broadcast::channel(16);
        Arc::new(AMutex::new(Some(BuddyService::new(
            dir.path().to_path_buf(),
            state,
            BuddySettings::default(),
            Vec::new(),
            crate::buddy::runtime_queue::RuntimeQueue::new(),
            tx,
            None,
        ))))
    }

    #[tokio::test]
    async fn humor_caps_at_5_per_hour() {
        let dir = tempfile::tempdir().unwrap();
        let scheduler = BuddyScheduler {
            jobs: (0..6)
                .map(|idx| {
                    Box::new(SpeechJob {
                        id: format!("humor_{idx}"),
                        priority: idx,
                        intent: SpeechIntent::Humor,
                        activity_title: "humor activity".to_string(),
                    }) as Box<dyn BuddyJob>
                })
                .collect(),
        };
        let buddy_arc = test_service(&dir, crate::buddy::state::default_buddy_state()).await;
        let gcx = crate::global_context::tests::make_test_gcx().await;

        for _ in 0..6 {
            scheduler
                .tick(gcx.clone(), buddy_arc.clone(), dir.path())
                .await;
            let mut buddy = buddy_arc.lock().await;
            let service = buddy.as_mut().unwrap();
            for job_id in service
                .state
                .job_cooldowns
                .keys()
                .cloned()
                .collect::<Vec<_>>()
            {
                if job_id.starts_with("humor_") {
                    service.state.job_cooldowns.remove(&job_id);
                }
            }
        }

        let buddy = buddy_arc.lock().await;
        let rotation = &buddy.as_ref().unwrap().state.speech_rotation;
        let state = rotation
            .by_intent
            .get(crate::buddy::speech_policy::intent_key(SpeechIntent::Humor))
            .unwrap();
        assert_eq!(state.hour_count, 5);
    }

    #[tokio::test]
    async fn scheduler_emits_one_speech_per_tick_but_other_outputs_apply() {
        let dir = tempfile::tempdir().unwrap();
        let scheduler = BuddyScheduler {
            jobs: vec![
                Box::new(SpeechJob {
                    id: "speech_low".to_string(),
                    priority: 0,
                    intent: SpeechIntent::Humor,
                    activity_title: "low activity".to_string(),
                }),
                Box::new(SpeechJob {
                    id: "speech_high".to_string(),
                    priority: 1,
                    intent: SpeechIntent::ErrorAlert,
                    activity_title: "high activity".to_string(),
                }),
            ],
        };
        let buddy_arc = test_service(&dir, crate::buddy::state::default_buddy_state()).await;
        let gcx = crate::global_context::tests::make_test_gcx().await;

        scheduler.tick(gcx, buddy_arc.clone(), dir.path()).await;

        let buddy = buddy_arc.lock().await;
        let service = buddy.as_ref().unwrap();
        assert_eq!(
            service.active_speech.as_ref().unwrap().text,
            "speech speech_high"
        );
        assert_eq!(service.state.recent_activities.len(), 2);
        let rotation = &service.state.speech_rotation;
        assert!(rotation
            .by_intent
            .contains_key(crate::buddy::speech_policy::intent_key(
                SpeechIntent::ErrorAlert
            )));
        assert!(!rotation
            .by_intent
            .contains_key(crate::buddy::speech_policy::intent_key(SpeechIntent::Humor)));
    }

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

    #[tokio::test]
    async fn unrecorded_no_output_result_does_not_advance_job_state() {
        let dir = tempfile::tempdir().unwrap();
        let job_id = "no_output_unrecorded".to_string();
        let mut state = crate::buddy::state::default_buddy_state();
        state.job_cooldowns.insert(
            job_id.clone(),
            BuddyJobState {
                last_run: None,
                last_result: Some("existing-json".to_string()),
                run_count: 7,
                snoozed_until: None,
                dismissed: false,
            },
        );
        let (tx, _) = tokio::sync::broadcast::channel(16);
        let service = BuddyService::new(
            dir.path().to_path_buf(),
            state,
            BuddySettings::default(),
            Vec::new(),
            crate::buddy::runtime_queue::RuntimeQueue::new(),
            tx,
            None,
        );
        let scheduler = BuddyScheduler {
            jobs: vec![Box::new(NoOutputUnrecordedJob)],
        };
        let buddy_arc = Arc::new(AMutex::new(Some(service)));
        let gcx = crate::global_context::tests::make_test_gcx().await;

        scheduler.tick(gcx, buddy_arc.clone(), dir.path()).await;

        let buddy = buddy_arc.lock().await;
        let job_state = buddy
            .as_ref()
            .unwrap()
            .state
            .job_cooldowns
            .get(&job_id)
            .unwrap();
        assert!(job_state.last_run.is_none());
        assert_eq!(job_state.run_count, 7);
        assert_eq!(job_state.last_result.as_deref(), Some("existing-json"));
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
