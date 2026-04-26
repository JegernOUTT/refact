use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;
use chrono::Utc;
use tokio::sync::{broadcast, RwLock as ARwLock};
use tracing::{info, warn};

use crate::global_context::GlobalContext;
use super::events::BuddyEvent;
use super::settings::BuddySettings;
use super::snapshot::BuddySnapshot;
use super::types::{BuddyActivity, BuddyState, BuddySuggestion};

const SAVE_INTERVAL_SECS: u64 = 60;
const SUGGESTION_RATE_LIMIT_SECS: u64 = 30;
const SUGGESTION_EXPIRY_SECS: i64 = 300;

pub struct BuddyService {
    pub state: BuddyState,
    pub settings: BuddySettings,
    pub events_tx: broadcast::Sender<BuddyEvent>,
    pub last_suggestion_at: Option<Instant>,
    pub recent_diagnostics: Vec<super::diagnostics::DiagnosticContext>,
    pub last_issue_at: Option<Instant>,
    pub recent_issue_errors: Vec<(String, chrono::DateTime<chrono::Utc>)>,
}

impl BuddyService {
    pub fn new(state: BuddyState, settings: BuddySettings, events_tx: broadcast::Sender<BuddyEvent>) -> Self {
        Self { state, settings, events_tx, last_suggestion_at: None, recent_diagnostics: Vec::new(), last_issue_at: None, recent_issue_errors: Vec::new() }
    }

    pub fn snapshot(&self) -> BuddySnapshot {
        BuddySnapshot {
            state: self.state.clone(),
            settings: self.settings.clone(),
            enabled: self.settings.enabled,
        }
    }

    pub fn add_activity(&mut self, activity: BuddyActivity) {
        super::state::add_activity(&mut self.state, activity.clone());
        let _ = self.events_tx.send(BuddyEvent::ActivityAdded { activity });
    }

    pub fn grant_xp(&mut self, amount: u64) {
        super::state::grant_xp(&mut self.state, amount);
        let _ = self.events_tx.send(BuddyEvent::StateUpdated { state: self.state.clone() });
    }

    pub fn add_suggestion(&mut self, suggestion: BuddySuggestion) {
        self.state.suggestion_state.push(suggestion.clone());
        self.last_suggestion_at = Some(Instant::now());
        let _ = self.events_tx.send(BuddyEvent::SuggestionAdded { suggestion });
    }

    pub fn maybe_add_suggestion(&mut self, suggestion: BuddySuggestion) -> bool {
        if let Some(last) = self.last_suggestion_at {
            if last.elapsed().as_secs() < SUGGESTION_RATE_LIMIT_SECS {
                return false;
            }
        }
        self.add_suggestion(suggestion);
        true
    }

    pub fn dismiss_suggestion(&mut self, id: &str) {
        if let Some(s) = self.state.suggestion_state.iter_mut().find(|s| s.id == id) {
            s.dismissed = true;
        }
        let _ = self.events_tx.send(BuddyEvent::SuggestionDismissed { suggestion_id: id.to_string() });
    }

    pub fn workflow_completed(&mut self, workflow_id: &str, xp: u64, activity: super::types::BuddyActivity) {
        self.add_activity(activity);
        self.grant_xp(xp);
        let now = Utc::now().to_rfc3339();
        if let Some(ws) = self.state.workflow_summaries.iter_mut().find(|w| w.workflow_id == workflow_id) {
            ws.last_run = Some(now);
            ws.run_count += 1;
            ws.last_outcome = Some("success".to_string());
        } else {
            self.state.workflow_summaries.push(super::types::BuddyWorkflowSummary {
                workflow_id: workflow_id.to_string(),
                last_run: Some(now),
                run_count: 1,
                last_outcome: Some("success".to_string()),
            });
        }
    }

    pub fn workflow_failed(&mut self, workflow_id: &str, activity: super::types::BuddyActivity) {
        self.add_activity(activity);
        let now = Utc::now().to_rfc3339();
        if let Some(ws) = self.state.workflow_summaries.iter_mut().find(|w| w.workflow_id == workflow_id) {
            ws.last_run = Some(now);
            ws.run_count += 1;
            ws.last_outcome = Some("failed".to_string());
        } else {
            self.state.workflow_summaries.push(super::types::BuddyWorkflowSummary {
                workflow_id: workflow_id.to_string(),
                last_run: Some(now),
                run_count: 1,
                last_outcome: Some("failed".to_string()),
            });
        }
    }

