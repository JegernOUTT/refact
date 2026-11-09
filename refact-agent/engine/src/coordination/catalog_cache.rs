use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};

use crate::app_state::AppState;
use super::catalog::WorkCatalog;
use super::error::WorkError;
use super::scope::WorkCaller;

#[derive(Default)]
struct Publications {
    epoch: u64,
    seq: u64,
    sessions: VecDeque<(u64, String)>,
}

#[derive(Default)]
pub(crate) struct SessionPublications(Mutex<Publications>);

impl SessionPublications {
    pub(crate) fn publish(&self, id: &str) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.seq = state.seq.wrapping_add(1);
        let seq = state.seq;
        state.sessions.push_back((seq, id.into()));
        while state.sessions.len() > 4096 {
            state.sessions.pop_front();
        }
    }
    pub(crate) fn invalidate(&self) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.epoch = state.epoch.wrapping_add(1);
    }
}

fn publications() -> &'static Mutex<Publications> {
    static STATE: OnceLock<Mutex<Publications>> = OnceLock::new();
    STATE.get_or_init(Default::default)
}

pub fn invalidate_sources() {
    let mut state = publications().lock().unwrap_or_else(|e| e.into_inner());
    state.epoch = state.epoch.wrapping_add(1);
}

pub(crate) fn publish_session(id: &str) {
    let mut state = publications().lock().unwrap_or_else(|e| e.into_inner());
    state.seq = state.seq.wrapping_add(1);
    let seq = state.seq;
    state.sessions.push_back((seq, id.into()));
    while state.sessions.len() > 4096 {
        state.sessions.pop_front();
    }
}

struct Entry {
    epoch: u64,
    session_epoch: u64,
    global_seq: u64,
    seq: u64,
    sources: (u64, u64, u64, refact_worktrees::service::RegistryRevision),
    catalog: Arc<WorkCatalog>,
    bytes: usize,
}

#[derive(Default)]
pub(super) struct CatalogCache(
    tokio::sync::Mutex<HashMap<String, Entry>>,
    Mutex<Option<Visible>>,
);

struct Visible {
    base: Arc<WorkCatalog>,
    disclosure: String,
    privacy: crate::privacy::classification_cache::PrivacyKey,
    catalog: Arc<WorkCatalog>,
}

pub(super) fn visible(
    app: &AppState,
    base: &Arc<WorkCatalog>,
    disclosure: &str,
) -> Option<Arc<WorkCatalog>> {
    let privacy = crate::privacy::classification_cache::privacy_key(&app.gcx).ok()?;
    let slot = app
        .work_views
        .2
         .1
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    slot.as_ref()
        .filter(|saved| {
            Arc::ptr_eq(&saved.base, base)
                && saved.disclosure == disclosure
                && saved.privacy == privacy
        })
        .map(|saved| saved.catalog.clone())
}

pub(super) fn save_visible(
    app: &AppState,
    base: Arc<WorkCatalog>,
    disclosure: String,
    privacy: crate::privacy::classification_cache::PrivacyKey,
    catalog: Arc<WorkCatalog>,
) {
    if crate::privacy::classification_cache::privacy_key(&app.gcx)
        .ok()
        .as_ref()
        == Some(&privacy)
        && size(&catalog) <= crate::file_activity::settings().max_work_history_bytes
    {
        *app.work_views
            .2
             .1
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(Visible {
            base,
            disclosure,
            privacy,
            catalog,
        });
    }
}

fn size(catalog: &WorkCatalog) -> usize {
    let roots = serde_json::to_vec(&(
        catalog.roots.all(),
        &catalog.degraded,
        &catalog.child_agents,
    ))
    .map_or(usize::MAX, |v| v.len());
    catalog.all_rows().iter().fold(roots, |bytes, row| {
        bytes.saturating_add(
            serde_json::to_vec(&(row, &row.source_records)).map_or(usize::MAX, |v| v.len()),
        )
    })
}

