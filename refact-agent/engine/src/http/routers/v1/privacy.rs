use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use hyper::StatusCode;
use refact_privacy::{
    Destination, DestinationId, DestinationKind, FileRecord, PrivacyAuditError, PrivacyAudited,
    PrivacyPolicy,
};
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::custom_error::ScratchError;
use crate::files_correction::registered_worktree_path_mappings;
use crate::files_in_workspace::strictest_zone_for_path;
use crate::privacy::records::DerivedPrivacyZones;

#[derive(Debug, Serialize)]
pub struct PrivacyPolicyResponse {
    pub policy: PrivacyPolicy,
    pub destinations: Vec<Destination>,
    pub match_counts: BTreeMap<String, usize>,
    pub discovery_deferred: bool,
    pub error: Option<String>,
    pub source_paths: Vec<String>,
    pub has_project_overrides: bool,
}

#[derive(Debug, Serialize)]
pub struct PrivacyObservationCapability {
    pub platform_supported: bool,
    pub runtime_available: bool,
    pub last_error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PrivacyStatusResponse {
    pub platform: String,
    pub observation: PrivacyObservationCapability,
    pub config_error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PrivacyInspectRequest {
    pub chat_id: String,
    pub destination: Destination,
    #[serde(default = "default_include_messages")]
    pub include_messages: bool,
}

fn default_include_messages() -> bool {
    true
}

#[derive(Debug, Serialize)]
pub struct PrivacyBlockedRecord {
    pub record_index: usize,
    pub record: FileRecord,
}

#[derive(Debug, Serialize)]
pub struct PrivacyInspectResponse {
    pub chat_id: String,
    pub destination: Destination,
    pub sendable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub would_send: Option<Vec<crate::call_validation::ChatMessage>>,
    pub records: Vec<FileRecord>,
    pub blocked: Vec<PrivacyBlockedRecord>,
    pub refusal: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PrivacyDerivedRequest {
    pub chat_id: String,
}

#[derive(Debug, Serialize)]
pub struct PrivacyDerivedEntry {
    pub path: String,
    pub zone: String,
    pub origin: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PrivacyDerivedResponse {
    pub chat_id: String,
    pub entries: Vec<PrivacyDerivedEntry>,
}

#[derive(Debug, Deserialize)]
pub struct PrivacyDerivedClearRequest {
    pub chat_id: String,
    #[serde(default)]
    pub paths: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct PrivacyDerivedClearResponse {
    pub cleared: usize,
    pub remaining: usize,
}

struct AuditedMessages {
    records: Result<Vec<(usize, FileRecord)>, PrivacyAuditError>,
}

impl PrivacyAudited for AuditedMessages {
    fn privacy_records(&self) -> Result<Vec<(usize, FileRecord)>, PrivacyAuditError> {
        self.records.clone()
    }
}

pub async fn handle_v1_privacy_policy_get(
    State(app): State<AppState>,
) -> Result<Json<PrivacyPolicyResponse>, ScratchError> {
    crate::privacy::load_privacy_if_needed(app.gcx.clone()).await;
    Ok(Json(build_policy_response(&app).await?))
}

pub async fn handle_v1_privacy_policy_post(
    State(app): State<AppState>,
    Json(policy): Json<PrivacyPolicy>,
) -> Result<Json<PrivacyPolicyResponse>, ScratchError> {
    if policy
        .blocked
        .iter()
        .chain(policy.zones.iter().flat_map(|zone| zone.patterns.iter()))
        .any(|pattern| {
            let normalized = pattern.replace('\\', "/");
            let literal = normalized.trim_matches(|c: char| c == '/' || c.is_whitespace());
            literal.is_empty()
                || literal == "."
                || (literal.len() == 2
                    && literal.as_bytes()[0].is_ascii_alphabetic()
                    && literal.ends_with(':'))
        })
    {
        return Err(ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Privacy patterns must not be empty, dot, or drive-root literals; use ** to mean everything".to_string(),
        ));
    }
    policy.compile().map_err(|error| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Invalid privacy policy: {error}"),
        )
    })?;
    crate::privacy::save_privacy_policy(app.gcx.clone(), policy)
        .await
        .map_err(|error| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, error))?;
    Ok(Json(build_saved_policy_response(&app).await))
}

pub async fn handle_v1_privacy_status(State(app): State<AppState>) -> Json<PrivacyStatusResponse> {
    crate::privacy::load_privacy_if_needed(app.gcx.clone()).await;
    let config_error = app.gcx.privacy_policy_load.read().unwrap().error.clone();
    let runtime = app.gcx.privacy_observation_runtime.read().unwrap().clone();
    Json(PrivacyStatusResponse {
        platform: std::env::consts::OS.to_string(),
        observation: observation_capability(runtime),
        config_error,
    })
}

pub async fn handle_v1_privacy_inspect(
    State(app): State<AppState>,
    Json(request): Json<PrivacyInspectRequest>,
) -> Result<Json<PrivacyInspectResponse>, ScratchError> {
    crate::privacy::load_privacy_if_needed(app.gcx.clone()).await;
    let include_messages = request.include_messages;
    let session = {
        let sessions = app.gcx.chat_sessions.read().await;
        sessions.get(&request.chat_id).cloned()
    }
    .ok_or_else(|| {
        ScratchError::new(StatusCode::NOT_FOUND, "Chat session not found".to_string())
    })?;
    let (indexed_records, would_send) = {
        let session = session.lock().await;
        let records = refact_privacy::records_from_messages(&session.messages);
        let messages = request.include_messages.then(|| {
            session
                .messages
                .iter()
                .cloned()
                .map(|mut message| {
                    message.extra.remove("privacy");
                    message
                })
                .collect::<Vec<_>>()
        });
        (records, messages)
    };
    let records = indexed_records
        .as_ref()
        .map(|records| records.iter().map(|(_, record)| record.clone()).collect())
        .unwrap_or_default();
    let clearance = crate::privacy::destinations::clear_current(
        &app.gcx,
        AuditedMessages {
            records: indexed_records,
        },
        &request.destination,
    );
    let (sendable, blocked, refusal) = match clearance {
        Ok(_) => (true, Vec::new(), None),
        Err(refusal) => (
            false,
            refusal
                .offending
                .into_iter()
                .map(|(record_index, record)| PrivacyBlockedRecord {
                    record_index,
                    record,
                })
                .collect(),
            Some(refusal.message),
        ),
    };

    Ok(Json(PrivacyInspectResponse {
        chat_id: request.chat_id,
        destination: request.destination,
        sendable,
        would_send: include_messages.then(|| {
            if sendable {
                would_send.unwrap_or_default()
            } else {
                Vec::new()
            }
        }),
        records,
        blocked,
        refusal,
    }))
}

pub async fn handle_v1_privacy_derived(
    State(app): State<AppState>,
    Json(request): Json<PrivacyDerivedRequest>,
) -> Result<Json<PrivacyDerivedResponse>, ScratchError> {
    let derived_zones = session_derived_zones(&app, &request.chat_id).await?;
    let mut entries = {
        let derived = derived_zones
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        derived
            .iter()
            .map(|(path, derived)| PrivacyDerivedEntry {
                path: path.to_string_lossy().into_owned(),
                zone: derived.zone.clone(),
                origin: derived.origin.clone(),
            })
            .collect::<Vec<_>>()
    };
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(Json(PrivacyDerivedResponse {
        chat_id: request.chat_id,
        entries,
    }))
}

pub async fn handle_v1_privacy_derived_clear(
    State(app): State<AppState>,
    Json(request): Json<PrivacyDerivedClearRequest>,
) -> Result<Json<PrivacyDerivedClearResponse>, ScratchError> {
    let derived_zones = session_derived_zones(&app, &request.chat_id).await?;
    let requested = request.paths.unwrap_or_default();
    let mut derived = derived_zones
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let before = derived.len();
    if requested.is_empty() {
        derived.clear();
    } else {
        for path in requested {
            derived.remove(std::path::Path::new(path.as_str()));
        }
    }
    let remaining = derived.len();
    Ok(Json(PrivacyDerivedClearResponse {
        cleared: before.saturating_sub(remaining),
        remaining,
    }))
}

#[derive(Debug, Deserialize)]
pub struct PrivacyRecordsClearRequest {
    pub chat_id: String,
    pub paths: Vec<String>,
}

#[derive(Debug, Default, Serialize, PartialEq, Eq)]
pub struct PrivacyRecordsClearResponse {
    pub cleared: usize,
    pub remaining: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persisted: Option<bool>,
}

pub async fn handle_v1_privacy_records_clear(
    State(app): State<AppState>,
    headers: axum::http::HeaderMap,
    body_bytes: hyper::body::Bytes,
) -> Result<Json<PrivacyRecordsClearResponse>, ScratchError> {
    if app
        .gcx
        .cmdline
        .daemon_auth_token
        .as_deref()
        .is_some_and(|token| !token.is_empty())
    {
        crate::buddy::operator_runtime::authenticated_operator_request(&app, &headers)
            .map_err(|error| ScratchError::new(StatusCode::FORBIDDEN, error))?;
    }
    let request: PrivacyRecordsClearRequest = serde_json::from_slice(&body_bytes)
        .map_err(|_| ScratchError::new(StatusCode::BAD_REQUEST, "Invalid JSON request".into()))?;
    if request.paths.is_empty() {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "paths must not be empty".to_string(),
        ));
    }
    let session_arc = {
        let sessions = app.gcx.chat_sessions.read().await;
        sessions.get(&request.chat_id).cloned()
    }
    .ok_or_else(|| {
        ScratchError::new(StatusCode::NOT_FOUND, "Chat session not found".to_string())
    })?;
    let paths = request
        .paths
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    crate::privacy::load_privacy_if_needed(app.gcx.clone()).await;
    let mut response = {
        let mut session = session_arc.lock().await;
        if session.has_runner_lease()
            || session.turn_depth > 0
            || !crate::chat::context_rebuild::explicit_context_ready(&session)
            || crate::chat::context_rebuild::compression_attempt_active(&session)
            || session
                .native_commands_in_flight
                .load(std::sync::atomic::Ordering::SeqCst)
                != 0
            || matches!(
                session.runtime.state,
                crate::chat::types::SessionState::Generating
                    | crate::chat::types::SessionState::ExecutingTools
            )
        {
            return Err(ScratchError::new(
                StatusCode::CONFLICT,
                "chat is busy; try again when it is idle".into(),
            ));
        }
        let classification = crate::privacy::classification_cache::snapshot(&app.gcx)
            .map_err(|error| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, error))?;
        let protected = |record: &FileRecord| {
            blocked_zone(&record.zone)
                || blocked_zone(
                    &classification
                        .classify(&app.gcx, std::path::Path::new(&record.path))
                        .zone
                        .name,
                )
        };
        let mut response = PrivacyRecordsClearResponse::default();
        let mut messages = session.messages.clone();
        for message in messages.iter_mut() {
            clear_message_records(&mut message.extra, &paths, &protected, &mut response);
        }
        let removable_derived = {
            let derived = session
                .derived_privacy_zones
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            paths
                .iter()
                .filter(|path| {
                    !derived
                        .get(std::path::Path::new(path))
                        .is_some_and(|entry| blocked_zone(&entry.zone))
                        && !blocked_zone(
                            &classification
                                .classify(&app.gcx, std::path::Path::new(path))
                                .zone
                                .name,
                        )
                })
                .map(|path| std::path::PathBuf::from(*path))
                .collect::<Vec<_>>()
        };
        classification
            .finish(&app.gcx)
            .map_err(|error| ScratchError::new(StatusCode::CONFLICT, error))?;
        {
            let mut derived = session
                .derived_privacy_zones
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            for path in removable_derived {
                derived.remove(&path);
            }
        }
        session.messages = messages;
        if response.cleared > 0 {
            session.increment_version();
            let event = session.snapshot();
            session.emit(event);
        }
        response
    };
    if response.cleared > 0 {
        if let Err(error) = crate::chat::trajectories::try_save_trajectory_with_intent(
            app.clone(),
            session_arc,
            crate::chat::types::TrajectoryCommitIntent::Required,
        )
        .await
        {
            tracing::warn!("privacy records cleared in memory but not persisted: {error}");
            response.persisted = Some(false);
        }
    }
    tracing::info!(
        "privacy records cleared: chat_id={} cleared={}",
        request.chat_id,
        response.cleared
    );
    Ok(Json(response))
}

