#![allow(dead_code)]

use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const HIGH_CONFIDENCE_APPROVAL_THRESHOLD: f32 = 0.85;

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryLifecycleOp {
    pub op_id: String,
    pub source: MemorySource,
    pub op_type: MemoryOpType,
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
        let evidence = evidence.into();
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
            let op = record.into_op().normalized();
            let existing_index = nonempty_key(&op.idempotency_key)
                .and_then(|key| idempotency_index.get(key).copied())
                .or_else(|| nonempty_key(&op.op_id).and_then(|key| op_id_index.get(key).copied()));

            match existing_index {
                Some(index) => {
                    if let Some(old) = ops.get(index).cloned() {
                        remove_indexed_key(&mut op_id_index, &old.op_id, index);
                        remove_indexed_key(&mut idempotency_index, &old.idempotency_key, index);
                    }
                    ops[index] = op.clone();
                    insert_op_indexes(&op, index, &mut op_id_index, &mut idempotency_index);
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
            .map(|op| MemoryOpsRecord::Op { op })
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
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
    if value.is_empty() { None } else { Some(value) }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryCandidate {
    pub candidate_id: String,
    pub source: MemorySource,
    pub title: String,
    pub content: String,
    pub tags: Vec<String>,
    pub kind: String,
    pub filenames: Vec<String>,
    pub related_files: Vec<String>,
    pub source_id: Option<String>,
    pub confidence: f32,
    pub status: MemoryCandidateStatus,
    pub content_hash: String,
    pub review_after_days: u32,
}

impl Default for MemoryCandidate {
    fn default() -> Self {
        Self {
            candidate_id: String::new(),
            source: MemorySource::default(),
            title: String::new(),
            content: String::new(),
            tags: Vec::new(),
            kind: "domain".to_string(),
            filenames: Vec::new(),
            related_files: Vec::new(),
            source_id: None,
            confidence: 0.0,
            status: MemoryCandidateStatus::Proposed,
            content_hash: String::new(),
            review_after_days: 0,
        }
    }
}

impl MemoryCandidate {
    pub fn normalized(mut self) -> Self {
        self.tags = normalize_tags(&self.tags);
        self.filenames = normalize_paths(&self.filenames);
        self.related_files = normalize_paths(&self.related_files);
        self.kind = normalize_kind(&self.kind);
        self.source_id = normalize_optional_string(self.source_id.as_deref());
        if self.content_hash.trim().is_empty() {
            self.content_hash = compute_content_hash(&self.content);
        }
        if self.review_after_days == 0 {
            self.review_after_days =
                default_review_after_days(&self.kind, self.source, self.status);
        }
        self
    }

    pub fn idempotency_input(&self, op_type: MemoryOpType) -> MemoryOpIdempotencyInput {
        MemoryOpIdempotencyInput {
            source: self.source,
            op_type,
            target_paths: self.filenames.clone(),
            tags: self.tags.clone(),
            kind: Some(self.kind.clone()),
            source_id: self.source_id.clone(),
            title: Some(self.title.clone()),
            content: Some(self.content.clone()),
            evidence: None,
        }
    }
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
            evidence: normalize_optional_text(self.evidence.as_deref()),
        }
    }
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
    path.split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .map(|part| part.to_string())
        .collect()
}

fn normalize_optional_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(|value| value.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|value| !value.is_empty())
}

fn normalize_optional_hash_text(value: Option<&str>) -> Option<String> {
    value
        .map(normalize_hash_text)
        .filter(|value| !value.is_empty())
}