    pub fn add_diagnostic(&mut self, ctx: super::diagnostics::DiagnosticContext) {
        self.recent_diagnostics.push(ctx);
        if self.recent_diagnostics.len() > 100 {
            self.recent_diagnostics.remove(0);
        }
    }

    pub fn record_issue_created(&mut self, error_message: String) {
        self.last_issue_at = Some(Instant::now());
        self.recent_issue_errors.push((error_message, chrono::Utc::now()));
        if self.recent_issue_errors.len() > 200 {
            self.recent_issue_errors.remove(0);
        }
    }

    pub fn expire_suggestions(&mut self) {
        let now = chrono::Utc::now();
        let mut changed = false;
        for s in self.state.suggestion_state.iter_mut() {
            if s.dismissed {
                continue;
            }
            if let Ok(created) = chrono::DateTime::parse_from_rfc3339(&s.created_at) {
                let age = now.signed_duration_since(created).num_seconds();
                if age > SUGGESTION_EXPIRY_SECS {
                    s.dismissed = true;
                    changed = true;
                }
            }
        }
        if changed {
            let _ = self.events_tx.send(BuddyEvent::StateUpdated { state: self.state.clone() });
        }
    }
}

pub async fn buddy_background_task(gcx: Arc<ARwLock<GlobalContext>>) {
    let project_root = loop {
        if gcx.read().await.shutdown_flag.load(Ordering::SeqCst) {
            return;
        }
        let dirs = crate::files_correction::get_project_dirs(gcx.clone()).await;
        if let Some(root) = dirs.into_iter().next() {
            break root;
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    };

    if let Err(e) = super::storage::bootstrap_buddy_storage(&project_root).await {
        warn!("buddy: failed to bootstrap storage: {}", e);
        return;
    }

    let state = super::state::load_state(&project_root).await;
    let settings_path = project_root.join(".refact/buddy/settings.json");
    let settings_existed = tokio::fs::metadata(&settings_path).await.is_ok();
    let mut settings = super::settings::load_settings(&project_root).await;
    if !settings_existed {
        settings.palette_index = state.identity.palette_index;
    }

    let events_tx = gcx.read().await.buddy_events_tx.clone().expect("buddy_events_tx must be set");
    let service = BuddyService::new(state, settings, events_tx);

    let buddy_arc = gcx.read().await.buddy.clone();
    *buddy_arc.lock().await = Some(service);

    let agents_md = project_root.join("AGENTS.md");
    let setup_done = tokio::fs::try_exists(&agents_md).await.unwrap_or(false);
    if !setup_done {
        let mut guard = buddy_arc.lock().await;
        if let Some(svc) = guard.as_mut() {
            let already = svc.state.suggestion_state.iter().any(|s| s.suggestion_type == "setup");
            if !already {
                let suggestion = BuddySuggestion {
                    id: "setup".to_string(),
                    suggestion_type: "setup".to_string(),
                    title: "Set up this project".to_string(),
                    description: "Run setup to generate guidelines, integrations, and toolbox commands.".to_string(),
                    created_at: chrono::Utc::now().to_rfc3339(),
                    dismissed: false,
                };
                svc.add_suggestion(suggestion);
            }
        }
    }

    info!("buddy: service started for {:?}", project_root);

    let shutdown_flag = gcx.read().await.shutdown_flag.clone();
    let mut last_save = Instant::now();
    let mut expiry_tick: u64 = 0;

    loop {
        if shutdown_flag.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        expiry_tick += 1;
        if expiry_tick % 60 == 0 {
            let mut buddy = buddy_arc.lock().await;
            if let Some(svc) = buddy.as_mut() {
                svc.expire_suggestions();
            }
        }
        if last_save.elapsed().as_secs() >= SAVE_INTERVAL_SECS {
            let state_opt = {
                let buddy = buddy_arc.lock().await;
                buddy.as_ref().map(|s| s.state.clone())
            };
            if let Some(s) = state_opt {
                if let Err(e) = super::state::save_state(&project_root, &s).await {
                    warn!("buddy: failed to save state: {}", e);
                }
            }
            last_save = Instant::now();
        }
    }

    let state_opt = {
        let buddy = buddy_arc.lock().await;
        buddy.as_ref().map(|s| s.state.clone())
    };
    if let Some(s) = state_opt {
        let _ = super::state::save_state(&project_root, &s).await;
    }

    info!("buddy: background task stopped");
}