fn blocked_zone(zone: &str) -> bool {
    zone == "blocked"
        || zone
            .strip_prefix("effective:")
            .is_some_and(|zones| zones.split('+').any(|zone| zone == "blocked"))
}

fn clear_message_records(
    message: &mut serde_json::Map<String, serde_json::Value>,
    paths: &HashSet<&str>,
    protected: &dyn Fn(&FileRecord) -> bool,
    response: &mut PrivacyRecordsClearResponse,
) {
    if let Some(nested) = message
        .get_mut(refact_core::active_context::COMPRESSION_REPORT_KEY)
        .and_then(|report| report.get_mut("payload"))
        .and_then(|payload| payload.get_mut("messages"))
        .and_then(serde_json::Value::as_array_mut)
    {
        for nested in nested
            .iter_mut()
            .filter_map(serde_json::Value::as_object_mut)
        {
            clear_message_records(nested, paths, protected, response);
        }
    }
    let Some(value) = message.get("privacy") else {
        return;
    };
    let Ok(mut privacy) = serde_json::from_value::<refact_privacy::PrivacyRecord>(value.clone())
    else {
        return;
    };
    let before = privacy.files.len();
    privacy
        .files
        .retain(|record| !paths.contains(record.path.as_str()) || protected(record));
    response.cleared += before - privacy.files.len();
    response.remaining += privacy.files.len();
    if privacy.files.len() == before {
        return;
    }
    if privacy.files.is_empty() {
        message.remove("privacy");
    } else if let Ok(value) = serde_json::to_value(privacy) {
        message.insert("privacy".to_string(), value);
    }
}

