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
use crate::call_validation::ChatMessage;
use crate::custom_error::ScratchError;
use crate::files_correction::registered_worktree_path_mappings;
use crate::files_in_workspace::strictest_zone_for_path;

#[derive(Debug, Serialize)]
pub struct PrivacyPolicyResponse {
    pub policy: PrivacyPolicy,
    pub destinations: Vec<Destination>,
    pub match_counts: BTreeMap<String, usize>,
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
    pub would_send: Vec<ChatMessage>,
    pub records: Vec<FileRecord>,
    pub blocked: Vec<PrivacyBlockedRecord>,
    pub refusal: Option<String>,
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
    policy.compile().map_err(|error| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Invalid privacy policy: {error}"),
        )
    })?;
    crate::privacy::save_privacy_policy(app.gcx.clone(), policy)
        .await
        .map_err(|error| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, error))?;
    Ok(Json(build_policy_response(&app).await?))
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
    let session = {
        let sessions = app.gcx.chat_sessions.read().await;
        sessions.get(&request.chat_id).cloned()
    }
    .ok_or_else(|| {
        ScratchError::new(StatusCode::NOT_FOUND, "Chat session not found".to_string())
    })?;
    let messages = Arc::new(session.lock().await.messages.clone());
    let indexed_records = refact_privacy::records_from_messages(&messages);
    let records = indexed_records
        .as_ref()
        .map(|records| records.iter().map(|(_, record)| record.clone()).collect())
        .unwrap_or_default();
    let policy = app.gcx.privacy_policy_load.read().unwrap().policy.clone();
    let compiled = policy.compile().map_err(|error| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to compile privacy policy: {error}"),
        )
    })?;
    let clearance = refact_privacy::clear(
        AuditedMessages {
            records: indexed_records,
        },
        &request.destination,
        &compiled,
    );
    let sendable_messages = messages
        .iter()
        .cloned()
        .map(|mut message| {
            message.extra.remove("privacy");
            message
        })
        .collect::<Vec<_>>();
    let (sendable, would_send, blocked, refusal) = match clearance {
        Ok(_) => (true, sendable_messages, Vec::new(), None),
        Err(refusal) => (
            false,
            Vec::new(),
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
        would_send,
        records,
        blocked,
        refusal,
    }))
}

async fn build_policy_response(app: &AppState) -> Result<PrivacyPolicyResponse, ScratchError> {
    let load = app.gcx.privacy_policy_load.read().unwrap().clone();
    let effective = load.policy.as_ref().clone();
    let policy = crate::privacy::global_only_policy(app.gcx.clone())
        .await
        .unwrap_or_else(|| effective.clone());
    let has_project_overrides = policy != effective;
    let destinations = collect_destinations(app, &effective).await;
    let match_counts = live_match_counts(app, &effective)?;
    Ok(PrivacyPolicyResponse {
        policy,
        destinations,
        match_counts,
        error: load.error,
        source_paths: load
            .source_paths
            .into_iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
        has_project_overrides,
    })
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

fn live_match_counts(
    app: &AppState,
    policy: &PrivacyPolicy,
) -> Result<BTreeMap<String, usize>, ScratchError> {
    let compiled = policy.compile().map_err(|error| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Loaded privacy policy does not compile: {error}"),
        )
    })?;
    let mappings = registered_worktree_path_mappings(app.gcx.cache_dir.as_path());
    let workspace_roots = app
        .gcx
        .documents_state
        .workspace_folders
        .lock()
        .unwrap()
        .clone();
    let files = app
        .gcx
        .documents_state
        .workspace_files
        .lock()
        .unwrap()
        .clone();
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
    Ok(counts)
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
    async fn privacy_policy_roundtrip_preserves_other_config_and_counts_live_files() {
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
        assert_eq!(posted["match_counts"]["secrets"], 1);
        assert_eq!(posted["match_counts"]["normal"], 1);
        assert!(posted["destinations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|destination| destination["id"] == "trusted"));

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
    async fn privacy_policy_get_surfaces_last_load_error() {
        let (_cache, config, app) = test_app().await;
        tokio::fs::write(config.path().join("privacy.yaml"), "privacy_rules: [")
            .await
            .unwrap();
        let router = crate::http::routers::make_refact_http_server(app);

        let (status, response) = json_request(
            router,
            Request::builder()
                .uri("/v1/privacy/policy")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert!(response["error"]
            .as_str()
            .unwrap()
            .contains("failed to parse"));
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

    #[tokio::test]
    async fn privacy_status_keeps_platform_support_when_runtime_is_unavailable() {
        let (_cache, _config, app) = test_app().await;
        crate::privacy::record_observation_status(
            &app.gcx,
            &refact_exec::ObservationStatus::Unavailable("ptrace denied".to_string()),
        );
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
        assert_eq!(
            response["observation"]["platform_supported"],
            platform_supported_for_tests()
        );
        assert_eq!(response["observation"]["runtime_available"], false);
        assert_eq!(response["observation"]["last_error"], "ptrace denied");
    }

    #[tokio::test]
    async fn privacy_inspect_reports_allowed_messages_and_blocking_records() {
        let (_cache, config, app) = test_app().await;
        let secret = config.path().join("secret.env");
        let parsed: PrivacyPolicy =
            serde_json::from_value(policy(&secret.to_string_lossy())).unwrap();
        *app.gcx.privacy_policy_load.write().unwrap() = refact_privacy::PolicyLoad {
            policy: Arc::new(parsed),
            error: None,
            source_paths: Vec::new(),
        };
        let mut message = ChatMessage::new("user".to_string(), "use the secret".to_string());
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
                json!({"chat_id": "chat-1", "destination": destination("trusted")}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(allowed["sendable"], true);
        assert_eq!(allowed["would_send"].as_array().unwrap().len(), 1);
        assert!(allowed["would_send"][0]["privacy"].is_null());
        assert_eq!(allowed["blocked"], json!([]));
    }
}
