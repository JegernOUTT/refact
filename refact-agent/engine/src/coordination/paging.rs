use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use refact_work_core::CoordinationSettings;
use serde::Serialize;
use uuid::Uuid;

use super::error::{WorkError, WorkFailureCause};
use super::service::WorkSnapshot;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ViewBinding {
    pub project: String,
    pub principal: String,
    pub disclosure: String,
    pub filter: String,
}

struct View {
    binding: ViewBinding,
    fingerprint: String,
    generation: u64,
    accessed_at: u64,
    snapshots: BTreeMap<u64, FrozenView>,
}

struct FrozenView {
    snapshot: Arc<WorkSnapshot>,
    fingerprint: String,
    observed_at: u64,
    expires_at: u64,
    units: usize,
    bytes: usize,
}

struct Cursor {
    binding: ViewBinding,
    view_id: String,
    generation: u64,
    offset: usize,
    expires_at: u64,
}

#[derive(Default)]
struct State {
    views: HashMap<String, View>,
    cursors: HashMap<String, Cursor>,
}

#[derive(Default)]
pub struct WorkViews(
    Mutex<State>,
    std::sync::OnceLock<(std::time::Instant, u64)>,
    pub(super) super::catalog_cache::CatalogCache,
);

pub struct PageWindow {
    pub snapshot_id: String,
    pub seq: u64,
    pub offset: usize,
    pub end: usize,
    pub next_cursor: Option<String>,
}

pub(super) struct FrozenPage {
    pub window: PageWindow,
    pub snapshot: WorkSnapshot,
    pub total: usize,
    pub observed_at: u64,
    pub stale: bool,
}

impl WorkViews {
    pub fn beginning_cursor(
        &self,
        snapshot_id: &str,
        seq: u64,
        now_ms: u64,
    ) -> Result<String, WorkError> {
        let settings = crate::file_activity::settings();
        settings
            .validate()
            .map_err(|_| WorkError::projection_failure(WorkFailureCause::SettingsUnavailable))?;
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.prune_snapshots(&settings, now_ms);
        let view = state
            .views
            .get(snapshot_id)
            .filter(|view| {
                view.snapshots.contains_key(&seq)
                    || (view.snapshots.is_empty() && view.generation == seq)
            })
            .ok_or_else(WorkError::cursor_expired)?;
        let binding = view.binding.clone();
        let expires_at = view.snapshots.get(&seq).map_or_else(
            || now_ms.saturating_add(settings.work_cursor_ttl_secs.saturating_mul(1000)),
            |snapshot| snapshot.expires_at,
        );
        if let Some((token, _)) = state.cursors.iter().find(|(_, cursor)| {
            cursor.view_id == snapshot_id
                && cursor.generation == seq
                && cursor.offset == 0
                && cursor.expires_at > now_ms
        }) {
            return Ok(token.clone());
        }
        while state.cursors.len() >= settings.max_work_cursors {
            let oldest = state
                .cursors
                .iter()
                .min_by_key(|(token, cursor)| (cursor.expires_at, *token))
                .map(|(token, _)| token.clone())
                .ok_or_else(WorkError::unavailable)?;
            state.cursors.remove(&oldest);
        }
        let token = format!("w1.{}", Uuid::new_v4());
        state.cursors.insert(
            token.clone(),
            Cursor {
                binding,
                view_id: snapshot_id.into(),
                generation: seq,
                offset: 0,
                expires_at,
            },
        );
        Ok(token)
    }

    pub fn observation_time(&self, instant: std::time::Instant) -> u64 {
        let (anchor, epoch) = self
            .1
            .get_or_init(|| (std::time::Instant::now(), super::service::now_ms()));
        match instant.checked_duration_since(*anchor) {
            Some(after) => epoch.saturating_add(after.as_millis() as u64),
            None => epoch.saturating_sub(anchor.duration_since(instant).as_millis() as u64),
        }
    }

