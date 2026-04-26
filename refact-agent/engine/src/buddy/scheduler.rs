use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tracing::{info, warn};
use tokio::sync::Mutex as AMutex;

use crate::global_context::GlobalContext;
use crate::custom_error::ScratchError;
use super::actor::BuddyService;
use super::types::{BuddyJobState, BuddySpeechItem, BuddySuggestion, BuddyActivity, BuddyRuntimeEvent, BuddyOnboarding};
use super::diagnostics::DiagnosticContext;

pub struct BuddyJobContext {
    pub identity_name: String,
    pub onboarding: BuddyOnboarding,
    pub recent_diagnostics: Vec<DiagnosticContext>,
    pub suggestion_unread_count: usize,
    pub project_root: std::path::PathBuf,
    pub job_state: BuddyJobState,
}

pub struct BuddyJobResult {
    pub speech: Option<BuddySpeechItem>,
    pub suggestion: Option<BuddySuggestion>,
    pub activity: Option<BuddyActivity>,
    pub runtime_event: Option<BuddyRuntimeEvent>,
}

impl Default for BuddyJobResult {
    fn default() -> Self {
        Self { speech: None, suggestion: None, activity: None, runtime_event: None }
    }
}

#[async_trait::async_trait]
pub trait BuddyJob: Send + Sync {
    fn id(&self) -> &str;
    fn cooldown_seconds(&self) -> u64;
    fn priority(&self) -> u32;
    async fn should_run(&self, gcx: Arc<tokio::sync::RwLock<GlobalContext>>, ctx: &BuddyJobContext) -> bool;
    async fn execute(&self, gcx: Arc<tokio::sync::RwLock<GlobalContext>>, ctx: BuddyJobContext) -> BuddyJobResult;
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
        s.jobs.push(Box::new(super::jobs::error_triage::ErrorTriageJob));
        s.jobs.push(Box::new(super::jobs::config_watcher::ConfigWatcherJob));
        s.jobs.push(Box::new(super::jobs::stats_watcher::StatsWatcherJob));
        s.jobs.push(Box::new(super::jobs::health_watcher::HealthWatcherJob));
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
                let unread = svc.state.suggestion_state.iter().filter(|s| !s.dismissed).count();
                if !svc.settings.proactive_enabled || unread >= MAX_UNREAD_SUGGESTIONS {
                    return None;
                }
                Some((svc.state.clone(), svc.recent_diagnostics.clone()))
            }).flatten()
        };
        let (state, diags) = match ctx_opt {
            Some(x) => x,
            None => return,
        };

        for job in &self.jobs {
            let job_state = state.job_cooldowns.get(job.id()).cloned().unwrap_or_default();
            if job_state.dismissed {
                continue;
            }
            let elapsed = job_state.last_run.as_deref()
                .and_then(|r| chrono::DateTime::parse_from_rfc3339(r).ok())
                .map(|t| chrono::Utc::now().signed_duration_since(t).num_seconds().max(0) as u64)
                .unwrap_or(u64::MAX);
            if elapsed < job.cooldown_seconds() {
                continue;
            }
            let ctx = BuddyJobContext {
                identity_name: state.identity.name.clone(),
                onboarding: state.onboarding.clone(),
                recent_diagnostics: diags.clone(),
                suggestion_unread_count: state.suggestion_state.iter().filter(|s| !s.dismissed).count(),
                project_root: project_root.to_path_buf(),
                job_state: job_state.clone(),
            };
            if !job.should_run(gcx.clone(), &ctx).await {
                continue;
            }
            let result = job.execute(gcx.clone(), ctx).await;
            let mut buddy = buddy_arc.lock().await;
            if let Some(svc) = buddy.as_mut() {
                let mut js = svc.state.job_cooldowns.entry(job.id().to_string()).or_default().clone();
                js.last_run = Some(chrono::Utc::now().to_rfc3339());
                js.run_count += 1;
                js.last_result = Some("ok".to_string());
                svc.state.job_cooldowns.insert(job.id().to_string(), js);
                svc.dirty = true;
                if let Some(suggestion) = result.suggestion {
                    svc.add_suggestion(suggestion);
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
            break; // max 1 job per tick
        }
    }
}
