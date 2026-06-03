use std::sync::Arc;
use std::path::Path;
use tokio::sync::Mutex as AMutex;

use super::actor::BuddyService;
use super::diagnostics::DiagnosticContext;
use super::settings::BuddySettings;
use super::types::{
    BuddyActivity, BuddyBubblePolicy, BuddyFact, BuddyJobState, BuddyOnboarding,
    BuddyPersonalityProfile, BuddyPetState, BuddyPulse, BuddyRuntimeEvent, BuddySpeechItem,
    BuddySuggestion, BuddyWorkflowSummary, BuddyChatPhraseBank,
};
use super::voice_service::SpeechIntent;
use crate::buddy::autonomous_workflows::is_autonomous_workflow_id;
use crate::app_state::AppState;

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
    pub recent_activities: Vec<BuddyActivity>,
}

pub struct BuddyJobResult {
    pub speech_intent: Option<SpeechIntent>,
    pub speech: Option<BuddySpeechItem>,
    pub suggestion: Option<BuddySuggestion>,
    pub activity: Option<BuddyActivity>,
    pub runtime_event: Option<BuddyRuntimeEvent>,
    pub chat_phrase_bank: Option<BuddyChatPhraseBank>,
    pub workflow_failure: Option<super::workflows::WorkflowFailureReport>,
    pub last_result: Option<String>,
    pub xp: u64,
}

impl Default for BuddyJobResult {
    fn default() -> Self {
        Self {
            speech_intent: None,
            speech: None,
            suggestion: None,
            activity: None,
            runtime_event: None,
            chat_phrase_bank: None,
            workflow_failure: None,
            last_result: None,
            xp: 0,
        }
    }
}

impl BuddyJobResult {
    fn has_visible_output(&self) -> bool {
        self.speech.is_some()
            || self.suggestion.is_some()
            || self.activity.is_some()
            || self.runtime_event.is_some()
            || self.chat_phrase_bank.is_some()
            || self.workflow_failure.is_some()
            || self.xp > 0
    }
}

fn next_last_result(existing: Option<&str>, result: Option<&str>) -> Option<String> {
    result.or(existing).map(ToString::to_string)
}

fn should_record_job_result(result: &BuddyJobResult, records_empty_result: bool) -> bool {
    records_empty_result || result.has_visible_output() || result.last_result.is_some()
}

pub(crate) fn speech_runtime_event(
    job_id: &str,
    intent: SpeechIntent,
    speech: &BuddySpeechItem,
    title: String,
    description: Option<String>,
) -> BuddyRuntimeEvent {
    let intent_key = super::speech_policy::intent_key(intent);
    let dedupe_key = speech
        .dedupe_key
        .as_deref()
        .map(|key| format!("speech_runtime:{job_id}:{key}"))
        .unwrap_or_else(|| format!("speech_runtime:{job_id}:{intent_key}"));
    let mut event = super::actor::make_runtime_event(
        &format!("speech_{intent_key}"),
        if title.trim().is_empty() {
            &speech.text
        } else {
            &title
        },
        job_id,
        &dedupe_key,
        "completed",
        Some(speech_runtime_priority(intent)),
    );
    event.description = description.filter(|text| !text.trim().is_empty());
    event.ttl_ms = Some(speech.ttl_seconds.saturating_mul(1000).max(1000));
    event.speech_text = Some(speech.text.clone());
    event.scene = Some(speech_runtime_scene(intent).to_string());
    event.duration_hint = Some(speech.ttl_seconds.min(u64::from(u32::MAX)) as u32);
    event.persistent = speech.persistent;
    event.controls = speech.controls.clone();
    event.chat_id = speech.chat_id.clone();
    event.bubble_policy = Some(speech_runtime_bubble_policy(intent));
    event
}

fn speech_runtime_bubble_policy(intent: SpeechIntent) -> BuddyBubblePolicy {
    match intent {
        SpeechIntent::Humor | SpeechIntent::Insight | SpeechIntent::MemoryPulseCommentary => {
            BuddyBubblePolicy::Ambient
        }
        SpeechIntent::Greeting
        | SpeechIntent::Tour
        | SpeechIntent::Milestone
        | SpeechIntent::Win
        | SpeechIntent::QuestAccept
        | SpeechIntent::QuestComplete
        | SpeechIntent::Suggestion
        | SpeechIntent::ErrorAlert => BuddyBubblePolicy::Durable,
    }
}

