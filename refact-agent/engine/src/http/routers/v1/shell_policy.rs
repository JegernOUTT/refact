use axum::extract::{Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::json;
use refact_tool_api::{default_catalogue, extract_command_segments, segment_command, RiskLevel};

use crate::app_state::AppState;
use crate::custom_error::ScratchError;
use crate::files_correction::get_unscoped_project_dirs;
use crate::tools::shell_gate::{
    evaluate, load_policy, read_audit, save_policy, ApprovalMode, GateContext, RiskEntryOverride,
    ShellExecutionDefaults, ShellGatePolicy, ShellLlmAuthority, ShellLlmOnFailure,
    ShellLlmValidation,
};

#[derive(Serialize)]
pub struct PolicyResponse {
    mode: ApprovalMode,
    deny: Vec<String>,
    ask: Vec<String>,
    allow: Vec<String>,
    trust_caller_confirmation: bool,
    llm_validation: ShellLlmValidation,
    execution: ShellExecutionDefaults,
    catalogue: Vec<CatalogueEntry>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct CatalogueEntry {
    id: String,
    exec: String,
    level: RiskLevel,
    reason: String,
    enabled: bool,
}

#[derive(Deserialize)]
struct PolicyRequest {
    mode: String,
    #[serde(default)]
    deny: Vec<String>,
    #[serde(default)]
    ask: Vec<String>,
    #[serde(default)]
    allow: Vec<String>,
    #[serde(default = "default_true")]
    trust_caller_confirmation: bool,
    #[serde(default)]
    llm_validation: LlmRequest,
    #[serde(default)]
    execution: ShellExecutionDefaults,
    #[serde(default)]
    catalogue: Vec<CatalogueEntry>,
}

#[derive(Deserialize)]
struct LlmRequest {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    model: String,
    #[serde(default = "default_authority")]
    authority: String,
    #[serde(default = "default_timeout")]
    timeout_secs: u64,
    #[serde(default = "default_failure")]
    on_failure: String,
    #[serde(default = "default_true")]
    cache_per_chat: bool,
}

impl Default for LlmRequest {
    fn default() -> Self {
        Self {
            enabled: false,
            model: String::new(),
            authority: default_authority(),
            timeout_secs: default_timeout(),
            on_failure: default_failure(),
            cache_per_chat: true,
        }
    }
}

#[derive(Deserialize)]
pub struct TestRequest {
    command: String,
}

#[derive(Deserialize)]
pub struct AuditQuery {
    limit: Option<usize>,
}

fn default_true() -> bool {
    true
}

fn default_authority() -> String {
    "ask_only".to_string()
}

fn default_failure() -> String {
    "pass".to_string()
}

fn default_timeout() -> u64 {
    8
}

fn bad_request(message: String) -> ScratchError {
    ScratchError::new(StatusCode::BAD_REQUEST, message)
}

fn effective_catalogue(policy: &ShellGatePolicy) -> Vec<CatalogueEntry> {
    let defaults = default_catalogue();
    defaults
        .into_iter()
        .map(|entry| {
            let entry_override = policy
                .catalogue_overrides
                .iter()
                .find(|item| item.id == entry.id);
            CatalogueEntry {
                id: entry.id,
                exec: entry.exec,
                level: entry_override
                    .and_then(|item| item.level)
                    .unwrap_or(entry.level),
                reason: entry.reason,
                enabled: entry_override.and_then(|item| item.enabled).unwrap_or(true),
            }
        })
        .collect()
}

fn response_from_policy(policy: ShellGatePolicy) -> PolicyResponse {
    PolicyResponse {
        mode: policy.mode.clone(),
        deny: policy.deny.clone(),
        ask: policy.ask.clone(),
        allow: policy.allow.clone(),
        trust_caller_confirmation: policy.trust_caller_confirmation,
        llm_validation: policy.llm_validation.clone(),
        execution: policy.execution.clone(),
        catalogue: effective_catalogue(&policy),
    }
}

pub async fn handle_v1_shell_policy_get(State(app): State<AppState>) -> axum::Json<PolicyResponse> {
    axum::Json(response_from_policy(load_policy(app.gcx.clone()).await))
}

pub async fn handle_v1_shell_policy_post(
    State(app): State<AppState>,
    body: hyper::body::Bytes,
) -> Result<axum::Json<PolicyResponse>, ScratchError> {
    let request: PolicyRequest = serde_json::from_slice(&body)
        .map_err(|error| bad_request(format!("invalid shell policy: {error}")))?;
    let mode = match request.mode.as_str() {
        "strict" => ApprovalMode::Strict,
        "balanced" => ApprovalMode::Balanced,
        "permissive" => ApprovalMode::Permissive,
        "yolo" => ApprovalMode::Yolo,
        value => return Err(bad_request(format!("unknown mode `{value}`"))),
    };
    let authority = match request.llm_validation.authority.as_str() {
        "ask_only" => ShellLlmAuthority::AskOnly,
        "ask_and_allow" => ShellLlmAuthority::AskAndAllow,
        value => return Err(bad_request(format!("unknown authority `{value}`"))),
    };
    let on_failure = match request.llm_validation.on_failure.as_str() {
        "pass" => ShellLlmOnFailure::Pass,
        "ask" => ShellLlmOnFailure::Ask,
        value => return Err(bad_request(format!("unknown on_failure `{value}`"))),
    };
    let defaults = default_catalogue();
    let mut overrides = Vec::new();
    for incoming in request.catalogue {
        let Some(default) = defaults.iter().find(|entry| entry.id == incoming.id) else {
            continue;
        };
        let level = incoming.level;
        if level != default.level || !incoming.enabled {
            overrides.push(RiskEntryOverride {
                id: incoming.id,
                level: (level != default.level).then_some(level),
                enabled: (!incoming.enabled).then_some(false),
            });
        }
    }
    let mut policy = ShellGatePolicy {
        mode,
        deny: request.deny,
        ask: request.ask,
        allow: request.allow,
        trust_caller_confirmation: request.trust_caller_confirmation,
        llm_validation: ShellLlmValidation {
            enabled: request.llm_validation.enabled,
            model: request.llm_validation.model,
            authority,
            timeout_secs: request.llm_validation.timeout_secs,
            on_failure,
            cache_per_chat: request.llm_validation.cache_per_chat,
        },
        execution: request.execution,
        catalogue_overrides: overrides,
        catalogue: Vec::new(),
    };
    policy.rebuild_catalogue();
    save_policy(app.gcx.clone(), &policy)
        .await
        .map_err(|error| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, error))?;
    Ok(axum::Json(response_from_policy(policy)))
}

pub async fn handle_v1_shell_policy_test(
    State(app): State<AppState>,
    axum::Json(request): axum::Json<TestRequest>,
) -> axum::Json<serde_json::Value> {
    let policy = load_policy(app.gcx.clone()).await;
    let workspace_roots = get_unscoped_project_dirs(app.gcx.clone()).await;
    let outcome = evaluate(
        &request.command,
        &policy,
        &GateContext {
            workspace_roots,
            needs_confirmation: false,
        },
    );
    let parsed = extract_command_segments(&request.command);
    let segments: Vec<String> = parsed.segments.iter().map(segment_command).collect();
    let decision = match outcome.decision.result {
        refact_tool_api::MatchConfirmDenyResult::PASS => "pass",
        refact_tool_api::MatchConfirmDenyResult::CONFIRMATION => "confirmation",
        refact_tool_api::MatchConfirmDenyResult::DENY => "deny",
    };
    axum::Json(json!({
        "decision": decision,
        "rule": outcome.decision.rule,
        "reason": outcome.reason,
        "risk_level": outcome.risk_level,
        "segments": segments,
    }))
}

pub async fn handle_v1_shell_policy_audit(
    State(app): State<AppState>,
    Query(query): Query<AuditQuery>,
) -> axum::Json<serde_json::Value> {
    let entries = read_audit(app.gcx.clone(), query.limit.unwrap_or(50).min(500)).await;
    axum::Json(json!({ "entries": entries }))
}
