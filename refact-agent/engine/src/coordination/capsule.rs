//! Significant-change context capsule (M1-08 / M1-09).
//!
//! Builds one `ChatMessage` summarising what changed elsewhere in the project
//! since the last capsule was shown to this chat, and only when something
//! *significant* changed: coarse work-state transitions, new attention rows,
//! or new/gone work. Tool chatter, streaming deltas, step counters and
//! heartbeats never move the fingerprint, so a capsule is never regenerated
//! just because something is still running.
//!
//! Cadence is controlled by `CoordinationSettings::min_interval_secs` (the
//! floor between two capsules for the same chat) and `debounce_secs` (how
//! long a just-changed fingerprint must stay stable before it is shown, so a
//! burst of transitions collapses into one capsule). There is no
//! force-generation path: callers ask for a capsule and get `None` when nothing
//! significant is ready to show.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Weak};

use refact_work_core::settings::CoordinationSettings;

use crate::app_state::AppState;
use crate::call_validation::ChatMessage;
use super::types::WorkRow;

pub const CAPSULE_MARKER_KEY: &str = "work_context_capsule";

#[cfg(test)]
tokio::task_local! {
    pub(crate) static ROW_DERIVATIONS: std::cell::Cell<usize>;
}

#[derive(Debug, Clone)]
pub(crate) struct ProjectionIdentity(Weak<super::catalog::WorkCatalog>);

impl PartialEq for ProjectionIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
    }
}

impl Eq for ProjectionIdentity {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CapsuleObservation {
    projection: ProjectionIdentity,
    chat_id: String,
    source_root: std::path::PathBuf,
    checkout_root: std::path::PathBuf,
    root_chat_id: Option<String>,
    principal: String,
    // Only capsule settings: unrelated scheduler maps need not be compared.
    settings: (u64, u64, u64, usize, usize),
    fingerprint: u64,
    observed_at_ms: u64,
    expires_at_ms: u64,
}

fn capsule_settings(settings: &CoordinationSettings) -> (u64, u64, u64, usize, usize) {
    (
        settings.recent_window_secs,
        settings.min_interval_secs,
        settings.debounce_secs,
        settings.capsule_token_budget,
        settings.capsule_max_rows,
    )
}

fn terminal_expiry(rows: &[WorkRow], recent_window_secs: u64) -> u64 {
    rows.iter()
        .filter(|row| row.state.is_terminal())
        .map(|row| {
            row.updated_at_ms
                .saturating_add(recent_window_secs.saturating_mul(1000))
                .saturating_add(1)
        })
        .min()
        .unwrap_or(u64::MAX)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The part of a row that counts toward the significant-change fingerprint.
/// Everything else (queue_size, step_count, current_tool, timestamps) is
/// deliberately excluded so tool chatter and streaming never trip it.
fn fingerprint_row(row: &WorkRow, hasher: &mut impl Hasher) {
    row.key.to_string().hash(hasher);
    row.kind.as_str().hash(hasher);
    row.state.as_str().hash(hasher);
    row.attention.hash(hasher);
    row.root.id.hash(hasher);
    row.root.path.hash(hasher);
    row.root.branch.hash(hasher);
    row.declared_files.hash(hasher);
    row.observed_files.hash(hasher);
    row.open_questions.hash(hasher);
}

/// A deterministic fingerprint of the catalog's significant shape: which work
/// items exist, their coarse state, attention flag and root. Two snapshots
/// with the same fingerprint are considered "nothing significant changed"
/// even if timestamps, queue depths or tool names differ.
pub fn significant_fingerprint(rows: &[WorkRow]) -> u64 {
    let mut keys: Vec<&WorkRow> = rows.iter().collect();
    keys.sort_by(|a, b| a.key.to_string().cmp(&b.key.to_string()));
    let mut hasher = DefaultHasher::new();
    keys.len().hash(&mut hasher);
    for row in keys {
        fingerprint_row(row, &mut hasher);
    }
    hasher.finish()
}

/// Per-chat bookkeeping needed to decide whether a new capsule is due.
/// Callers own storage of this (e.g. one slot per `ChatSession`); the capsule
/// builder is pure with respect to it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapsuleState {
    pub model_id: String,
    pub policy_key: String,
    pub last_fingerprint: Option<u64>,
    pub last_shown_at_ms: Option<u64>,
    pub pending_fingerprint: Option<u64>,
    pub pending_since_ms: Option<u64>,
    pub(crate) observation: Option<CapsuleObservation>,
}

impl CapsuleState {
    fn unchanged_projection(
        &self,
        catalog: &Arc<super::catalog::WorkCatalog>,
        caller: &super::scope::WorkCaller,
        chat_id: &str,
        settings: &CoordinationSettings,
        now: u64,
    ) -> bool {
        let Some(memo) = &self.observation else {
            return false;
        };
        self.last_fingerprint == Some(memo.fingerprint)
            && self.pending_fingerprint.is_none()
            && self.pending_since_ms.is_none()
            && memo
                .projection
                .0
                .upgrade()
                .is_some_and(|prior| Arc::ptr_eq(&prior, catalog))
            && memo.chat_id == chat_id
            && memo.source_root == caller.source_root
            && memo.checkout_root == caller.checkout_root
            && memo.root_chat_id == caller.root_chat_id
            && memo.principal == caller.principal
            && memo.settings == capsule_settings(settings)
            && now >= memo.observed_at_ms
            && now < memo.expires_at_ms
    }