fn speech_runtime_priority(intent: SpeechIntent) -> &'static str {
    match intent {
        SpeechIntent::ErrorAlert => "high",
        SpeechIntent::Win
        | SpeechIntent::Milestone
        | SpeechIntent::QuestAccept
        | SpeechIntent::QuestComplete => "normal",
        SpeechIntent::Insight | SpeechIntent::MemoryPulseCommentary => "normal",
        SpeechIntent::Greeting | SpeechIntent::Tour | SpeechIntent::Suggestion => "low",
        SpeechIntent::Humor => "low",
    }
}

fn speech_runtime_scene(intent: SpeechIntent) -> &'static str {
    match intent {
        SpeechIntent::ErrorAlert => "alert",
        SpeechIntent::Win | SpeechIntent::Milestone | SpeechIntent::QuestComplete => "celebrate",
        SpeechIntent::Insight | SpeechIntent::MemoryPulseCommentary | SpeechIntent::Suggestion => {
            "insight"
        }
        SpeechIntent::Greeting | SpeechIntent::Tour | SpeechIntent::QuestAccept => "welcome",
        SpeechIntent::Humor => "playful",
    }
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
    fn is_autonomous(&self) -> bool {
        is_autonomous_workflow_id(self.id())
    }
    async fn should_run(&self, gcx: AppState, ctx: &BuddyJobContext) -> bool;
    async fn execute(&self, gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult;
}