    pub fn page(
        &self,
        binding: ViewBinding,
        fingerprint: String,
        total: usize,
        limit: usize,
        cursor: Option<&str>,
        settings: &CoordinationSettings,
        now_ms: u64,
    ) -> Result<PageWindow, WorkError> {
        settings
            .validate()
            .map_err(|_| WorkError::projection_failure(WorkFailureCause::SettingsUnavailable))?;
        if limit == 0 || limit > settings.overview_max_rows {
            return Err(WorkError::invalid(
                "limit is outside the configured page range",
            ));
        }
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        // Read the requested cursor before pruning so known expiry is reported
        // accurately. A missing/evicted token also requires a fresh snapshot.
        let continuation = match cursor {
            None => None,
            Some(token) => {
                if !token.starts_with("w1.") || Uuid::parse_str(&token[3..]).is_err() {
                    return Err(WorkError::invalid_cursor());
                }
                let saved = state
                    .cursors
                    .get(token)
                    .ok_or_else(WorkError::cursor_expired)?;
                if saved.binding != binding {
                    return Err(WorkError::invalid_cursor());
                }
                if now_ms >= saved.expires_at {
                    return Err(WorkError::cursor_expired());
                }
                Some((saved.view_id.clone(), saved.generation, saved.offset))
            }
        };
        state.cursors.retain(|_, c| now_ms < c.expires_at);
        let view_id = state
            .views
            .iter()
            .find(|(_, view)| view.binding == binding)
            .map(|(id, _)| id.clone());
        let view_id = match view_id {
            Some(id) => id,
            None if continuation.is_some() => return Err(WorkError::cursor_expired()),
            None => {
                while state.views.len() >= settings.max_work_views {
                    let oldest = state
                        .views
                        .iter()
                        .min_by_key(|(id, view)| (view.accessed_at, *id))
                        .map(|(id, _)| id.clone());
                    let Some(oldest) = oldest else { break };
                    state.views.remove(&oldest);
                    state.cursors.retain(|_, cursor| cursor.view_id != oldest);
                }
                let id = Uuid::new_v4().to_string();
                state.views.insert(
                    id.clone(),
                    View {
                        binding: binding.clone(),
                        fingerprint: fingerprint.clone(),
                        generation: 1,
                        accessed_at: now_ms,
                        snapshots: BTreeMap::new(),
                    },
                );
                id
            }
        };
        let view = state
            .views
            .get_mut(&view_id)
            .expect("view inserted under same lock");
        if view.fingerprint != fingerprint {
            view.fingerprint = fingerprint;
            view.generation = view.generation.saturating_add(1);
        }
        view.accessed_at = now_ms;
        let seq = view.generation;
        let offset = match continuation {
            Some((id, generation, offset)) if id == view_id && generation == seq => offset,
            Some(_) => return Err(WorkError::cursor_expired()),
            None => 0,
        };
        if offset > total {
            return Err(WorkError::cursor_expired());
        }
        let end = offset.saturating_add(limit).min(total);
        let next_cursor = if end < total {
            // Repeated transport retries return the same continuation rather
            // than allocating a new cursor for the same page on every retry.
            let existing = state
                .cursors
                .iter()
                .find(|(_, c)| c.view_id == view_id && c.generation == seq && c.offset == end)
                .map(|(token, _)| token.clone());
            Some(match existing {
                Some(token) => token,
                None => {
                    while state.cursors.len() >= settings.max_work_cursors {
                        let oldest = state
                            .cursors
                            .iter()
                            .min_by_key(|(token, c)| (c.expires_at, *token))
                            .map(|(token, _)| token.clone());
                        let Some(oldest) = oldest else { break };
                        state.cursors.remove(&oldest);
                    }
                    let token = format!("w1.{}", Uuid::new_v4());
                    state.cursors.insert(
                        token.clone(),
                        Cursor {
                            binding,
                            view_id: view_id.clone(),
                            generation: seq,
                            offset: end,
                            expires_at: now_ms
                                .saturating_add(settings.work_cursor_ttl_secs.saturating_mul(1000)),
                        },
                    );
                    token
                }
            })
        } else {
            None
        };
        Ok(PageWindow {
            snapshot_id: view_id,
            seq,
            offset,
            end,
            next_cursor,
        })
    }
}

impl WorkViews {
    pub(super) fn frozen_page(
        &self,
        binding: ViewBinding,
        current: WorkSnapshot,
        limit: usize,
        cursor: Option<&str>,
        settings: &CoordinationSettings,
        now_ms: u64,
        revalidate: impl FnOnce(&WorkSnapshot) -> Result<(), WorkError>,
    ) -> Result<FrozenPage, WorkError> {
        self.frozen_page_select(
            binding,
            current,
            limit,
            cursor,
            settings,
            now_ms,
            revalidate,
            |_, _, end| Ok(end),
        )
    }