async fn session_derived_zones(
    app: &AppState,
    chat_id: &str,
) -> Result<DerivedPrivacyZones, ScratchError> {
    let session = {
        let sessions = app.gcx.chat_sessions.read().await;
        sessions.get(chat_id).cloned()
    }
    .ok_or_else(|| {
        ScratchError::new(StatusCode::NOT_FOUND, "Chat session not found".to_string())
    })?;
    let derived_zones = session.lock().await.derived_privacy_zones.clone();
    Ok(derived_zones)
}

async fn build_policy_response(app: &AppState) -> Result<PrivacyPolicyResponse, ScratchError> {
    let load = app.gcx.privacy_policy_load.read().unwrap().clone();
    let effective = load.policy.as_ref().clone();
    let policy = crate::privacy::global_only_policy(app.gcx.clone())
        .await
        .unwrap_or_else(|| effective.clone());
    let destinations = collect_destinations(app, &effective).await;
    let match_counts = live_match_counts(app, &effective)?;
    Ok(policy_response(
        policy,
        effective,
        load,
        destinations,
        match_counts,
        false,
    ))
}

async fn build_saved_policy_response(app: &AppState) -> PrivacyPolicyResponse {
    let load = app.gcx.privacy_policy_load.read().unwrap().clone();
    let effective = load.policy.as_ref().clone();
    let policy = crate::privacy::global_only_policy(app.gcx.clone())
        .await
        .unwrap_or_else(|| effective.clone());
    policy_response(policy, effective, load, Vec::new(), BTreeMap::new(), true)
}

fn policy_response(
    policy: PrivacyPolicy,
    effective: PrivacyPolicy,
    load: refact_privacy::PolicyLoad,
    destinations: Vec<Destination>,
    match_counts: BTreeMap<String, usize>,
    discovery_deferred: bool,
) -> PrivacyPolicyResponse {
    let has_project_overrides = policy != effective;
    PrivacyPolicyResponse {
        policy,
        destinations,
        match_counts,
        discovery_deferred,
        error: load.error,
        source_paths: load
            .source_paths
            .into_iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
        has_project_overrides,
    }
}

