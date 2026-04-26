use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::{broadcast, RwLock as ARwLock};
use tracing::{info, warn};

use crate::global_context::GlobalContext;
use super::events::BuddyEvent;
use super::settings::BuddySettings;
use super::snapshot::BuddySnapshot;
use super::types::{BuddyActivity, BuddyState, BuddySuggestion};

const SAVE_INTERVAL_SECS: u64 = 60;

pub struct BuddyService {
    pub state: BuddyState,
    pub settings: BuddySettings,
    pub events_tx: broadcast::Sender<BuddyEvent>,
}

impl BuddyService {
    pub fn new(state: BuddyState, settings: BuddySettings, events_tx: broadcast::Sender<BuddyEvent>) -> Self {
        Self { state, settings, events_tx }
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
        let _ = self.events_tx.send(BuddyEvent::SuggestionAdded { suggestion });
    }

    pub fn dismiss_suggestion(&mut self, id: &str) {
        if let Some(s) = self.state.suggestion_state.iter_mut().find(|s| s.id == id) {
            s.dismissed = true;
        }
        let _ = self.events_tx.send(BuddyEvent::SuggestionDismissed { suggestion_id: id.to_string() });
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

    info!("buddy: service started for {:?}", project_root);

    let shutdown_flag = gcx.read().await.shutdown_flag.clone();
    let mut last_save = std::time::Instant::now();

    loop {
        if shutdown_flag.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
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
            last_save = std::time::Instant::now();
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
