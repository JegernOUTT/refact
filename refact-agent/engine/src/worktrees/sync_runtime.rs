use std::sync::Arc;

use crate::worktrees::service::WorktreeService;
use crate::worktrees::sync_authority;
use crate::worktrees::types::{WorktreeSyncRequest, WorktreeSyncResponse};

#[derive(serde::Serialize)]
pub(crate) struct SyncResponse {
    #[serde(flatten)]
    pub sync: WorktreeSyncResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub writer_readiness: Option<sync_authority::WriterReadiness>,
}

pub(crate) async fn status_readiness(
    app: &crate::app_state::AppState,
    service: &WorktreeService,
    id: &str,
    actor: Option<&str>,
) -> Result<sync_authority::WriterReadiness, String> {
    let registry = service.load_registry().await?;
    let record = registry
        .records
        .iter()
        .find(|r| r.meta.id == id)
        .ok_or("Source worktree not found")?;
    sync_authority::source_writer_readiness(app, &record.meta, &record.references, actor).await
}

pub(crate) async fn sync_under_lease(
    app: &crate::app_state::AppState,
    service: &WorktreeService,
    id: &str,
    request: WorktreeSyncRequest,
    actor: String,
) -> Result<WorktreeSyncResponse, String> {
    let registry = service.load_registry().await?;
    let view = registry
        .records
        .into_iter()
        .find(|r| r.meta.id == id)
        .ok_or("Source worktree not found")?;
    let mut _transitions = Vec::new();
    if !matches!(request, WorktreeSyncRequest::Status { .. }) {
        let sessions: Vec<_> = app.chat.sessions.read().await.values().cloned().collect();
        for session in sessions {
            let attached = session
                .lock()
                .await
                .thread
                .worktree
                .as_ref()
                .is_some_and(|w| w == &view.meta);
            if attached {
                _transitions.push(crate::chat::worktree_lifecycle::try_transition_lock(
                    &session,
                )?);
            }
        }
    }
    let mut service = service
        .acquire_merge_permit()
        .await?
        .with_sync_actor(actor.clone());
    if !matches!(request, WorktreeSyncRequest::Status { .. }) {
        let app = app.clone();
        let meta = view.meta.clone();
        let registry_service = service.clone();
        let runtime = tokio::runtime::Handle::current();
        service = service.with_merge_authority(Arc::new(move || {
            runtime.block_on(async {
                let registry = registry_service.load_registry().await?;
                let record = registry
                    .records
                    .iter()
                    .find(|r| r.meta == meta)
                    .ok_or("Source registration changed")?;
                sync_authority::ensure_source_quiescent(
                    &app,
                    &meta,
                    &record.references,
                    Some(&actor),
                )
                .await
            })
        }));
    }
    let Some(leases) =
        crate::coordination::leases::merge_leases_for(app, service.source_workspace_root()).await?
    else {
        return service.sync_worktree(id, request).await;
    };
    let gate = crate::coordination::merge_gate::MergeGate::new(
        leases.clone(),
        &crate::file_activity::settings(),
    );
    let slot = gate
        .acquire(&crate::coordination::merge_gate::checkout_key(
            &view.meta.root,
        ))
        .await
        .map_err(|e| e.to_string())?;
    let service = service.with_merge_authority(leases.phase_validator(slot.holder().clone()));
    let mut renewal =
        crate::coordination::leases::LeaseRenewal::spawn(leases, slot.holder().clone());
    let result = service.sync_worktree(id, request).await;
    if let Err(error) = renewal.shutdown().await {
        tracing::warn!(%error, "sync lease release failed");
    }
    result
}