    pub(super) fn frozen_page_select(
        &self,
        binding: ViewBinding,
        current: WorkSnapshot,
        limit: usize,
        cursor: Option<&str>,
        settings: &CoordinationSettings,
        now_ms: u64,
        revalidate: impl FnOnce(&WorkSnapshot) -> Result<(), WorkError>,
        select_end: impl FnOnce(&WorkSnapshot, usize, usize) -> Result<usize, WorkError>,
    ) -> Result<FrozenPage, WorkError> {
        settings
            .validate()
            .map_err(|_| WorkError::projection_failure(WorkFailureCause::SettingsUnavailable))?;
        if limit == 0 || limit > settings.overview_max_rows {
            return Err(WorkError::invalid(
                "limit is outside the configured page range",
            ));
        }
        let started = std::time::Instant::now();
        let (fingerprint, bytes) = current.signature(&binding)?;
        let units = current.units();
        use crate::chat::perf_diagnostics::{self, PerfComponent, PerfOutcome};
        let mut wait = perf_diagnostics::span(PerfComponent::WorkPagingLockWait, None, None);
        let mut hold;
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        wait.pause();
        hold = perf_diagnostics::span(PerfComponent::WorkPagingLockHold, None, None);
        state.prune_snapshots(settings, now_ms);
        let (view_id, seq, offset) = match cursor {
            Some(token) => {
                if !token.starts_with("w1.") || Uuid::parse_str(&token[3..]).is_err() {
                    return Err(WorkError::invalid_cursor());
                }
                let saved = state
                    .cursors
                    .get(token)
                    .ok_or_else(WorkError::cursor_expired)?;
                if saved.binding != binding {
                    return Err(WorkError::invalid_cursor());
                }
                if now_ms >= saved.expires_at {
                    return Err(WorkError::cursor_expired());
                }
                (saved.view_id.clone(), saved.generation, saved.offset)
            }
            None => {
                if units > settings.max_work_history_rows {
                    return Err(WorkError::projection_failure(
                        WorkFailureCause::SnapshotRowCapacity,
                    ));
                }
                if bytes > settings.max_work_history_bytes {
                    return Err(WorkError::projection_failure(
                        WorkFailureCause::SnapshotByteCapacity,
                    ));
                }
                let existing = state
                    .views
                    .iter()
                    .find(|(_, view)| view.binding == binding && !view.snapshots.is_empty())
                    .map(|(id, view)| (id.clone(), view.generation));
                let (id, mut seq) = existing
                    .clone()
                    .unwrap_or_else(|| (Uuid::new_v4().to_string(), 0));
                let unchanged = state
                    .views
                    .get(&id)
                    .and_then(|v| v.snapshots.get(&seq))
                    .is_some_and(|s| s.fingerprint == fingerprint);
                if !unchanged {
                    seq = seq.checked_add(1).ok_or_else(WorkError::unavailable)?;
                    state.reserve_snapshot(settings, units, bytes, &id)?;
                    let view = state.views.entry(id.clone()).or_insert_with(|| View {
                        binding: binding.clone(),
                        fingerprint: String::new(),
                        generation: seq,
                        accessed_at: now_ms,
                        snapshots: BTreeMap::new(),
                    });
                    view.generation = seq;
                    view.snapshots.insert(
                        seq,
                        FrozenView {
                            snapshot: Arc::new(current),
                            fingerprint: fingerprint.clone(),
                            observed_at: now_ms,
                            expires_at: now_ms
                                .saturating_add(settings.work_cursor_ttl_secs.saturating_mul(1000)),
                            units,
                            bytes,
                        },
                    );
                }
                (id, seq, 0)
            }
        };
        let saved = state
            .views
            .get(&view_id)
            .and_then(|view| view.snapshots.get(&seq))
            .ok_or_else(WorkError::cursor_expired)?;
        let retained = Arc::clone(&saved.snapshot);
        let observed_at = saved.observed_at;
        let expires_at = saved.expires_at;
        let stale = saved.fingerprint != fingerprint;
        hold.pause();
        drop(state);
        wait.finish(PerfOutcome::Success, None, None, None, None);
        hold.finish(PerfOutcome::Success, None, None, None, None);
        let validate = perf_diagnostics::span(PerfComponent::WorkPagingValidate, None, None);
        let validation = revalidate(&retained);
        validate.finish(
            if validation.is_ok() {
                PerfOutcome::Success
            } else {
                PerfOutcome::Failure
            },
            None,
            None,
            None,
            None,
        );
        let total = retained.total();
        if validation.is_ok() && offset > total {
            return Err(WorkError::cursor_expired());
        }
        let requested_end = offset.saturating_add(limit).min(total);
        let select = perf_diagnostics::span(PerfComponent::WorkPagingSelect, None, None);
        let selected = if validation.is_ok() {
            Some(select_end(&retained, offset, requested_end))
        } else {
            None
        };
        let snapshot = selected
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .filter(|&&end| end <= requested_end && end >= offset)
            .map(|&end| retained.slice(offset, end));
        select.finish(
            if selected.as_ref().is_some_and(Result::is_ok) {
                PerfOutcome::Success
            } else {
                PerfOutcome::Failure
            },
            None,
            None,
            None,
            None,
        );
        let mut wait = perf_diagnostics::span(PerfComponent::WorkPagingLockWait, None, None);
        let mut hold;
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        wait.pause();
        hold = perf_diagnostics::span(PerfComponent::WorkPagingLockHold, None, None);
        let commit_now =
            now_ms.saturating_add(started.elapsed().as_millis().min(u64::MAX as u128) as u64);
        state.prune_snapshots(settings, commit_now);
        let same = state.views.get(&view_id).is_some_and(|view| {
            view.binding == binding
                && view
                    .snapshots
                    .get(&seq)
                    .is_some_and(|saved| Arc::ptr_eq(&saved.snapshot, &retained))
        });
        if let Err(error) = validation {
            if same {
                state.remove_snapshot(&view_id, seq);
            }
            return Err(error);
        }
        if !same || commit_now >= expires_at {
            return Err(WorkError::cursor_expired());
        }
        if let Some(token) = cursor {
            if !state.cursors.get(token).is_some_and(|saved| {
                saved.binding == binding
                    && saved.view_id == view_id
                    && saved.generation == seq
                    && saved.offset == offset
                    && commit_now < saved.expires_at
            }) {
                return Err(WorkError::cursor_expired());
            }
        }
        let end = selected.expect("selection follows successful validation")?;
        if end > requested_end || end < offset || (end == offset && offset < total) {
            return Err(WorkError::projection_failure(
                WorkFailureCause::ProvenanceCapacity,
            ));
        }
        let snapshot = snapshot.ok_or_else(WorkError::cursor_expired)?;
        state
            .views
            .get_mut(&view_id)
            .ok_or_else(WorkError::cursor_expired)?
            .accessed_at = commit_now;
        let next_cursor = if end < total {
            let existing = state
                .cursors
                .iter()
                .find(|(_, c)| {
                    c.view_id == view_id
                        && c.generation == seq
                        && c.offset == end
                        && commit_now < c.expires_at
                })
                .map(|(token, _)| token.clone());
            Some(match existing {
                Some(token) => token,
                None => {
                    while state.cursors.len() >= settings.max_work_cursors {
                        let oldest = state
                            .cursors
                            .iter()
                            .min_by_key(|(token, c)| (c.expires_at, *token))
                            .map(|(token, _)| token.clone())
                            .ok_or_else(WorkError::unavailable)?;
                        state.cursors.remove(&oldest);
                    }
                    let token = format!("w1.{}", Uuid::new_v4());
                    state.cursors.insert(
                        token.clone(),
                        Cursor {
                            binding,
                            view_id: view_id.clone(),
                            generation: seq,
                            offset: end,
                            expires_at,
                        },
                    );
                    token
                }
            })
        } else {
            None
        };
        hold.pause();
        drop(state);
        wait.finish(PerfOutcome::Success, None, None, None, None);
        hold.finish(PerfOutcome::Success, None, None, None, None);
        Ok(FrozenPage {
            window: PageWindow {
                snapshot_id: view_id,
                seq,
                offset,
                end,
                next_cursor,
            },
            snapshot,
            total,
            observed_at,
            stale,
        })
    }
}