    pub fn reconcile(&mut self, messages: &[ChatMessage]) {
        let latest = messages
            .iter()
            .rev()
            .find_map(|message| message.extra.get(CAPSULE_MARKER_KEY));
        let restored = latest.and_then(|marker| {
            Some(Self {
                model_id: marker.get("model_id")?.as_str()?.into(),
                policy_key: marker.get("policy_key")?.as_str()?.into(),
                last_fingerprint: Some(marker.get("fingerprint")?.as_str()?.parse().ok()?),
                last_shown_at_ms: Some(marker.get("generated_at_ms")?.as_u64()?),
                ..Default::default()
            })
        });
        if let Some(restored) = restored {
            if self.last_fingerprint != restored.last_fingerprint
                || self.last_shown_at_ms != restored.last_shown_at_ms
            {
                *self = restored;
            }
        } else {
            *self = Self::default();
        }
    }
}

/// Outcome of asking whether a capsule should be (re)built now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapsuleDecision {
    /// No capsule ever shown for this chat: emit an initial capsule right away.
    Initial,
    /// The fingerprint changed and has been stable for at least
    /// `debounce_secs`, and at least `min_interval_secs` passed since the
    /// last capsule: emit a fresh one.
    Changed,
    /// Nothing to show right now (unchanged, still debouncing, or throttled).
    Skip,
}

/// Decide whether a capsule is due, updating `state` in place.
///
/// This is the whole cadence policy in one place: unchanged fingerprints,
/// active debounce windows and the minimum interval floor are all evaluated
/// here so `prepare_work_context` stays a thin wrapper.
pub fn decide(
    state: &mut CapsuleState,
    fingerprint: u64,
    settings: &CoordinationSettings,
    now_ms: u64,
) -> CapsuleDecision {
    if state.last_fingerprint.is_none() {
        state.last_fingerprint = Some(fingerprint);
        state.last_shown_at_ms = Some(now_ms);
        state.pending_fingerprint = None;
        state.pending_since_ms = None;
        return CapsuleDecision::Initial;
    }

    if state.last_fingerprint == Some(fingerprint) {
        // Fingerprint reverted to the last shown value before the debounce
        // window elapsed; the in-flight change is no longer pending.
        state.pending_fingerprint = None;
        state.pending_since_ms = None;
        return CapsuleDecision::Skip;
    }

    if state.pending_fingerprint != Some(fingerprint) {
        state.pending_fingerprint = Some(fingerprint);
        if state.pending_since_ms.is_none() {
            state.pending_since_ms = Some(now_ms);
        }
    }

    let debounce_ms = settings.debounce_secs.saturating_mul(1000);
    let since = state.pending_since_ms.unwrap_or(now_ms);
    if now_ms.saturating_sub(since) < debounce_ms {
        return CapsuleDecision::Skip;
    }

    let min_interval_ms = settings.min_interval_secs.saturating_mul(1000);
    let last_shown = state.last_shown_at_ms.unwrap_or(0);
    if now_ms.saturating_sub(last_shown) < min_interval_ms {
        return CapsuleDecision::Skip;
    }

    state.last_fingerprint = Some(fingerprint);
    state.last_shown_at_ms = Some(now_ms);
    state.pending_fingerprint = None;
    state.pending_since_ms = None;
    CapsuleDecision::Changed
}

