use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub schema_version: u32,
    pub id: String,
    pub revision: u64,
    pub owner: DefinitionOwner,
    pub description: String,
    pub enabled: bool,
    pub trigger: AutomationTrigger,
    pub action: AutomationAction,
    pub deliveries: Vec<DeliveryTarget>,
    pub lifecycle: Lifecycle,
    pub policy: ExecutionPolicy,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DefinitionOwner {
    Project,
    App { app_id: String, source_id: String },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AutomationTrigger {
    Schedule { schedule: crate::types::Trigger },
    Continuous,
    ChannelEvent { channel: String, filter: Value },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AutomationAction {
    Legacy {
        action: crate::types::Action,
    },
    Collect {
        driver: refact_apps_core::SourceDriver,
        #[serde(default)]
        policy: crate::sources::SourcePolicy,
    },
    Transform {
        input_channel: String,
        output_channel: String,
        select_fields: Vec<String>,
        max_output_bytes: usize,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeliveryTarget {
    pub id: String,
    pub channel: String,
    pub credential_ref: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Lifecycle {
    OnDemand,
    Background {
        resume_after_restart: bool,
        expires_at_ms: Option<u64>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExecutionPolicy {
    pub idempotent: bool,
    pub max_pending_events: usize,
    pub max_event_bytes: usize,
}
impl Default for ExecutionPolicy {
    fn default() -> Self {
        Self {
            idempotent: false,
            max_pending_events: 1024,
            max_event_bytes: 262144,
        }
    }
}
impl Definition {
    /// Description/revision do not grant privileges. Every execution-bearing field does.
    pub fn security_digest(&self) -> Result<String, String> {
        digest_json(
            &serde_json::json!({"owner":self.owner,"enabled":self.enabled,"trigger":self.trigger,"action":self.action,"deliveries":self.deliveries,"lifecycle":self.lifecycle,"policy":self.policy}),
        )
    }
    pub fn validate(&self) -> Result<(), String> {
        identifier(&self.id)?;
        if self.schema_version != 1
            || self.revision == 0
            || self.deliveries.len() > 32
            || self.policy.max_event_bytes > 262144
            || self.policy.max_pending_events > 4096
        {
            return Err("invalid definition bounds/schema".into());
        }
        if let AutomationAction::Collect {
            policy, ..
        } = &self.action
        {
            policy.validate()?;
            if policy.owner_definition_digest.is_some() {
                return Err("owner definition digest is host-only".into());
            }
        }
        if let AutomationAction::Transform {
            select_fields,
            max_output_bytes,
            ..
        } = &self.action
        {
            if select_fields.len() > 128 || *max_output_bytes > 262144 {
                return Err("transform bounds".into());
            }
        }
        Ok(())
    }
}
pub fn digest_json(value: &Value) -> Result<String, String> {
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
pub(crate) fn identifier(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 256
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
    {
        return Err("invalid automation identifier".into());
    }
    Ok(())
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReconcileStatus {
    NeedsApproval,
    Approved,
    Unchanged,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GrantRecord {
    pub app_id: String,
    pub source_id: String,
    pub config_digest: String,
    pub endpoint_grant: Option<Value>,
    pub credential_refs: Vec<String>,
    pub granted: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SourceControl {
    pub paused: bool,
    pub demand: bool,
    pub background: bool,
    pub resume_after_restart: bool,
    pub expires_at_ms: Option<u64>,
    pub next_wake_ms: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceRuntime {
    pub config_digest: String,
    pub source_generation: String,
    pub approved: bool,
    pub control: SourceControl,
    pub grant: Option<GrantRecord>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceRecord {
    pub app_id: String,
    pub source_id: String,
    pub compiled_spec: Value,
    pub runtime: SourceRuntime,
}
#[derive(Clone, Debug)]
pub struct IngestRequest {
    pub app_id: String,
    pub source_id: String,
    pub source_generation: String,
    pub producer_id: String,
    pub producer_epoch: String,
    pub event_id: String,
    pub cursor: Option<String>,
    pub source_config_digest: String,
    pub data: Value,
    pub deliveries: Vec<String>,
    pub at_ms: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IngestOutcome {
    Inserted { event_seq: i64 },
    Duplicate,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OutboxDelivery {
    pub event_seq: i64,
    pub target: String,
    pub data: Value,
    pub attempts: u32,
    pub next_attempt_ms: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FailedDelivery {
    pub event_seq: i64,
    pub target: String,
    pub attempts: u32,
    pub definition_id: Option<String>,
    /// False once bounded terminal-event retention has discarded the payload.
    pub payload_retained: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunClaim {
    pub run_id: String,
    pub job_id: String,
    pub occurrence_key: String,
    pub config_digest: String,
    pub owner_epoch: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Succeeded,
    Failed,
    Unknown,
    NeedsReview,
}
impl RunStatus {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
            Self::NeedsReview => "needs_review",
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WakeSummary {
    pub next_wake_ms: Option<u64>,
    /// Authorized live background demand, including session-only leases.
    pub background_active: bool,
    /// Live background demand that also authorizes restarting a stopped worker.
    #[serde(default)]
    pub background_restart: bool,
    pub pending_delivery: usize,
    /// Terminal delivery failures, not successful acknowledgements or wake requests.
    #[serde(default)]
    pub failed_delivery: usize,
    pub recovery: usize,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CatalogDocument {
    pub revision: u64,
    pub definitions: Vec<Definition>,
    #[serde(default)]
    pub legacy_jobs: Vec<crate::types::Job>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DefinitionScheduleState {
    pub created_at_ms: u64,
    pub last_fired_at_ms: Option<u64>,
}