async fn collect_destinations(app: &AppState, policy: &PrivacyPolicy) -> Vec<Destination> {
    let mut destinations = Vec::new();
    let caps = crate::global_context::try_load_caps_quickly_if_not_present(app.gcx.clone(), 0)
        .await
        .ok();
    if let Some(caps) = caps {
        for model in caps.chat_models.values() {
            let id = provider_id(&model.base.id);
            destinations.push(Destination {
                id: DestinationId(id.clone()),
                kind: DestinationKind::Provider,
                display_name: id,
            });
        }
        for model in caps.completion_models.values() {
            let id = provider_id(&model.base.id);
            destinations.push(Destination {
                id: DestinationId(id.clone()),
                kind: DestinationKind::Completion,
                display_name: id,
            });
        }
        if !caps.embedding_model.base.id.is_empty() {
            let id = provider_id(&caps.embedding_model.base.id);
            destinations.push(Destination {
                id: DestinationId(id.clone()),
                kind: DestinationKind::Provider,
                display_name: id,
            });
        }
    }
    let mut mcp_paths =
        crate::integrations::setting_up_integrations::integrations_all(app.gcx.clone(), false)
            .await
            .integrations
            .into_iter()
            .filter(|record| record.integr_name.starts_with("mcp_") && record.integr_config_exists)
            .map(|record| record.integr_config_path)
            .collect::<Vec<_>>();
    {
        let sessions = app.gcx.integration_sessions.lock().await;
        mcp_paths.extend(sessions.keys().cloned());
    }
    for path in mcp_paths {
        let id = crate::integrations::mcp::mcp_interactions::server_name_from_config_path(&path);
        destinations.push(Destination {
            id: DestinationId(id.clone()),
            kind: DestinationKind::Mcp,
            display_name: id,
        });
    }
    let known_ids = destinations
        .iter()
        .map(|destination| destination.id.0.clone())
        .collect::<HashSet<_>>();
    for id in policy
        .zones
        .iter()
        .flat_map(|zone| zone.send_to.iter())
        .filter(|id| id.as_str() != "*" && !known_ids.contains(id.as_str()))
    {
        destinations.push(Destination {
            id: DestinationId(id.clone()),
            kind: DestinationKind::Provider,
            display_name: id.clone(),
        });
    }
    destinations.sort_by(|left, right| destination_key(left).cmp(&destination_key(right)));
    destinations.dedup_by(|left, right| left.id == right.id && left.kind == right.kind);
    destinations
}

const MATCH_COUNTS_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(60);

fn policy_fingerprint(policy: &PrivacyPolicy) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(policy)
        .unwrap_or_default()
        .hash(&mut hasher);
    hasher.finish()
}

fn live_match_counts(
    app: &AppState,
    policy: &PrivacyPolicy,
) -> Result<BTreeMap<String, usize>, ScratchError> {
    let fingerprint = policy_fingerprint(policy);
    let (cached, cold, needs_refresh) = {
        let cache = app
            .gcx
            .privacy_match_counts
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let fresh = cache.fingerprint == fingerprint
            && cache
                .computed_at
                .is_some_and(|at| at.elapsed() < MATCH_COUNTS_MAX_AGE);
        (
            cache.counts.clone(),
            cache.computed_at.is_none(),
            !fresh && !cache.computing,
        )
    };
    if cold {
        let counts = compute_match_counts(&app.gcx, policy).unwrap_or_default();
        let mut cache = app
            .gcx
            .privacy_match_counts
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.counts = counts.clone();
        cache.fingerprint = fingerprint;
        cache.computed_at = Some(std::time::Instant::now());
        return Ok(counts);
    }
    if needs_refresh {
        spawn_match_counts_refresh(app.gcx.clone(), policy.clone(), fingerprint);
    }
    Ok(cached)
}

fn spawn_match_counts_refresh(
    gcx: Arc<crate::global_context::GlobalContext>,
    policy: PrivacyPolicy,
    fingerprint: u64,
) {
    {
        let mut cache = gcx
            .privacy_match_counts
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if cache.computing {
            return;
        }
        cache.computing = true;
    }
    tokio::task::spawn_blocking(move || {
        let computed = compute_match_counts(&gcx, &policy);
        let mut cache = gcx
            .privacy_match_counts
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.computing = false;
        if let Some(counts) = computed {
            cache.counts = counts;
            cache.fingerprint = fingerprint;
            cache.computed_at = Some(std::time::Instant::now());
        }
    });
}

fn compute_match_counts(
    gcx: &Arc<crate::global_context::GlobalContext>,
    policy: &PrivacyPolicy,
) -> Option<BTreeMap<String, usize>> {
    let compiled = policy.compile().ok()?;
    let mappings = registered_worktree_path_mappings(gcx.cache_dir.as_path());
    let workspace_roots = gcx
        .documents_state
        .workspace_folders
        .lock()
        .unwrap()
        .clone();
    let files = gcx.documents_state.workspace_files.lock().unwrap().clone();
    let mut counts = policy
        .zones
        .iter()
        .map(|zone| (zone.name.clone(), 0))
        .collect::<BTreeMap<_, _>>();
    counts.entry("blocked".to_string()).or_default();
    for path in files {
        let zone = strictest_zone_for_path(&compiled, &path, &workspace_roots, &mappings);
        *counts.entry(zone.name.clone()).or_default() += 1;
    }
    Some(counts)
}

fn provider_id(model_id: &str) -> String {
    model_id
        .split_once('/')
        .map_or(model_id, |(provider, _)| provider)
        .to_string()
}

fn destination_key(destination: &Destination) -> (u8, &str) {
    let kind = match destination.kind {
        DestinationKind::Provider => 0,
        DestinationKind::Mcp => 1,
        DestinationKind::SubagentModel => 2,
        DestinationKind::Completion => 3,
    };
    (kind, destination.id.0.as_str())
}