pub(super) async fn neutral(
    app: &AppState,
    caller: &WorkCaller,
) -> Result<Arc<WorkCatalog>, WorkError> {
    let key = caller.source_root.to_string_lossy().into_owned();
    let mut entries = app.work_views.2 .0.lock().await;
    let session_epoch = app
        .chat
        .sessions
        .1
         .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .epoch;
    let sources = (
        app.agents.revision(),
        app.work_scheduler.revision(),
        crate::file_activity::observations_revision(),
        refact_worktrees::service::registry_revision(&app.gcx.cache_dir),
    );
    let (epoch, global_seq, seq, mut dirty) = {
        let state = publications().lock().unwrap_or_else(|e| e.into_inner());
        let local = app
            .chat
            .sessions
            .1
             .0
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let entry = entries.get(&key);
        let valid = entry.is_some_and(|entry| {
            entry.epoch == state.epoch
                && entry.session_epoch == session_epoch
                && entry.sources == sources
                && state
                    .sessions
                    .front()
                    .is_none_or(|(first, _)| entry.global_seq.saturating_add(1) >= *first)
                && local
                    .sessions
                    .front()
                    .is_none_or(|(first, _)| entry.seq.saturating_add(1) >= *first)
        });
        if valid && entry.unwrap().seq == local.seq && entry.unwrap().global_seq == state.seq {
            return Ok(entry.unwrap().catalog.clone());
        }
        let dirty = valid.then(|| {
            state
                .sessions
                .iter()
                .filter(|(seq, _)| *seq > entry.unwrap().global_seq)
                .chain(
                    local
                        .sessions
                        .iter()
                        .filter(|(seq, _)| *seq > entry.unwrap().seq),
                )
                .map(|(_, id)| id.clone())
                .collect::<std::collections::HashSet<_>>()
        });
        (state.epoch, state.seq, local.seq, dirty)
    };
    if dirty.as_ref().is_some_and(|dirty| dirty.is_empty()) {
        return Ok(entries[&key].catalog.clone());
    }
    if let Some(ids) = &dirty {
        let sessions = app.chat.sessions.read().await;
        let scope_changed = ids.iter().any(|id| {
            sessions.get(id).is_some_and(|session| {
                session.try_lock().is_ok_and(|session| {
                    super::scope::session_source_root(&session).is_none_or(|root| {
                        super::roots::project_identity(&root) != entries[&key].catalog.project
                    })
                })
            })
        });
        if scope_changed || ids.iter().any(|id| !sessions.contains_key(id)) {
            dirty = None;
        }
    }
    let catalog = match dirty {
        Some(ref dirty) => {
            let mut catalog = (*entries[&key].catalog).clone();
            let tickets = app.work_scheduler.snapshot().tickets;
            for id in dirty {
                let session = app.chat.sessions.read().await.get(id).cloned();
                let row = if let Some(session) = session {
                    let session = session.try_lock().map_err(|_| WorkError::unavailable())?;
                    super::scope::session_source_root(&session)
                        .filter(|root| super::roots::project_identity(root) == catalog.project)
                        .map(|_| super::catalog::row_from_session(&session, &catalog.roots, false))
                } else {
                    None
                };
                catalog.refresh_session(app, id, row, &tickets).await?;
            }
            catalog
        }
        None => {
            #[cfg(test)]
            let _ = CATALOG_BUILDS.try_with(|count| count.set(count.get() + 1));
            super::catalog::build_catalog_for_destination(app, Some(&key), true)
                .await
                .map_err(|_| WorkError::unavailable())?
        }
    };
    let bytes = size(&catalog);
    let state = publications().lock().unwrap_or_else(|e| e.into_inner());
    let local = app
        .chat
        .sessions
        .1
         .0
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if state.epoch != epoch
        || state.seq != global_seq
        || local.seq != seq
        || local.epoch != session_epoch
        || sources
            != (
                app.agents.revision(),
                app.work_scheduler.revision(),
                crate::file_activity::observations_revision(),
                refact_worktrees::service::registry_revision(&app.gcx.cache_dir),
            )
    {
        return Err(WorkError::unavailable());
    }
    let catalog = Arc::new(catalog);
    let settings = crate::file_activity::settings();
    if catalog.degraded.is_empty()
        && bytes <= settings.max_work_history_bytes
        && catalog.len() <= settings.max_work_history_rows
    {
        entries.remove(&key);
        while entries.len() >= settings.max_work_views.max(1)
            || entries
                .values()
                .map(|entry| entry.bytes)
                .sum::<usize>()
                .saturating_add(bytes)
                > settings.max_work_history_bytes
        {
            let Some(oldest) = entries.keys().next().cloned() else {
                break;
            };
            entries.remove(&oldest);
        }
        entries.insert(
            key,
            Entry {
                epoch,
                session_epoch,
                global_seq,
                seq,
                sources,
                catalog: catalog.clone(),
                bytes,
            },
        );
    }
    Ok(catalog)
}

#[cfg(test)]
tokio::task_local! {
    pub(super) static CATALOG_BUILDS: std::cell::Cell<usize>;
}
