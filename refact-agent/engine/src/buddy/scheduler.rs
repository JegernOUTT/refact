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
use super::voice_service::{SpeechIntent, SpeechIntentWireToken};
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
    pub speech_candidate: Option<SpeechCandidate>,
    pub suggestion: Option<BuddySuggestion>,
    pub activity: Option<BuddyActivity>,
    pub runtime_event: Option<BuddyRuntimeEvent>,
    pub chat_phrase_bank: Option<BuddyChatPhraseBank>,
    pub workflow_failure: Option<super::workflows::WorkflowFailureReport>,
    pub opportunities: Vec<(super::types::BuddyOpportunity, u64)>,
    pub last_result: Option<String>,
    pub xp: u64,
    pub mark_greeted: bool,
}

impl Default for BuddyJobResult {
    fn default() -> Self {
        Self {
            speech_intent: None,
            speech: None,
            speech_candidate: None,
            suggestion: None,
            activity: None,
            runtime_event: None,
            chat_phrase_bank: None,
            workflow_failure: None,
            opportunities: Vec::new(),
            last_result: None,
            xp: 0,
            mark_greeted: false,
        }
    }
}

#[derive(Clone)]
pub struct SpeechCandidate {
    pub intent: SpeechIntent,
    pub editorial: super::editorial::Candidate,
    pub persona: BuddyPersonalityProfile,
    pub identity_name: String,
    pub pulse: BuddyPulse,
    pub workflow_id: Option<String>,
    pub workflow_summary: String,
    pub fallback_text: String,
    pub runtime_event_title: String,
    pub runtime_event_description: Option<String>,
    pub template: Option<BuddySpeechItem>,
}

impl SpeechCandidate {
    pub fn from_context(
        ctx: &BuddyJobContext,
        intent: SpeechIntent,
        workflow_id: Option<String>,
        workflow_summary: String,
        fallback_text: String,
        runtime_event_title: String,
        runtime_event_description: Option<String>,
    ) -> Self {
        Self {
            editorial: super::editorial::Candidate::new(
                intent.wire_token(),
                workflow_id.as_deref().unwrap_or(intent.wire_token()),
                &workflow_summary,
                chrono::Utc::now().timestamp(),
            ),
            intent,
            persona: ctx.personality.clone(),
            identity_name: ctx.identity_name.clone(),
            pulse: ctx.pulse.clone(),
            workflow_id,
            workflow_summary,
            fallback_text,
            runtime_event_title,
            runtime_event_description,
            template: None,
        }
    }
}

impl BuddyJobResult {
    fn has_visible_output(&self) -> bool {
        self.speech.is_some()
            || self.speech_candidate.is_some()
            || self.suggestion.is_some()
            || self.activity.is_some()
            || self.runtime_event.is_some()
            || self.chat_phrase_bank.is_some()
            || self.workflow_failure.is_some()
            || !self.opportunities.is_empty()
            || self.xp > 0
    }
}

pub(crate) async fn render_candidate(
    gcx: AppState,
    candidate: &SpeechCandidate,
) -> BuddySpeechItem {
    let rendered = super::actor::render_buddy_speech(
        gcx,
        candidate.persona.clone(),
        candidate.identity_name.clone(),
        candidate.pulse.clone(),
        candidate.workflow_id.clone(),
        candidate.workflow_summary.clone(),
        candidate.intent,
        candidate.fallback_text.clone(),
    )
    .await;
    if let Some(template) = candidate.template.clone() {
        super::jobs::tour::normalize_tour_speech(template, rendered)
    } else {
        rendered
    }
}

fn next_last_result(existing: Option<&str>, result: Option<&str>) -> Option<String> {
    result.or(existing).map(ToString::to_string)
}

fn should_record_job_result(result: &BuddyJobResult, records_empty_result: bool) -> bool {
    records_empty_result || result.has_visible_output() || result.last_result.is_some()
}