pub async fn prepare_work_context(
    app: &AppState,
    state: &mut CapsuleState,
    chat_id: &str,
    model_id: &str,
) -> Option<ChatMessage> {
    let settings = crate::file_activity::settings();
    if !settings.enabled || !settings.auto_inject_enabled {
        return None;
    }

    use crate::chat::perf_diagnostics::{self, PerfComponent, PerfOutcome};
    let caller_span = perf_diagnostics::span(PerfComponent::WorkContextCaller, None, None);
    let caller = match super::scope::model_caller(app, chat_id, model_id).await {
        Ok(caller) => caller,
        Err(_) => {
            caller_span.finish(PerfOutcome::Failure, None, None, None, None);
            return None;
        }
    };
    caller_span.finish(PerfOutcome::Success, None, None, None, None);
    let projection_span = perf_diagnostics::span(PerfComponent::WorkContextProjection, None, None);
    let privacy = match crate::privacy::classification_cache::privacy_key(&app.gcx) {
        Ok(privacy) => privacy,
        Err(_) => {
            projection_span.finish(PerfOutcome::Failure, None, None, None, None);
            return None;
        }
    };
    let (catalog, disclosure) = match super::service::context_projection(app, &caller).await {
        Ok(projection) => projection,
        Err(_) => {
            projection_span.finish(PerfOutcome::Failure, None, None, None, None);
            return None;
        }
    };
    projection_span.finish(PerfOutcome::Success, None, None, None, None);
    let observed_at = now_ms();
    let policy_key = format!(
        "{}:{}:{}",
        catalog.project,
        caller.checkout_root.display(),
        disclosure
    );
    if state.model_id != model_id || state.policy_key != policy_key {
        *state = CapsuleState {
            model_id: model_id.to_string(),
            policy_key,
            ..Default::default()
        };
    }
    // The current caller and privacy-cleared projection are always resolved above.
    // A weak receipt cannot keep stale catalogs alive or suffer pointer-reuse ABA.
    if state.unchanged_projection(&catalog, &caller, chat_id, &settings, observed_at) {
        return None;
    }
    #[cfg(test)]
    let _ = ROW_DERIVATIONS.try_with(|count| count.set(count.get() + 1));
    let own = catalog
        .all_rows()
        .into_iter()
        .find(|row| row.chat_id.as_deref() == Some(chat_id));
    let mut lineage = std::collections::HashSet::from([chat_id.to_string()]);
    let mut ancestor = chat_id.to_string();
    let lineage_span = perf_diagnostics::span(PerfComponent::WorkContextLineage, None, None);
    for _ in 0..128 {
        let Some(Some(parent)) =
            crate::chat::trajectories::find_trajectory_parent_id(app.gcx.clone(), &ancestor).await
        else {
            break;
        };
        if !lineage.insert(parent.clone()) {
            break;
        }
        ancestor = parent;
    }
    lineage_span.finish(PerfOutcome::Success, None, None, None, None);
    let recent = observed_at.saturating_sub(settings.recent_window_secs.saturating_mul(1000));
    let visible_rows = catalog
        .sorted_rows()
        .into_iter()
        .filter(|row| row.chat_id.as_ref().is_none_or(|id| !lineage.contains(id)))
        .filter(|row| !row.state.is_terminal() || row.updated_at_ms >= recent)
        .collect::<Vec<_>>();
    let mut hasher = DefaultHasher::new();
    significant_fingerprint(&visible_rows).hash(&mut hasher);
    let overlap_span = perf_diagnostics::span(PerfComponent::WorkContextOverlap, None, None);
    let overlapping = super::capsule_render::overlapping_work(&visible_rows, own.as_ref());
    let mut overlap_keys: Vec<_> = overlapping.iter().collect();
    overlap_keys.sort();
    overlap_keys.hash(&mut hasher);
    overlap_span.finish(PerfOutcome::Success, None, None, None, None);
    let fingerprint = hasher.finish();
    let mut candidate = state.clone();
    candidate.observation = Some(CapsuleObservation {
        projection: ProjectionIdentity(Arc::downgrade(&catalog)),
        chat_id: chat_id.to_string(),
        source_root: caller.source_root.clone(),
        checkout_root: caller.checkout_root.clone(),
        root_chat_id: caller.root_chat_id.clone(),
        principal: caller.principal.clone(),
        settings: capsule_settings(&settings),
        fingerprint,
        observed_at_ms: observed_at,
        expires_at_ms: terminal_expiry(&visible_rows, settings.recent_window_secs),
    });
    if decide(&mut candidate, fingerprint, &settings, observed_at) == CapsuleDecision::Skip {
        *state = candidate;
        return None;
    }
    let mut render_settings = settings.clone();
    render_settings.capsule_token_budget = render_settings.capsule_token_budget.saturating_sub(128);
    let tokenizer_key = crate::caps::strip_model_from_finetune(model_id);
    let tokenizer = app
        .gcx
        .tokenizer_state
        .read()
        .ok()?
        .map
        .get(&tokenizer_key)
        .cloned()
        .flatten();
    let mut inventory_cursor = None;
    let count =
        |text: &str| crate::tokens::count_text_tokens_with_fallback(tokenizer.clone(), text);
    let message = loop {
        let render_span = perf_diagnostics::span(PerfComponent::WorkContextRender, None, None);
        let rendered = super::capsule_render::render(
            &catalog.project,
            &visible_rows,
            &overlapping,
            &caller,
            &render_settings,
            observed_at,
            &count,
        );
        let Some(mut rendered) = rendered else {
            render_span.finish(PerfOutcome::Skipped, None, None, None, None);
            return None;
        };
        render_span.finish(PerfOutcome::Success, None, None, None, None);
        if rendered.omitted > 0 {
            if inventory_cursor.is_none() {
                let inventory_span =
                    perf_diagnostics::span(PerfComponent::WorkContextInventory, None, None);
                inventory_cursor = match super::service::inventory_cursor_from_projection(
                    app,
                    &caller,
                    Arc::clone(&catalog),
                    &disclosure,
                    &privacy,
                )
                .await
                {
                    Ok(cursor) => Some(cursor),
                    Err(_) => {
                        inventory_span.finish(PerfOutcome::Failure, None, None, None, None);
                        return None;
                    }
                };
                inventory_span.finish(PerfOutcome::Success, None, None, None, None);
            }
            let cursor = inventory_cursor.as_ref()?;
            rendered.body.push_str(&format!(
                "\nFull inventory from the beginning: work_overview(cursor={cursor:?})."
            ));
        }
        if super::privacy_guard::policy_key(&app.gcx, caller.model.as_ref()?).ok()? != disclosure {
            return None;
        }
        let mut message = crate::chat::internal_roles::event(
            crate::chat::internal_roles::EventSubkind::SystemNotice,
            "coordination.work",
            serde_json::json!({"scope":"project", "coverage":"partial"}),
            rendered.body,
        );
        crate::privacy::records::merge_records(&mut message, rendered.records);
        message.extra.insert(
            CAPSULE_MARKER_KEY.to_string(),
            serde_json::json!({
                "fingerprint": fingerprint.to_string(),
                "generated_at_ms": observed_at,
                "model_id": candidate.model_id,
                "policy_key": candidate.policy_key,
                "disclosure": disclosure,
                "resolved_model": caller.model.as_ref()?.id,
            }),
        );
        let wire = refact_llm::adapters::render_extra::render_event_message(&message);
        let cost = count(&wire);
        if cost <= settings.capsule_token_budget {
            break message;
        }
        if render_settings.capsule_max_rows == 0 {
            return None;
        }
        render_settings.capsule_max_rows /= 2;
    };
    *state = candidate;
    Some(message)
}

