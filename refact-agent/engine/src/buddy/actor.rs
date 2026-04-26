use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;
use chrono::Utc;
use tokio::sync::{broadcast, RwLock as ARwLock};
use tracing::{info, warn};
use uuid::Uuid;

use crate::global_context::GlobalContext;
use super::events::BuddyEvent;
use super::runtime_queue::RuntimeQueue;
use super::settings::BuddySettings;
use super::snapshot::BuddySnapshot;
use super::types::{BuddyActivity, BuddyRuntimeEvent, BuddyState, BuddySuggestion};

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
    pub runtime_queue: RuntimeQueue,
}

impl BuddyService {
    pub fn new(state: BuddyState, settings: BuddySettings, events_tx: broadcast::Sender<BuddyEvent>) -> Self {
        Self { state, settings, events_tx, last_suggestion_at: None, recent_diagnostics: Vec::new(), last_issue_at: None, recent_issue_errors: Vec::new(), runtime_queue: RuntimeQueue::new() }
    }

    pub fn snapshot(&self) -> BuddySnapshot {
        BuddySnapshot {
            state: self.state.clone(),
            settings: self.settings.clone(),
            enabled: self.settings.enabled,
            runtime_queue: self.runtime_queue.items.iter().cloned().collect(),
            now_playing: self.runtime_queue.now_playing.clone(),
        }
    }

    pub fn enqueue_runtime_event(&mut self, event: BuddyRuntimeEvent) {
        let _ = self.events_tx.send(BuddyEvent::RuntimeEvent { event: event.clone() });
        self.runtime_queue.enqueue(event);
    }

    pub fn update_runtime_progress(&mut self, dedupe_key: &str, progress: u8, title: Option<&str>) {
        self.runtime_queue.update_progress(dedupe_key, progress, title);
        if let Some(e) = self.runtime_queue.items.iter().find(|e| e.dedupe_key.as_deref() == Some(dedupe_key)) {
            let _ = self.events_tx.send(BuddyEvent::RuntimeEvent { event: e.clone() });
        }
    }

