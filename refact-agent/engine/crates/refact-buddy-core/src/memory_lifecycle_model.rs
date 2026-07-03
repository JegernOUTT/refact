use std::collections::HashMap;

use chrono::{Local, NaiveDate};
use refact_core::string_utils::redact_sensitive;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const HIGH_CONFIDENCE_APPROVAL_THRESHOLD: f32 = 0.85;
pub const PAYLOAD_CONTENT_MAX_CHARS: usize = 12000;
pub const MEMORY_OP_EVIDENCE_MAX_CHARS: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySource {
    Buddy,
    Trajectory,
    Git,
    Manual,
    BehaviorLearner,
    MemoryGarden,
    KnowledgeConflictResolver,
}

impl Default for MemorySource {
    fn default() -> Self {
        Self::Buddy
    }
}

impl MemorySource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Buddy => "buddy",
            Self::Trajectory => "trajectory",
            Self::Git => "git",
            Self::Manual => "manual",
            Self::BehaviorLearner => "behavior_learner",
            Self::MemoryGarden => "memory_garden",
            Self::KnowledgeConflictResolver => "knowledge_conflict_resolver",
        }
    }

    pub fn is_autonomous(self) -> bool {
        !matches!(self, Self::Manual)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryOpType {
    CreateMemory,
    UpdateMemory,
    Retag,
    RepairLinks,
    Refresh,
    ArchiveCandidate,
    Archive,
    MergeArchive,
    DeleteCandidate,
    PromoteDigest,
    MarkReviewNeeded,
    MarkStale,
}

impl Default for MemoryOpType {
    fn default() -> Self {
        Self::CreateMemory
    }
}

impl MemoryOpType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CreateMemory => "create_memory",
            Self::UpdateMemory => "update_memory",
            Self::Retag => "retag",
            Self::RepairLinks => "repair_links",
            Self::Refresh => "refresh",
            Self::ArchiveCandidate => "archive_candidate",
            Self::Archive => "archive",
            Self::MergeArchive => "merge_archive",
            Self::DeleteCandidate => "delete_candidate",
            Self::PromoteDigest => "promote_digest",
            Self::MarkReviewNeeded => "mark_review_needed",
            Self::MarkStale => "mark_stale",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryOpStatus {
    Pending,
    Approved,
    Applied,
    Rejected,
    Failed,
    Skipped,
}

impl Default for MemoryOpStatus {
    fn default() -> Self {
        Self::Pending
    }
}

impl MemoryOpStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Applied => "applied",
            Self::Rejected => "rejected",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryCandidateStatus {
    Proposed,
    Approved,
    Promoted,
    Rejected,
    Skipped,
}

impl Default for MemoryCandidateStatus {
    fn default() -> Self {
        Self::Proposed
    }
}

impl MemoryCandidateStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Approved => "approved",
            Self::Promoted => "promoted",
            Self::Rejected => "rejected",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryCreatePayload {
    pub title: Option<String>,
    pub content: String,
    pub tags: Vec<String>,
    pub kind: String,
    pub status: Option<String>,
    pub filenames: Vec<String>,
    pub related_files: Vec<String>,
    pub links: Vec<String>,
    pub source_commit: Option<String>,
    pub review_after: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryLifecyclePayload {
    pub title: Option<String>,
    pub content: Option<String>,
    pub tags: Option<Vec<String>>,
    pub kind: Option<String>,
    pub filenames: Option<Vec<String>>,
    pub related_files: Option<Vec<String>>,
    pub links: Option<Vec<String>>,
    pub review_after: Option<String>,
    pub source_id: Option<String>,
    pub source_message_range: Option<String>,
    pub source_content_hash: Option<String>,
    pub superseded_by: Option<String>,
    pub superseded_paths: Vec<String>,
    pub canonical: Option<MemoryCreatePayload>,
}

impl MemoryCreatePayload {
    pub fn normalized(mut self) -> Self {
        self.title = normalize_optional_text(self.title.as_deref());
        self.content = redact_and_cap_payload_text(&self.content, PAYLOAD_CONTENT_MAX_CHARS);
        self.tags = normalize_tags(&self.tags);
        self.kind = normalize_kind(&self.kind);
        self.status = self
            .status
            .as_deref()
            .map(|status| normalize_memory_status(Some(status)));
        self.filenames = normalize_paths(&self.filenames);
        self.related_files = normalize_paths(&self.related_files);
        self.links = normalize_strings(&self.links);
        self.source_commit = normalize_optional_string(self.source_commit.as_deref());
        self.review_after = normalize_review_after(self.review_after.as_deref());
        self
    }
}

impl MemoryLifecyclePayload {
    pub fn normalized(mut self) -> Self {
        self.title = normalize_optional_text(self.title.as_deref());
        self.content = self
            .content
            .as_deref()
            .map(|content| redact_and_cap_payload_text(content, PAYLOAD_CONTENT_MAX_CHARS))
            .filter(|content| !content.is_empty());
        self.tags = self.tags.map(|tags| normalize_tags(&tags));
        self.kind = self.kind.as_deref().map(normalize_kind);
        self.filenames = self.filenames.map(|paths| normalize_paths(&paths));
        self.related_files = self.related_files.map(|paths| normalize_paths(&paths));
        self.links = self.links.map(|links| normalize_strings(&links));
        self.review_after = normalize_review_after(self.review_after.as_deref());
        self.source_id = normalize_optional_string(self.source_id.as_deref());
        self.source_message_range = normalize_optional_string(self.source_message_range.as_deref());
        self.source_content_hash = normalize_optional_string(self.source_content_hash.as_deref());
        self.superseded_by = normalize_optional_string(self.superseded_by.as_deref());
        self.superseded_paths = normalize_paths(&self.superseded_paths);
        self.canonical = self.canonical.map(|canonical| canonical.normalized());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryLifecycleOp {
    pub op_id: String,
    pub source: MemorySource,
    pub op_type: MemoryOpType,
    pub payload: MemoryLifecyclePayload,
    pub target_paths: Vec<String>,
    pub evidence: String,
    pub confidence: f32,
    pub requires_approval: bool,
    pub status: MemoryOpStatus,
    pub created_at: String,
    pub applied_at: Option<String>,
    pub idempotency_key: String,
    pub error: Option<String>,
}

impl Default for MemoryLifecycleOp {
    fn default() -> Self {
        Self {
            op_id: String::new(),
            source: MemorySource::default(),
            op_type: MemoryOpType::default(),
            payload: MemoryLifecyclePayload::default(),
            target_paths: Vec::new(),
            evidence: String::new(),
            confidence: 0.0,
            requires_approval: true,
            status: MemoryOpStatus::default(),
            created_at: String::new(),
            applied_at: None,
            idempotency_key: String::new(),
            error: None,
        }
    }
}

impl MemoryLifecycleOp {
    pub fn pending(
        op_id: impl Into<String>,
        source: MemorySource,
        op_type: MemoryOpType,
        target_paths: Vec<String>,
        evidence: impl Into<String>,
        confidence: f32,
        created_at: impl Into<String>,
    ) -> Self {
        let target_paths = normalize_paths(&target_paths);
        let evidence = normalize_evidence_text(&evidence.into());
        let idempotency_key = compute_idempotency_key(&MemoryOpIdempotencyInput {
            source,
            op_type,
            target_paths: target_paths.clone(),
            tags: Vec::new(),
            kind: None,
            source_id: None,
            title: None,
            content: None,
            evidence: Some(evidence.clone()),
        });
        Self {
            op_id: op_id.into(),
            source,
            op_type,
            payload: MemoryLifecyclePayload::default(),
            target_paths,
            evidence,
            confidence,
            requires_approval: default_requires_approval(op_type, confidence),
            status: MemoryOpStatus::Pending,
            created_at: created_at.into(),
            applied_at: None,
            idempotency_key,
            error: None,
        }
    }

    pub fn normalized(mut self) -> Self {
        self.op_id = self.op_id.trim().to_string();
        self.created_at = self.created_at.trim().to_string();
        self.idempotency_key = self.idempotency_key.trim().to_string();
        self.target_paths = normalize_paths(&self.target_paths);
        self.payload = self.payload.normalized();
        self.evidence = normalize_evidence_text(&self.evidence);
        self.applied_at = normalize_optional_string(self.applied_at.as_deref());
        self.error = normalize_optional_string(self.error.as_deref());
        if self.idempotency_key.trim().is_empty() {
            self.idempotency_key = compute_idempotency_key(&MemoryOpIdempotencyInput {
                source: self.source,
                op_type: self.op_type,
                target_paths: self.target_paths.clone(),
                tags: Vec::new(),
                kind: None,
                source_id: None,
                title: None,
                content: None,
                evidence: Some(self.evidence.clone()),
            });
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MemoryOpsRecord {
    Op { op: MemoryLifecycleOp },
}

impl MemoryOpsRecord {
    pub fn into_op(self) -> MemoryLifecycleOp {
        match self {
            Self::Op { op } => op,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryOpsState {
    pub ops: Vec<MemoryLifecycleOp>,
    pub total_records: u32,
    pub malformed_lines: u32,
    pub pending_count: u32,
    pub approved_count: u32,
    pub applied_count: u32,
    pub rejected_count: u32,
    pub failed_count: u32,
    pub skipped_count: u32,
}

impl MemoryOpsState {
    pub fn from_records(records: impl IntoIterator<Item = MemoryOpsRecord>) -> Self {
        Self::from_records_with_malformed(records, 0)
    }

    pub fn from_records_with_malformed(
        records: impl IntoIterator<Item = MemoryOpsRecord>,
        malformed_lines: u32,
    ) -> Self {
        let mut ops: Vec<MemoryLifecycleOp> = Vec::new();
        let mut op_id_index: HashMap<String, usize> = HashMap::new();
        let mut idempotency_index: HashMap<String, usize> = HashMap::new();
        let mut total_records = 0u32;

        for record in records {
            total_records = total_records.saturating_add(1);
            let incoming = record.into_op();
            let existing_index = matching_op_index(&incoming, &op_id_index, &idempotency_index);
            let op = incoming.normalized();

            match existing_index {
                Some(index) => {
                    if memory_op_duplicate_should_replace(ops[index].status, op.status) {
                        let old = ops[index].clone();
                        remove_indexed_key(&mut op_id_index, &old.op_id, index);
                        remove_indexed_key(&mut idempotency_index, &old.idempotency_key, index);
                        ops[index] = op.clone();
                        insert_op_indexes(&op, index, &mut op_id_index, &mut idempotency_index);
                    }
                }
                None => {
                    let index = ops.len();
                    insert_op_indexes(&op, index, &mut op_id_index, &mut idempotency_index);
                    ops.push(op);
                }
            }
        }

        let mut state = Self {
            ops,
            total_records,
            malformed_lines,
            ..Self::default()
        };
        state.recount();
        state
    }

    pub fn canonical_records(&self) -> Vec<MemoryOpsRecord> {
        self.ops
            .iter()
            .cloned()
            .map(|op| MemoryOpsRecord::Op {
                op: op.normalized(),
            })
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    pub fn matching_op(&self, op: &MemoryLifecycleOp) -> Option<&MemoryLifecycleOp> {
        if let Some(key) = incoming_idempotency_key(op) {
            return self
                .ops
                .iter()
                .find(|existing| nonempty_key(&existing.idempotency_key) == Some(key));
        }
        let key = nonempty_key(&op.op_id)?;
        self.ops
            .iter()
            .find(|existing| nonempty_key(&existing.op_id) == Some(key))
    }

    fn recount(&mut self) {
        self.pending_count = 0;
        self.approved_count = 0;
        self.applied_count = 0;
        self.rejected_count = 0;
        self.failed_count = 0;
        self.skipped_count = 0;

        for op in &self.ops {
            match op.status {
                MemoryOpStatus::Pending => self.pending_count += 1,
                MemoryOpStatus::Approved => self.approved_count += 1,
                MemoryOpStatus::Applied => self.applied_count += 1,
                MemoryOpStatus::Rejected => self.rejected_count += 1,
                MemoryOpStatus::Failed => self.failed_count += 1,
                MemoryOpStatus::Skipped => self.skipped_count += 1,
            }
        }
    }
}

fn insert_op_indexes(
    op: &MemoryLifecycleOp,
    index: usize,
    op_id_index: &mut HashMap<String, usize>,
    idempotency_index: &mut HashMap<String, usize>,
) {
    if let Some(key) = nonempty_key(&op.op_id) {
        op_id_index.insert(key.to_string(), index);
    }
    if let Some(key) = nonempty_key(&op.idempotency_key) {
        idempotency_index.insert(key.to_string(), index);
    }
}

fn remove_indexed_key(index: &mut HashMap<String, usize>, key: &str, expected_index: usize) {
    let Some(key) = nonempty_key(key) else {
        return;
    };
    if index.get(key) == Some(&expected_index) {
        index.remove(key);
    }
}

fn nonempty_key(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn incoming_idempotency_key(op: &MemoryLifecycleOp) -> Option<&str> {
    nonempty_key(&op.idempotency_key)
}

fn matching_op_index(
    incoming: &MemoryLifecycleOp,
    op_id_index: &HashMap<String, usize>,
    idempotency_index: &HashMap<String, usize>,
) -> Option<usize> {
    if let Some(key) = incoming_idempotency_key(incoming) {
        return idempotency_index.get(key).copied();
    }
    nonempty_key(&incoming.op_id).and_then(|key| op_id_index.get(key).copied())
}

pub fn memory_op_duplicate_should_replace(
    existing: MemoryOpStatus,
    incoming: MemoryOpStatus,
) -> bool {
    match existing {
        MemoryOpStatus::Pending => true,
        MemoryOpStatus::Approved => incoming != MemoryOpStatus::Pending,
        MemoryOpStatus::Applied
        | MemoryOpStatus::Rejected
        | MemoryOpStatus::Failed
        | MemoryOpStatus::Skipped => memory_op_status_is_finalized(incoming),
    }
}

pub fn memory_op_status_is_finalized(status: MemoryOpStatus) -> bool {
    matches!(
        status,
        MemoryOpStatus::Applied
            | MemoryOpStatus::Rejected
            | MemoryOpStatus::Failed
            | MemoryOpStatus::Skipped
    )
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryOpIdempotencyInput {
    pub source: MemorySource,
    pub op_type: MemoryOpType,
    pub target_paths: Vec<String>,
    pub tags: Vec<String>,
    pub kind: Option<String>,
    pub source_id: Option<String>,
    pub title: Option<String>,
    pub content: Option<String>,
    pub evidence: Option<String>,
}

impl Default for MemoryOpIdempotencyInput {
    fn default() -> Self {
        Self {
            source: MemorySource::default(),
            op_type: MemoryOpType::default(),
            target_paths: Vec::new(),
            tags: Vec::new(),
            kind: None,
            source_id: None,
            title: None,
            content: None,
            evidence: None,
        }
    }
}

impl MemoryOpIdempotencyInput {
    pub fn normalized(&self) -> Self {
        Self {
            source: self.source,
            op_type: self.op_type,
            target_paths: normalize_paths(&self.target_paths),
            tags: normalize_tags(&self.tags),
            kind: self.kind.as_deref().map(normalize_kind),
            source_id: normalize_optional_string(self.source_id.as_deref()),
            title: normalize_optional_text(self.title.as_deref()),
            content: normalize_optional_hash_text(self.content.as_deref()),
            evidence: normalize_optional_evidence(self.evidence.as_deref()),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MemoryLifecycleOpCounts {
    pub duplicate_candidates: u32,
    pub merge_candidates: u32,
    pub archive_candidates: u32,
    pub review_candidates: u32,
    pub conflict_candidates: u32,
}

pub const MEMORY_OP_EXACT_DUPLICATE_EVIDENCE: &str = "exact content_hash duplicate";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryBatchSummary {
    pub batch_key: String,
    pub count: u64,
    pub preview: Vec<String>,
}

pub const MEMORY_BATCH_KEYS: &[&str] = &[
    "merge_exact_duplicate",
    "merge_near_duplicate",
    "archive",
    "review",
    "delete",
    "create",
    "digest",
    "maintenance",
];

pub fn memory_op_batch_key(op: &MemoryLifecycleOp) -> &'static str {
    match op.op_type {
        MemoryOpType::MergeArchive => {
            if op.evidence.starts_with(MEMORY_OP_EXACT_DUPLICATE_EVIDENCE) {
                "merge_exact_duplicate"
            } else {
                "merge_near_duplicate"
            }
        }
        MemoryOpType::ArchiveCandidate | MemoryOpType::Archive => "archive",
        MemoryOpType::MarkReviewNeeded | MemoryOpType::MarkStale => "review",
        MemoryOpType::DeleteCandidate => "delete",
        MemoryOpType::CreateMemory => "create",
        MemoryOpType::PromoteDigest => "digest",
        MemoryOpType::UpdateMemory
        | MemoryOpType::Retag
        | MemoryOpType::RepairLinks
        | MemoryOpType::Refresh => "maintenance",
    }
}

pub fn memory_op_batch_label(batch_key: &str, count: u64) -> String {
    match batch_key {
        "merge_exact_duplicate" => format!("Merge {} exact-duplicate memories", count),
        "merge_near_duplicate" => format!("Merge {} near-duplicate memories", count),
        "archive" => format!("Archive {} stale memories", count),
        "review" => format!("Review {} flagged memories", count),
        "delete" => format!("Delete {} memory candidates", count),
        "create" => format!("Save {} proposed memories", count),
        "digest" => format!("Promote {} digests", count),
        _ => format!("Apply {} memory maintenance ops", count),
    }
}

pub fn memory_op_awaits_approval(op: &MemoryLifecycleOp) -> bool {
    op.status == MemoryOpStatus::Pending && op.requires_approval
}

pub fn memory_op_batches(ops: &[MemoryLifecycleOp]) -> Vec<MemoryBatchSummary> {
    let mut grouped: std::collections::BTreeMap<&'static str, (u64, Vec<String>)> =
        std::collections::BTreeMap::new();
    for op in ops {
        if !memory_op_awaits_approval(op) {
            continue;
        }
        let key = memory_op_batch_key(op);
        let entry = grouped.entry(key).or_default();
        entry.0 += 1;
        if entry.1.len() < 5 {
            let title = op
                .payload
                .title
                .clone()
                .or_else(|| op.target_paths.first().cloned())
                .unwrap_or_else(|| op.evidence.chars().take(80).collect());
            if !title.is_empty() {
                entry.1.push(title);
            }
        }
    }
    grouped
        .into_iter()
        .map(|(batch_key, (count, preview))| MemoryBatchSummary {
            batch_key: batch_key.to_string(),
            count,
            preview,
        })
        .collect()
}

pub fn memory_lifecycle_op_counts(ops: &[MemoryLifecycleOp]) -> MemoryLifecycleOpCounts {
    let mut counts = MemoryLifecycleOpCounts::default();
    for op in ops {
        if !matches!(
            op.status,
            MemoryOpStatus::Pending | MemoryOpStatus::Approved
        ) {
            continue;
        }
        let evidence = op.evidence.to_lowercase();
        match op.op_type {
            MemoryOpType::MergeArchive => {
                counts.merge_candidates = counts.merge_candidates.saturating_add(1);
                if evidence.contains("duplicate") {
                    counts.duplicate_candidates = counts.duplicate_candidates.saturating_add(1);
                }
            }
            MemoryOpType::ArchiveCandidate | MemoryOpType::Archive => {
                counts.archive_candidates = counts.archive_candidates.saturating_add(1);
            }
            MemoryOpType::MarkReviewNeeded | MemoryOpType::MarkStale => {
                counts.review_candidates = counts.review_candidates.saturating_add(1);
            }
            _ => {}
        }
        if evidence.contains("conflict") || evidence.contains("contradict") {
            counts.conflict_candidates = counts.conflict_candidates.saturating_add(1);
        }
    }
    counts
}

pub fn normalize_tags(tags: &[String]) -> Vec<String> {
    let mut normalized: Vec<String> = tags
        .iter()
        .map(|tag| tag.trim().to_lowercase())
        .filter(|tag| !tag.is_empty())
        .collect();
    normalized.sort();
    normalized.dedup();
    normalized
}

pub fn parse_memory_lifecycle_status(status: &str) -> Option<String> {
    let mut normalized = String::new();
    let mut last_separator = false;
    for ch in status.trim().to_lowercase().chars() {
        if ch == '-' || ch == '_' || ch.is_whitespace() {
            if !normalized.is_empty() && !last_separator {
                normalized.push('_');
                last_separator = true;
            }
        } else {
            normalized.push(ch);
            last_separator = false;
        }
    }
    if normalized.ends_with('_') {
        normalized.pop();
    }
    match normalized.as_str() {
        "proposed" | "active" | "pinned" | "archived" | "deprecated" => Some(normalized),
        "review" | "review_needed" | "needs_review" => Some("proposed".to_string()),
        "stale" | "obsolete" => Some("deprecated".to_string()),
        "inactive" | "archive" => Some("archived".to_string()),
        _ => None,
    }
}

pub fn normalize_memory_status(status: Option<&str>) -> String {
    status
        .and_then(parse_memory_lifecycle_status)
        .unwrap_or_else(|| "active".to_string())
}

pub fn normalize_paths(paths: &[String]) -> Vec<String> {
    let mut normalized: Vec<String> = paths
        .iter()
        .filter_map(|path| normalize_path(path))
        .collect();
    normalized.sort();
    normalized.dedup();
    normalized
}

pub fn normalize_path(path: &str) -> Option<String> {
    let path = path.trim().replace('\\', "/");
    if path.is_empty() {
        return None;
    }

    let bytes = path.as_bytes();
    let drive = if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        Some((bytes[0] as char).to_ascii_uppercase())
    } else {
        None
    };

    if let Some(drive) = drive {
        let rest = &path[2..];
        let absolute = rest.starts_with('/');
        let parts = normalize_path_parts(rest);
        return Some(match (absolute, parts.is_empty()) {
            (true, true) => format!("{}:/", drive),
            (true, false) => format!("{}:/{}", drive, parts.join("/")),
            (false, true) => format!("{}:", drive),
            (false, false) => format!("{}:{}", drive, parts.join("/")),
        });
    }

    let unc = path.starts_with("//");
    let absolute = path.starts_with('/') && !unc;
    let parts = normalize_path_parts(if unc { &path[2..] } else { &path });

    if unc {
        if parts.is_empty() {
            Some("//".to_string())
        } else {
            Some(format!("//{}", parts.join("/")))
        }
    } else if absolute {
        if parts.is_empty() {
            Some("/".to_string())
        } else {
            Some(format!("/{}", parts.join("/")))
        }
    } else if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

pub fn normalize_kind(kind: &str) -> String {
    let normalized = kind
        .trim()
        .to_lowercase()
        .replace('-', "_")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("_");
    if normalized.is_empty() {
        "domain".to_string()
    } else {
        normalized
    }
}

pub fn normalize_idempotency_input(input: &MemoryOpIdempotencyInput) -> MemoryOpIdempotencyInput {
    input.normalized()
}

pub fn compute_content_hash(content: &str) -> String {
    let mut h = Sha256::new();
    h.update(normalize_hash_text(content).as_bytes());
    hex::encode(h.finalize())
}

pub fn compute_idempotency_key(input: &MemoryOpIdempotencyInput) -> String {
    let normalized = input.normalized();
    let content_hash = normalized
        .content
        .as_deref()
        .map(compute_content_hash)
        .unwrap_or_default();
    let evidence_hash = normalized
        .evidence
        .as_deref()
        .map(compute_content_hash)
        .unwrap_or_default();
    let mut h = Sha256::new();
    hash_field(&mut h, "source", normalized.source.as_str());
    hash_field(&mut h, "op_type", normalized.op_type.as_str());
    hash_list(&mut h, "target_path", &normalized.target_paths);
    hash_list(&mut h, "tag", &normalized.tags);
    hash_field(&mut h, "kind", normalized.kind.as_deref().unwrap_or(""));
    hash_field(
        &mut h,
        "source_id",
        normalized.source_id.as_deref().unwrap_or(""),
    );
    hash_field(&mut h, "title", normalized.title.as_deref().unwrap_or(""));
    hash_field(&mut h, "content_hash", &content_hash);
    hash_field(&mut h, "evidence_hash", &evidence_hash);
    format!("memop_{}", hex::encode(h.finalize()))
}

pub fn default_requires_approval(op_type: MemoryOpType, confidence: f32) -> bool {
    match op_type {
        MemoryOpType::ArchiveCandidate
        | MemoryOpType::Archive
        | MemoryOpType::MergeArchive
        | MemoryOpType::DeleteCandidate => true,
        MemoryOpType::CreateMemory | MemoryOpType::Retag | MemoryOpType::RepairLinks => {
            confidence < HIGH_CONFIDENCE_APPROVAL_THRESHOLD
        }
        _ => true,
    }
}

pub fn normalize_strings(values: &[String]) -> Vec<String> {
    let mut normalized: Vec<String> = values
        .iter()
        .filter_map(|value| normalize_optional_string(Some(value)))
        .collect();
    normalized.sort();
    normalized.dedup();
    normalized
}

pub fn normalize_review_after(value: Option<&str>) -> Option<String> {
    let value = normalize_optional_string(value)?;
    NaiveDate::parse_from_str(&value, "%Y-%m-%d")
        .ok()
        .map(|date| date.format("%Y-%m-%d").to_string())
}

pub fn today_string() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

pub fn redact_and_cap_payload_text(text: &str, max_chars: usize) -> String {
    let scan_cap = max_chars.saturating_add(4096);
    let scanned = safe_truncate(text, scan_cap);
    let redacted = redact_sensitive(scanned);
    safe_truncate(&redacted, max_chars).trim().to_string()
}

fn safe_truncate(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        return s;
    }
    let mut end = max_len.min(s.len());
    while !s.is_char_boundary(end) && end > 0 {
        end -= 1;
    }
    &s[..end]
}

pub fn normalize_evidence_text(text: &str) -> String {
    redact_and_cap_payload_text(text, MEMORY_OP_EVIDENCE_MAX_CHARS)
}

pub fn normalize_optional_evidence(value: Option<&str>) -> Option<String> {
    let evidence = normalize_evidence_text(value?);
    normalize_optional_text(Some(&evidence))
}

pub fn default_review_after_days(
    kind: &str,
    source: MemorySource,
    status: MemoryCandidateStatus,
) -> u32 {
    let base = match normalize_kind(kind).as_str() {
        "decision" | "decisions" | "architecture" | "code" => 180,
        "preference" => 365,
        "task" | "task_report" | "task_summary" | "task_report_summary" => 30,
        "research" | "research_note" | "domain" | "trajectory" => 90,
        "digest" | "summary" => 60,
        _ => 90,
    };
    let source_adjusted = match source {
        MemorySource::Manual => base,
        MemorySource::Git => base.min(120),
        MemorySource::Trajectory => base.min(90),
        MemorySource::BehaviorLearner => base.min(60),
        MemorySource::MemoryGarden
        | MemorySource::KnowledgeConflictResolver
        | MemorySource::Buddy => base.min(75),
    };
    if status == MemoryCandidateStatus::Proposed && source.is_autonomous() {
        source_adjusted.min(30)
    } else {
        source_adjusted
    }
}

pub fn default_review_after_date(
    created: chrono::NaiveDate,
    kind: &str,
    source: MemorySource,
    status: MemoryCandidateStatus,
) -> String {
    let days = default_review_after_days(kind, source, status) as i64;
    (created + chrono::Duration::days(days))
        .format("%Y-%m-%d")
        .to_string()
}

fn normalize_path_parts(path: &str) -> Vec<String> {
    let mut parts = Vec::new();
    for part in path.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            parts.push(part.to_string());
            continue;
        }
        parts.push(part.to_string());
    }
    parts
}

pub fn normalize_optional_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(|value| value.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|value| !value.is_empty())
}

pub fn normalize_optional_hash_text(value: Option<&str>) -> Option<String> {
    value
        .map(normalize_hash_text)
        .filter(|value| !value.is_empty())
}

pub fn normalize_hash_text(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .trim()
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn hash_field(h: &mut Sha256, name: &str, value: &str) {
    h.update(name.as_bytes());
    h.update([0]);
    h.update(value.as_bytes());
    h.update([0]);
}

pub fn hash_list(h: &mut Sha256, name: &str, values: &[String]) {
    for value in values {
        hash_field(h, name, value);
    }
}
#[cfg(test)]
mod batch_tests {
    use super::*;

    fn op(op_type: MemoryOpType, evidence: &str, status: MemoryOpStatus) -> MemoryLifecycleOp {
        let mut op = MemoryLifecycleOp::pending(
            uuid_like(evidence),
            MemorySource::MemoryGarden,
            op_type,
            vec![format!("/k/{}.md", evidence.len())],
            evidence,
            0.9,
            "2026-07-01T00:00:00Z",
        );
        op.requires_approval = true;
        op.status = status;
        op
    }

    fn uuid_like(seed: &str) -> String {
        format!("op-{}", compute_content_hash(seed))
    }

    #[test]
    fn every_op_type_batch_key_is_registered() {
        let op_types = [
            MemoryOpType::MergeArchive,
            MemoryOpType::ArchiveCandidate,
            MemoryOpType::Archive,
            MemoryOpType::MarkReviewNeeded,
            MemoryOpType::MarkStale,
            MemoryOpType::DeleteCandidate,
            MemoryOpType::CreateMemory,
            MemoryOpType::PromoteDigest,
            MemoryOpType::UpdateMemory,
            MemoryOpType::Retag,
            MemoryOpType::RepairLinks,
            MemoryOpType::Refresh,
        ];
        for op_type in op_types {
            let sample = op(op_type, "registered batch key", MemoryOpStatus::Pending);
            let key = memory_op_batch_key(&sample);
            assert!(
                MEMORY_BATCH_KEYS.contains(&key),
                "batch key {} for {:?} missing from MEMORY_BATCH_KEYS",
                key,
                op_type
            );
        }
        let exact = op(
            MemoryOpType::MergeArchive,
            &format!("{}: canonical=a", MEMORY_OP_EXACT_DUPLICATE_EVIDENCE),
            MemoryOpStatus::Pending,
        );
        assert!(MEMORY_BATCH_KEYS.contains(&memory_op_batch_key(&exact)));
    }

    #[test]
    fn batch_key_classifies_merge_variants() {
        let exact = op(
            MemoryOpType::MergeArchive,
            &format!("{}: canonical=a", MEMORY_OP_EXACT_DUPLICATE_EVIDENCE),
            MemoryOpStatus::Pending,
        );
        let near = op(
            MemoryOpType::MergeArchive,
            "near duplicate: canonical=a",
            MemoryOpStatus::Pending,
        );
        assert_eq!(memory_op_batch_key(&exact), "merge_exact_duplicate");
        assert_eq!(memory_op_batch_key(&near), "merge_near_duplicate");
    }

    #[test]
    fn batches_group_pending_approval_ops_with_preview() {
        let mut ops = vec![
            op(
                MemoryOpType::MergeArchive,
                "near duplicate 1",
                MemoryOpStatus::Pending,
            ),
            op(
                MemoryOpType::MergeArchive,
                "near duplicate 2",
                MemoryOpStatus::Pending,
            ),
            op(MemoryOpType::Archive, "stale doc", MemoryOpStatus::Pending),
            op(
                MemoryOpType::MergeArchive,
                "already done",
                MemoryOpStatus::Applied,
            ),
        ];
        ops[0].payload.title = Some("First dup".to_string());

        let batches = memory_op_batches(&ops);

        let merge = batches
            .iter()
            .find(|b| b.batch_key == "merge_near_duplicate")
            .unwrap();
        assert_eq!(merge.count, 2);
        assert_eq!(merge.preview[0], "First dup");
        let archive = batches.iter().find(|b| b.batch_key == "archive").unwrap();
        assert_eq!(archive.count, 1);
        assert!(!batches.iter().any(|b| b.count == 0));
    }

    #[test]
    fn non_approval_ops_do_not_batch() {
        let mut auto = op(
            MemoryOpType::CreateMemory,
            "high confidence create",
            MemoryOpStatus::Pending,
        );
        auto.requires_approval = false;
        assert!(memory_op_batches(&[auto]).is_empty());
    }

    #[test]
    fn batch_labels_are_human_readable() {
        assert_eq!(
            memory_op_batch_label("merge_near_duplicate", 312),
            "Merge 312 near-duplicate memories"
        );
        assert_eq!(
            memory_op_batch_label("archive", 7),
            "Archive 7 stale memories"
        );
    }
}