impl State {
    fn remove_snapshot(&mut self, id: &str, seq: u64) {
        if let Some(view) = self.views.get_mut(id) {
            view.snapshots.remove(&seq);
            if view.snapshots.is_empty() {
                self.views.remove(id);
            }
        }
        self.cursors
            .retain(|_, c| c.view_id != id || c.generation != seq);
    }

    fn snapshot_usage(&self) -> (usize, usize, usize) {
        self.views.values().flat_map(|v| v.snapshots.values()).fold(
            (0usize, 0usize, 0usize),
            |(count, units, bytes), s| {
                (
                    count.saturating_add(1),
                    units.saturating_add(s.units),
                    bytes.saturating_add(s.bytes),
                )
            },
        )
    }

    fn evict_snapshot(&mut self) -> Result<(), WorkError> {
        let (id, seq) = self
            .views
            .iter()
            .flat_map(|(id, v)| {
                v.snapshots
                    .iter()
                    .map(move |(seq, s)| (s.observed_at, id.clone(), *seq))
            })
            .min()
            .map(|(_, id, seq)| (id, seq))
            .ok_or_else(WorkError::unavailable)?;
        self.remove_snapshot(&id, seq);
        Ok(())
    }

    fn prune_snapshots(&mut self, settings: &CoordinationSettings, now_ms: u64) {
        let expired: Vec<_> = self
            .views
            .iter()
            .flat_map(|(id, v)| {
                v.snapshots
                    .iter()
                    .filter(move |(_, s)| {
                        now_ms >= s.expires_at
                            || now_ms.saturating_sub(s.observed_at)
                                >= settings.work_cursor_ttl_secs.saturating_mul(1000)
                    })
                    .map(move |(seq, _)| (id.clone(), *seq))
            })
            .collect();
        for (id, seq) in expired {
            self.remove_snapshot(&id, seq);
        }
        self.cursors.retain(|_, c| now_ms < c.expires_at);
        while {
            let (count, units, bytes) = self.snapshot_usage();
            count > settings.max_work_views
                || units > settings.max_work_history_rows
                || bytes > settings.max_work_history_bytes
        } {
            if self.evict_snapshot().is_err() {
                break;
            }
        }
    }