    pub fn complete_runtime_event(&mut self, dedupe_key: &str, status: &str) {
        self.runtime_queue.complete(dedupe_key, status);
        if let Some(e) = self.runtime_queue.items.iter().find(|e| e.dedupe_key.as_deref() == Some(dedupe_key)) {
            let _ = self.events_tx.send(BuddyEvent::RuntimeEvent { event: e.clone() });
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
        if self.state.suggestion_state.len() >= 50 {
            if let Some(pos) = self.state.suggestion_state.iter().position(|s| s.dismissed) {
                self.state.suggestion_state.remove(pos);
            }
        }
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
        let dupe = self.state.suggestion_state.iter().any(|s| {
            !s.dismissed && s.suggestion_type == suggestion.suggestion_type && s.title == suggestion.title
        });
        if dupe {
            return false;
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
        super::state::add_activity(&mut self.state, activity.clone());
        let _ = self.events_tx.send(BuddyEvent::ActivityAdded { activity });
        super::state::grant_xp(&mut self.state, xp);
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
        let _ = self.events_tx.send(BuddyEvent::StateUpdated { state: self.state.clone() });
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
        let _ = self.events_tx.send(BuddyEvent::StateUpdated { state: self.state.clone() });
    }

    pub fn add_diagnostic(&mut self, ctx: super::diagnostics::DiagnosticContext) {
        self.recent_diagnostics.push(ctx.clone());
        if self.recent_diagnostics.len() > 100 {
            self.recent_diagnostics.remove(0);
        }
        let _ = self.events_tx.send(BuddyEvent::DiagnosticAdded { diagnostic: ctx });
    }

    pub fn record_issue_created(&mut self, error_message: String) {
        self.last_issue_at = Some(Instant::now());
        self.recent_issue_errors.push((error_message, chrono::Utc::now()));
        if self.recent_issue_errors.len() > 200 {
            self.recent_issue_errors.remove(0);
        }
    }

    pub async fn append_workflow_transcript(
        &self,
        project_root: &std::path::Path,
        workflow_id: &str,
        output_summary: &str,
        success: bool,
    ) {
        let path = project_root.join(format!(".refact/buddy/chats/workflows/{}.json", workflow_id));
        super::workflows::append_workflow_entry(&path, output_summary, success).await;
    }

    pub fn report_error(&mut self, error_type: &str, error_msg: &str, source: Option<&str>, chat_id: Option<&str>) {
        let lower = error_msg.to_lowercase();
        let severity = if lower.contains("critical") || lower.contains("panic") {
            super::diagnostics::DiagnosticSeverity::Critical
        } else if lower.contains("error") {
            super::diagnostics::DiagnosticSeverity::High
        } else if lower.contains("warn") {
            super::diagnostics::DiagnosticSeverity::Medium
        } else {
            super::diagnostics::DiagnosticSeverity::High
        };
        let ctx = super::diagnostics::DiagnosticContext {
            error_type: error_type.to_string(),
            error_message: error_msg.to_string(),
            source_file: source.map(|s| s.to_string()),
            tool_name: None,
            chat_id: chat_id.map(|s| s.to_string()),
            collected_at: Utc::now().to_rfc3339(),
            severity,
        };
        self.add_diagnostic(ctx);
        let end = 80.min(error_msg.len());
        let truncated = &error_msg[..end];
        self.add_activity(BuddyActivity {
            icon: "⚠️".to_string(),
            title: format!("{}: {}", error_type, truncated),
            description: error_msg.to_string(),
            timestamp: Utc::now().to_rfc3339(),
            activity_type: "error".to_string(),
        });
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
        let before = self.state.suggestion_state.len();
        self.state.suggestion_state.retain(|s| {
            if !s.dismissed {
                return true;
            }
            if let Ok(created) = chrono::DateTime::parse_from_rfc3339(&s.created_at) {
                now.signed_duration_since(created).num_seconds() < 3600
            } else {
                false
            }
        });
        if changed || self.state.suggestion_state.len() != before {
            let _ = self.events_tx.send(BuddyEvent::StateUpdated { state: self.state.clone() });
        }
    }
}

pub fn make_runtime_event(
    signal_type: &str,
    title: &str,
    source: &str,
    dedupe_key: &str,
    status: &str,
    priority: Option<&str>,
) -> BuddyRuntimeEvent {
    BuddyRuntimeEvent {
        id: Uuid::new_v4().to_string(),
        signal_type: signal_type.to_string(),
        title: title.to_string(),
        description: None,
        source: source.to_string(),
        status: status.to_string(),
        progress: None,
        dedupe_key: Some(dedupe_key.to_string()),
        priority: priority.unwrap_or("normal").to_string(),
        created_at: Utc::now().to_rfc3339(),
        ttl_ms: None,
    }
}

pub async fn buddy_report_bg(gcx: Arc<ARwLock<GlobalContext>>, signal_type: &str, _icon: &str, title: &str, _description: &str) {
    let event = make_runtime_event(signal_type, title, "system", signal_type, "completed", None);
    buddy_enqueue_event(gcx, event).await;
}

pub async fn buddy_complete_event(gcx: Arc<ARwLock<GlobalContext>>, dedupe_key: &str, status: &str) {
    let buddy_arc = gcx.read().await.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    if let Some(svc) = lock.as_mut() {
        svc.complete_runtime_event(dedupe_key, status);
    }
}

pub async fn buddy_enqueue_event(gcx: Arc<ARwLock<GlobalContext>>, event: BuddyRuntimeEvent) {
    let buddy_arc = gcx.read().await.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    if let Some(svc) = lock.as_mut() {
        svc.enqueue_runtime_event(event);
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
    let settings = if tokio::fs::metadata(&settings_path).await.is_ok() {
        super::settings::load_settings(&project_root).await
    } else {
        let mut s = BuddySettings::default();
        s.palette_index = state.identity.palette_index;
        super::settings::save_settings(&project_root, &s).await.ok();
        s
    };

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
