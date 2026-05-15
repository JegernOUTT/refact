use std::future::Future;
use std::sync::Arc;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock as ARwLock;
use tracing::warn;

use crate::global_context::GlobalContext;
use super::types::BuddyActivity;

pub fn workflow_label(workflow_id: &str) -> &str {
    match workflow_id {
        "commit_msg" => "commit message generation",
        "follow_up" => "follow-up suggestions",
        "compression" => "chat compression",
        "memory_extract" => "memo extraction",
        "knowledge_update" => "knowledge graph update",
        "title_generating" => "title generation",
        // Legacy IDs still map to labels for backwards-compat transcripts
        "commit_message" => "commit message generation",
        "compress_trajectory" => "chat compression",
        "memo_extraction" => "memo extraction",
        "kg_enrich" => "knowledge graph enrichment",
        "kg_deprecate" => "knowledge cleanup",
        _ => workflow_id,
    }
}

/// Maps internal workflow IDs to canonical Buddy signal_type names.
/// The GUI uses these names in its signal catalog.
pub fn canonical_signal_type(workflow_id: &str) -> &str {
    match workflow_id {
        "commit_message" | "commit_msg" => "commit_msg",
        "compress_trajectory" | "compression" => "compression",
        "memo_extraction" | "memory_extract" => "memory_extract",
        "kg_enrich" | "kg_deprecate" | "knowledge_update" => "knowledge_update",
        "title_generating" | "title_generation" => "title_generating",
        "follow_up" => "generating",
        other => other,
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct WorkflowEntry {
    timestamp: String,
    input_summary: String,
    output_summary: String,
    success: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct WorkflowTranscript {
    entries: Vec<WorkflowEntry>,
}

const MAX_ENTRIES: usize = 100;

pub async fn buddy_wrap_workflow<T, F, Fut>(
    gcx: Arc<ARwLock<GlobalContext>>,
    workflow_id: &str,
    icon: &str,
    xp: u64,
    summary_fn: impl Fn(&T) -> String,
    workflow_fn: F,
) -> Result<T, String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, String>>,
{
    let label = workflow_label(workflow_id);
    let signal_type = canonical_signal_type(workflow_id);
    let dedupe_key = format!("workflow_{}", workflow_id);
    let mut started = crate::buddy::actor::make_runtime_event(
        signal_type,
        &format!("Running {}...", label),
        "system",
        &dedupe_key,
        "started",
        None,
    );
    started.speech_text = Some(format!("I'm working on {}...", label));
    started.scene = Some("working".to_string());
    started.persistent = true;
    crate::buddy::actor::buddy_enqueue_event(gcx.clone(), started).await;

    let result = workflow_fn().await;

    let (success, summary) = match &result {
        Ok(output) => (true, summary_fn(output)),
        Err(e) => (false, e.clone()),
    };

    let buddy_arc = gcx.read().await.buddy.clone();
    let voice_gcx = gcx.clone();
    let project_dirs = crate::files_correction::get_project_dirs(gcx.clone()).await;
    let project_root = project_dirs.into_iter().next();
    let workflow_id_owned = workflow_id.to_string();
    let icon_owned = icon.to_string();

    tokio::spawn(async move {
        let activity = BuddyActivity {
            icon: icon_owned,
            title: summary.clone(),
            description: String::new(),
            timestamp: Utc::now().to_rfc3339(),
            activity_type: "workflow".to_string(),
            chat_id: None,
        };

        let mut completed_quest = None;
        let mut quest_voice_state = None;
        {
            let mut buddy = buddy_arc.lock().await;
            if let Some(svc) = buddy.as_mut() {
                let status = if success { "completed" } else { "failed" };
                svc.complete_runtime_event(&dedupe_key, status);
                if success {
                    svc.add_activity(activity);
                    crate::buddy::state::grant_xp(&mut svc.state, xp);
                    let now = Utc::now().to_rfc3339();
                    if let Some(ws) = svc
                        .state
                        .workflow_summaries
                        .iter_mut()
                        .find(|ws| ws.workflow_id == workflow_id_owned)
                    {
                        ws.run_count = ws.run_count.saturating_add(1);
                        ws.last_run = Some(now.clone());
                        ws.last_outcome = Some("success".to_string());
                    } else {
                        svc.state.workflow_summaries.push(
                            crate::buddy::types::BuddyWorkflowSummary {
                                workflow_id: workflow_id_owned.clone(),
                                last_run: Some(now.clone()),
                                run_count: 1,
                                last_outcome: Some("success".to_string()),
                            },
                        );
                    }
                    svc.refresh_active_quest();
                    svc.dirty = true;
                    let _ = svc
                        .events_tx
                        .send(crate::buddy::events::BuddyEvent::StateUpdated {
                            state: svc.state.clone(),
                        });
                    let reward = svc
                        .state
                        .active_quest
                        .as_ref()
                        .filter(|quest| quest.status == "active" && quest.progress >= quest.goal)
                        .map(|quest| quest.reward_xp);
                    if let Some(reward) = reward {
                        completed_quest =
                            crate::buddy::state::complete_active_quest(&mut svc.state);
                        quest_voice_state = Some((
                            svc.state.personality.clone(),
                            svc.state.identity.name.clone(),
                            svc.pulse.clone(),
                            reward,
                        ));
                        svc.dirty = true;
                        let _ =
                            svc.events_tx
                                .send(crate::buddy::events::BuddyEvent::StateUpdated {
                                    state: svc.state.clone(),
                                });
                    }
                } else {
                    svc.workflow_failed(&workflow_id_owned, activity);
                }
                if let Some(ref root) = project_root {
                    svc.append_workflow_transcript(root, &workflow_id_owned, &summary, success)
                        .await;
                }
            }
        }

        if let (Some(quest), Some((persona, identity_name, pulse, reward))) =
            (completed_quest, quest_voice_state)
        {
            let completed = crate::buddy::actor::complete_quest_with_voice(
                voice_gcx.clone(),
                quest,
                persona,
                identity_name,
                pulse,
            )
            .await;
            crate::buddy::actor::buddy_update_speech(voice_gcx.clone(), completed.speech).await;
            crate::buddy::actor::buddy_apply(voice_gcx.clone(), completed.mutation).await;
            if reward > 0 {
                let buddy_arc = voice_gcx.read().await.buddy.clone();
                let mut buddy = buddy_arc.lock().await;
                if let Some(svc) = buddy.as_mut() {
                    svc.grant_xp(reward);
                }
            }
        }
    });

    result
}

pub async fn append_workflow_entry(path: &std::path::Path, output_summary: &str, success: bool) {
    let entry = WorkflowEntry {
        timestamp: Utc::now().to_rfc3339(),
        input_summary: String::new(),
        output_summary: output_summary.to_string(),
        success,
    };

    let mut transcript = match tokio::fs::read_to_string(path).await {
        Ok(content) => serde_json::from_str::<WorkflowTranscript>(&content)
            .unwrap_or(WorkflowTranscript { entries: vec![] }),
        Err(_) => WorkflowTranscript { entries: vec![] },
    };

    transcript.entries.push(entry);
    if transcript.entries.len() > MAX_ENTRIES {
        let drain = transcript.entries.len() - MAX_ENTRIES;
        transcript.entries.drain(0..drain);
    }

    if let Err(e) = super::storage::atomic_write_json(path, &transcript).await {
        warn!(
            "buddy: failed to write workflow transcript {:?}: {}",
            path, e
        );
    }
}
