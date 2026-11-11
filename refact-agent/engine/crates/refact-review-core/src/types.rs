use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct StackManifest {
    pub schema_version: u32,
    pub id: String,
    pub owner_chat_id: String,
    pub revision: u64,
    pub title: String,
    pub repository: String,
    pub source: StackSource,
    pub root_base_ref: String,
    pub root_base_oid: Option<String>,
    pub scope_choice: StackScopeChoice,
    pub members: Vec<StackMember>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct StackSource {
    pub kind: String,
    pub reference: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum StackScope {
    Single,
    Whole,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct StackScopeChoice {
    pub scope: StackScope,
    pub selected_member_id: String,
    pub confirmed: bool,
    pub explicit_request: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PrIdentity {
    pub host: String,
    pub repository: String,
    pub number: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum StackMemberState {
    Ready,
    Missing,
    Unauthorized,
    Ambiguous,
    Removed,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct StackMember {
    pub id: String,
    pub branch_ref: String,
    pub pr_identity: Option<PrIdentity>,
    pub parent_member_id: Option<String>,
    pub declared_base_ref: String,
    pub resolved_base_oid: Option<String>,
    pub head_oid: Option<String>,
    pub review_id: Option<String>,
    pub state: StackMemberState,
    pub reason: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct StackMemberReviewSnapshot {
    pub member_id: String,
    pub review: Review,
    pub object_history: Vec<ObjectRevision>,
    pub delivery_statuses: Vec<ReviewDelivery>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct HandoffSnapshot {
    pub review: Review,
    pub object_history: Vec<ObjectRevision>,
    pub delivery_statuses: Vec<ReviewDelivery>,
    pub stack_manifest: Option<StackManifest>,
    pub stack_history: Vec<StackManifest>,
    pub stack_member_reviews: Vec<StackMemberReviewSnapshot>,
}

pub const SCHEMA_VERSION: u32 = 1;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Review {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub privacy: Vec<refact_privacy::FileRecord>,
    pub schema_version: u32,
    pub id: String,
    pub owner_chat_id: String,
    pub revision: u64,
    pub current_candidate_id: Option<String>,
    pub title: String,
    pub origin: Origin,
    pub target_reference: String,
    pub candidates: Vec<Candidate>,
    pub tour: Vec<TourStep>,
    pub annotations: Vec<Annotation>,
    pub threads: Vec<Thread>,
    pub evidence: Vec<Evidence>,
    pub runs: Vec<Run>,
    pub materials: Vec<Material>,
    pub activity: Vec<Activity>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remote_links: Vec<RemoteLink>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RemoteLinkKind {
    Contribution,
    Annotation,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RemoteLinkState {
    Imported,
    Requested,
    Published,
}
/// Ties one local comment or finding to its counterpart on the review source.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RemoteLink {
    pub kind: RemoteLinkKind,
    pub local_id: String,
    pub state: RemoteLinkState,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    pub at_ms: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Local,
    Imported,
    Agent,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Candidate {
    pub id: String,
    pub base_oid: String,
    pub head_oid: String,
    pub content_digest: String,
    #[serde(default)]
    pub snapshot_scope: String,
    pub created_at_ms: i64,
    pub repository: String,
    pub origin: Origin,
    pub policy: CapturePolicy,
    pub files: Vec<CandidateFile>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct CapturePolicy {
    pub staged: bool,
    pub working: bool,
    pub untracked: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CandidateFile {
    pub path: String,
    pub base: Option<BlobRef>,
    pub head: Option<BlobRef>,
    pub base_mode: Option<u32>,
    pub head_mode: Option<u32>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BlobRef {
    pub digest: String,
    pub size: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Base,
    Head,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AnchorPosition {
    pub candidate_id: String,
    pub path: String,
    pub side: Side,
    pub start_line: u32,
    pub end_line: u32,
    pub content_hash: String,
    pub context_before: Vec<String>,
    pub context_after: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Anchor {
    pub position: AnchorPosition,
    pub status: AnchorStatus,
    pub stalled_reason: Option<String>,
    pub history: Vec<AnchorPosition>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AnchorStatus {
    Current,
    Stalled,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RichBlock {
    pub id: String,
    pub revision: u64,
    pub content: RichBlockContent,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RichBlockContent {
    Markdown {
        body: String,
    },
    Html {
        html: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        static_fallback: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        anchors: Vec<Anchor>,
    },
    Code {
        anchor: Anchor,
    },
    Diff {
        candidate_id: String,
        path: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        anchors: Vec<Anchor>,
    },
    Sources {
        evidence_ids: Vec<String>,
        relation: SourceRelation,
    },
    Evidence {
        evidence_ids: Vec<String>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SourceRelation {
    Consulted,
    Supports,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TourDiscussionTarget {
    pub candidate_id: String,
    pub step_id: String,
    pub version: u64,
    pub block_id: Option<String>,
    pub block_revision: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TourVersion {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<RichBlock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_id: Option<String>,
    pub version: u64,
    pub title: String,
    pub body: String,
    pub anchors: Vec<Anchor>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TourStep {
    pub id: String,
    pub current: TourVersion,
    pub history: Vec<TourVersion>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationKind {
    Explanation,
    Highlight,
    Finding,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Unknown,
    Low,
    Medium,
    High,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Assessment {
    Unassessed,
    Supported,
    Disputed,
    Refuted,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    Open,
    RepairProposed,
    Fixed,
    RiskAccepted,
    Dismissed,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Applicability {
    Current,
    NeedsRevalidation,
    Historical,
    NoLongerApplicable,
    Unknown,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Annotation {
    pub id: String,
    pub kind: AnnotationKind,
    pub title: String,
    pub body: String,
    pub anchors: Vec<Anchor>,
    pub severity: Severity,
    pub confidence: Confidence,
    pub assessment: Assessment,
    pub disposition: Disposition,
    pub applicability: Applicability,
    pub evidence_ids: Vec<String>,
    pub run_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Actor {
    pub kind: ActorKind,
    pub id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    User,
    Agent,
    System,
    /// A person on the review source; never the local user and never an instruction.
    External,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Contribution {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<RichBlock>,
    pub id: String,
    pub thread_id: String,
    pub actor: Actor,
    pub body: String,
    pub created_at_ms: i64,
    pub chat_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<ContributionContext>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ContributionContext {
    pub candidate_id: Option<String>,
    pub run_id: Option<String>,
    pub applicability: Applicability,
    pub privacy: Vec<refact_privacy::FileRecord>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Thread {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<TourDiscussionTarget>,
    pub id: String,
    pub anchors: Vec<Anchor>,
    pub annotation_id: Option<String>,
    pub resolved: bool,
    pub contributions: Vec<Contribution>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MaterialKind {
    Imported,
    Investigation,
    Repair,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Material {
    pub id: String,
    pub kind: MaterialKind,
    pub candidate_id: String,
    pub agent_id: Option<String>,
    pub description: String,
    pub content_digest: String,
    pub input_digests: std::collections::BTreeMap<String, String>,
    pub environment: Option<String>,
    pub worktree_id: Option<String>,
    pub worktree_root: Option<String>,
    pub artifacts: Vec<BlobRef>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EvidenceSource {
    Source {
        path: String,
        anchor: Option<Anchor>,
    },
    External {
        url: String,
    },
    Executed {
        command: Vec<String>,
        workdir: String,
        exit_code: Option<i32>,
        stdout: Option<BlobRef>,
        stderr: Option<BlobRef>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Evidence {
    pub id: String,
    pub candidate_id: String,
    pub material_id: Option<String>,
    pub run_id: Option<String>,
    pub source: EvidenceSource,
    pub summary: String,
    pub applicability: Applicability,
    pub created_at_ms: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Queued,
    Running,
    Completed,
    Partial,
    Failed,
    CancelRequested,
    Cancelled,
    Interrupted,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Run {
    pub id: String,
    pub candidate_id: String,
    pub agent_id: String,
    pub purpose: String,
    pub model: Option<String>,
    pub domain: Option<String>,
    pub status: RunStatus,
    pub started_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Activity {
    pub id: String,
    pub at_ms: i64,
    pub actor: Actor,
    pub action: String,
    pub candidate_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Operation {
    /// Runtime-owned atomic context activation; never accepted by generic client mutation APIs.
    ActivateSnapshot {
        candidate: Candidate,
        imported: Material,
        investigation: Material,
        target_reference: String,
    },

    RequestRunCancel {
        run_id: String,
    },
    Rename {
        title: String,
    },
    SetTargetReference {
        target_reference: String,
    },
    CreateThread {
        thread: Thread,
        contribution: Contribution,
    },
    StageCandidate {
        candidate: Candidate,
    },
    ActivateCandidate {
        candidate_id: String,
    },
    PutTourStep {
        step: TourStep,
    },
    PutAnnotation {
        annotation: Annotation,
    },
    PutThread {
        thread: Thread,
    },
    AddContribution {
        contribution: Contribution,
    },
    AddEvidence {
        evidence: Evidence,
    },
    PutRun {
        run: Run,
    },
    AddMaterial {
        material: Material,
    },
    PutRemoteLink {
        link: RemoteLink,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Mutation {
    pub expected_revision: u64,
    pub request_key: String,
    pub actor: Actor,
    pub at_ms: i64,
    pub operation: Operation,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaptureRequest {
    pub base_oid: String,
    pub head_oid: String,
    pub policy: CapturePolicy,
    pub origin: Origin,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileView {
    pub path: String,
    pub base: Option<Vec<u8>>,
    pub head: Option<Vec<u8>>,
    pub diff: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ReviewDelivery {
    pub id: String,
    pub review_id: String,
    pub contribution_id: String,
    pub owner_chat_id: String,
    pub delivered: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum HistoricalObject {
    Annotation(Annotation),
    Thread(Thread),
    TourStep(TourStep),
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ObjectRevision {
    pub review_id: String,
    pub revision: u64,
    pub actor: Actor,
    pub at_ms: i64,
    pub object: HistoricalObject,
}