fn normalize_hash_text(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .trim()
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

fn hash_field(h: &mut Sha256, name: &str, value: &str) {
    h.update(name.as_bytes());
    h.update([0]);
    h.update(value.as_bytes());
    h.update([0]);
}

fn hash_list(h: &mut Sha256, name: &str, values: &[String]) {
    for value in values {
        hash_field(h, name, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn test_op(op_id: &str, evidence: &str, status: MemoryOpStatus) -> MemoryLifecycleOp {
        let mut op = MemoryLifecycleOp::pending(
            op_id,
            MemorySource::MemoryGarden,
            MemoryOpType::CreateMemory,
            strings(&[".refact/knowledge/item.md"]),
            evidence,
            0.91,
            "2026-05-02T00:00:00Z",
        );
        op.status = status;
        op
    }

    #[test]
    fn serde_roundtrip_every_source_op_and_status_variant() {
        let sources = [
            MemorySource::Buddy,
            MemorySource::Trajectory,
            MemorySource::Git,
            MemorySource::Manual,
            MemorySource::BehaviorLearner,
            MemorySource::MemoryGarden,
            MemorySource::KnowledgeConflictResolver,
        ];
        for source in sources {
            let json = serde_json::to_string(&source).unwrap();
            assert_eq!(serde_json::from_str::<MemorySource>(&json).unwrap(), source);
        }

        let op_types = [
            MemoryOpType::CreateMemory,
            MemoryOpType::UpdateMemory,
            MemoryOpType::Retag,
            MemoryOpType::RepairLinks,
            MemoryOpType::Refresh,
            MemoryOpType::ArchiveCandidate,
            MemoryOpType::Archive,
            MemoryOpType::MergeArchive,
            MemoryOpType::DeleteCandidate,
            MemoryOpType::PromoteDigest,
            MemoryOpType::MarkReviewNeeded,
            MemoryOpType::MarkStale,
        ];
        for op_type in op_types {
            let json = serde_json::to_string(&op_type).unwrap();
            assert_eq!(
                serde_json::from_str::<MemoryOpType>(&json).unwrap(),
                op_type
            );
        }

        let statuses = [
            MemoryOpStatus::Pending,
            MemoryOpStatus::Approved,
            MemoryOpStatus::Applied,
            MemoryOpStatus::Rejected,
            MemoryOpStatus::Failed,
            MemoryOpStatus::Skipped,
        ];
        for status in statuses {
            let json = serde_json::to_string(&status).unwrap();
            assert_eq!(
                serde_json::from_str::<MemoryOpStatus>(&json).unwrap(),
                status
            );
        }

        let op = MemoryLifecycleOp::pending(
            "op-1",
            MemorySource::MemoryGarden,
            MemoryOpType::Retag,
            strings(&["src//lib.rs", "src/lib.rs"]),
            "Memory tags were stale",
            0.91,
            "2026-05-02T00:00:00Z",
        );
        let json = serde_json::to_string(&op).unwrap();
        assert_eq!(
            serde_json::from_str::<MemoryLifecycleOp>(&json).unwrap(),
            op
        );
    }

    #[test]
    fn idempotency_key_is_stable_for_tag_and_path_order() {
        let first = MemoryOpIdempotencyInput {
            source: MemorySource::Trajectory,
            op_type: MemoryOpType::CreateMemory,
            target_paths: strings(&["src//buddy/memory_lifecycle.rs", "README.md"]),
            tags: strings(&[" Buddy ", "Memory", "buddy"]),
            kind: Some(" Research Note ".to_string()),
            source_id: Some(" trajectory-1 ".to_string()),
            title: Some("  Useful discovery  ".to_string()),
            content: Some("Line one\r\nLine two\n".to_string()),
            evidence: Some(" observed in trajectory ".to_string()),
        };
        let second = MemoryOpIdempotencyInput {
            source: MemorySource::Trajectory,
            op_type: MemoryOpType::CreateMemory,
            target_paths: strings(&["README.md", "src/buddy/memory_lifecycle.rs"]),
            tags: strings(&["memory", "buddy"]),
            kind: Some("research_note".to_string()),
            source_id: Some("trajectory-1".to_string()),
            title: Some("Useful discovery".to_string()),
            content: Some("Line one\nLine two".to_string()),
            evidence: Some("observed in trajectory".to_string()),
        };

        assert_eq!(
            compute_idempotency_key(&first),
            compute_idempotency_key(&second)
        );
    }

    #[test]
    fn path_normalization_handles_unix_relative_and_windows_inputs() {
        assert_eq!(
            normalize_path("/tmp//project/./src/lib.rs"),
            Some("/tmp/project/src/lib.rs".to_string())
        );
        assert_eq!(
            normalize_path(" ./relative//path/ "),
            Some("relative/path".to_string())
        );
        assert_eq!(
            normalize_path("src\\buddy//memory_lifecycle.rs"),
            Some("src/buddy/memory_lifecycle.rs".to_string())
        );
        assert_eq!(
            normalize_path("c:\\Users\\Ada\\project\\file.md"),
            Some("C:/Users/Ada/project/file.md".to_string())
        );
        assert_eq!(
            normalize_paths(&strings(&["b//c", "a\\d", "b/c", ""])),
            strings(&["a/d", "b/c"])
        );
    }

    #[test]
    fn tag_normalization_trims_lowercases_sorts_and_dedupes() {
        assert_eq!(
            normalize_tags(&strings(&[" Buddy ", "memory", "", "MEMORY", "alpha"])),
            strings(&["alpha", "buddy", "memory"])
        );
    }

    #[test]
    fn default_approval_policy_requires_destructive_and_allows_high_confidence_safe_ops() {
        assert!(default_requires_approval(MemoryOpType::Archive, 0.99));
        assert!(default_requires_approval(
            MemoryOpType::ArchiveCandidate,
            0.99
        ));
        assert!(default_requires_approval(MemoryOpType::MergeArchive, 0.99));
        assert!(default_requires_approval(
            MemoryOpType::DeleteCandidate,
            0.99
        ));

        assert!(!default_requires_approval(MemoryOpType::CreateMemory, 0.90));
        assert!(!default_requires_approval(MemoryOpType::Retag, 0.90));
        assert!(!default_requires_approval(MemoryOpType::RepairLinks, 0.90));
        assert!(default_requires_approval(MemoryOpType::CreateMemory, 0.70));
        assert!(default_requires_approval(MemoryOpType::UpdateMemory, 0.95));
    }

    #[test]
    fn review_after_policy_varies_by_kind_source_and_status() {
        let manual_code = default_review_after_days(
            "code",
            MemorySource::Manual,
            MemoryCandidateStatus::Promoted,
        );
        let manual_research = default_review_after_days(
            "research",
            MemorySource::Manual,
            MemoryCandidateStatus::Promoted,
        );
        let manual_task = default_review_after_days(
            "task_report",
            MemorySource::Manual,
            MemoryCandidateStatus::Promoted,
        );
        let proposed_auto_code = default_review_after_days(
            "code",
            MemorySource::BehaviorLearner,
            MemoryCandidateStatus::Proposed,
        );

        assert!(manual_code > manual_research);
        assert!(manual_research > manual_task);
        assert!(proposed_auto_code < manual_code);
        assert_eq!(proposed_auto_code, 30);
        assert_eq!(
            default_review_after_date(
                chrono::NaiveDate::from_ymd_opt(2026, 5, 2).unwrap(),
                "task_report",
                MemorySource::Manual,
                MemoryCandidateStatus::Promoted,
            ),
            "2026-06-01"
        );
    }

    #[test]
    fn memory_ops_state_preserves_first_seen_order() {
        let first = test_op("op-1", "first", MemoryOpStatus::Pending);
        let second = test_op("op-2", "second", MemoryOpStatus::Approved);
        let state = MemoryOpsState::from_records(vec![
            MemoryOpsRecord::Op { op: first.clone() },
            MemoryOpsRecord::Op { op: second.clone() },
        ]);

        assert_eq!(state.ops, vec![first.normalized(), second.normalized()]);
        assert_eq!(state.pending_count, 1);
        assert_eq!(state.approved_count, 1);
    }

    #[test]
    fn memory_ops_state_duplicate_idempotency_key_uses_latest_slot() {
        let first = test_op("op-1", "same", MemoryOpStatus::Pending);
        let mut second = test_op("op-2", "same", MemoryOpStatus::Applied);
        second.idempotency_key = first.idempotency_key.clone();

        let state = MemoryOpsState::from_records(vec![
            MemoryOpsRecord::Op { op: first },
            MemoryOpsRecord::Op { op: second.clone() },
        ]);

        assert_eq!(state.ops.len(), 1);
        assert_eq!(state.ops[0].op_id, "op-2");
        assert_eq!(state.ops[0].status, MemoryOpStatus::Applied);
        assert_eq!(state.applied_count, 1);
    }

    #[test]
    fn memory_ops_state_compaction_records_latest_per_op_and_key() {
        let first = test_op("op-1", "first", MemoryOpStatus::Pending);
        let mut second = first.clone();
        second.status = MemoryOpStatus::Failed;
        second.error = Some("write failed".to_string());
        let third = test_op("op-2", "second", MemoryOpStatus::Applied);

        let state = MemoryOpsState::from_records(vec![
            MemoryOpsRecord::Op { op: first },
            MemoryOpsRecord::Op { op: second.clone() },
            MemoryOpsRecord::Op { op: third.clone() },
        ]);
        let compacted = MemoryOpsState::from_records(state.canonical_records());

        assert_eq!(compacted.ops, vec![second.normalized(), third.normalized()]);
        assert_eq!(compacted.failed_count, 1);
        assert_eq!(compacted.applied_count, 1);
    }
}