pub(crate) fn still_cleared(app: &AppState, message: &ChatMessage) -> bool {
    let Some(marker) = message.extra.get(CAPSULE_MARKER_KEY) else {
        return false;
    };
    let Some(model) = marker
        .get("resolved_model")
        .and_then(serde_json::Value::as_str)
    else {
        return false;
    };
    let record = crate::caps::BaseModelRecord {
        id: model.into(),
        ..Default::default()
    };
    super::privacy_guard::policy_key(&app.gcx, &record)
        .ok()
        .as_deref()
        == marker.get("disclosure").and_then(serde_json::Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordination::roots::project_root_ref;
    use crate::coordination::types::{Disclosure, WorkKindTag, WorkStateTag};
    use refact_work_core::identity::WorkKey;
    use std::path::PathBuf;

    fn row(id: &str, state: WorkStateTag, attention: bool) -> WorkRow {
        let root = project_root_ref(&PathBuf::from("/tmp/capsule-tests"));
        WorkRow {
            key: WorkKey::background_agent(id),
            metadata: Default::default(),
            kind: WorkKindTag::Subagent,
            state,
            attention,
            title: Some(id.to_string()),
            summary: None,
            root,
            parent_chat_id: None,
            root_chat_id: None,
            chat_id: Some(format!("chat-{id}")),
            model: None,
            current_tool: Some("cat".to_string()),
            declared_files: Vec::new(),
            observed_files: Vec::new(),
            queue_size: 0,
            step_count: 0,
            open_questions: 0,
            updated_at_ms: 1,
            stale: false,
            disclosure: Disclosure::Full,
            source_records: Vec::new(),
            provenance_complete: true,
        }
    }

    fn settings_with(min_interval: u64, debounce: u64) -> CoordinationSettings {
        CoordinationSettings {
            min_interval_secs: min_interval,
            debounce_secs: debounce,
            ..CoordinationSettings::default()
        }
    }

    #[serial_test::serial(runtime_settings)]
    #[tokio::test]
    async fn observation_expires_at_terminal_boundary_and_rejects_changed_inputs() {
        let (app, _dir, _) = super::super::capsule_tests::fixture().await;
        let mut state = CapsuleState::default();
        assert!(
            prepare_work_context(&app, &mut state, "capsule-caller", "trusted/model")
                .await
                .is_some()
        );
        let caller = super::super::scope::model_caller(&app, "capsule-caller", "trusted/model")
            .await
            .unwrap();
        let (catalog, _) = super::super::service::context_projection(&app, &caller)
            .await
            .unwrap();
        let settings = crate::file_activity::settings();
        let mut terminal = row("terminal", WorkStateTag::Completed, false);
        terminal.updated_at_ms = 1_000;
        let expiry = terminal_expiry(&[terminal], 2);
        assert_eq!(
            expiry, 3_001,
            "terminal row is included at the exact window boundary"
        );
        let memo = state.observation.as_mut().unwrap();
        memo.observed_at_ms = 1_000;
        memo.expires_at_ms = expiry;
        assert!(state.unchanged_projection(&catalog, &caller, "capsule-caller", &settings, 3_000));
        assert!(!state.unchanged_projection(&catalog, &caller, "capsule-caller", &settings, 3_001));
        assert!(!state.unchanged_projection(&catalog, &caller, "capsule-caller", &settings, 999));
        assert!(!state.unchanged_projection(&catalog, &caller, "another-chat", &settings, 2_000));
        let mut changed = settings.clone();
        changed.recent_window_secs += 1;
        assert!(!state.unchanged_projection(&catalog, &caller, "capsule-caller", &changed, 2_000));
        let different_arc = Arc::new((*catalog).clone());
        assert!(!state.unchanged_projection(
            &different_arc,
            &caller,
            "capsule-caller",
            &settings,
            2_000
        ));
        let mut changed_caller = caller.clone();
        changed_caller.checkout_root.push("other");
        assert!(!state.unchanged_projection(
            &catalog,
            &changed_caller,
            "capsule-caller",
            &settings,
            2_000
        ));
        state.last_fingerprint = Some(state.last_fingerprint.unwrap().wrapping_add(1));
        assert!(!state.unchanged_projection(&catalog, &caller, "capsule-caller", &settings, 2_000));
    }

    #[test]
    fn continuous_material_changes_coalesce_without_postponing_delivery_forever() {
        let mut state = CapsuleState::default();
        let settings = settings_with(0, 5);
        assert_eq!(
            decide(&mut state, 1, &settings, 0),
            CapsuleDecision::Initial
        );
        assert_eq!(
            decide(&mut state, 2, &settings, 1_000),
            CapsuleDecision::Skip
        );
        assert_eq!(
            decide(&mut state, 3, &settings, 3_000),
            CapsuleDecision::Skip
        );
        assert_eq!(
            decide(&mut state, 4, &settings, 6_000),
            CapsuleDecision::Changed
        );
        assert_eq!(state.last_fingerprint, Some(4));
        assert_eq!(
            decide(&mut state, 4, &settings, 20_000),
            CapsuleDecision::Skip
        );
    }
}