fn arbitration_speech_stub() -> BuddySpeechItem {
    BuddySpeechItem {
        editorial_delivery_required: false,
        id: String::new(),
        text: String::new(),
        mood: String::new(),
        scope: String::new(),
        persistent: false,
        ttl_seconds: 0,
        dedupe_key: None,
        speech_intent: None,
        created_at: String::new(),
        controls: vec![],
        chat_id: None,
    }
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

pub(crate) fn planned_speech_runtime_event(
    job_id: &str,
    candidate: &SpeechCandidate,
) -> BuddyRuntimeEvent {
    let intent_key = super::speech_policy::intent_key(candidate.intent);
    let mut event = super::actor::make_runtime_event(
        &format!("speech_{intent_key}"),
        &candidate.runtime_event_title,
        job_id,
        &format!("speech_runtime:{job_id}:{intent_key}"),
        "completed",
        Some(speech_runtime_priority(candidate.intent)),
    );
    event.description = candidate.runtime_event_description.clone();
    event.scene = Some(speech_runtime_scene(candidate.intent).to_string());
    event.bubble_policy = Some(speech_runtime_bubble_policy(candidate.intent));
    event
}

fn speech_runtime_bubble_policy(intent: SpeechIntent) -> BuddyBubblePolicy {
    match intent {
        SpeechIntent::Humor
        | SpeechIntent::Insight
        | SpeechIntent::MemoryPulseCommentary
        | SpeechIntent::ChatReaction => BuddyBubblePolicy::Ambient,
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
        SpeechIntent::Humor | SpeechIntent::ChatReaction => "low",
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
        SpeechIntent::Humor | SpeechIntent::ChatReaction => "playful",
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
    #[cfg(test)]
    pub(crate) fn with_test_job(job: Box<dyn BuddyJob>) -> Self {
        Self { jobs: vec![job] }
    }

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
        s.jobs
            .push(Box::new(super::jobs::buddy_briefing::BuddyBriefingJob));
        s.jobs.push(Box::new(
            super::jobs::buddy_handbook_distiller::BuddyHandbookDistillerJob,
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
            // Phrase-bank generation is eager ambient rendering, not a selected
            // editorial candidate. Keep it off until it joins the shared arbiter.
            if job.id() == "chat_phrase_bank"
                && buddy_arc
                    .lock()
                    .await
                    .as_ref()
                    .is_some_and(|svc| svc.operator_features.editorial)
            {
                continue;
            }
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
        let editorial_enabled = buddy_arc
            .lock()
            .await
            .as_ref()
            .is_some_and(|svc| svc.operator_features.editorial);
        let editorial_runtime = if editorial_enabled {
            super::operator_runtime::runtime(
                &gcx,
                super::operator_runtime::OperatorCapability::Editorial,
            )
            .await
            .ok()
        } else {
            None
        };
        let metadata: Vec<_> = ready_results
            .iter()
            .map(|(job, result, _)| {
                result
                    .speech_candidate
                    .as_ref()
                    .map(|c| c.editorial.clone())
                    .or_else(|| {
                        result
                            .speech_intent
                            .zip(result.speech.as_ref())
                            .map(|(intent, speech)| {
                                super::editorial::Candidate::new(
                                    intent.wire_token(),
                                    job,
                                    &speech.text,
                                    now.timestamp(),
                                )
                            })
                    })
            })
            .collect();
        let policy_allowed: Vec<bool> = {
            let buddy = buddy_arc.lock().await;
            metadata
                .iter()
                .enumerate()
                .map(|(idx, candidate)| {
                    let result = &ready_results[idx].1;
                    let intent = result
                        .speech_candidate
                        .as_ref()
                        .map(|c| c.intent)
                        .or(result.speech_intent);
                    candidate.is_some()
                        && buddy.as_ref().is_some_and(|svc| {
                            super::speech_policy::gate_speech(
                                &svc.settings,
                                &svc.state.speech_rotation,
                                intent,
                                None,
                                chrono::Timelike::hour(&chrono::Local::now()),
                                svc.auto_quiet_window,
                                now,
                            )
                            .allowed
                        })
                })
                .collect()
        };
        let editorial_allowed = if let Some(runtime) = &editorial_runtime {
            let metadata = metadata.clone();
            let policy = policy_allowed.clone();
            runtime
                .with_store(move |store| {
                    super::editorial::with_state(store, |state| {
                        state.prune(now.timestamp());
                        let allowed: Vec<bool> = metadata
                            .iter()
                            .enumerate()
                            .map(|(idx, candidate)| {
                                candidate.as_ref().is_some_and(|candidate| {
                                    state.counters.generated =
                                        state.counters.generated.saturating_add(1);
                                    let allowed = policy[idx]
                                        && state.evaluate(candidate, now.timestamp())
                                            == super::editorial::Decision::Eligible;
                                    if allowed {
                                        state.counters.eligible =
                                            state.counters.eligible.saturating_add(1);
                                    }
                                    allowed
                                })
                            })
                            .collect();
                        Ok(allowed)
                    })
                })
                .await
                .unwrap_or_else(|_| vec![false; ready_results.len()])
        } else {
            vec![!editorial_enabled; ready_results.len()]
        };
        let mut winner = {
            let buddy = buddy_arc.lock().await;
            let Some(svc) = buddy.as_ref() else {
                return;
            };
            let speech_result_indexes = ready_results
                .iter()
                .enumerate()
                .filter_map(|(idx, (_, result, _))| {
                    if !editorial_allowed[idx] {
                        return None;
                    }
                    let intent = result
                        .speech_candidate
                        .as_ref()
                        .map(|candidate| candidate.intent)
                        .or_else(|| {
                            result
                                .speech_intent
                                .zip(result.speech.as_ref())
                                .map(|(intent, _)| intent)
                        })?;
                    {
                        let local_hour = chrono::Timelike::hour(&chrono::Local::now());
                        super::speech_policy::gate_speech(
                            &svc.settings,
                            &svc.state.speech_rotation,
                            Some(intent),
                            None,
                            local_hour,
                            svc.auto_quiet_window,
                            now,
                        )
                        .allowed
                        .then_some(idx)
                    }
                })
                .collect::<Vec<_>>();
            let candidates = speech_result_indexes
                .iter()
                .filter_map(|idx| {
                    let (_, result, _) = &ready_results[*idx];
                    result
                        .speech_candidate
                        .as_ref()
                        .map(|candidate| (candidate.intent, arbitration_speech_stub()))
                        .or_else(|| result.speech_intent.zip(result.speech.clone()))
                })
                .collect::<Vec<_>>();
            let candidate_winner = super::speech_policy::pick_speech_intent(
                &candidates,
                &svc.state.speech_rotation,
                now,
            );
            candidate_winner.and_then(|winner_idx| speech_result_indexes.get(winner_idx).copied())
        };

        if let Some(runtime) = &editorial_runtime {
            if let Some(idx) = winner {
                if let Some(candidate) = metadata[idx].clone() {
                    let reserved = runtime
                        .with_store(move |store| {
                            super::editorial::with_state(store, |state| {
                                Ok(state.reserve(candidate, chrono::Utc::now().timestamp()))
                            })
                        })
                        .await;
                    if !matches!(reserved, Ok(super::editorial::Decision::Eligible)) {
                        winner = None;
                    }
                } else {
                    winner = None;
                }
            }
            let skipped = metadata
                .iter()
                .filter(|c| c.is_some())
                .count()
                .saturating_sub(usize::from(winner.is_some())) as u64;
            let _ = runtime
                .with_store(move |store| {
                    super::editorial::with_state(store, |state| {
                        state.counters.skipped = state.counters.skipped.saturating_add(skipped);
                        Ok(())
                    })
                })
                .await;
        }
        if let Some(winner_idx) = winner {
            if let Some(candidate) = ready_results[winner_idx].1.speech_candidate.clone() {
                let speech = render_candidate(gcx.clone(), &candidate).await;
                let still_allowed = {
                    let buddy = buddy_arc.lock().await;
                    buddy.as_ref().is_some_and(|svc| {
                        (!editorial_enabled || svc.operator_features.editorial)
                            && candidate.editorial.expires_at > chrono::Utc::now().timestamp()
                            && super::speech_policy::gate_speech(
                                &svc.settings,
                                &svc.state.speech_rotation,
                                Some(candidate.intent),
                                None,
                                chrono::Timelike::hour(&chrono::Local::now()),
                                svc.auto_quiet_window,
                                chrono::Utc::now(),
                            )
                            .allowed
                    })
                };
                if still_allowed {
                    let job_id = ready_results[winner_idx].0;
                    let result = &mut ready_results[winner_idx].1;
                    result.speech_intent = Some(candidate.intent);
                    result.speech = Some(speech.clone());
                    if candidate.template.is_none() {
                        result.runtime_event = Some(speech_runtime_event(
                            job_id,
                            candidate.intent,
                            &speech,
                            candidate.runtime_event_title,
                            candidate.runtime_event_description,
                        ));
                    }
                }
            }
        }

        if let (Some(runtime), Some(idx)) = (&editorial_runtime, winner) {
            let publishable = metadata[idx]
                .clone()
                .zip(ready_results[idx].1.speech.as_ref().map(|s| s.id.clone()));
            let published = if let Some((candidate, speech_id)) = publishable {
                runtime
                    .with_store(move |store| {
                        super::editorial::with_state(store, |state| {
                            state.publish(&candidate.id, &speech_id, chrono::Utc::now().timestamp())
                        })
                    })
                    .await
                    .is_ok()
            } else {
                false
            };
            if published {
                if let Some(speech) = ready_results[idx].1.speech.as_mut() {
                    speech.editorial_delivery_required = true;
                }
            }
            if !published {
                winner = None;
                let _ = runtime
                    .with_store(|store| {
                        super::editorial::with_state(store, |state| {
                            state.counters.skipped = state.counters.skipped.saturating_add(1);
                            Ok(())
                        })
                    })
                    .await;
            }
        }
        for (idx, (job_id, mut result, records_empty_result)) in
            ready_results.into_iter().enumerate()
        {
            let template_only = result
                .speech_candidate
                .as_ref()
                .is_some_and(|candidate| candidate.template.is_some());
            result.speech_candidate = None;
            if editorial_enabled {
                // Runtime events have no delivery claim contract. Keep factual state,
                // but never copy claimed speech into a second unclaimed surface.
                if let Some(event) = result.runtime_event.as_mut() {
                    event.speech_text = None;
                }
            }
            if editorial_enabled && Some(idx) != winner {
                result.speech = None;
                result.speech_intent = None;
            }
            if result.speech.is_some() && result.speech_intent.is_some() && Some(idx) != winner {
                result.speech = None;
                result.speech_intent = None;
            }
            if should_record_job_result(&result, records_empty_result) {
                let mut workflow_failure_write = None;
                let mut buddy = buddy_arc.lock().await;
                if let Some(svc) = buddy.as_mut() {
                    if template_only {
                        let Some(mut speech) = result.speech.take() else {
                            continue;
                        };
                        if let Some(intent) = result.speech_intent {
                            speech.speech_intent = Some(intent.wire_token().to_string());
                        }
                        if !svc.update_speech(speech) {
                            continue;
                        }
                        if result.mark_greeted {
                            super::state::mark_greeted(&mut svc.state);
                        }
                        result.last_result = Some("completed".into());
                    }
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
                    if let Some(activity) = result.activity {
                        svc.add_activity(activity);
                    }
                    if let Some(mut speech) = result.speech {
                        if let Some(intent) = result.speech_intent {
                            speech.speech_intent = Some(intent.wire_token().to_string());
                        }
                        let displayed = svc.update_speech(speech);
                        if displayed
                            && result.mark_greeted
                            && super::state::mark_greeted(&mut svc.state)
                        {
                            svc.dirty = true;
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
                let source_owner = buddy.as_ref().map(BuddyService::admission_identity);
                drop(buddy);
                if let Some(suggestion) = result.suggestion {
                    let admission = {
                        let mut buddy = buddy_arc.lock().await;
                        buddy
                            .as_mut()
                            .filter(|svc| {
                                source_owner
                                    .as_ref()
                                    .is_some_and(|owner| svc.has_admission_identity(owner))
                            })
                            .ok_or("Buddy job owner changed".to_string())
                            .and_then(|svc| svc.prepare_suggestion(suggestion))
                    };
                    if let Err(error) = async { admission?.commit(&buddy_arc).await }.await {
                        tracing::warn!(%error, "operator suggestion admission failed");
                    }
                }
                for (opportunity, cooldown_secs) in result.opportunities {
                    let admission = {
                        let mut guard = buddy_arc.lock().await;
                        guard
                            .as_mut()
                            .filter(|svc| {
                                source_owner
                                    .as_ref()
                                    .is_some_and(|owner| svc.has_admission_identity(owner))
                            })
                            .ok_or("Buddy job owner changed".to_string())
                            .and_then(|svc| svc.prepare_opportunity(opportunity, cooldown_secs))
                    };
                    if let Err(error) = async { admission?.commit(&buddy_arc).await }.await {
                        tracing::warn!(%error, "operator job admission failed");
                    }
                }
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
                    editorial_delivery_required: false,
                    id: "no-intent-speech".to_string(),
                    text: "no intent speech".to_string(),
                    mood: "happy".to_string(),
                    scope: "global".to_string(),
                    persistent: false,
                    ttl_seconds: 10,
                    dedupe_key: Some("no-intent-speech".to_string()),
                    speech_intent: None,
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

    struct DeferredSpeechJob {
        id: String,
        intent: SpeechIntent,
        activity_title: String,
    }

    #[async_trait::async_trait]
    impl BuddyJob for DeferredSpeechJob {
        fn id(&self) -> &str {
            &self.id
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

        async fn execute(&self, _gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
            let candidate = SpeechCandidate::from_context(
                &ctx,
                self.intent,
                None,
                self.activity_title.clone(),
                self.activity_title.clone(),
                self.activity_title.clone(),
                Some(self.activity_title.clone()),
            );
            BuddyJobResult {
                activity: Some(BuddyActivity {
                    icon: "•".to_string(),
                    title: self.activity_title.clone(),
                    description: self.activity_title.clone(),
                    timestamp: chrono::Utc::now().to_rfc3339(),
                    activity_type: "test".to_string(),
                    chat_id: None,
                    failure_category: None,
                    failure_summary: None,
                }),
                runtime_event: Some(planned_speech_runtime_event(self.id(), &candidate)),
                speech_candidate: Some(candidate),
                ..Default::default()
            }
        }
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
                    editorial_delivery_required: false,
                    id: format!("speech-{}", self.id),
                    text: format!("speech {}", self.id),
                    mood: "happy".to_string(),
                    scope: "global".to_string(),
                    persistent: false,
                    ttl_seconds: 10,
                    dedupe_key: Some(format!("speech-{}", self.id)),
                    speech_intent: None,
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
    async fn enabled_editorial_without_storage_fails_closed_before_rendering() {
        let (service, renderer) = crate::buddy::voice_service::test_voice_service_with_responses(
            vec![Some("must not render".into())],
        );
        let _guard = crate::buddy::voice_service::install_test_voice_service(service).await;
        let dir = tempfile::tempdir().unwrap();
        let scheduler = BuddyScheduler {
            jobs: vec![Box::new(DeferredSpeechJob {
                id: "unavailable_editorial".into(),
                intent: SpeechIntent::Insight,
                activity_title: "saved factual event".into(),
            })],
        };
        let buddy_arc = test_service(&dir, crate::buddy::state::default_buddy_state()).await;
        buddy_arc
            .lock()
            .await
            .as_mut()
            .unwrap()
            .operator_features
            .editorial = true;
        let gcx = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        scheduler.tick(gcx, buddy_arc.clone(), dir.path()).await;
        assert_eq!(renderer.calls(), 0);
        let buddy = buddy_arc.lock().await;
        let service = buddy.as_ref().unwrap();
        assert!(service.active_speech.is_none());
        assert_eq!(service.state.recent_activities.len(), 1);
        assert_eq!(service.runtime_queue.items.len(), 1);
        assert!(service.runtime_queue.items[0].speech_text.is_none());
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
}
