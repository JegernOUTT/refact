//! Opaque, bounded, visibility-bound pagination. Stores offsets, never private
//! rows or transcripts. Every continuation revalidates the current projection.

use std::collections::HashMap;
use std::sync::Mutex;

use refact_work_core::CoordinationSettings;
use uuid::Uuid;

use super::error::WorkError;

#[derive(Clone, Debug, PartialEq, Eq)]
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
pub struct WorkViews(Mutex<State>, std::sync::OnceLock<(std::time::Instant, u64)>);

pub struct PageWindow {
    pub snapshot_id: String,
    pub seq: u64,
    pub offset: usize,
    pub end: usize,
    pub next_cursor: Option<String>,
}

impl WorkViews {
    pub fn beginning_cursor(
        &self,
        snapshot_id: &str,
        seq: u64,
        now_ms: u64,
    ) -> Result<String, WorkError> {
        let settings = crate::file_activity::settings();
        settings.validate().map_err(|_| WorkError::unavailable())?;
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let binding = state
            .views
            .get(snapshot_id)
            .filter(|view| view.generation == seq)
            .ok_or_else(WorkError::cursor_expired)?
            .binding
            .clone();
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
                expires_at: now_ms
                    .saturating_add(settings.work_cursor_ttl_secs.saturating_mul(1000)),
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
        settings.validate().map_err(|_| WorkError::unavailable())?;
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