    fn reserve_snapshot(
        &mut self,
        settings: &CoordinationSettings,
        units: usize,
        bytes: usize,
        view_id: &str,
    ) -> Result<(), WorkError> {
        if units > settings.max_work_history_rows {
            return Err(WorkError::projection_failure(
                WorkFailureCause::SnapshotRowCapacity,
            ));
        }
        if bytes > settings.max_work_history_bytes {
            return Err(WorkError::projection_failure(
                WorkFailureCause::SnapshotByteCapacity,
            ));
        }
        while {
            let (count, used_units, used_bytes) = self.snapshot_usage();
            count >= settings.max_work_views
                || used_units.saturating_add(units) > settings.max_work_history_rows
                || used_bytes.saturating_add(bytes) > settings.max_work_history_bytes
        } {
            self.evict_snapshot()?;
        }
        while !self.views.contains_key(view_id) && self.views.len() >= settings.max_work_views {
            let id = self
                .views
                .iter()
                .min_by_key(|(id, v)| (v.accessed_at, *id))
                .map(|(id, _)| id.clone())
                .ok_or_else(WorkError::unavailable)?;
            self.views.remove(&id);
            self.cursors.retain(|_, c| c.view_id != id);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding() -> ViewBinding {
        ViewBinding {
            project: "p".into(),
            principal: "chat:c".into(),
            disclosure: "policy:model".into(),
            filter: "all".into(),
        }
    }

    fn snapshot(label: &str) -> WorkSnapshot {
        WorkSnapshot {
            roots: (0..3)
                .map(|id| super::super::types::WorkRootRef {
                    id: format!("{label}-{id}"),
                    kind: super::super::types::WorkRootKind::Project,
                    path: None,
                    branch: None,
                    project: "p".into(),
                    source_path: None,
                    presence: Default::default(),
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn frozen_page_revalidation_does_not_hold_global_state_lock() {
        use std::sync::mpsc;
        use std::time::Duration;

        let views = Arc::new(WorkViews::default());
        let settings = CoordinationSettings::default();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (b_tx, b_rx) = mpsc::channel();
        let a_views = Arc::clone(&views);
        let a_settings = settings.clone();
        let a = std::thread::spawn(move || {
            a_views.frozen_page_select(
                binding(),
                snapshot("a"),
                1,
                None,
                &a_settings,
                10,
                |_| {
                    entered_tx.send(()).unwrap();
                    let _ = release_rx.recv_timeout(Duration::from_secs(5));
                    Ok(())
                },
                |_, _, end| Ok(end),
            )
        });
        let entered = entered_rx.recv_timeout(Duration::from_secs(2)).is_ok();
        if !entered {
            let _ = release_tx.send(());
            let _ = a.join();
            panic!("revalidation callback did not start");
        }
        let b_views = Arc::clone(&views);
        let b_settings = settings.clone();
        let b = std::thread::spawn(move || {
            let mut other = binding();
            other.principal = "other".into();
            let result = b_views
                .frozen_page_select(
                    other,
                    snapshot("b"),
                    1,
                    None,
                    &b_settings,
                    11,
                    |_| Ok(()),
                    |_, _, end| Ok(end),
                )
                .map(|page| page.window.snapshot_id);
            let _ = b_tx.send(result);
        });
        let result_before_release = b_rx.recv_timeout(Duration::from_secs(2));
        let _ = release_tx.send(());
        assert!(a.join().unwrap().is_ok());
        b.join().unwrap();
        assert!(
            result_before_release.is_ok_and(|result| result.is_ok()),
            "independent frozen page did not complete successfully before release"
        );
    }

    #[test]
    fn revalidation_commit_after_eviction_cannot_remove_replacement_view() {
        use std::sync::mpsc;
        use std::time::Duration;

        let views = Arc::new(WorkViews::default());
        let settings = CoordinationSettings {
            max_work_views: 1,
            ..Default::default()
        };
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let a_views = Arc::clone(&views);
        let a_settings = settings.clone();
        let a = std::thread::spawn(move || {
            a_views.frozen_page_select(
                binding(),
                snapshot("a"),
                1,
                None,
                &a_settings,
                10,
                |_| {
                    entered_tx.send(()).unwrap();
                    let _ = release_rx.recv_timeout(Duration::from_secs(5));
                    Ok(())
                },
                |_, _, end| Ok(end),
            )
        });
        let entered = entered_rx.recv_timeout(Duration::from_secs(2)).is_ok();
        if !entered {
            let _ = release_tx.send(());
            let _ = a.join();
            panic!("revalidation callback did not start");
        }
        let mut other = binding();
        other.principal = "other".into();
        let replacement = views
            .frozen_page_select(
                other.clone(),
                snapshot("b"),
                1,
                None,
                &settings,
                11,
                |_| Ok(()),
                |_, _, end| Ok(end),
            )
            .unwrap();
        let _ = release_tx.send(());
        let error = a.join().unwrap().err().unwrap();
        assert_eq!(error.code, "cursor_expired");
        let retained = views
            .frozen_page_select(
                other,
                snapshot("b"),
                1,
                None,
                &settings,
                12,
                |_| Ok(()),
                |_, _, end| Ok(end),
            )
            .unwrap();
        assert_eq!(retained.window.snapshot_id, replacement.window.snapshot_id);
        assert_eq!(retained.window.seq, replacement.window.seq);
        assert_eq!(retained.snapshot.roots[0].id, "b-0");
    }

    #[test]
    fn frozen_inventory_retries_and_capsule_beginning_survive_new_generations() {
        let views = WorkViews::default();
        let settings = CoordinationSettings::default();
        let first = views
            .frozen_page(binding(), snapshot("a"), 1, None, &settings, 10, |_| Ok(()))
            .unwrap();
        let second = views
            .frozen_page(
                binding(),
                snapshot("changed"),
                1,
                first.window.next_cursor.as_deref(),
                &settings,
                11,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(second.snapshot.roots[0].id, "a-1");
        assert!(second.stale);
        let fresh = views
            .frozen_page(
                binding(),
                snapshot("changed"),
                1,
                None,
                &settings,
                12,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(fresh.window.snapshot_id, first.window.snapshot_id);
        assert_eq!(fresh.window.seq, first.window.seq + 1);
        let retry = views
            .frozen_page(
                binding(),
                snapshot("changed"),
                1,
                first.window.next_cursor.as_deref(),
                &settings,
                13,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(retry.snapshot.roots, second.snapshot.roots);
        assert_eq!(retry.window.next_cursor, second.window.next_cursor);
        let token = views
            .beginning_cursor(&first.window.snapshot_id, first.window.seq, 14)
            .unwrap();
        assert_eq!(
            views
                .beginning_cursor(&first.window.snapshot_id, first.window.seq, 15)
                .unwrap(),
            token
        );
        let beginning = views
            .frozen_page(
                binding(),
                snapshot("changed"),
                1,
                Some(&token),
                &settings,
                16,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(beginning.snapshot.roots[0].id, "a-0");
        assert_eq!(beginning.observed_at, 10);
        for which in 0..4 {
            let mut other = binding();
            match which {
                0 => other.project = "other".into(),
                1 => other.principal = "other".into(),
                2 => other.disclosure = "other".into(),
                _ => other.filter = "other".into(),
            }
            assert_eq!(
                views
                    .frozen_page(
                        other,
                        snapshot("a"),
                        1,
                        Some(&token),
                        &settings,
                        17,
                        |_| Ok(())
                    )
                    .err()
                    .unwrap()
                    .code,
                "invalid_cursor"
            );
        }
        assert_eq!(
            views
                .frozen_page(
                    binding(),
                    snapshot("a"),
                    1,
                    Some(&token),
                    &settings,
                    18,
                    |_| Err(WorkError::cursor_expired())
                )
                .err()
                .unwrap()
                .code,
            "cursor_expired"
        );
        assert_eq!(
            views
                .frozen_page(
                    binding(),
                    snapshot("a"),
                    1,
                    Some(&token),
                    &settings,
                    19,
                    |_| Ok(())
                )
                .err()
                .unwrap()
                .code,
            "cursor_expired"
        );
    }

    #[test]
    fn frozen_snapshots_have_absolute_ttl_and_aggregate_row_byte_and_view_bounds() {
        let settings = CoordinationSettings {
            work_cursor_ttl_secs: 1,
            ..Default::default()
        };
        let views = WorkViews::default();
        let first = views
            .frozen_page(binding(), snapshot("a"), 1, None, &settings, 0, |_| Ok(()))
            .unwrap();
        let next = views
            .frozen_page(
                binding(),
                snapshot("a"),
                1,
                first.window.next_cursor.as_deref(),
                &settings,
                900,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(
            views
                .frozen_page(
                    binding(),
                    snapshot("a"),
                    1,
                    next.window.next_cursor.as_deref(),
                    &settings,
                    1000,
                    |_| Ok(())
                )
                .err()
                .unwrap()
                .code,
            "cursor_expired"
        );
        assert_eq!(views.0.lock().unwrap().snapshot_usage(), (0, 0, 0));
        let bytes = snapshot("a").signature(&binding()).unwrap().1;
        for settings in [
            CoordinationSettings {
                max_work_history_rows: 3,
                ..Default::default()
            },
            CoordinationSettings {
                max_work_history_bytes: bytes,
                ..Default::default()
            },
            CoordinationSettings {
                max_work_views: 1,
                ..Default::default()
            },
        ] {
            let views = WorkViews::default();
            let first = views
                .frozen_page(binding(), snapshot("a"), 1, None, &settings, 0, |_| Ok(()))
                .unwrap();
            let next = views
                .frozen_page(binding(), snapshot("b"), 1, None, &settings, 1, |_| Ok(()))
                .unwrap();
            assert_eq!(next.window.seq, first.window.seq + 1);
            assert_eq!(
                views
                    .frozen_page(
                        binding(),
                        snapshot("b"),
                        1,
                        first.window.next_cursor.as_deref(),
                        &settings,
                        2,
                        |_| Ok(())
                    )
                    .err()
                    .unwrap()
                    .code,
                "cursor_expired"
            );
            let state = views.0.lock().unwrap();
            let (count, rows, bytes) = state.snapshot_usage();
            assert!(count <= settings.max_work_views);
            assert!(rows <= settings.max_work_history_rows);
            assert!(bytes <= settings.max_work_history_bytes);
        }
        for settings in [
            CoordinationSettings {
                max_work_history_rows: 2,
                ..Default::default()
            },
            CoordinationSettings {
                max_work_history_bytes: bytes - 1,
                ..Default::default()
            },
        ] {
            let views = WorkViews::default();
            let whole = snapshot("a");
            let selected = whole.slice(0, 1);
            assert!(selected.units() <= settings.max_work_history_rows);
            assert!(selected.signature(&binding()).unwrap().1 <= settings.max_work_history_bytes);
            let error = views
                .frozen_page(binding(), whole, 1, None, &settings, 0, |_| Ok(()))
                .err()
                .unwrap();
            assert_eq!(error.code, "unavailable");
            assert_eq!(
                error.cause,
                Some(if settings.max_work_history_rows == 2 {
                    WorkFailureCause::SnapshotRowCapacity
                } else {
                    WorkFailureCause::SnapshotByteCapacity
                })
            );
            assert!(error
                .message
                .contains("Reducing limit only changes page size"));
            assert!(uuid::Uuid::parse_str(&error.request_id).is_ok());
            assert!(views.0.lock().unwrap().views.is_empty());
            let control = views
                .frozen_page(binding(), selected, 1, None, &settings, 1, |_| Ok(()))
                .unwrap();
            assert_eq!(control.total, 1);
            assert_eq!(control.snapshot.roots[0].id, "a-0");
        }
    }

    #[test]
    fn paging_retries_are_stable_and_changes_cannot_splice_pages() {
        let views = WorkViews::default();
        let settings = CoordinationSettings::default();
        let first = views
            .page(binding(), "a".into(), 7, 3, None, &settings, 1)
            .unwrap();
        let retry = views
            .page(binding(), "a".into(), 7, 3, None, &settings, 2)
            .unwrap();
        assert_eq!(first.next_cursor, retry.next_cursor);
        let next = views
            .page(
                binding(),
                "a".into(),
                7,
                3,
                first.next_cursor.as_deref(),
                &settings,
                3,
            )
            .unwrap();
        assert_eq!((next.offset, next.end, next.seq), (3, 6, 1));
        let last = views
            .page(
                binding(),
                "a".into(),
                7,
                3,
                next.next_cursor.as_deref(),
                &settings,
                4,
            )
            .unwrap();
        assert_eq!((last.offset, last.end), (6, 7));
        assert!(last.next_cursor.is_none());
        let error = views
            .page(
                binding(),
                "changed".into(),
                7,
                3,
                first.next_cursor.as_deref(),
                &settings,
                5,
            )
            .err()
            .unwrap();
        assert_eq!(error.code, "cursor_expired");
    }

    #[test]
    fn cursor_binds_project_principal_policy_model_and_filter() {
        let views = WorkViews::default();
        let settings = CoordinationSettings::default();
        let first = views
            .page(binding(), "a".into(), 7, 3, None, &settings, 1)
            .unwrap();
        for which in 0..4 {
            let mut foreign = binding();
            match which {
                0 => foreign.project = "other".into(),
                1 => foreign.principal = "other".into(),
                2 => foreign.disclosure = "other".into(),
                _ => foreign.filter = "other".into(),
            }
            let error = views
                .page(
                    foreign,
                    "a".into(),
                    7,
                    3,
                    first.next_cursor.as_deref(),
                    &settings,
                    2,
                )
                .err()
                .unwrap();
            assert_eq!(error.code, "invalid_cursor");
            assert!(!error.to_string().contains("chat:c"));
        }
    }

    #[test]
    fn expiry_eviction_and_invalid_limits_are_explicit() {
        let views = WorkViews::default();
        let settings = CoordinationSettings {
            max_work_views: 1,
            max_work_cursors: 1,
            work_cursor_ttl_secs: 1,
            ..Default::default()
        };
        let first = views
            .page(binding(), "a".into(), 7, 3, None, &settings, 0)
            .unwrap();
        let error = views
            .page(
                binding(),
                "a".into(),
                7,
                3,
                first.next_cursor.as_deref(),
                &settings,
                1000,
            )
            .err()
            .unwrap();
        assert_eq!(error.code, "cursor_expired");
        let mut other = binding();
        other.principal = "other".into();
        views
            .page(other, "a".into(), 7, 3, None, &settings, 1001)
            .unwrap();
        assert_eq!(views.0.lock().unwrap().views.len(), 1);
        assert_eq!(views.0.lock().unwrap().cursors.len(), 1);
        for limit in [0, settings.overview_max_rows + 1] {
            assert_eq!(
                views
                    .page(binding(), "a".into(), 7, limit, None, &settings, 1002)
                    .err()
                    .unwrap()
                    .code,
                "invalid_argument"
            );
        }
        assert_eq!(
            views
                .page(binding(), "a".into(), 7, 3, Some("o3"), &settings, 1002)
                .err()
                .unwrap()
                .code,
            "invalid_cursor"
        );
    }
}
