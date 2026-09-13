use serde::{de::Error as _, Deserialize, Deserializer, Serialize};

pub const OPERATOR_SCHEMA_VERSION_V1: u16 = 1;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ProjectIdentity {
    pub project_id: String,
    pub canonical_root: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct WorkerIdentity {
    pub worker_id: String,
    pub engine_instance_id: String,
    #[serde(deserialize_with = "deserialize_safe_u64")]
    pub epoch: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct NativeSourceIdentity {
    pub source_kind: NativeSourceKind,
    pub source_id: String,
    pub source_version: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeSourceKind {
    Chat,
    Goal,
    Task,
    Tool,
    Question,
    Approval,
    File,
    Commit,
    Check,
    External,
    Workflow,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ResourceRef {
    pub resource_kind: String,
    pub resource_id: String,
    pub version: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemState {
    NeedsAttention,
    Working,
    Delivered,
    Watching,
    Blocked,
    Cancelled,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct WorkItem {
    pub schema_version: u16,
    pub id: String,
    pub project: ProjectIdentity,
    pub subject: ResourceRef,
    pub kind: String,
    pub title: String,
    pub state: WorkItemState,
    pub native_sources: Vec<NativeSourceIdentity>,
    pub current_artifact_id: Option<String>,
    pub current_artifact_revision: Option<u32>,
    pub last_material_change_at: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    NoChange,
    Partial,
    Inconclusive,
    Blocked,
    Failed,
    Cancelled,
    Completed,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct EvidenceRef {
    pub evidence_id: String,
    pub source: NativeSourceIdentity,
    pub locator: String,
    pub observed_at: String,
    pub digest: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectState {
    Applied,
    Skipped,
    Failed,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ResourceEffect {
    pub resource: ResourceRef,
    pub state: EffectState,
    pub evidence_refs: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct RunOutcome {
    pub schema_version: u16,
    pub run_id: String,
    pub work_id: String,
    pub parent_run_id: Option<String>,
    pub project: ProjectIdentity,
    pub worker: WorkerIdentity,
    pub origin: String,
    pub workflow: Option<String>,
    pub input_fingerprint: String,
    pub coverage: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub status: RunStatus,
    pub evidence: Vec<EvidenceRef>,
    pub effects: Vec<ResourceEffect>,
    pub failure_class: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ArtifactRevision {
    pub schema_version: u16,
    pub artifact_id: String,
    pub revision: u32,
    pub project: ProjectIdentity,
    pub source_run_ids: Vec<String>,
    pub title: String,
    pub conclusion: String,
    pub delta: String,
    pub body: String,
    pub uncertainty: Option<String>,
    pub proposed_action_ids: Vec<String>,
    pub evidence: Vec<EvidenceRef>,
    pub provenance: ArtifactProvenance,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ArtifactProvenance {
    pub source_run_ids: Vec<String>,
    pub evidence_refs: Vec<String>,
    pub contains_untrusted_source: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct BudgetLimit {
    #[serde(default, deserialize_with = "deserialize_safe_option_u64")]
    pub max_tokens: Option<u64>,
    pub max_turns: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_safe_option_u64")]
    pub max_cost_micros: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_safe_option_u64")]
    pub max_duration_seconds: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ActionCapabilities {
    pub can_preview: bool,
    pub can_execute: bool,
    pub can_verify: bool,
    pub can_recover: bool,
    pub can_cancel: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ActionSpec {
    pub schema_version: u16,
    pub action_id: String,
    pub kind: String,
    pub adapter: String,
    pub project: ProjectIdentity,
    pub resource: ResourceRef,
    pub expected_version: String,
    pub parameter_ref: Option<String>,
    pub availability: ActionAvailability,
    pub confirmation: ConfirmationClass,
    pub authority_class: AuthorityClass,
    pub capabilities: ActionCapabilities,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionAvailability {
    Available,
    Unavailable,
    Stale,
    Unsupported,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationClass {
    None,
    UserConfirmationRequired,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityClass {
    Navigation,
    Advisory,
    ReadOnly,
    Mutating,
    DataEgress,
    CredentialChange,
    Publication,
    Deployment,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionExecutionState {
    Prepared,
    AwaitingConfirmation,
    Claimed,
    Running,
    Reconciling,
    OutcomeUnknown,
    Completed,
    Partial,
    Blocked,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct RecoveryRef {
    pub recovery_id: String,
    pub action_id: String,
    pub expected_post_version: String,
    pub available: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ActionExecution {
    pub schema_version: u16,
    pub execution_id: String,
    pub project: ProjectIdentity,
    pub worker: WorkerIdentity,
    pub idempotency_key: String,
    pub action_id: String,
    pub spec_hash: String,
    pub requester: NativeSourceIdentity,
    pub decision: Option<DecisionReference>,
    pub status: ActionExecutionState,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub effects: Vec<ResourceEffect>,
    pub verification_evidence: Vec<String>,
    pub recovery: Option<RecoveryRef>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct DecisionReference {
    pub project: ProjectIdentity,
    pub worker: WorkerIdentity,
    pub native: NativeSourceIdentity,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct DecisionOption {
    pub option_id: String,
    pub label: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionState {
    Pending,
    Acknowledged,
    Resolved,
    Expired,
    Superseded,
    Unsupported,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct DecisionProjection {
    pub schema_version: u16,
    pub decision: DecisionReference,
    pub title: String,
    pub impact: String,
    pub options: Vec<DecisionOption>,
    pub recommended_option_id: Option<String>,
    pub status: DecisionState,
    pub response_authority: AuthorityClass,
    pub expires_at: Option<String>,
    pub revalidation_required: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Mandate {
    pub schema_version: u16,
    pub mandate_id: String,
    pub project: ProjectIdentity,
    pub owner: NativeSourceIdentity,
    #[serde(deserialize_with = "deserialize_safe_u64")]
    pub owner_epoch: u64,
    pub resource_scopes: Vec<ResourceRef>,
    pub allowed_operations: Vec<String>,
    pub allowed_authority_classes: Vec<AuthorityClass>,
    pub model_constraints: Vec<String>,
    pub budget_limit: Option<BudgetLimit>,
    pub expires_at: Option<String>,
    pub revoked_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
    Pending,
    Delivered,
    Seen,
    Read,
    Snoozed,
    Muted,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Delivery {
    pub schema_version: u16,
    pub delivery_id: String,
    pub project: ProjectIdentity,
    pub work_id: String,
    pub artifact_id: String,
    pub artifact_revision: u32,
    pub surface: String,
    pub client_id: String,
    pub state: DeliveryState,
    pub delivered_at: Option<String>,
    pub seen_at: Option<String>,
    pub attention_category: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ReadState {
    pub schema_version: u16,
    pub project: ProjectIdentity,
    pub work_id: String,
    pub artifact_id: String,
    pub last_read_revision: u32,
    pub read_at: String,
    pub snoozed_until: Option<String>,
    pub muted: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct WorkSnooze {
    pub schema_version: u16,
    pub project: ProjectIdentity,
    pub work_id: String,
    pub artifact_id: String,
    pub artifact_revision: u32,
    pub until: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ImprovementCandidate {
    pub schema_version: u16,
    pub candidate_id: String,
    pub project: ProjectIdentity,
    pub target: String,
    pub worktree_ref: Option<String>,
    pub config_draft_ref: Option<String>,
    pub patch_digest: Option<String>,
    pub build_identity: Option<String>,
    pub evaluation_spec_version: String,
    pub proof_refs: Vec<String>,
    pub activation_decision: Option<DecisionReference>,
    pub previous_known_good_identity: Option<String>,
}

pub trait OperatorContract: Serialize {
    fn schema_version(&self) -> u16;
    fn identity(&self) -> &str;

    fn validate(&self) -> Result<(), OperatorContractError> {
        if self.schema_version() != OPERATOR_SCHEMA_VERSION_V1 {
            return Err(OperatorContractError::UnsupportedSchemaVersion(
                self.schema_version(),
            ));
        }
        if self.identity().trim().is_empty() {
            return Err(OperatorContractError::InvalidIdentity);
        }
        validate_contract_value(
            &serde_json::to_value(self).expect("operator contracts serialize"),
        )?;
        Ok(())
    }
}

macro_rules! impl_operator_contract {
    ($type:ty, $id:ident) => {
        impl OperatorContract for $type {
            fn schema_version(&self) -> u16 {
                self.schema_version
            }
            fn identity(&self) -> &str {
                &self.$id
            }
        }
    };
}

impl_operator_contract!(WorkItem, id);
impl_operator_contract!(RunOutcome, run_id);
impl_operator_contract!(ArtifactRevision, artifact_id);
impl_operator_contract!(ActionSpec, action_id);
impl_operator_contract!(ActionExecution, execution_id);
impl_operator_contract!(Mandate, mandate_id);
impl_operator_contract!(Delivery, delivery_id);
impl_operator_contract!(ReadState, work_id);
impl_operator_contract!(WorkSnooze, work_id);
impl_operator_contract!(ImprovementCandidate, candidate_id);

impl OperatorContract for DecisionProjection {
    fn schema_version(&self) -> u16 {
        self.schema_version
    }

    fn identity(&self) -> &str {
        &self.decision.native.source_id
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperatorContractError {
    UnsupportedSchemaVersion(u16),
    InvalidIdentity,
}

impl std::fmt::Display for OperatorContractError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedSchemaVersion(version) => {
                write!(formatter, "unsupported operator schema version: {version}")
            }
            Self::InvalidIdentity => {
                write!(formatter, "operator contract identity must not be empty")
            }
        }
    }
}

impl std::error::Error for OperatorContractError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsupportedOperatorSchemaVersion(pub u16);

impl std::fmt::Display for UnsupportedOperatorSchemaVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "unsupported operator schema version: {}", self.0)
    }
}

impl std::error::Error for UnsupportedOperatorSchemaVersion {}

const MAX_SAFE_JSON_INTEGER: u64 = 9_007_199_254_740_991;

fn validate_contract_value(value: &serde_json::Value) -> Result<(), OperatorContractError> {
    const IDENTITY_FIELDS: &[&str] = &[
        "project_id",
        "canonical_root",
        "worker_id",
        "engine_instance_id",
        "source_id",
        "source_version",
        "resource_kind",
        "resource_id",
        "version",
        "work_id",
        "artifact_id",
        "action_id",
        "run_id",
        "execution_id",
        "mandate_id",
        "delivery_id",
        "candidate_id",
        "evidence_id",
        "recovery_id",
        "option_id",
    ];
    match value {
        serde_json::Value::Object(values) => {
            for (key, value) in values {
                if IDENTITY_FIELDS.contains(&key.as_str())
                    && matches!(value, serde_json::Value::String(text) if text.trim().is_empty())
                {
                    return Err(OperatorContractError::InvalidIdentity);
                }
                validate_contract_value(value)?;
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                validate_contract_value(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn deserialize_safe_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = u64::deserialize(deserializer)?;
    if value > MAX_SAFE_JSON_INTEGER {
        return Err(D::Error::custom(
            "integer exceeds JavaScript safe integer range",
        ));
    }
    Ok(value)
}

fn deserialize_safe_option_u64<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<u64>::deserialize(deserializer)?.map_or(Ok(None), |value| {
        if value > MAX_SAFE_JSON_INTEGER {
            Err(D::Error::custom(
                "integer exceeds JavaScript safe integer range",
            ))
        } else {
            Ok(Some(value))
        }
    })
}

pub fn parse_operator_contract<T>(input: &str) -> Result<T, OperatorContractParseError>
where
    T: DeserializeOwnedOperatorContract,
{
    let contract: T =
        serde_json::from_str(input).map_err(OperatorContractParseError::Deserialize)?;
    contract
        .validate()
        .map_err(OperatorContractParseError::Validate)?;
    Ok(contract)
}

pub trait DeserializeOwnedOperatorContract: OperatorContract + serde::de::DeserializeOwned {}

impl<T> DeserializeOwnedOperatorContract for T where
    T: OperatorContract + serde::de::DeserializeOwned
{
}

#[derive(Debug)]
pub enum OperatorContractParseError {
    Deserialize(serde_json::Error),
    Validate(OperatorContractError),
}

impl std::fmt::Display for OperatorContractParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Deserialize(error) => error.fmt(formatter),
            Self::Validate(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for OperatorContractParseError {}

#[cfg(test)]
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
struct OperatorContractFixture {
    schema_version: u16,
    work_item: WorkItem,
    run_outcome: RunOutcome,
    artifact_revision: ArtifactRevision,
    action_spec: ActionSpec,
    action_execution: ActionExecution,
    decision_projection: DecisionProjection,
    mandate: Mandate,
    delivery: Delivery,
    read_state: ReadState,
    improvement_candidate: ImprovementCandidate,
}

#[cfg(test)]
impl OperatorContractFixture {
    fn validate_schema_version(&self) -> Result<(), OperatorContractError> {
        if self.schema_version != OPERATOR_SCHEMA_VERSION_V1 {
            return Err(OperatorContractError::UnsupportedSchemaVersion(
                self.schema_version,
            ));
        }
        self.work_item.validate()?;
        self.run_outcome.validate()?;
        self.artifact_revision.validate()?;
        self.action_spec.validate()?;
        self.action_execution.validate()?;
        self.decision_projection.validate()?;
        self.mandate.validate()?;
        self.delivery.validate()?;
        self.read_state.validate()?;
        self.improvement_candidate.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_v1_fixture_deserializes_and_validates() {
        let fixture: OperatorContractFixture =
            serde_json::from_str(include_str!("../tests/fixtures/operator_contract_v1.json"))
                .unwrap();
        assert_eq!(fixture.schema_version, OPERATOR_SCHEMA_VERSION_V1);
        fixture.validate_schema_version().unwrap();
        assert_eq!(fixture.run_outcome.status, RunStatus::Inconclusive);
    }

    #[test]
    fn golden_entities_round_trip_through_production_parser() {
        let fixture: OperatorContractFixture =
            serde_json::from_str(include_str!("../tests/fixtures/operator_contract_v1.json"))
                .unwrap();
        macro_rules! round_trip {
            ($entity:expr, $type:ty) => {
                let wire = serde_json::to_string(&$entity).unwrap();
                assert_eq!(parse_operator_contract::<$type>(&wire).unwrap(), $entity);
            };
        }
        round_trip!(fixture.work_item, WorkItem);
        round_trip!(fixture.run_outcome, RunOutcome);
        round_trip!(fixture.artifact_revision, ArtifactRevision);
        round_trip!(fixture.action_spec, ActionSpec);
        round_trip!(fixture.action_execution, ActionExecution);
        round_trip!(fixture.decision_projection, DecisionProjection);
        round_trip!(fixture.mandate, Mandate);
        round_trip!(fixture.delivery, Delivery);
        round_trip!(fixture.read_state, ReadState);
        round_trip!(fixture.improvement_candidate, ImprovementCandidate);
    }

    #[test]
    fn production_parser_rejects_invalid_contract_values() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/operator_contract_v1.json"))
                .unwrap();
        let mut work_item = fixture["work_item"].clone();
        work_item["schema_version"] = serde_json::json!(2);
        assert!(matches!(
            parse_operator_contract::<WorkItem>(&work_item.to_string()),
            Err(OperatorContractParseError::Validate(
                OperatorContractError::UnsupportedSchemaVersion(2)
            ))
        ));

        work_item = fixture["work_item"].clone();
        work_item["id"] = serde_json::json!(" ");
        assert!(matches!(
            parse_operator_contract::<WorkItem>(&work_item.to_string()),
            Err(OperatorContractParseError::Validate(
                OperatorContractError::InvalidIdentity
            ))
        ));

        work_item = fixture["work_item"].clone();
        work_item["project"]["project_id"] = serde_json::json!(" ");
        assert!(matches!(
            parse_operator_contract::<WorkItem>(&work_item.to_string()),
            Err(OperatorContractParseError::Validate(
                OperatorContractError::InvalidIdentity
            ))
        ));

        let mut artifact_revision = fixture["artifact_revision"].clone();
        artifact_revision["revision"] = serde_json::json!(u64::from(u32::MAX) + 1);
        assert!(matches!(
            parse_operator_contract::<ArtifactRevision>(&artifact_revision.to_string()),
            Err(OperatorContractParseError::Deserialize(_))
        ));

        work_item = fixture["work_item"].clone();
        work_item["unknown"] = serde_json::json!(true);
        assert!(matches!(
            parse_operator_contract::<WorkItem>(&work_item.to_string()),
            Err(OperatorContractParseError::Deserialize(_))
        ));

        work_item = fixture["work_item"].clone();
        work_item
            .as_object_mut()
            .unwrap()
            .remove("current_artifact_id");
        assert!(parse_operator_contract::<WorkItem>(&work_item.to_string()).is_ok());

        work_item = fixture["work_item"].clone();
        work_item["state"] = serde_json::json!("not_a_state");
        assert!(matches!(
            parse_operator_contract::<WorkItem>(&work_item.to_string()),
            Err(OperatorContractParseError::Deserialize(_))
        ));

        let mut run_outcome = fixture["run_outcome"].clone();
        run_outcome["worker"]["epoch"] = serde_json::json!(MAX_SAFE_JSON_INTEGER + 1);
        assert!(matches!(
            parse_operator_contract::<RunOutcome>(&run_outcome.to_string()),
            Err(OperatorContractParseError::Deserialize(_))
        ));

        let mut mandate = fixture["mandate"].clone();
        mandate["budget_limit"]["max_tokens"] = serde_json::json!(MAX_SAFE_JSON_INTEGER + 1);
        assert!(matches!(
            parse_operator_contract::<Mandate>(&mandate.to_string()),
            Err(OperatorContractParseError::Deserialize(_))
        ));
    }
}