pub(crate) const MAX_UNREAD_SUGGESTIONS: usize = 3;
pub(crate) const MAX_AUTONOMOUS_JOBS_PER_TICK: usize = 2;

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
            super::jobs::buddy_daily_digest::BuddyDailyDigestJob,
        ));
        s.jobs.push(Box::new(
            super::jobs::buddy_friday_retro::BuddyFridayRetroJob,
        ));
        s.jobs
            .push(Box::new(super::jobs::chat_phrase_bank::ChatPhraseBankJob));
        s.jobs.push(Box::new(
            super::jobs::buddy_idle_suggester::BuddyIdleSuggesterJob,
        ));
        s.jobs.push(Box::new(
            super::jobs::buddy_pr_issue_matchmaker::BuddyPrIssueMatchmakerJob,
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
        s.jobs
            .push(Box::new(super::jobs::buddy_onboarding::BuddyOnboardingJob));
        s.jobs.push(Box::new(
            super::jobs::buddy_refactor_hunter::BuddyRefactorHunterJob,
        ));
        s.jobs.push(Box::new(
            super::jobs::buddy_skill_author::BuddySkillAuthorJob,
        ));
        s.jobs.push(Box::new(
            super::jobs::buddy_test_coverage_watcher::BuddyTestCoverageWatcherJob,
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
        gcx: AppState,
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
        let mut autonomous_jobs_run = 0usize;
        for job in &self.jobs {
            let is_autonomous = job.is_autonomous();
            if is_autonomous && autonomous_jobs_run >= MAX_AUTONOMOUS_JOBS_PER_TICK {
                continue;
            }
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
                recent_activities: state.recent_activities.clone(),
            };
            if !job.should_run(gcx.clone(), &ctx).await {
                continue;
            }
            if is_autonomous {
                autonomous_jobs_run += 1;
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
            if result.speech.is_some() && result.speech_intent.is_some() && Some(idx) != winner {
                result.speech = None;
                result.speech_intent = None;
            }
            if should_record_job_result(&result, records_empty_result) {
                let mut workflow_failure_write = None;
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
                    if let Some(speech) = result.speech {
                        svc.update_speech(speech);
                        if let Some(intent) = result.speech_intent {
                            super::speech_policy::record_emission(
                                &mut svc.state.speech_rotation,
                                intent,
                                now,
                            );
                        }
                        svc.dirty = true;
                    }
                    if let Some(event) = result.runtime_event {
                        svc.enqueue_runtime_event(event);
                    }
                    if let Some(phrase_bank) = result.chat_phrase_bank {
                        svc.state.chat_phrase_bank = Some(phrase_bank);
                        svc.dirty = true;
                    }
                    if let Some(report) = result.workflow_failure {
                        workflow_failure_write = svc.record_workflow_failure_report(report);
                    }
                    if result.xp > 0 {
                        svc.grant_xp(result.xp);
                    }
                }
                drop(buddy);
                if let Some((path, report)) = workflow_failure_write {
                    BuddyService::append_workflow_failure_transcript(&path, &report).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buddy::autonomous_workflows::AUTONOMOUS_BUDDY_WORKFLOWS;

    struct NoOutputUnrecordedJob {
        last_result: Option<String>,
    }

    struct NoOutputRecordedJob {
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    struct ReadyAutonomousJob {
        id: String,
    }

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

        async fn should_run(&self, _gcx: AppState, _ctx: &BuddyJobContext) -> bool {
            true
        }

        async fn execute(&self, _gcx: AppState, _ctx: BuddyJobContext) -> BuddyJobResult {
            BuddyJobResult {
                last_result: self.last_result.clone(),
                ..Default::default()
            }
        }
    }

    #[async_trait::async_trait]
    impl BuddyJob for NoOutputRecordedJob {
        fn id(&self) -> &str {
            "no_output_recorded"
        }

        fn cooldown_seconds(&self) -> u64 {
            60 * 60
        }

        fn priority(&self) -> u32 {
            0
        }

        fn records_empty_result(&self) -> bool {
            true
        }

        async fn should_run(&self, _gcx: AppState, _ctx: &BuddyJobContext) -> bool {
            true
        }

        async fn execute(&self, _gcx: AppState, _ctx: BuddyJobContext) -> BuddyJobResult {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            BuddyJobResult::default()
        }
    }

    #[async_trait::async_trait]
    impl BuddyJob for ReadyAutonomousJob {
        fn id(&self) -> &str {
            &self.id
        }

        fn cooldown_seconds(&self) -> u64 {
            0
        }

        fn priority(&self) -> u32 {
            0
        }

        fn is_autonomous(&self) -> bool {
            true
        }

        async fn should_run(&self, _gcx: AppState, _ctx: &BuddyJobContext) -> bool {
            true
        }

        async fn execute(&self, _gcx: AppState, _ctx: BuddyJobContext) -> BuddyJobResult {
            BuddyJobResult {
                xp: 4,
                activity: Some(BuddyActivity {
                    icon: "•".to_string(),
                    title: self.id.clone(),
                    description: self.id.clone(),
                    timestamp: chrono::Utc::now().to_rfc3339(),
                    activity_type: "test".to_string(),
                    chat_id: None,
                    failure_category: None,
                    failure_summary: None,
                }),
                ..Default::default()
            }
        }
    }

    struct NoIntentSpeechJob;

    #[async_trait::async_trait]
    impl BuddyJob for NoIntentSpeechJob {
        fn id(&self) -> &str {
            "no_intent_speech"
        }

        fn cooldown_seconds(&self) -> u64 {
            0
        }

        fn priority(&self) -> u32 {
            0
        }

        async fn should_run(&self, _gcx: AppState, _ctx: &BuddyJobContext) -> bool {
            true
        }

        async fn execute(&self, _gcx: AppState, _ctx: BuddyJobContext) -> BuddyJobResult {
            BuddyJobResult {
                speech: Some(BuddySpeechItem {
                    id: "no-intent-speech".to_string(),
                    text: "no intent speech".to_string(),
                    mood: "happy".to_string(),
                    scope: "global".to_string(),
                    persistent: false,
                    ttl_seconds: 10,
                    dedupe_key: Some("no-intent-speech".to_string()),
                    created_at: chrono::Utc::now().to_rfc3339(),
                    controls: vec![],
                    chat_id: None,
                }),
                speech_intent: None,
                ..Default::default()
            }
        }
    }

    struct SpeechJob {
        id: String,
        priority: u32,
        intent: SpeechIntent,
        activity_title: String,
        runtime_event: bool,
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

        async fn should_run(&self, _gcx: AppState, _ctx: &BuddyJobContext) -> bool {
            true
        }

        async fn execute(&self, _gcx: AppState, _ctx: BuddyJobContext) -> BuddyJobResult {
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
                    failure_category: None,
                    failure_summary: None,
                }),
                runtime_event: self.runtime_event.then(|| {
                    let title = format!("runtime {}", self.id);
                    let mut event = crate::buddy::actor::make_runtime_event(
                        "test_signal",
                        &title,
                        self.id(),
                        &format!("runtime-{}", self.id),
                        "completed",
                        None,
                    );
                    event.speech_text = Some(format!("runtime speech {}", self.id));
                    event
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
    async fn speech_without_intent_is_not_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let scheduler = BuddyScheduler {
            jobs: vec![Box::new(NoIntentSpeechJob)],
        };
        let buddy_arc = test_service(&dir, crate::buddy::state::default_buddy_state()).await;
        let gcx = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;

        scheduler.tick(gcx, buddy_arc.clone(), dir.path()).await;

        let buddy = buddy_arc.lock().await;
        let service = buddy.as_ref().unwrap();
        assert!(service.active_speech.is_some());
    }

    fn speech_item(intent: SpeechIntent) -> BuddySpeechItem {
        BuddySpeechItem {
            id: format!("speech-{}", crate::buddy::speech_policy::intent_key(intent)),
            text: "test speech".to_string(),
            mood: intent.mood().to_string(),
            scope: "global".to_string(),
            persistent: false,
            ttl_seconds: 10,
            dedupe_key: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            controls: vec![],
            chat_id: None,
        }
    }

    #[test]
    fn speech_runtime_event_sets_ambient_policy_for_humor() {
        let event = speech_runtime_event(
            "test_job",
            SpeechIntent::Humor,
            &speech_item(SpeechIntent::Humor),
            "Humor".to_string(),
            None,
        );

        assert_eq!(event.bubble_policy, Some(BuddyBubblePolicy::Ambient));
    }

    #[test]
    fn speech_runtime_event_sets_durable_policy_for_greeting() {
        let event = speech_runtime_event(
            "test_job",
            SpeechIntent::Greeting,
            &speech_item(SpeechIntent::Greeting),
            "Greeting".to_string(),
            None,
        );

        assert_eq!(event.bubble_policy, Some(BuddyBubblePolicy::Durable));
    }

    #[tokio::test]
    async fn competing_speech_intents_pick_one_winner() {
        let dir = tempfile::tempdir().unwrap();
        let scheduler = BuddyScheduler {
            jobs: vec![
                Box::new(SpeechJob {
                    id: "speech_one".to_string(),
                    priority: 0,
                    intent: SpeechIntent::Humor,
                    activity_title: "one activity".to_string(),
                    runtime_event: false,
                }),
                Box::new(SpeechJob {
                    id: "speech_two".to_string(),
                    priority: 1,
                    intent: SpeechIntent::ErrorAlert,
                    activity_title: "two activity".to_string(),
                    runtime_event: false,
                }),
            ],
        };
        let buddy_arc = test_service(&dir, crate::buddy::state::default_buddy_state()).await;
        let gcx = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;

        scheduler.tick(gcx, buddy_arc.clone(), dir.path()).await;

        let buddy = buddy_arc.lock().await;
        let service = buddy.as_ref().unwrap();
        assert_eq!(
            service.active_speech.as_ref().unwrap().text,
            "speech speech_two"
        );
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
                        runtime_event: false,
                    }) as Box<dyn BuddyJob>
                })
                .collect(),
        };
        let buddy_arc = test_service(&dir, crate::buddy::state::default_buddy_state()).await;
        let gcx = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;

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
                    runtime_event: false,
                }),
                Box::new(SpeechJob {
                    id: "speech_high".to_string(),
                    priority: 1,
                    intent: SpeechIntent::ErrorAlert,
                    activity_title: "high activity".to_string(),
                    runtime_event: false,
                }),
            ],
        };
        let buddy_arc = test_service(&dir, crate::buddy::state::default_buddy_state()).await;
        let gcx = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;

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

    #[tokio::test]
    async fn scheduler_preserves_runtime_events_from_unselected_speech_jobs() {
        let dir = tempfile::tempdir().unwrap();
        let scheduler = BuddyScheduler {
            jobs: vec![
                Box::new(SpeechJob {
                    id: "speech_low".to_string(),
                    priority: 0,
                    intent: SpeechIntent::Humor,
                    activity_title: "low activity".to_string(),
                    runtime_event: true,
                }),
                Box::new(SpeechJob {
                    id: "speech_high".to_string(),
                    priority: 1,
                    intent: SpeechIntent::ErrorAlert,
                    activity_title: "high activity".to_string(),
                    runtime_event: true,
                }),
            ],
        };
        let buddy_arc = test_service(&dir, crate::buddy::state::default_buddy_state()).await;
        let gcx = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;

        scheduler.tick(gcx, buddy_arc.clone(), dir.path()).await;

        let buddy = buddy_arc.lock().await;
        let service = buddy.as_ref().unwrap();
        assert_eq!(
            service.active_speech.as_ref().unwrap().text,
            "speech speech_high"
        );
        assert_eq!(service.runtime_queue.items.len(), 2);
        assert!(service
            .runtime_queue
            .items
            .iter()
            .any(|event| event.title == "runtime speech_low"
                && event.speech_text.as_deref() == Some("runtime speech speech_low")));
        assert!(service
            .runtime_queue
            .items
            .iter()
            .any(|event| event.title == "runtime speech_high"
                && event.speech_text.as_deref() == Some("runtime speech speech_high")));
    }

    #[tokio::test]
    async fn speech_rotation_allows_win_when_error_over_budget() {
        let dir = tempfile::tempdir().unwrap();
        let scheduler = BuddyScheduler {
            jobs: vec![
                Box::new(SpeechJob {
                    id: "speech_error".to_string(),
                    priority: 0,
                    intent: SpeechIntent::ErrorAlert,
                    activity_title: "error activity".to_string(),
                    runtime_event: false,
                }),
                Box::new(SpeechJob {
                    id: "speech_win".to_string(),
                    priority: 1,
                    intent: SpeechIntent::Win,
                    activity_title: "win activity".to_string(),
                    runtime_event: false,
                }),
            ],
        };
        let mut state = crate::buddy::state::default_buddy_state();
        let now = chrono::Utc::now();
        state.speech_rotation.by_intent.insert(
            crate::buddy::speech_policy::intent_key(SpeechIntent::ErrorAlert).to_string(),
            crate::buddy::state::IntentBudgetState {
                last_emitted_at: Some(now),
                hour_count: 2,
                day_count: 2,
                hour_window_start: Some(now),
                day_window_start: Some(now),
            },
        );
        let buddy_arc = test_service(&dir, state).await;
        let gcx = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;

        scheduler.tick(gcx, buddy_arc.clone(), dir.path()).await;

        let buddy = buddy_arc.lock().await;
        let service = buddy.as_ref().unwrap();
        assert_eq!(
            service.active_speech.as_ref().unwrap().text,
            "speech speech_win"
        );
        assert!(service
            .state
            .speech_rotation
            .by_intent
            .contains_key(crate::buddy::speech_policy::intent_key(SpeechIntent::Win)));
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
            jobs: vec![Box::new(NoOutputUnrecordedJob { last_result: None })],
        };
        let buddy_arc = Arc::new(AMutex::new(Some(service)));
        let gcx = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;

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

    #[tokio::test]
    async fn no_output_last_result_advances_job_state_for_backoff() {
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
        let scheduler = BuddyScheduler {
            jobs: vec![Box::new(NoOutputUnrecordedJob {
                last_result: Some("failed:today".to_string()),
            })],
        };
        let buddy_arc = test_service(&dir, state).await;
        let gcx = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;

        scheduler.tick(gcx, buddy_arc.clone(), dir.path()).await;

        let buddy = buddy_arc.lock().await;
        let job_state = buddy
            .as_ref()
            .unwrap()
            .state
            .job_cooldowns
            .get(&job_id)
            .unwrap();
        assert!(job_state.last_run.is_some());
        assert_eq!(job_state.run_count, 8);
        assert_eq!(job_state.last_result.as_deref(), Some("failed:today"));
    }

    #[tokio::test]
    async fn recorded_no_output_result_advances_cooldown_and_blocks_next_tick() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::buddy::state::default_buddy_state();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let scheduler = BuddyScheduler {
            jobs: vec![Box::new(NoOutputRecordedJob {
                calls: calls.clone(),
            })],
        };
        let buddy_arc = test_service(&dir, state).await;
        let gcx = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;

        scheduler
            .tick(gcx.clone(), buddy_arc.clone(), dir.path())
            .await;
        scheduler.tick(gcx, buddy_arc.clone(), dir.path()).await;

        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        let buddy = buddy_arc.lock().await;
        let job_state = buddy
            .as_ref()
            .unwrap()
            .state
            .job_cooldowns
            .get("no_output_recorded")
            .unwrap();
        assert!(job_state.last_run.is_some());
        assert_eq!(job_state.run_count, 1);
    }

    #[tokio::test]
    async fn failed_phrase_bank_generation_records_scheduler_backoff() {
        let (service, renderer) = crate::buddy::voice_service::test_voice_service_with_responses(
            vec![Some("humor: only one usable kind".to_string())],
        );
        let _guard = crate::buddy::voice_service::install_test_voice_service(service).await;
        let dir = tempfile::tempdir().unwrap();
        let mut settings = BuddySettings::default();
        settings.daily_digest_hour = Some(0);
        let (tx, _) = tokio::sync::broadcast::channel(16);
        let service = BuddyService::new(
            dir.path().to_path_buf(),
            crate::buddy::state::default_buddy_state(),
            settings,
            Vec::new(),
            crate::buddy::runtime_queue::RuntimeQueue::new(),
            tx,
            None,
        );
        let scheduler = BuddyScheduler {
            jobs: vec![Box::new(
                crate::buddy::jobs::chat_phrase_bank::ChatPhraseBankJob,
            )],
        };
        let buddy_arc = Arc::new(AMutex::new(Some(service)));
        let gcx = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;

        scheduler
            .tick(gcx.clone(), buddy_arc.clone(), dir.path())
            .await;
        scheduler.tick(gcx, buddy_arc.clone(), dir.path()).await;

        assert_eq!(renderer.calls(), 1);
        let buddy = buddy_arc.lock().await;
        let job_state = buddy
            .as_ref()
            .unwrap()
            .state
            .job_cooldowns
            .get("chat_phrase_bank")
            .unwrap();
        assert!(job_state.last_run.is_some());
        assert_eq!(job_state.run_count, 1);
        assert!(job_state
            .last_result
            .as_deref()
            .is_some_and(|result| result.starts_with("failed:")));
    }

    #[tokio::test]
    async fn tick_caps_autonomous_jobs_at_two() {
        let dir = tempfile::tempdir().unwrap();
        let scheduler = BuddyScheduler {
            jobs: (0..4)
                .map(|idx| {
                    Box::new(ReadyAutonomousJob {
                        id: format!("autonomous_{idx}"),
                    }) as Box<dyn BuddyJob>
                })
                .collect(),
        };
        let buddy_arc = test_service(&dir, crate::buddy::state::default_buddy_state()).await;
        let gcx = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;

        scheduler.tick(gcx, buddy_arc.clone(), dir.path()).await;

        let buddy = buddy_arc.lock().await;
        let job_cooldowns = &buddy.as_ref().unwrap().state.job_cooldowns;
        assert_eq!(job_cooldowns.len(), MAX_AUTONOMOUS_JOBS_PER_TICK);
        assert!(job_cooldowns.contains_key("autonomous_0"));
        assert!(job_cooldowns.contains_key("autonomous_1"));
        assert!(!job_cooldowns.contains_key("autonomous_2"));
        assert!(!job_cooldowns.contains_key("autonomous_3"));
    }

    #[tokio::test]
    async fn autonomous_job_success_grants_bounded_xp() {
        let dir = tempfile::tempdir().unwrap();
        let scheduler = BuddyScheduler {
            jobs: vec![Box::new(ReadyAutonomousJob {
                id: "autonomous_xp".to_string(),
            })],
        };
        let buddy_arc = test_service(&dir, crate::buddy::state::default_buddy_state()).await;
        let gcx = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;

        scheduler
            .tick(gcx.clone(), buddy_arc.clone(), dir.path())
            .await;

        {
            let buddy = buddy_arc.lock().await;
            let service = buddy.as_ref().unwrap();
            assert_eq!(service.state.progression.xp, 4);
            assert_eq!(service.state.progression.stage, 0);
        }

        for _ in 0..4 {
            {
                let mut buddy = buddy_arc.lock().await;
                buddy
                    .as_mut()
                    .unwrap()
                    .state
                    .job_cooldowns
                    .remove("autonomous_xp");
            }
            scheduler
                .tick(gcx.clone(), buddy_arc.clone(), dir.path())
                .await;
        }

        let buddy = buddy_arc.lock().await;
        let service = buddy.as_ref().unwrap();
        assert_eq!(service.state.progression.stage, 1);
        assert_eq!(service.state.progression.xp, 0);
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
                failure_category: None,
                failure_summary: None,
                progress: None,
                dedupe_key: None,
                priority: "normal".to_string(),
                created_at: chrono::Utc::now().to_rfc3339(),
                ttl_ms: None,
                bubble_policy: None,
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