fn observation_capability(
    runtime: crate::privacy::PrivacyObservationRuntimeState,
) -> PrivacyObservationCapability {
    PrivacyObservationCapability {
        platform_supported: cfg!(all(
            target_os = "linux",
            any(target_arch = "x86_64", target_arch = "aarch64")
        )),
        runtime_available: runtime.runtime_available,
        last_error: runtime.last_error,
    }
}

#[cfg(test)]
fn platform_supported_for_tests() -> bool {
    cfg!(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::Request;
    use hyper::body::to_bytes;
    use serde_json::{json, Value};
    use tokio::sync::Mutex as AMutex;
    use tower::ServiceExt;

    use super::*;
    use crate::call_validation::ChatMessage;
    use crate::chat::types::ChatSession;
    use crate::privacy::{FilePrivacySettings, PrivacySettings};

    async fn test_app() -> (tempfile::TempDir, tempfile::TempDir, AppState) {
        let cache = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            cache.path().to_path_buf(),
            config.path().to_path_buf(),
        )
        .await;
        *gcx.privacy_settings.write().unwrap() = Arc::new(PrivacySettings {
            privacy_rules: FilePrivacySettings {
                blocked: Vec::new(),
                only_send_to_servers_I_control: Vec::new(),
            },
            loaded_ts: 0,
        });
        (cache, config, AppState::from_gcx(gcx).await)
    }

    async fn json_request(router: axum::Router, request: Request<Body>) -> (StatusCode, Value) {
        let response = router.oneshot(request).await.unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body()).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }

    fn post_json(uri: &str, body: Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    fn policy(secret_path: &str) -> Value {
        json!({
            "blocked": [],
            "zones": [
                {
                    "name": "secrets",
                    "patterns": [secret_path],
                    "send_to": ["trusted"],
                    "on_shell_read": "withhold"
                },
                {
                    "name": "normal",
                    "patterns": ["**"],
                    "send_to": ["*"],
                    "on_shell_read": "withhold"
                }
            ],
            "subagents": { "report_declassifies": true }
        })
    }

    #[tokio::test]
    async fn privacy_policy_post_rejects_empty_patterns() {
        let (_cache, _config, app) = test_app().await;
        let router = crate::http::routers::make_refact_http_server(app);
        for pattern in ["", " ", "\t\n", "/", " / ", ".", "./", "C:\\", "c:"] {
            for blocked in [false, true] {
                let mut body = policy(".env");
                if blocked {
                    body["blocked"] = json!([pattern]);
                } else {
                    body["zones"][0]["patterns"] = json!([pattern]);
                }
                let response = router
                    .clone()
                    .oneshot(post_json("/v1/privacy/policy", body))
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
                let body = to_bytes(response.into_body()).await.unwrap();
                assert!(String::from_utf8_lossy(&body).contains("use ** to mean everything"));
            }
        }
    }

    #[tokio::test]
    async fn privacy_policy_post_defers_discovery_and_get_returns_complete_response() {
        let (_cache, config, app) = test_app().await;
        let secret = config.path().join("secret.env");
        let normal = config.path().join("normal.rs");
        tokio::fs::write(&secret, "secret").await.unwrap();
        tokio::fs::write(&normal, "normal").await.unwrap();
        *app.gcx.documents_state.workspace_files.lock().unwrap() =
            vec![secret.clone(), normal.clone()];
        tokio::fs::write(
            config.path().join("privacy.yaml"),
            "hooks:\n  trusted_projects:\n    - /kept\n",
        )
        .await
        .unwrap();
        let router = crate::http::routers::make_refact_http_server(app);

        let (status, posted) = json_request(
            router.clone(),
            post_json("/v1/privacy/policy", policy(&secret.to_string_lossy())),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(posted["policy"]["zones"][0]["name"], "secrets");
        assert_eq!(posted["match_counts"], json!({}));
        assert_eq!(posted["destinations"], json!([]));
        assert_eq!(posted["discovery_deferred"], true);
        assert!(posted["error"].is_null());
        assert_eq!(
            posted["source_paths"],
            json!([config
                .path()
                .join("privacy.yaml")
                .to_string_lossy()
                .into_owned()])
        );
        assert_eq!(posted["has_project_overrides"], false);

        let (status, loaded) = json_request(
            router,
            Request::builder()
                .uri("/v1/privacy/policy")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(loaded["policy"], posted["policy"]);
        assert_eq!(loaded["discovery_deferred"], false);
        assert_eq!(loaded["match_counts"]["secrets"], 1);
        assert_eq!(loaded["match_counts"]["normal"], 1);
        assert!(loaded["destinations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|destination| destination["id"] == "trusted"));
        let saved = tokio::fs::read_to_string(config.path().join("privacy.yaml"))
            .await
            .unwrap();
        let saved: serde_yaml::Value = serde_yaml::from_str(&saved).unwrap();
        assert_eq!(saved["hooks"]["trusted_projects"][0], "/kept");
    }

    #[tokio::test]
    async fn privacy_policy_save_replaces_and_backs_up_malformed_config() {
        let (_cache, config, app) = test_app().await;
        let path = config.path().join("privacy.yaml");
        let malformed = "privacy_rules: [";
        tokio::fs::write(&path, malformed).await.unwrap();
        let router = crate::http::routers::make_refact_http_server(app);

        let (status, response) =
            json_request(router, post_json("/v1/privacy/policy", policy("**/.env"))).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(response["policy"]["zones"][0]["name"], "secrets");
        let saved = tokio::fs::read_to_string(&path).await.unwrap();
        let saved: serde_yaml::Value = serde_yaml::from_str(&saved).unwrap();
        assert_eq!(saved["privacy_rules"]["zones"][0]["name"], "secrets");
        let mut backups = tokio::fs::read_dir(config.path()).await.unwrap();
        let mut backup_paths = Vec::new();
        while let Some(entry) = backups.next_entry().await.unwrap() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("privacy.yaml.corrupt-") {
                backup_paths.push(entry.path());
            }
        }
        assert_eq!(backup_paths.len(), 1);
        assert_eq!(
            tokio::fs::read_to_string(&backup_paths[0]).await.unwrap(),
            malformed
        );
    }

    #[tokio::test]
    async fn privacy_status_before_observation_is_not_optimistic() {
        let (_cache, config, app) = test_app().await;
        tokio::fs::write(config.path().join("privacy.yaml"), "privacy_rules: [")
            .await
            .unwrap();
        let router = crate::http::routers::make_refact_http_server(app);

        let (status, response) = json_request(
            router,
            Request::builder()
                .uri("/v1/privacy/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(response["platform"], std::env::consts::OS);
        assert!(response["config_error"]
            .as_str()
            .unwrap()
            .contains("failed to parse"));
        assert_eq!(
            response["observation"]["platform_supported"],
            platform_supported_for_tests()
        );
        assert_eq!(response["observation"]["runtime_available"], false);
        assert!(response["observation"]["last_error"].is_null());
    }

    fn guarded(path: &str, zone: &str) -> FileRecord {
        FileRecord {
            path: path.to_string(),
            zone: zone.to_string(),
            attribution: refact_privacy::Attribution::Observed,
        }
    }

    fn message_with(id: &str, records: Vec<FileRecord>) -> ChatMessage {
        let mut message = ChatMessage::new("user".to_string(), id.to_string());
        message.message_id = id.to_string();
        crate::privacy::records::merge_records(&mut message, records);
        message
    }

    async fn insert_session(app: &AppState, chat_id: &str, messages: Vec<ChatMessage>) {
        let mut session = ChatSession::new(chat_id.to_string());
        session.messages = messages;
        app.gcx
            .chat_sessions
            .write()
            .await
            .insert(chat_id.to_string(), Arc::new(AMutex::new(session)));
    }

    #[tokio::test]
    async fn privacy_records_clear_keeps_derived_blocked_taint() {
        let (_cache, _config, app) = test_app().await;
        insert_session(&app, "derived-clear", vec![]).await;
        let session = app
            .gcx
            .chat_sessions
            .read()
            .await
            .get("derived-clear")
            .cloned()
            .unwrap();
        for (path, zone) in [
            ("/derived-blocked", "blocked"),
            ("/derived-composite", "effective:blocked+secrets"),
        ] {
            session
                .lock()
                .await
                .derived_privacy_zones
                .write()
                .unwrap()
                .insert(
                    std::path::PathBuf::from(path),
                    crate::privacy::records::DerivedZone {
                        zone: zone.into(),
                        origin: None,
                    },
                );
        }
        let router = crate::http::routers::make_refact_http_server(app);
        let (status, _) = json_request(router, post_json("/v1/privacy/records/clear", json!({"chat_id": "derived-clear", "paths": ["/derived-blocked", "/derived-composite"]}))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            session
                .lock()
                .await
                .derived_privacy_zones
                .read()
                .unwrap()
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn privacy_records_clear_removes_nested_records_persists_and_keeps_blocked() {
        let (_cache, config, app) = test_app().await;
        *app.gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![config.path().to_path_buf()];
        let stale = "/home/user/.ssh/config";
        let payload = vec![message_with(
            "payload",
            vec![
                guarded(stale, "secrets"),
                guarded("/blocked.key", "blocked"),
            ],
        )];
        let report =
            refact_core::active_context::make_reconstruction_report(payload, Default::default())
                .unwrap();
        let mut summary_report = report.clone();
        crate::privacy::records::merge_records(
            &mut summary_report,
            vec![guarded(stale, "secrets"), guarded("/kept.rs", "secrets")],
        );
        insert_session(
            &app,
            "chat-clear",
            vec![
                message_with("raw", vec![guarded(stale, "secrets")]),
                summary_report,
                message_with("after", vec![guarded(stale, "secrets")]),
            ],
        )
        .await;
        {
            let sessions = app.gcx.chat_sessions.read().await;
            let session = sessions.get("chat-clear").unwrap().lock().await;
            session.derived_privacy_zones.write().unwrap().insert(
                std::path::PathBuf::from(stale),
                crate::privacy::records::DerivedZone {
                    zone: "secrets".to_string(),
                    origin: None,
                },
            );
        }
        let router = crate::http::routers::make_refact_http_server(app.clone());

        let (status, cleared) = json_request(
            router.clone(),
            post_json(
                "/v1/privacy/records/clear",
                json!({"chat_id": "chat-clear", "paths": [stale, "/blocked.key"]}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(cleared, json!({"cleared": 4, "remaining": 2}));

        let (status, inspected) = json_request(
            router,
            post_json(
                "/v1/privacy/inspect",
                json!({
                    "chat_id": "chat-clear",
                    "destination": {"id": "x", "kind": "provider", "display_name": "x"},
                    "include_messages": false
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(!inspected["records"]
            .as_array()
            .unwrap()
            .iter()
            .any(|record| record["path"] == stale));

        let has_stale = |messages: &[ChatMessage]| {
            let active = refact_core::active_context::active_context(messages).unwrap();
            refact_privacy::records_from_messages(&active.messages)
                .unwrap()
                .into_iter()
                .chain(refact_privacy::records_from_messages(messages).unwrap())
                .map(|(_, record)| record)
                .collect::<Vec<_>>()
        };
        let session = app
            .gcx
            .chat_sessions
            .read()
            .await
            .get("chat-clear")
            .cloned()
            .unwrap();
        let live = has_stale(&session.lock().await.messages);
        assert!(!live.iter().any(|record| record.path == stale));
        assert!(live.contains(&guarded("/blocked.key", "blocked")));
        assert!(live.contains(&guarded("/kept.rs", "secrets")));
        assert!(session
            .lock()
            .await
            .derived_privacy_zones
            .read()
            .unwrap()
            .is_empty());

        let loaded =
            crate::chat::trajectories::load_trajectory_for_chat(app.gcx.clone(), "chat-clear")
                .await
                .unwrap();
        let persisted = has_stale(&loaded.messages);
        assert!(!persisted.iter().any(|record| record.path == stale));
        assert!(persisted.contains(&guarded("/blocked.key", "blocked")));
    }

    #[tokio::test]
    async fn privacy_records_clear_current_blocked_and_busy_sessions() {
        let (_cache, config, app) = test_app().await;
        let blocked = config
            .path()
            .join("now-blocked.key")
            .to_string_lossy()
            .into_owned();
        let parsed = PrivacyPolicy {
            blocked: vec![blocked.clone()],
            ..Default::default()
        };
        crate::privacy::save_privacy_policy(app.gcx.clone(), parsed)
            .await
            .unwrap();
        insert_session(
            &app,
            "busy-clear",
            vec![message_with(
                "m",
                vec![
                    guarded(&blocked, "secrets"),
                    guarded("/composite", "effective:blocked+secrets"),
                    guarded("/ordinary", "old-zone"),
                ],
            )],
        )
        .await;
        let session = app
            .gcx
            .chat_sessions
            .read()
            .await
            .get("busy-clear")
            .unwrap()
            .clone();
        let router = crate::http::routers::make_refact_http_server(app.clone());
        let request = || {
            post_json(
                "/v1/privacy/records/clear",
                json!({
                    "chat_id": "busy-clear", "paths": [&blocked, "/composite", "/ordinary"]
                }),
            )
        };
        for state in [
            crate::chat::types::SessionState::Generating,
            crate::chat::types::SessionState::ExecutingTools,
        ] {
            session.lock().await.runtime.state = state;
            let response = router.clone().oneshot(request()).await.unwrap();
            assert_eq!(response.status(), StatusCode::CONFLICT);
        }
        {
            let mut session = session.lock().await;
            session.runtime.state = crate::chat::types::SessionState::Idle;
            session
                .claim_runner(
                    "owner".into(),
                    Arc::new(std::sync::atomic::AtomicBool::new(false)),
                )
                .unwrap();
        }
        assert_eq!(
            router.clone().oneshot(request()).await.unwrap().status(),
            StatusCode::CONFLICT
        );
        session.lock().await.release_runner("owner");
        let (status, response) = json_request(router, request()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(response["cleared"], 1);
        assert_eq!(response["remaining"], 2);
        let records =
            refact_privacy::records_from_messages(&session.lock().await.messages).unwrap();
        assert!(records.iter().any(|(_, record)| record.path == blocked));
        assert!(records
            .iter()
            .any(|(_, record)| record.zone == "effective:blocked+secrets"));
    }

    #[tokio::test]
    async fn privacy_records_clear_reports_failed_persistence_and_keeps_dirty_memory() {
        let (_cache, config, app) = test_app().await;
        *app.gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![config.path().to_path_buf()];
        crate::privacy::load_privacy_if_needed(app.gcx.clone()).await;
        std::fs::write(config.path().join(".refact"), "not a directory").unwrap();
        insert_session(
            &app,
            "clear-save-failure",
            vec![message_with("m", vec![guarded("/ordinary", "old-zone")])],
        )
        .await;
        let router = crate::http::routers::make_refact_http_server(app.clone());
        let (status, response) = json_request(
            router,
            post_json(
                "/v1/privacy/records/clear",
                json!({"chat_id": "clear-save-failure", "paths": ["/ordinary"]}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            response,
            json!({"cleared": 1, "remaining": 0, "persisted": false})
        );
        let sessions = app.gcx.chat_sessions.read().await;
        let session = sessions.get("clear-save-failure").unwrap().lock().await;
        assert!(session.trajectory_dirty);
        assert!(refact_privacy::records_from_messages(&session.messages)
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn privacy_records_clear_rejects_empty_paths_and_unknown_chat() {
        let (_cache, _config, app) = test_app().await;
        insert_session(&app, "chat-known", Vec::new()).await;
        let router = crate::http::routers::make_refact_http_server(app);
        for (body, expected) in [
            (
                json!({"chat_id": "chat-known", "paths": []}),
                StatusCode::BAD_REQUEST,
            ),
            (json!({"chat_id": "chat-known"}), StatusCode::BAD_REQUEST),
            (
                json!({"chat_id": "missing", "paths": ["/a"]}),
                StatusCode::NOT_FOUND,
            ),
        ] {
            let response = router
                .clone()
                .oneshot(post_json("/v1/privacy/records/clear", body))
                .await
                .unwrap();
            assert_eq!(response.status(), expected);
        }
    }

    #[tokio::test]
    async fn privacy_records_clear_requires_operator_when_token_configured() {
        let mut gcx = crate::global_context::tests::make_test_gcx().await;
        Arc::get_mut(&mut gcx).unwrap().cmdline.daemon_auth_token = Some("clear-secret".into());
        let app = AppState::from_gcx(gcx).await;
        for token in [None, Some("Bearer wrong"), Some("Bearer clear-secret")] {
            let mut headers = axum::http::HeaderMap::new();
            if let Some(token) = token {
                headers.insert("authorization", token.parse().unwrap());
            }
            let error = handle_v1_privacy_records_clear(
                State(app.clone()),
                headers,
                hyper::body::Bytes::from_static(br#"{"chat_id":"missing","paths":["/a"]}"#),
            )
            .await
            .unwrap_err();
            assert_eq!(
                error.status_code,
                if token == Some("Bearer clear-secret") {
                    StatusCode::NOT_FOUND
                } else {
                    StatusCode::FORBIDDEN
                }
            );
        }
    }

    #[tokio::test]
    async fn privacy_inspect_keeps_large_allowed_histories_out_of_the_response() {
        let (_cache, config, app) = test_app().await;
        let secret = config.path().join("secret.env");
        let parsed: PrivacyPolicy =
            serde_json::from_value(policy(&secret.to_string_lossy())).unwrap();
        crate::privacy::save_privacy_policy(app.gcx.clone(), parsed)
            .await
            .unwrap();
        let mut message = ChatMessage::new("user".to_string(), "x".repeat(1024 * 1024));
        crate::privacy::records::attach_record(
            &mut message,
            FileRecord {
                path: secret.to_string_lossy().into_owned(),
                zone: "secrets".to_string(),
                attribution: refact_privacy::Attribution::Declared,
            },
        );
        let mut session = ChatSession::new("chat-1".to_string());
        session.messages.push(message);
        app.gcx
            .chat_sessions
            .write()
            .await
            .insert("chat-1".to_string(), Arc::new(AMutex::new(session)));
        let router = crate::http::routers::make_refact_http_server(app);
        let destination = |id: &str| {
            json!({
                "id": id,
                "kind": "provider",
                "display_name": id
            })
        };

        let (status, blocked) = json_request(
            router.clone(),
            post_json(
                "/v1/privacy/inspect",
                json!({"chat_id": "chat-1", "destination": destination("untrusted")}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(blocked["sendable"], false);
        assert_eq!(blocked["would_send"], json!([]));
        assert_eq!(blocked["blocked"][0]["record_index"], 0);
        assert_eq!(blocked["blocked"][0]["record"]["zone"], "secrets");

        let (status, allowed) = json_request(
            router,
            post_json(
                "/v1/privacy/inspect",
                json!({
                    "chat_id": "chat-1",
                    "destination": destination("trusted"),
                    "include_messages": false
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(allowed["sendable"], true);
        assert!(allowed.get("would_send").is_none());
        assert_eq!(allowed["blocked"], json!([]));
        assert!(serde_json::to_vec(&allowed).unwrap().len() < 4096);
    }

    #[tokio::test]
    async fn privacy_inspect_agrees_with_enforcement_for_removed_zones() {
        let (_cache, config, app) = test_app().await;
        let secret = config.path().join("secret.env");
        let normal = config.path().join("normal.rs");
        tokio::fs::write(&secret, "secret").await.unwrap();
        tokio::fs::write(&normal, "normal").await.unwrap();
        let parsed: PrivacyPolicy =
            serde_json::from_value(policy(&secret.to_string_lossy())).unwrap();
        crate::privacy::save_privacy_policy(app.gcx.clone(), parsed)
            .await
            .unwrap();
        let stale_record = |path: &std::path::Path, zone: &str| FileRecord {
            path: path.to_string_lossy().into_owned(),
            zone: zone.to_string(),
            attribution: refact_privacy::Attribution::Observed,
        };
        let records = vec![
            stale_record(&normal, "removed-zone"),
            stale_record(&secret, "another-removed-zone"),
        ];
        insert_session(
            &app,
            "chat-removed",
            vec![message_with("m", records.clone())],
        )
        .await;
        let router = crate::http::routers::make_refact_http_server(app.clone());
        let destination = |id: &str| json!({"id": id, "kind": "provider", "display_name": id});

        for (id, expected_sendable) in [("untrusted", false), ("trusted", true)] {
            let (status, inspected) = json_request(
                router.clone(),
                post_json(
                    "/v1/privacy/inspect",
                    json!({
                        "chat_id": "chat-removed",
                        "destination": destination(id),
                        "include_messages": false
                    }),
                ),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(inspected["sendable"], expected_sendable, "{id}");
            let blocked_paths = inspected["blocked"]
                .as_array()
                .unwrap()
                .iter()
                .map(|entry| entry["record"]["path"].as_str().unwrap().to_string())
                .collect::<std::collections::HashSet<_>>();
            let expected_blocked = if expected_sendable {
                std::collections::HashSet::new()
            } else {
                std::collections::HashSet::from([secret.to_string_lossy().into_owned()])
            };
            assert_eq!(blocked_paths, expected_blocked, "{id}");

            let destination: refact_privacy::Destination =
                serde_json::from_value(destination(id)).unwrap();
            let enforced = crate::privacy::destinations::clear_current(
                &app.gcx,
                AuditedMessages {
                    records: Ok(records.iter().cloned().enumerate().collect()),
                },
                &destination,
            );
            assert_eq!(enforced.is_ok(), expected_sendable, "{id}");
        }
    }
}
