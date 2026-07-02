#![allow(dead_code)]

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::file_filter::KNOWLEDGE_FOLDER_NAME;
use crate::files_correction::get_project_dirs;
use crate::app_state::AppState;
use crate::git::operations::{
    GitCoChangePair, GitCommitClassification, GitCommitSummary, GitFileChangeStatus,
    GitHistoryReport, GitHotspot,
};
use crate::knowledge_graph::kg_structs::{KnowledgeDoc, KnowledgeFrontmatter};
use crate::memories::{
    create_frontmatter, get_global_knowledge_dir, memories_add, normalize_memory_tags,
    update_memory_document_frontmatter,
};

pub use refact_buddy_core::memory_lifecycle_model::*;

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
    pub source_message_range: Option<String>,
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
            source_message_range: None,
            confidence: 0.0,
            status: MemoryCandidateStatus::Proposed,
            content_hash: String::new(),
            review_after_days: 0,
        }
    }
}

impl MemoryCandidate {
    pub fn normalized(mut self) -> Self {
        self.title = normalize_optional_text(Some(&self.title)).unwrap_or_default();
        self.content = redact_and_cap_payload_text(&self.content, PAYLOAD_CONTENT_MAX_CHARS);
        self.tags = normalize_tags(&self.tags);
        if !self.tags.iter().any(|tag| tag == "memory") {
            self.tags.push("memory".to_string());
            self.tags = normalize_tags(&self.tags);
        }
        self.filenames = normalize_paths(&self.filenames);
        self.related_files = normalize_paths(&self.related_files);
        self.kind = normalize_kind(&self.kind);
        self.source_id = normalize_optional_string(self.source_id.as_deref());
        self.source_message_range = normalize_optional_string(self.source_message_range.as_deref());
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
            content: Some(self.content_hash.clone()),
            evidence: None,
        }
    }

    pub fn into_create_memory_op(
        self,
        op_id: impl Into<String>,
        evidence: impl Into<String>,
        created_at: impl Into<String>,
    ) -> MemoryLifecycleOp {
        let candidate = self.normalized();
        let created_at = created_at.into();
        let created_date = DateTime::parse_from_rfc3339(&created_at)
            .ok()
            .map(|dt| dt.with_timezone(&Utc).date_naive())
            .unwrap_or_else(|| Utc::now().date_naive());
        let review_after = default_review_after_date(
            created_date,
            &candidate.kind,
            candidate.source,
            candidate.status,
        );
        let mut op = MemoryLifecycleOp::pending(
            op_id,
            candidate.source,
            MemoryOpType::CreateMemory,
            candidate.filenames.clone(),
            evidence,
            candidate.confidence,
            created_at,
        );
        op.payload = MemoryLifecyclePayload {
            title: Some(candidate.title.clone()),
            content: Some(candidate.content.clone()),
            tags: Some(candidate.tags.clone()),
            kind: Some(candidate.kind.clone()),
            filenames: Some(candidate.filenames.clone()),
            related_files: Some(candidate.related_files.clone()),
            review_after: Some(review_after),
            source_id: candidate.source_id.clone(),
            source_message_range: candidate.source_message_range.clone(),
            source_content_hash: Some(candidate.content_hash.clone()),
            ..Default::default()
        };
        op.idempotency_key =
            compute_idempotency_key(&candidate.idempotency_input(MemoryOpType::CreateMemory));
        op.requires_approval = default_requires_approval(op.op_type, op.confidence)
            || (candidate.source.is_autonomous()
                && candidate.status == MemoryCandidateStatus::Proposed);
        op.status = MemoryOpStatus::Pending;
        op.normalized()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemorySourceClass {
    UserAuthored,
    AutoGenerated,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemoryScoreInput {
    pub status: String,
    pub source_class: MemorySourceClass,
    pub source_confidence: Option<f32>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub review_after: Option<String>,
    pub use_count: u32,
    pub last_used_at: Option<String>,
    pub last_injected_at: Option<String>,
    pub dismissed_count: u32,
    pub tag_overlap: u32,
    pub file_overlap: u32,
    pub entity_overlap: u32,
    pub duplicate_penalty: f32,
    pub conflict_risk: f32,
}

impl Default for MemoryScoreInput {
    fn default() -> Self {
        Self {
            status: "active".to_string(),
            source_class: MemorySourceClass::AutoGenerated,
            source_confidence: None,
            created_at: None,
            updated_at: None,
            review_after: None,
            use_count: 0,
            last_used_at: None,
            last_injected_at: None,
            dismissed_count: 0,
            tag_overlap: 0,
            file_overlap: 0,
            entity_overlap: 0,
            duplicate_penalty: 0.0,
            conflict_risk: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MemoryUsefulnessScore {
    pub score: f32,
    pub duplicate_penalty: f32,
    pub conflict_penalty: f32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MemoryDocSnapshot {
    pub id: String,
    pub path: String,
    pub title: String,
    pub content: String,
    pub tags: Vec<String>,
    pub filenames: Vec<String>,
    pub related_files: Vec<String>,
    pub links: Vec<String>,
    pub entities: Vec<String>,
    pub status: String,
    pub kind: String,
    pub source_class: Option<MemorySourceClass>,
    pub source_confidence: Option<f32>,
    pub source_id: Option<String>,
    pub source_tool: Option<String>,
    pub source_chat_id: Option<String>,
    pub source_trajectory_id: Option<String>,
    pub source_message_range: Option<String>,
    pub source_commit: Option<String>,
    pub topic: Option<String>,
    pub content_hash: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub review_after: Option<String>,
    pub use_count: u32,
    pub last_used_at: Option<String>,
    pub last_injected_at: Option<String>,
    pub dismissed_count: u32,
}

const MAX_MEMORY_LIFECYCLE_DOCS: usize = 500;
const MAX_MEMORY_LIFECYCLE_OPS: usize = 100;
const MAX_MEMORY_LIFECYCLE_SCAN_ENTRIES: usize = 5_000;
const MAX_MEMORY_LIFECYCLE_FILE_BYTES: u64 = 256 * 1024;
const MAX_GIT_MEMORY_OPS: usize = 80;
const MAX_GIT_MEMORY_PATHS: usize = 12;
const MAX_GIT_CREATE_OPS_PER_KIND: usize = 8;
pub const MEMORY_OP_EXACT_DUPLICATE_REASON: &str = "exact content_hash duplicate";
pub const MEMORY_OP_NEAR_DUPLICATE_REASON: &str = "vecdb near-duplicate on write";
pub const MEMORY_OP_AUTO_APPLY_MIN_CONFIDENCE: f32 = 0.94;
const MEMORY_OP_AUTO_APPLY_MIN_ALPHABETIC_CHARS: usize = 12;
const NEAR_DUPLICATE_MAX_DISTANCE: f32 = 0.10;

impl MemoryDocSnapshot {
    pub fn from_knowledge_doc(doc: &KnowledgeDoc) -> Self {
        let frontmatter = &doc.frontmatter;
        let id = frontmatter
            .id
            .clone()
            .unwrap_or_else(|| doc.path.to_string_lossy().to_string());
        Self {
            id,
            path: doc.path.to_string_lossy().to_string(),
            title: frontmatter.title.clone().unwrap_or_default(),
            content: doc.content.clone(),
            tags: frontmatter.tags.clone(),
            filenames: frontmatter.filenames.clone(),
            related_files: frontmatter.related_files.clone(),
            links: frontmatter.links.clone(),
            entities: if frontmatter.entities.is_empty() {
                doc.entities.clone()
            } else {
                frontmatter.entities.clone()
            },
            status: frontmatter
                .status
                .clone()
                .unwrap_or_else(|| "active".to_string()),
            kind: frontmatter.kind_or_default().to_string(),
            source_class: Some(memory_source_class(frontmatter)),
            source_confidence: frontmatter.source_confidence,
            source_id: frontmatter.source_id.clone(),
            source_tool: frontmatter.source_tool.clone(),
            source_chat_id: frontmatter.source_chat_id.clone(),
            source_trajectory_id: frontmatter.source_trajectory_id.clone(),
            source_message_range: frontmatter.source_message_range.clone(),
            source_commit: frontmatter.source_commit.clone(),
            topic: frontmatter.topic.clone(),
            content_hash: frontmatter
                .content_hash
                .clone()
                .unwrap_or_else(|| compute_content_hash(&doc.content)),
            created_at: frontmatter
                .created_at
                .clone()
                .or_else(|| frontmatter.created.clone()),
            updated_at: frontmatter.updated.clone(),
            review_after: frontmatter.review_after.clone(),
            use_count: frontmatter.use_count,
            last_used_at: frontmatter.last_used_at.clone(),
            last_injected_at: frontmatter.last_injected_at.clone(),
            dismissed_count: frontmatter.dismissed_count,
        }
        .normalized()
    }

    pub fn from_parts(path: PathBuf, frontmatter: KnowledgeFrontmatter, content: String) -> Self {
        let id = frontmatter
            .id
            .clone()
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        Self {
            id,
            path: path.to_string_lossy().to_string(),
            title: frontmatter.title.clone().unwrap_or_default(),
            content: content.clone(),
            tags: frontmatter.tags.clone(),
            filenames: frontmatter.filenames.clone(),
            related_files: frontmatter.related_files.clone(),
            links: frontmatter.links.clone(),
            entities: frontmatter.entities.clone(),
            status: frontmatter
                .status
                .clone()
                .unwrap_or_else(|| "active".to_string()),
            kind: frontmatter.kind_or_default().to_string(),
            source_class: Some(memory_source_class(&frontmatter)),
            source_confidence: frontmatter.source_confidence,
            source_id: frontmatter.source_id.clone(),
            source_tool: frontmatter.source_tool.clone(),
            source_chat_id: frontmatter.source_chat_id.clone(),
            source_trajectory_id: frontmatter.source_trajectory_id.clone(),
            source_message_range: frontmatter.source_message_range.clone(),
            source_commit: frontmatter.source_commit.clone(),
            topic: frontmatter.topic.clone(),
            content_hash: frontmatter
                .content_hash
                .clone()
                .unwrap_or_else(|| compute_content_hash(&content)),
            created_at: frontmatter
                .created_at
                .clone()
                .or_else(|| frontmatter.created.clone()),
            updated_at: frontmatter.updated.clone(),
            review_after: frontmatter.review_after.clone(),
            use_count: frontmatter.use_count,
            last_used_at: frontmatter.last_used_at.clone(),
            last_injected_at: frontmatter.last_injected_at.clone(),
            dismissed_count: frontmatter.dismissed_count,
        }
        .normalized()
    }

    pub fn normalized(mut self) -> Self {
        self.id = self.id.trim().to_string();
        self.path = normalize_path(&self.path).unwrap_or_else(|| self.path.trim().to_string());
        self.title = normalize_optional_text(Some(&self.title)).unwrap_or_default();
        self.tags = normalize_tags(&self.tags);
        self.filenames = normalize_paths(&self.filenames);
        self.related_files = normalize_paths(&self.related_files);
        self.links = normalize_strings(&self.links);
        self.entities = normalize_strings(&self.entities);
        self.status = normalize_memory_status(Some(&self.status));
        self.kind = normalize_kind(&self.kind);
        self.source_tool = normalize_optional_string(self.source_tool.as_deref());
        self.source_id = normalize_optional_string(self.source_id.as_deref());
        self.source_chat_id = normalize_optional_string(self.source_chat_id.as_deref());
        self.source_trajectory_id = normalize_optional_string(self.source_trajectory_id.as_deref());
        self.source_message_range = normalize_optional_string(self.source_message_range.as_deref());
        self.source_commit = normalize_optional_string(self.source_commit.as_deref());
        self.topic = normalize_optional_text(self.topic.as_deref());
        self.content_hash = normalize_optional_string(Some(&self.content_hash))
            .unwrap_or_else(|| compute_content_hash(&self.content));
        self.created_at = normalize_optional_string(self.created_at.as_deref());
        self.updated_at = normalize_optional_string(self.updated_at.as_deref());
        self.review_after = normalize_review_after(self.review_after.as_deref());
        self.last_used_at = normalize_optional_string(self.last_used_at.as_deref());
        self.last_injected_at = normalize_optional_string(self.last_injected_at.as_deref());
        self
    }

    fn stable_key(&self) -> String {
        if self.id.is_empty() {
            self.path.clone()
        } else {
            self.id.clone()
        }
    }

    fn source_class(&self) -> MemorySourceClass {
        self.source_class.unwrap_or_else(|| {
            if self
                .source_tool
                .as_deref()
                .map(source_tool_is_autonomous)
                .unwrap_or(false)
            {
                MemorySourceClass::AutoGenerated
            } else {
                MemorySourceClass::UserAuthored
            }
        })
    }

    fn protected(&self) -> bool {
        self.status == "pinned" || self.source_class() == MemorySourceClass::UserAuthored
    }

    fn all_files(&self) -> Vec<String> {
        let mut files = self.filenames.clone();
        files.extend(self.related_files.clone());
        normalize_paths(&files)
    }

    fn score_input(&self, duplicate_penalty: f32, conflict_risk: f32) -> MemoryScoreInput {
        MemoryScoreInput {
            status: self.status.clone(),
            source_class: self.source_class(),
            source_confidence: self.source_confidence,
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
            review_after: self.review_after.clone(),
            use_count: self.use_count,
            last_used_at: self.last_used_at.clone(),
            last_injected_at: self.last_injected_at.clone(),
            dismissed_count: self.dismissed_count,
            duplicate_penalty,
            conflict_risk,
            ..MemoryScoreInput::default()
        }
    }
}

pub fn memory_source_class(frontmatter: &KnowledgeFrontmatter) -> MemorySourceClass {
    if frontmatter.is_pinned() {
        return MemorySourceClass::UserAuthored;
    }
    if frontmatter
        .source_tool
        .as_deref()
        .map(source_tool_is_autonomous)
        .unwrap_or(false)
    {
        MemorySourceClass::AutoGenerated
    } else {
        MemorySourceClass::UserAuthored
    }
}

fn source_tool_is_autonomous(tool: &str) -> bool {
    let tool = tool.trim().to_lowercase();
    tool.contains("buddy")
        || tool.contains("memo_extraction")
        || tool.contains("memories_add_enriched")
        || tool.contains("knowledge")
}

pub fn score_memory_usefulness(
    input: &MemoryScoreInput,
    now: DateTime<Utc>,
) -> MemoryUsefulnessScore {
    let status = normalize_memory_status(Some(&input.status));
    let mut score = 0.45f32;

    score += match status.as_str() {
        "pinned" => 0.35,
        "active" => 0.15,
        "proposed" => -0.12,
        "archived" | "deprecated" => -0.45,
        _ => 0.0,
    };

    if input.source_class == MemorySourceClass::UserAuthored {
        score += 0.12;
    }

    if let Some(confidence) = input.source_confidence {
        score += ((confidence.clamp(0.0, 1.0) - 0.5) * 0.24).clamp(-0.12, 0.12);
    }

    if let Some(days) = best_age_days(input, now) {
        score += recency_score(days);
    }

    if let Some(review_after) = parse_date(input.review_after.as_deref()) {
        if now.date_naive() > review_after {
            score -= if status == "proposed" { 0.16 } else { 0.08 };
        }
    }

    if input.use_count > 0 {
        let capped = input.use_count.min(32) as f32;
        score += (capped.ln_1p() / 32.0f32.ln_1p()) * 0.18;
    }

    if let Some(days) = age_days_from_str(input.last_used_at.as_deref(), now) {
        score += recent_usage_score(days, 0.12);
    }
    if let Some(days) = age_days_from_str(input.last_injected_at.as_deref(), now) {
        score += recent_usage_score(days, 0.08);
    }

    let overlap_bonus = input.tag_overlap.min(5) as f32 * 0.025
        + input.file_overlap.min(5) as f32 * 0.04
        + input.entity_overlap.min(5) as f32 * 0.035;
    score += overlap_bonus.min(0.18);

    let dismissed_penalty = (input.dismissed_count.min(8) as f32 * 0.035).min(0.24);
    let duplicate_penalty = input.duplicate_penalty.clamp(0.0, 0.40);
    let conflict_penalty = input.conflict_risk.clamp(0.0, 0.35);
    score -= dismissed_penalty + duplicate_penalty + conflict_penalty;

    MemoryUsefulnessScore {
        score: score.clamp(0.0, 1.0),
        duplicate_penalty,
        conflict_penalty,
    }
}

pub fn record_memory_usage_metadata(
    frontmatter: &mut KnowledgeFrontmatter,
    at: DateTime<Utc>,
    injected: bool,
    dismissed: bool,
) -> bool {
    let timestamp = at.to_rfc3339();
    let mut changed = false;
    if dismissed {
        frontmatter.dismissed_count = frontmatter.dismissed_count.saturating_add(1);
        changed = true;
    } else {
        let previous_use_count = frontmatter.use_count;
        frontmatter.use_count = frontmatter.use_count.saturating_add(1);
        if frontmatter.use_count != previous_use_count {
            changed = true;
        }
        if frontmatter.last_used_at.as_deref() != Some(timestamp.as_str()) {
            frontmatter.last_used_at = Some(timestamp.clone());
            changed = true;
        }
    }
    if injected && frontmatter.last_injected_at.as_deref() != Some(timestamp.as_str()) {
        frontmatter.last_injected_at = Some(timestamp);
        changed = true;
    }
    changed
}

pub fn detect_memory_lifecycle_ops(
    docs: &[MemoryDocSnapshot],
    now: DateTime<Utc>,
) -> Vec<MemoryLifecycleOp> {
    let mut docs: Vec<MemoryDocSnapshot> = docs
        .iter()
        .cloned()
        .map(MemoryDocSnapshot::normalized)
        .filter(|doc| doc.status != "archived" && doc.status != "deprecated")
        .collect();
    docs.sort_by(|a, b| {
        a.stable_key()
            .cmp(&b.stable_key())
            .then_with(|| a.path.cmp(&b.path))
    });
    docs.truncate(MAX_MEMORY_LIFECYCLE_DOCS);

    let mut ops = Vec::new();
    let mut merge_groups = BTreeSet::new();

    let mut by_hash: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (idx, doc) in docs.iter().enumerate() {
        if !doc.content_hash.is_empty() {
            by_hash
                .entry(doc.content_hash.clone())
                .or_default()
                .push(idx);
        }
    }
    for (hash, group) in by_hash {
        if group.len() < 2 {
            continue;
        }
        if merge_groups.insert(format!("hash:{hash}")) {
            if let Some(op) = build_merge_candidate(
                &docs,
                &group,
                MEMORY_OP_EXACT_DUPLICATE_REASON,
                MEMORY_OP_AUTO_APPLY_MIN_CONFIDENCE,
                now,
            ) {
                ops.push(op);
            }
        }
    }

    let mut by_source: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (idx, doc) in docs.iter().enumerate() {
        for key in source_range_keys(doc) {
            by_source.entry(key).or_default().push(idx);
        }
    }
    for (key, group) in by_source {
        if group.len() < 2 {
            continue;
        }
        if merge_groups.insert(format!("source:{key}")) {
            if let Some(op) = build_merge_candidate(
                &docs,
                &group,
                "same source commit/trajectory range/topic",
                0.82,
                now,
            ) {
                ops.push(op);
            }
        }
    }

    let mut by_title: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (idx, doc) in docs.iter().enumerate() {
        let title = normalized_title_key(&doc.title);
        if !title.is_empty() {
            by_title.entry(title).or_default().push(idx);
        }
    }
    let mut seen_title_pairs = BTreeSet::new();
    for (_title, group) in by_title {
        for i in 0..group.len() {
            for j in (i + 1)..group.len() {
                let a = group[i];
                let b = group[j];
                if overlap_signal(&docs[a], &docs[b]) == 0 {
                    continue;
                }
                let pair_key = pair_key(&docs[a], &docs[b]);
                if !seen_title_pairs.insert(pair_key) {
                    continue;
                }
                if let Some(op) = build_merge_candidate(
                    &docs,
                    &[a, b],
                    "same normalized title with overlapping tags/files/entities",
                    0.78,
                    now,
                ) {
                    ops.push(op);
                }
            }
        }
    }

    let mut seen_conflicts = BTreeSet::new();
    for i in 0..docs.len() {
        for j in (i + 1)..docs.len() {
            let Some(summary) = has_negation_conflict(&docs[i].title, &docs[j].title) else {
                continue;
            };
            let pair_key = pair_key(&docs[i], &docs[j]);
            if !seen_conflicts.insert(pair_key) {
                continue;
            }
            if let Some(op) = build_conflict_candidate(&docs[i], &docs[j], &summary, now) {
                ops.push(op);
            }
        }
    }

    for doc in &docs {
        if review_after_is_past(doc.review_after.as_deref(), now) {
            ops.push(build_review_candidate(
                &[doc],
                "memory passed review_after date",
                0.72,
                now,
            ));
        }
        if doc.status == "proposed" && !doc.protected() && doc.dismissed_count >= 2 {
            ops.push(build_archive_candidate(
                doc,
                "proposed autogenerated memory was repeatedly dismissed",
                0.76,
                now,
            ));
        }
    }

    sort_memory_ops(&mut ops);
    ops.dedup_by(|a, b| a.idempotency_key == b.idempotency_key);
    ops.truncate(MAX_MEMORY_LIFECYCLE_OPS);
    ops
}

pub fn memory_op_auto_apply_eligible(op: &MemoryLifecycleOp) -> bool {
    op.op_type == MemoryOpType::MergeArchive
        && op.confidence >= MEMORY_OP_AUTO_APPLY_MIN_CONFIDENCE
        && op.evidence.starts_with(MEMORY_OP_EXACT_DUPLICATE_REASON)
        && op
            .payload
            .canonical
            .as_ref()
            .is_some_and(|canonical| canonical_body_is_sane(&canonical.content))
}

fn canonical_body_is_sane(content: &str) -> bool {
    content.chars().filter(|c| c.is_alphabetic()).count()
        >= MEMORY_OP_AUTO_APPLY_MIN_ALPHABETIC_CHARS
}

fn best_age_days(input: &MemoryScoreInput, now: DateTime<Utc>) -> Option<i64> {
    input
        .last_used_at
        .as_deref()
        .or(input.last_injected_at.as_deref())
        .or(input.updated_at.as_deref())
        .or(input.created_at.as_deref())
        .and_then(|value| age_days_from_str(Some(value), now))
}

fn recency_score(days: i64) -> f32 {
    match days {
        d if d <= 7 => 0.12,
        d if d <= 30 => 0.08,
        d if d <= 90 => 0.04,
        d if d <= 180 => 0.0,
        d if d <= 365 => -0.08,
        _ => -0.14,
    }
}

fn recent_usage_score(days: i64, max_score: f32) -> f32 {
    match days {
        d if d <= 14 => max_score,
        d if d <= 60 => max_score * 0.6,
        d if d <= 180 => max_score * 0.25,
        _ => 0.0,
    }
}

fn age_days_from_str(value: Option<&str>, now: DateTime<Utc>) -> Option<i64> {
    let parsed = parse_datetime(value?)?;
    Some(now.signed_duration_since(parsed).num_days().max(0))
}

fn parse_datetime(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|| {
            parse_date(Some(value))
                .and_then(|date| date.and_hms_opt(0, 0, 0).map(|dt| dt.and_utc()))
        })
}

fn parse_date(value: Option<&str>) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value?.trim(), "%Y-%m-%d").ok()
}

fn review_after_is_past(value: Option<&str>, now: DateTime<Utc>) -> bool {
    parse_date(value)
        .map(|date| now.date_naive() > date)
        .unwrap_or(false)
}

fn source_range_keys(doc: &MemoryDocSnapshot) -> Vec<String> {
    let mut keys = Vec::new();
    if let (Some(commit), Some(topic)) = (&doc.source_commit, &doc.topic) {
        keys.push(format!(
            "commit:{commit}:topic:{}",
            normalized_title_key(topic)
        ));
    }
    if let (Some(trajectory), Some(range)) = (&doc.source_trajectory_id, &doc.source_message_range)
    {
        keys.push(format!("trajectory:{trajectory}:range:{range}"));
    }
    if let (Some(trajectory), Some(topic)) = (&doc.source_trajectory_id, &doc.topic) {
        keys.push(format!(
            "trajectory:{trajectory}:topic:{}",
            normalized_title_key(topic)
        ));
    }
    keys
}

fn normalized_title_key(title: &str) -> String {
    title
        .to_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn pair_key(a: &MemoryDocSnapshot, b: &MemoryDocSnapshot) -> String {
    let a_key = a.stable_key();
    let b_key = b.stable_key();
    if a_key <= b_key {
        format!("{a_key}\0{b_key}")
    } else {
        format!("{b_key}\0{a_key}")
    }
}

fn overlap_signal(a: &MemoryDocSnapshot, b: &MemoryDocSnapshot) -> usize {
    overlap_count(&a.tags, &b.tags)
        + overlap_count(&a.all_files(), &b.all_files())
        + overlap_count(&a.entities, &b.entities)
}

fn overlap_count(a: &[String], b: &[String]) -> usize {
    let a_set: BTreeSet<&str> = a.iter().map(String::as_str).collect();
    b.iter()
        .filter(|value| a_set.contains(value.as_str()))
        .count()
}

fn choose_canonical_index(
    docs: &[MemoryDocSnapshot],
    indexes: &[usize],
    now: DateTime<Utc>,
) -> usize {
    indexes
        .iter()
        .copied()
        .max_by(|a, b| compare_canonical_docs(&docs[*a], &docs[*b], now))
        .unwrap_or(indexes[0])
}

fn compare_canonical_docs(
    a: &MemoryDocSnapshot,
    b: &MemoryDocSnapshot,
    now: DateTime<Utc>,
) -> Ordering {
    let a_score = score_memory_usefulness(&a.score_input(0.0, 0.0), now).score;
    let b_score = score_memory_usefulness(&b.score_input(0.0, 0.0), now).score;
    a_score
        .total_cmp(&b_score)
        .then_with(|| protection_rank(a).cmp(&protection_rank(b)))
        .then_with(|| b.stable_key().cmp(&a.stable_key()))
}

fn protection_rank(doc: &MemoryDocSnapshot) -> u8 {
    if doc.status == "pinned" {
        3
    } else if doc.source_class() == MemorySourceClass::UserAuthored {
        2
    } else if doc.status == "active" {
        1
    } else {
        0
    }
}

fn build_merge_candidate(
    docs: &[MemoryDocSnapshot],
    indexes: &[usize],
    reason: &str,
    confidence: f32,
    now: DateTime<Utc>,
) -> Option<MemoryLifecycleOp> {
    let mut indexes: Vec<usize> = indexes
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if indexes.len() < 2 {
        return None;
    }
    indexes.sort_by(|a, b| docs[*a].stable_key().cmp(&docs[*b].stable_key()));
    let canonical_index = choose_canonical_index(docs, &indexes, now);
    let canonical = &docs[canonical_index];
    let superseded: Vec<&MemoryDocSnapshot> = indexes
        .iter()
        .filter(|idx| **idx != canonical_index)
        .map(|idx| &docs[*idx])
        .filter(|doc| !doc.protected())
        .collect();
    if superseded.is_empty() {
        return None;
    }

    let superseded_paths: Vec<String> = superseded.iter().map(|doc| doc.path.clone()).collect();
    let superseded_ids: Vec<String> = superseded.iter().map(|doc| doc.stable_key()).collect();
    let all_docs: Vec<&MemoryDocSnapshot> = indexes.iter().map(|idx| &docs[*idx]).collect();
    let mut op = MemoryLifecycleOp::pending(
        deterministic_op_id(
            "merge",
            &[
                reason.to_string(),
                canonical.stable_key(),
                superseded_ids.join(","),
            ],
        ),
        MemorySource::MemoryGarden,
        MemoryOpType::MergeArchive,
        vec![canonical.path.clone()],
        format!(
            "{reason}: canonical={}, superseded={}",
            canonical.stable_key(),
            superseded_ids.join(",")
        ),
        confidence,
        now.to_rfc3339(),
    );
    op.requires_approval = true;
    op.payload.superseded_by = Some(canonical.stable_key());
    op.payload.superseded_paths = superseded_paths;
    op.payload.canonical = Some(MemoryCreatePayload {
        title: Some(canonical.title.clone()).filter(|title| !title.is_empty()),
        content: canonical.content.clone(),
        tags: union_field(&all_docs, |doc| &doc.tags),
        kind: canonical.kind.clone(),
        status: None,
        filenames: union_field(&all_docs, |doc| &doc.filenames),
        related_files: union_field(&all_docs, |doc| &doc.related_files),
        links: union_field(&all_docs, |doc| &doc.links),
        source_commit: canonical.source_commit.clone(),
        review_after: canonical.review_after.clone(),
    });
    op.idempotency_key = compute_idempotency_key(&MemoryOpIdempotencyInput {
        source: op.source,
        op_type: op.op_type,
        target_paths: op.target_paths.clone(),
        tags: op
            .payload
            .canonical
            .as_ref()
            .map(|payload| payload.tags.clone())
            .unwrap_or_default(),
        kind: Some(canonical.kind.clone()),
        source_id: Some(canonical.stable_key()),
        title: Some(canonical.title.clone()),
        content: Some(canonical.content.clone()),
        evidence: Some(op.evidence.clone()),
    });
    Some(op.normalized())
}

fn union_field<F>(docs: &[&MemoryDocSnapshot], field: F) -> Vec<String>
where
    F: Fn(&MemoryDocSnapshot) -> &Vec<String>,
{
    let mut out = BTreeSet::new();
    for doc in docs {
        for value in field(doc) {
            out.insert(value.clone());
        }
    }
    out.into_iter().collect()
}

fn build_conflict_candidate(
    a: &MemoryDocSnapshot,
    b: &MemoryDocSnapshot,
    summary: &str,
    now: DateTime<Utc>,
) -> Option<MemoryLifecycleOp> {
    let a_rank = protection_rank(a);
    let b_rank = protection_rank(b);
    let (targets, evidence) = if a_rank > b_rank && !b.protected() {
        (
            vec![b.path.clone()],
            format!(
                "conflict candidate: {} takes precedence over {}; {}",
                a.stable_key(),
                b.stable_key(),
                summary
            ),
        )
    } else if b_rank > a_rank && !a.protected() {
        (
            vec![a.path.clone()],
            format!(
                "conflict candidate: {} takes precedence over {}; {}",
                b.stable_key(),
                a.stable_key(),
                summary
            ),
        )
    } else {
        (
            vec![a.path.clone(), b.path.clone()],
            format!("conflict candidate: {}; {}", pair_key(a, b), summary),
        )
    };
    if targets.is_empty() {
        return None;
    }
    let mut op = MemoryLifecycleOp::pending(
        deterministic_op_id(
            "conflict",
            &[a.stable_key(), b.stable_key(), summary.to_string()],
        ),
        MemorySource::KnowledgeConflictResolver,
        MemoryOpType::MarkReviewNeeded,
        targets,
        evidence,
        if a_rank != b_rank { 0.82 } else { 0.68 },
        now.to_rfc3339(),
    );
    op.requires_approval = true;
    op.payload.review_after = Some(now.date_naive().format("%Y-%m-%d").to_string());
    Some(op.normalized())
}

fn build_review_candidate(
    docs: &[&MemoryDocSnapshot],
    reason: &str,
    confidence: f32,
    now: DateTime<Utc>,
) -> MemoryLifecycleOp {
    let target_paths: Vec<String> = docs.iter().map(|doc| doc.path.clone()).collect();
    let ids: Vec<String> = docs.iter().map(|doc| doc.stable_key()).collect();
    let mut op = MemoryLifecycleOp::pending(
        deterministic_op_id("review", &[reason.to_string(), ids.join(",")]),
        MemorySource::MemoryGarden,
        MemoryOpType::MarkReviewNeeded,
        target_paths,
        format!("review candidate: {reason}: {}", ids.join(",")),
        confidence,
        now.to_rfc3339(),
    );
    op.requires_approval = true;
    op.payload.review_after = Some(now.date_naive().format("%Y-%m-%d").to_string());
    op.normalized()
}

fn build_archive_candidate(
    doc: &MemoryDocSnapshot,
    reason: &str,
    confidence: f32,
    now: DateTime<Utc>,
) -> MemoryLifecycleOp {
    let mut op = MemoryLifecycleOp::pending(
        deterministic_op_id("archive", &[reason.to_string(), doc.stable_key()]),
        MemorySource::MemoryGarden,
        MemoryOpType::ArchiveCandidate,
        vec![doc.path.clone()],
        format!("archive candidate: {reason}: {}", doc.stable_key()),
        confidence,
        now.to_rfc3339(),
    );
    op.requires_approval = true;
    op.normalized()
}

fn deterministic_op_id(prefix: &str, parts: &[String]) -> String {
    let mut h = Sha256::new();
    hash_field(&mut h, "prefix", prefix);
    for part in parts {
        hash_field(&mut h, "part", part);
    }
    format!("memop_{}_{}", prefix, hex::encode(h.finalize()))
}

fn normalized_negation_subject(title: &str) -> Option<(bool, String)> {
    let normalized = title
        .trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let pairs = [
        (true, "do not use "),
        (true, "don't use "),
        (true, "do not "),
        (true, "don't "),
        (true, "avoid "),
        (true, "disable "),
        (false, "use "),
        (false, "enable "),
        (false, "prefer "),
        (false, "do "),
    ];
    for (negated, prefix) in pairs {
        let Some(subject) = normalized.strip_prefix(prefix) else {
            continue;
        };
        let subject = subject
            .trim_matches(|ch: char| ch.is_ascii_punctuation() || ch.is_whitespace())
            .to_string();
        if !subject.is_empty() {
            return Some((negated, subject));
        }
    }
    None
}

fn has_negation_conflict(a_title: &str, b_title: &str) -> Option<String> {
    let (a_negated, a_subject) = normalized_negation_subject(a_title)?;
    let (b_negated, b_subject) = normalized_negation_subject(b_title)?;
    if a_subject == b_subject && a_negated != b_negated {
        return Some(format!("negation subject: {}", a_subject));
    }
    None
}

fn sort_memory_ops(ops: &mut [MemoryLifecycleOp]) {
    ops.sort_by(|a, b| {
        memory_op_sort_rank(a.op_type)
            .cmp(&memory_op_sort_rank(b.op_type))
            .then_with(|| a.target_paths.cmp(&b.target_paths))
            .then_with(|| a.evidence.cmp(&b.evidence))
            .then_with(|| a.idempotency_key.cmp(&b.idempotency_key))
    });
}

fn memory_op_sort_rank(op_type: MemoryOpType) -> u8 {
    match op_type {
        MemoryOpType::MergeArchive => 0,
        MemoryOpType::MarkReviewNeeded => 1,
        MemoryOpType::ArchiveCandidate => 2,
        MemoryOpType::Retag => 3,
        MemoryOpType::RepairLinks => 4,
        MemoryOpType::CreateMemory => 5,
        MemoryOpType::MarkStale => 6,
        MemoryOpType::UpdateMemory => 7,
        MemoryOpType::Refresh => 8,
        MemoryOpType::Archive => 9,
        MemoryOpType::DeleteCandidate => 10,
        MemoryOpType::PromoteDigest => 11,
    }
}

pub fn status_is_pinned_or_user_authored(frontmatter: &KnowledgeFrontmatter) -> bool {
    frontmatter.is_pinned() || memory_source_class(frontmatter) == MemorySourceClass::UserAuthored
}

pub async fn detect_memory_lifecycle_ops_from_knowledge_dirs(
    dirs: &[PathBuf],
    now: DateTime<Utc>,
) -> Vec<MemoryLifecycleOp> {
    let docs = load_memory_doc_snapshots_from_knowledge_dirs(dirs).await;
    detect_memory_lifecycle_ops(&docs, now)
}

pub async fn load_memory_doc_snapshots_from_knowledge_dirs(
    dirs: &[PathBuf],
) -> Vec<MemoryDocSnapshot> {
    let mut docs = Vec::new();
    let mut stack: Vec<PathBuf> = dirs.iter().cloned().collect();
    stack.sort();
    let mut visited_entries = 0usize;
    while let Some(dir) = stack.pop() {
        if visited_entries >= MAX_MEMORY_LIFECYCLE_SCAN_ENTRIES {
            break;
        }
        let Ok(metadata) = tokio::fs::symlink_metadata(&dir).await else {
            continue;
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }
        let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
            continue;
        };
        let mut pending_dirs = Vec::new();
        while let Ok(Some(entry)) = entries.next_entry().await {
            if visited_entries >= MAX_MEMORY_LIFECYCLE_SCAN_ENTRIES
                || docs.len() >= MAX_MEMORY_LIFECYCLE_DOCS
            {
                break;
            }
            visited_entries += 1;
            let path = entry.path();
            let Ok(metadata) = tokio::fs::symlink_metadata(&path).await else {
                continue;
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                pending_dirs.push(path);
                continue;
            }
            if !metadata.is_file()
                || path.extension().and_then(|ext| ext.to_str()) != Some("md")
                || metadata.len() > MAX_MEMORY_LIFECYCLE_FILE_BYTES
            {
                continue;
            }
            let Ok(text) = tokio::fs::read_to_string(&path).await else {
                continue;
            };
            let (frontmatter, body_start) = KnowledgeFrontmatter::parse(&text);
            if frontmatter.is_archived() || frontmatter.is_deprecated() {
                continue;
            }
            let body = text.get(body_start..).unwrap_or("").to_string();
            docs.push(MemoryDocSnapshot::from_parts(path, frontmatter, body));
        }
        pending_dirs.sort();
        stack.extend(pending_dirs.into_iter().rev());
    }
    docs
}

pub async fn propose_supersede_for_near_duplicate(
    gcx: std::sync::Arc<crate::global_context::GlobalContext>,
    new_path: &Path,
) -> Option<MemoryLifecycleOp> {
    let text = tokio::fs::read_to_string(new_path).await.ok()?;
    let (frontmatter, body_start) = KnowledgeFrontmatter::parse(&text);
    if frontmatter.is_archived() || frontmatter.is_deprecated() {
        return None;
    }
    let body = text.get(body_start..).unwrap_or("");
    let mut knowledge_dirs: Vec<PathBuf> = get_project_dirs(gcx.clone())
        .await
        .into_iter()
        .map(|dir| dir.join(KNOWLEDGE_FOLDER_NAME))
        .collect();
    knowledge_dirs.push(get_global_knowledge_dir(gcx.clone()).await);
    let new_path = new_path.to_path_buf();
    if !knowledge_dirs.iter().any(|dir| new_path.starts_with(dir)) {
        return None;
    }
    let vecdb_arc = gcx.vec_db.clone();
    let vecdb = vecdb_arc.lock().await.clone()?;
    let query: String = body.chars().take(2000).collect();
    if query.trim().is_empty() {
        return None;
    }
    let embedding = vecdb.embed_query(&query).await.ok()?;
    let mut hits = Vec::new();
    for kd in &knowledge_dirs {
        let prefix = if kd.to_string_lossy().ends_with(std::path::MAIN_SEPARATOR) {
            kd.to_string_lossy().to_string()
        } else {
            format!("{}{}", kd.to_string_lossy(), std::path::MAIN_SEPARATOR)
        };
        let filter = format!("(scope LIKE '{}%')", prefix.replace('"', "\\\""));
        if let Ok(mut res) = vecdb
            .vecdb_search_with_embedding(&embedding, 8, Some(filter))
            .await
        {
            hits.append(&mut res);
        }
    }
    hits.sort_by(|a, b| a.distance.total_cmp(&b.distance));
    let new_canon = tokio::fs::canonicalize(&new_path).await.ok();
    let mut hit = None;
    for rec in hits {
        if rec.distance > NEAR_DUPLICATE_MAX_DISTANCE
            || rec.file_path.extension().and_then(|ext| ext.to_str()) != Some("md")
            || !knowledge_dirs
                .iter()
                .any(|dir| rec.file_path.starts_with(dir))
            || rec
                .file_path
                .components()
                .any(|c| c.as_os_str() == "trajectories")
            || rec.file_path == new_path
        {
            continue;
        }
        if let Some(new_canon) = &new_canon {
            if tokio::fs::canonicalize(&rec.file_path).await.ok().as_ref() == Some(new_canon) {
                continue;
            }
        }
        hit = Some(rec);
        break;
    }
    let hit = hit?;
    let existing_text = tokio::fs::read_to_string(&hit.file_path).await.ok()?;
    let (existing_frontmatter, existing_body_start) = KnowledgeFrontmatter::parse(&existing_text);
    if existing_frontmatter.is_archived()
        || existing_frontmatter.is_deprecated()
        || existing_frontmatter.is_pinned()
    {
        return None;
    }
    let existing_body = existing_text
        .get(existing_body_start..)
        .unwrap_or("")
        .to_string();
    let new_doc = MemoryDocSnapshot::from_parts(new_path, frontmatter, body.to_string());
    let existing_doc =
        MemoryDocSnapshot::from_parts(hit.file_path, existing_frontmatter, existing_body);
    let docs = vec![new_doc, existing_doc];
    build_merge_candidate(
        &docs,
        &[0, 1],
        MEMORY_OP_NEAR_DUPLICATE_REASON,
        0.85,
        Utc::now(),
    )
}

pub async fn find_memory_by_source(
    gcx: std::sync::Arc<crate::global_context::GlobalContext>,
    source: MemorySource,
    source_id: Option<&str>,
    source_content_hash: Option<&str>,
) -> Option<PathBuf> {
    let source_id = normalize_optional_string(source_id);
    let source_content_hash = normalize_optional_string(source_content_hash);
    if source_id.is_none() && source_content_hash.is_none() {
        return None;
    }
    let roots = KnowledgeRoots {
        local: get_project_dirs(gcx.clone())
            .await
            .into_iter()
            .map(|dir| dir.join(KNOWLEDGE_FOLDER_NAME))
            .collect(),
        global: get_global_knowledge_dir(gcx).await,
    };
    for root in roots.all() {
        let docs = load_memory_doc_snapshots_from_knowledge_dirs(&[root]).await;
        for doc in docs {
            let Ok(text) = tokio::fs::read_to_string(&doc.path).await else {
                continue;
            };
            let (frontmatter, body_start) = KnowledgeFrontmatter::parse(&text);
            // Status is authoritative: skip only docs whose status marks them inactive, so an
            // active/pinned doc with a stale deprecated_at/superseded_by field still dedups.
            if frontmatter.is_archived() || frontmatter.is_deprecated() {
                continue;
            }
            if !source_tool_matches_source(frontmatter.source_tool.as_deref(), source) {
                continue;
            }
            if let Some(wanted) = source_id.as_deref() {
                if frontmatter.source_id.as_deref() == Some(wanted)
                    || (source == MemorySource::Trajectory
                        && frontmatter.source_trajectory_id.as_deref() == Some(wanted))
                {
                    return Some(doc.path.into());
                }
            }
            if let Some(wanted) = source_content_hash.as_deref() {
                if frontmatter.source_content_hash.as_deref() == Some(wanted)
                    || frontmatter.content_hash.as_deref() == Some(wanted)
                    || compute_content_hash(text.get(body_start..).unwrap_or("").trim()) == wanted
                {
                    return Some(doc.path.into());
                }
            }
        }
    }
    None
}

fn source_tool_matches_source(source_tool: Option<&str>, source: MemorySource) -> bool {
    let Some(source_tool) = source_tool.map(str::trim).filter(|tool| !tool.is_empty()) else {
        return false;
    };
    if source_tool == "buddy_memory_create" {
        return source == MemorySource::Buddy;
    }
    if let Some(raw_source) = source_tool.strip_prefix("buddy_memory_lifecycle:") {
        return raw_source == source.as_str();
    }
    source == MemorySource::Trajectory && source_tool.starts_with("trajectory")
}

pub fn detect_git_memory_ops(
    report: &GitHistoryReport,
    docs: &[MemoryDocSnapshot],
    now: DateTime<Utc>,
) -> Vec<MemoryLifecycleOp> {
    let docs = docs
        .iter()
        .cloned()
        .map(MemoryDocSnapshot::normalized)
        .collect::<Vec<_>>();
    let mut ops = Vec::new();
    ops.extend(git_rename_repair_ops(report, &docs, now));
    ops.extend(git_stale_and_revert_ops(report, &docs, now));
    ops.extend(git_commit_create_ops(report, now));
    ops.extend(git_hotspot_create_ops(report, now));
    ops.extend(git_cochange_create_ops(report, now));
    sort_memory_ops(&mut ops);
    ops.dedup_by(|a, b| a.idempotency_key == b.idempotency_key);
    ops.truncate(MAX_GIT_MEMORY_OPS);
    ops
}

fn git_rename_repair_ops(
    report: &GitHistoryReport,
    docs: &[MemoryDocSnapshot],
    now: DateTime<Utc>,
) -> Vec<MemoryLifecycleOp> {
    let mut rename_map = BTreeMap::<String, String>::new();
    let mut rename_commit = BTreeMap::<String, String>::new();
    for commit in &report.commits {
        for change in &commit.changes {
            if change.status != GitFileChangeStatus::Renamed {
                continue;
            }
            let Some(old_path) = &change.old_path else {
                continue;
            };
            rename_map.insert(old_path.clone(), change.path.clone());
            rename_commit.insert(old_path.clone(), commit.short_oid.clone());
        }
    }

    let mut ops = Vec::new();
    for doc in docs {
        if doc.status == "archived" || doc.status == "deprecated" {
            continue;
        }
        let renamed_files = doc
            .filenames
            .iter()
            .filter_map(|path| {
                rename_map
                    .get(path)
                    .map(|new_path| (path.clone(), new_path.clone()))
            })
            .collect::<Vec<_>>();
        let renamed_related = doc
            .related_files
            .iter()
            .filter_map(|path| {
                rename_map
                    .get(path)
                    .map(|new_path| (path.clone(), new_path.clone()))
            })
            .collect::<Vec<_>>();
        if renamed_files.is_empty() && renamed_related.is_empty() {
            continue;
        }
        let new_filenames = rewrite_paths_with_renames(&doc.filenames, &rename_map);
        let new_related = rewrite_paths_with_renames(&doc.related_files, &rename_map);
        let mut evidence_pairs = renamed_files.clone();
        evidence_pairs.extend(renamed_related.clone());
        evidence_pairs.sort();
        evidence_pairs.truncate(MAX_GIT_MEMORY_PATHS);
        let commits = evidence_pairs
            .iter()
            .filter_map(|(old, _)| rename_commit.get(old).cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let evidence = format!(
            "git rename memory repair: commits={} paths={}",
            commits.join(","),
            evidence_pairs
                .iter()
                .map(|(old, new)| format!("{old}->{new}"))
                .collect::<Vec<_>>()
                .join(",")
        );
        let mut op = MemoryLifecycleOp::pending(
            deterministic_op_id("git_repair_links", &[doc.stable_key(), evidence.clone()]),
            MemorySource::Git,
            MemoryOpType::RepairLinks,
            vec![doc.path.clone()],
            evidence,
            0.92,
            now.to_rfc3339(),
        );
        op.requires_approval = false;
        op.payload.filenames = Some(new_filenames);
        op.payload.related_files = Some(new_related);
        op.idempotency_key = compute_idempotency_key(&MemoryOpIdempotencyInput {
            source: op.source,
            op_type: op.op_type,
            target_paths: op.target_paths.clone(),
            tags: Vec::new(),
            kind: None,
            source_id: Some(doc.stable_key()),
            title: Some("git rename repair".to_string()),
            content: None,
            evidence: Some(op.evidence.clone()),
        });
        ops.push(op.normalized());
    }
    ops
}

fn git_stale_and_revert_ops(
    report: &GitHistoryReport,
    docs: &[MemoryDocSnapshot],
    now: DateTime<Utc>,
) -> Vec<MemoryLifecycleOp> {
    let mut changed_paths_by_commit = BTreeMap::<String, Vec<String>>::new();
    let mut reverted_commits = BTreeSet::new();
    for commit in &report.commits {
        let paths = commit_change_paths(commit);
        changed_paths_by_commit.insert(commit.short_oid.clone(), paths.clone());
        if commit
            .classifications
            .contains(&GitCommitClassification::Revert)
        {
            for reverted in parse_reverted_commit_refs(&commit.message) {
                reverted_commits.insert(reverted);
            }
        }
    }

    let mut ops = Vec::new();
    for doc in docs {
        if doc.status == "archived" || doc.status == "deprecated" || doc.protected() {
            continue;
        }
        if let Some(source_commit) = &doc.source_commit {
            if reverted_commits
                .iter()
                .any(|reverted| commit_ref_matches(source_commit, reverted))
            {
                let evidence = format!(
                    "git revert detector: memory {} sourced from reverted commit {}",
                    doc.stable_key(),
                    source_commit
                );
                let mut op = MemoryLifecycleOp::pending(
                    deterministic_op_id(
                        "git_revert_stale",
                        &[doc.stable_key(), source_commit.clone()],
                    ),
                    MemorySource::Git,
                    MemoryOpType::MarkStale,
                    vec![doc.path.clone()],
                    evidence,
                    0.88,
                    now.to_rfc3339(),
                );
                op.requires_approval = true;
                op.payload.review_after = Some(now.date_naive().format("%Y-%m-%d").to_string());
                ops.push(op.normalized());
                continue;
            }
        }
        let files = doc.all_files();
        if files.is_empty() {
            continue;
        }
        let mut overlapping_commits = Vec::new();
        for commit in &report.commits {
            let paths = changed_paths_by_commit
                .get(&commit.short_oid)
                .cloned()
                .unwrap_or_default();
            if paths.iter().any(|path| files.contains(path)) {
                overlapping_commits.push(commit.short_oid.clone());
            }
            if overlapping_commits.len() >= 3 {
                break;
            }
        }
        if overlapping_commits.len() < 2 {
            continue;
        }
        let evidence = format!(
            "stale memory after code change: commits={} memory={} paths={}",
            overlapping_commits.join(","),
            doc.stable_key(),
            files
                .into_iter()
                .take(MAX_GIT_MEMORY_PATHS)
                .collect::<Vec<_>>()
                .join(",")
        );
        let mut op = MemoryLifecycleOp::pending(
            deterministic_op_id("git_review", &[doc.stable_key(), evidence.clone()]),
            MemorySource::Git,
            MemoryOpType::MarkReviewNeeded,
            vec![doc.path.clone()],
            evidence,
            0.73,
            now.to_rfc3339(),
        );
        op.requires_approval = true;
        op.payload.review_after = Some(now.date_naive().format("%Y-%m-%d").to_string());
        ops.push(op.normalized());
    }
    ops
}

fn git_commit_create_ops(report: &GitHistoryReport, now: DateTime<Utc>) -> Vec<MemoryLifecycleOp> {
    let mut lesson_count = 0usize;
    let mut decision_count = 0usize;
    let mut ops = Vec::new();
    for commit in &report.commits {
        let is_lesson = commit.classifications.iter().any(|class| {
            matches!(
                class,
                GitCommitClassification::Bugfix | GitCommitClassification::Revert
            )
        });
        if is_lesson && lesson_count < MAX_GIT_CREATE_OPS_PER_KIND {
            if let Some(op) = git_commit_memory_create_op(commit, "lesson", now) {
                ops.push(op);
                lesson_count += 1;
            }
        }
        let is_decision = commit.classifications.iter().any(|class| {
            matches!(
                class,
                GitCommitClassification::Decision
                    | GitCommitClassification::Rationale
                    | GitCommitClassification::Migration
            )
        });
        if is_decision && decision_count < MAX_GIT_CREATE_OPS_PER_KIND {
            if let Some(op) = git_commit_memory_create_op(commit, "decision", now) {
                ops.push(op);
                decision_count += 1;
            }
        }
    }
    ops
}

fn git_commit_memory_create_op(
    commit: &GitCommitSummary,
    kind: &str,
    now: DateTime<Utc>,
) -> Option<MemoryLifecycleOp> {
    if commit.message.trim().is_empty() {
        return None;
    }
    let paths = commit_change_paths(commit)
        .into_iter()
        .take(MAX_GIT_MEMORY_PATHS)
        .collect::<Vec<_>>();
    let title = format!("Git {} from {}", kind, commit.short_oid);
    let content = format!(
        "{}\n\nSource commit: {}\nPaths: {}\nSummary: {}",
        title,
        commit.short_oid,
        paths.join(", "),
        commit.message
    );
    let evidence = format!(
        "commit {} classified as {} paths={} message={}",
        commit.short_oid,
        kind,
        paths.join(","),
        commit.message
    );
    let mut op = MemoryLifecycleOp::pending(
        deterministic_op_id(&format!("git_{kind}"), &[commit.oid.clone()]),
        MemorySource::Git,
        MemoryOpType::CreateMemory,
        Vec::new(),
        evidence,
        if kind == "lesson" { 0.86 } else { 0.82 },
        now.to_rfc3339(),
    );
    op.requires_approval = true;
    op.payload.canonical = Some(MemoryCreatePayload {
        title: Some(title),
        content,
        tags: vec!["git".to_string(), kind.to_string()],
        kind: kind.to_string(),
        status: Some("proposed".to_string()),
        filenames: paths,
        related_files: Vec::new(),
        links: Vec::new(),
        source_commit: Some(commit.oid.clone()),
        review_after: Some(default_review_after_date(
            now.date_naive(),
            kind,
            MemorySource::Git,
            MemoryCandidateStatus::Proposed,
        )),
    });
    op.idempotency_key = compute_idempotency_key(&MemoryOpIdempotencyInput {
        source: op.source,
        op_type: op.op_type,
        target_paths: Vec::new(),
        tags: vec!["git".to_string(), kind.to_string()],
        kind: Some(kind.to_string()),
        source_id: Some(commit.oid.clone()),
        title: None,
        content: None,
        evidence: None,
    });
    Some(op.normalized())
}

fn git_hotspot_create_ops(report: &GitHistoryReport, now: DateTime<Utc>) -> Vec<MemoryLifecycleOp> {
    report
        .hotspots
        .iter()
        .take(MAX_GIT_CREATE_OPS_PER_KIND)
        .filter(|hotspot| hotspot.edit_count >= 3 || hotspot.score >= 100)
        .map(|hotspot| git_hotspot_create_op(hotspot, now))
        .collect()
}

fn git_hotspot_create_op(hotspot: &GitHotspot, now: DateTime<Utc>) -> MemoryLifecycleOp {
    let source_id = git_hotspot_source_id(&hotspot.path);
    let title = format!("Git hotspot: {}", hotspot.path);
    let content = format!(
        "{}\n\nRepeated edits: {}\nApproximate churn: +{} -{}\nLatest commit: {}",
        title, hotspot.edit_count, hotspot.additions, hotspot.deletions, hotspot.latest_commit
    );
    let evidence = format!(
        "hotspot score={} edits={} path={} latest_commit={}",
        hotspot.score, hotspot.edit_count, hotspot.path, hotspot.latest_commit
    );
    let mut op = MemoryLifecycleOp::pending(
        deterministic_op_id("git_hotspot", &[source_id.clone()]),
        MemorySource::Git,
        MemoryOpType::CreateMemory,
        Vec::new(),
        evidence,
        0.74,
        now.to_rfc3339(),
    );
    op.requires_approval = true;
    op.payload.canonical = Some(MemoryCreatePayload {
        title: Some(title),
        content,
        tags: vec!["git".to_string(), "hotspot".to_string(), "code".to_string()],
        kind: "code".to_string(),
        status: Some("proposed".to_string()),
        filenames: vec![hotspot.path.clone()],
        related_files: Vec::new(),
        links: Vec::new(),
        source_commit: Some(hotspot.latest_commit.clone()),
        review_after: Some(default_review_after_date(
            now.date_naive(),
            "code",
            MemorySource::Git,
            MemoryCandidateStatus::Proposed,
        )),
    });
    op.idempotency_key = compute_idempotency_key(&MemoryOpIdempotencyInput {
        source: op.source,
        op_type: op.op_type,
        target_paths: vec![hotspot.path.clone()],
        tags: vec!["git".to_string(), "hotspot".to_string(), "code".to_string()],
        kind: Some("code".to_string()),
        source_id: Some(source_id),
        title: None,
        content: None,
        evidence: None,
    });
    op.normalized()
}

fn git_cochange_create_ops(
    report: &GitHistoryReport,
    now: DateTime<Utc>,
) -> Vec<MemoryLifecycleOp> {
    report
        .cochanges
        .iter()
        .take(MAX_GIT_CREATE_OPS_PER_KIND)
        .map(|pair| git_cochange_create_op(pair, now))
        .collect()
}

fn git_cochange_create_op(pair: &GitCoChangePair, now: DateTime<Utc>) -> MemoryLifecycleOp {
    let source_id = git_cochange_source_id(&pair.path_a, &pair.path_b);
    let title = format!("Git co-change pattern: {} + {}", pair.path_a, pair.path_b);
    let content = format!(
        "{}\n\nThese paths changed together {} times in recent history.\nCommits: {}",
        title,
        pair.count,
        pair.commits.join(", ")
    );
    let evidence = format!(
        "co-change count={} paths={},{} commits={}",
        pair.count,
        pair.path_a,
        pair.path_b,
        pair.commits.join(",")
    );
    let mut op = MemoryLifecycleOp::pending(
        deterministic_op_id("git_cochange", &[source_id.clone()]),
        MemorySource::Git,
        MemoryOpType::CreateMemory,
        Vec::new(),
        evidence,
        0.78,
        now.to_rfc3339(),
    );
    op.requires_approval = true;
    op.payload.canonical = Some(MemoryCreatePayload {
        title: Some(title),
        content,
        tags: vec![
            "git".to_string(),
            "cochange".to_string(),
            "pattern".to_string(),
        ],
        kind: "pattern".to_string(),
        status: Some("proposed".to_string()),
        filenames: vec![pair.path_a.clone(), pair.path_b.clone()],
        related_files: Vec::new(),
        links: Vec::new(),
        source_commit: pair.commits.first().cloned(),
        review_after: Some(default_review_after_date(
            now.date_naive(),
            "pattern",
            MemorySource::Git,
            MemoryCandidateStatus::Proposed,
        )),
    });
    op.idempotency_key = compute_idempotency_key(&MemoryOpIdempotencyInput {
        source: op.source,
        op_type: op.op_type,
        target_paths: vec![pair.path_a.clone(), pair.path_b.clone()],
        tags: vec![
            "git".to_string(),
            "cochange".to_string(),
            "pattern".to_string(),
        ],
        kind: Some("pattern".to_string()),
        source_id: Some(source_id),
        title: None,
        content: None,
        evidence: None,
    });
    op.normalized()
}

fn git_hotspot_source_id(path: &str) -> String {
    let path = normalize_path(path).unwrap_or_else(|| path.trim().replace('\\', "/"));
    format!("hotspot:{path}")
}

fn git_cochange_source_id(path_a: &str, path_b: &str) -> String {
    let mut paths = normalize_paths(&[path_a.to_string(), path_b.to_string()]);
    if paths.len() != 2 {
        paths = vec![
            path_a.trim().replace('\\', "/"),
            path_b.trim().replace('\\', "/"),
        ];
        paths.sort();
    }
    format!("cochange:{}:{}", paths[0], paths[1])
}

fn rewrite_paths_with_renames(paths: &[String], renames: &BTreeMap<String, String>) -> Vec<String> {
    normalize_paths(
        &paths
            .iter()
            .map(|path| renames.get(path).cloned().unwrap_or_else(|| path.clone()))
            .collect::<Vec<_>>(),
    )
}

fn commit_change_paths(commit: &GitCommitSummary) -> Vec<String> {
    let mut paths = BTreeSet::new();
    for change in &commit.changes {
        paths.insert(change.path.clone());
        if let Some(old_path) = &change.old_path {
            paths.insert(old_path.clone());
        }
    }
    paths.into_iter().collect()
}

fn parse_reverted_commit_refs(message: &str) -> Vec<String> {
    let mut refs = Vec::new();
    for word in message.split(|ch: char| !(ch.is_ascii_hexdigit())) {
        if word.len() >= 7 && word.len() <= 40 && word.chars().all(|ch| ch.is_ascii_hexdigit()) {
            refs.push(word.to_ascii_lowercase());
        }
    }
    refs.sort();
    refs.dedup();
    refs
}

fn commit_ref_matches(source_commit: &str, reverted: &str) -> bool {
    let source = source_commit.to_ascii_lowercase();
    let reverted = reverted.to_ascii_lowercase();
    source.starts_with(&reverted) || reverted.starts_with(&source)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryApplyOutcome {
    pub status: MemoryOpStatus,
    pub applied_paths: Vec<PathBuf>,
    pub message: Option<String>,
}

impl MemoryApplyOutcome {
    fn applied(paths: Vec<PathBuf>) -> Self {
        Self {
            status: MemoryOpStatus::Applied,
            applied_paths: paths,
            message: None,
        }
    }

    fn skipped(message: impl Into<String>) -> Self {
        Self {
            status: MemoryOpStatus::Skipped,
            applied_paths: Vec::new(),
            message: Some(message.into()),
        }
    }
}

#[derive(Debug, Clone)]
struct KnowledgeRoots {
    local: Vec<PathBuf>,
    global: PathBuf,
}

impl KnowledgeRoots {
    fn all(&self) -> Vec<PathBuf> {
        let mut roots = self.local.clone();
        roots.push(self.global.clone());
        roots
    }
}

pub async fn apply_memory_lifecycle_op(
    gcx: AppState,
    op: &MemoryLifecycleOp,
) -> Result<MemoryApplyOutcome, String> {
    let op = op.clone().normalized();
    if memory_op_status_is_finalized(op.status) {
        return Ok(MemoryApplyOutcome {
            status: op.status,
            applied_paths: Vec::new(),
            message: op.error.clone(),
        });
    }
    if destructive_memory_op(op.op_type) && op.status != MemoryOpStatus::Approved {
        return Err("archive, delete, and merge operations require approval".to_string());
    }
    if op.requires_approval && op.status != MemoryOpStatus::Approved {
        return Err("operation requires approval".to_string());
    }

    match op.op_type {
        MemoryOpType::CreateMemory => apply_create_memory(gcx, &op).await,
        MemoryOpType::Retag => apply_retag(gcx, &op).await,
        MemoryOpType::RepairLinks => apply_repair_links(gcx, &op).await,
        MemoryOpType::MarkReviewNeeded => apply_mark_review_needed(gcx, &op).await,
        MemoryOpType::MarkStale => apply_review_status(gcx, &op, "deprecated").await,
        MemoryOpType::Archive | MemoryOpType::ArchiveCandidate => {
            apply_archive(gcx, &op, None).await
        }
        MemoryOpType::MergeArchive => apply_merge_archive(gcx, &op).await,
        MemoryOpType::DeleteCandidate => {
            Err("hard delete is not supported by memory lifecycle applier".to_string())
        }
        _ => Err(format!(
            "unsupported memory lifecycle operation: {}",
            op.op_type.as_str()
        )),
    }
}

pub async fn apply_memory_lifecycle_op_status(
    gcx: AppState,
    op: &MemoryLifecycleOp,
) -> MemoryLifecycleOp {
    let mut updated = op.clone().normalized();
    if memory_op_status_is_finalized(updated.status) {
        return updated;
    }
    match apply_memory_lifecycle_op(gcx, &updated).await {
        Ok(outcome) => {
            updated.status = outcome.status;
            updated.error = outcome.message;
            if updated.status == MemoryOpStatus::Applied {
                updated.applied_at = Some(Utc::now().to_rfc3339());
            }
        }
        Err(err) => {
            if updated.status == MemoryOpStatus::Pending && apply_error_is_missing_approval(&err) {
                updated.error = None;
            } else {
                updated.status = MemoryOpStatus::Failed;
                updated.error = Some(err);
            }
        }
    }
    updated
}

fn apply_error_is_missing_approval(err: &str) -> bool {
    err.contains("requires approval") || err.contains("require approval")
}

fn destructive_memory_op(op_type: MemoryOpType) -> bool {
    matches!(
        op_type,
        MemoryOpType::ArchiveCandidate
            | MemoryOpType::Archive
            | MemoryOpType::MergeArchive
            | MemoryOpType::DeleteCandidate
    )
}

async fn rewrite_memory_document(
    path: &Path,
    frontmatter: KnowledgeFrontmatter,
    content: &str,
    extras: &[(&str, String)],
) -> Result<(), String> {
    debug_assert!(extras.is_empty());
    let mapping = serde_yaml::to_value(&frontmatter)
        .ok()
        .and_then(|value| value.as_mapping().cloned())
        .unwrap_or_default();
    let yaml = serde_yaml::to_string(&mapping)
        .map_err(|e| format!("failed to serialize memory document: {e}"))?;
    tokio::fs::write(path, format!("---\n{}---\n\n{}", yaml, content.trim()))
        .await
        .map_err(|e| format!("failed to rewrite memory document: {e}"))
}

async fn apply_create_memory(
    gcx: AppState,
    op: &MemoryLifecycleOp,
) -> Result<MemoryApplyOutcome, String> {
    let payload = op
        .payload
        .canonical
        .clone()
        .unwrap_or_else(|| MemoryCreatePayload {
            title: op.payload.title.clone(),
            content: op
                .payload
                .content
                .clone()
                .unwrap_or_else(|| op.evidence.clone()),
            tags: op.payload.tags.clone().unwrap_or_default(),
            kind: op
                .payload
                .kind
                .clone()
                .unwrap_or_else(|| "domain".to_string()),
            status: None,
            filenames: op.payload.filenames.clone().unwrap_or_default(),
            related_files: op.payload.related_files.clone().unwrap_or_default(),
            links: op.payload.links.clone().unwrap_or_default(),
            source_commit: None,
            review_after: op.payload.review_after.clone(),
        })
        .normalized();

    let content = if payload.content.trim().is_empty() {
        return Err("create_memory payload content is empty".to_string());
    } else {
        payload.content.trim().to_string()
    };

    let source_content_hash = op
        .payload
        .source_content_hash
        .clone()
        .unwrap_or_else(|| compute_content_hash(&content));
    if let Some(path) = find_memory_by_source(
        gcx.gcx.clone(),
        op.source,
        op.payload.source_id.as_deref(),
        Some(&source_content_hash),
    )
    .await
    {
        return Ok(MemoryApplyOutcome::skipped(format!(
            "source memory already exists at {}",
            path.display()
        )));
    }

    let mut tags = payload.tags.clone();
    if tags.is_empty() {
        tags.push("memory".to_string());
    }
    let links = payload.links.clone();
    let mut frontmatter = create_frontmatter(
        payload.title.as_deref(),
        &tags,
        &payload.filenames,
        &links,
        &payload.kind,
    );
    frontmatter.related_files = payload.related_files;
    frontmatter.status = Some(if let Some(status) = payload.status {
        status
    } else if op.source.is_autonomous()
        && !(op.status == MemoryOpStatus::Approved
            || (!op.requires_approval && op.confidence >= HIGH_CONFIDENCE_APPROVAL_THRESHOLD))
    {
        "proposed".to_string()
    } else {
        "active".to_string()
    });
    if let Some(review_after) = payload.review_after {
        frontmatter.review_after = Some(review_after);
    }
    if let Some(source_commit) = payload.source_commit {
        frontmatter.source_commit = Some(source_commit);
    }
    frontmatter.source_tool = Some(format!("buddy_memory_lifecycle:{}", op.source.as_str()));
    frontmatter.source_confidence = Some(op.confidence);
    frontmatter.source_trajectory_id = op
        .payload
        .source_id
        .clone()
        .filter(|_| op.source == MemorySource::Trajectory);
    frontmatter.source_message_range = op.payload.source_message_range.clone();
    frontmatter.content_hash = Some(source_content_hash.clone());
    frontmatter.source_id = op.payload.source_id.clone();
    frontmatter.source_content_hash = Some(source_content_hash);

    let path = memories_add(gcx.gcx.clone(), &frontmatter, &content).await?;
    if let Some(supersede_op) = propose_supersede_for_near_duplicate(gcx.gcx.clone(), &path).await {
        let roots = get_project_dirs(gcx.gcx.clone()).await;
        if let Some(project_root) = roots
            .iter()
            .find(|root| path.starts_with(root))
            .or_else(|| roots.first())
        {
            if let Err(err) =
                crate::buddy::storage::enqueue_memory_op(project_root, supersede_op).await
            {
                tracing::warn!("failed to enqueue near-duplicate memory op: {}", err);
            }
        }
    }
    Ok(MemoryApplyOutcome::applied(vec![path]))
}

async fn apply_retag(gcx: AppState, op: &MemoryLifecycleOp) -> Result<MemoryApplyOutcome, String> {
    let tags = op
        .payload
        .tags
        .clone()
        .ok_or_else(|| "retag payload missing tags".to_string())?;
    let roots = knowledge_roots(gcx.clone()).await;
    let mut paths = Vec::new();
    for target in &op.target_paths {
        let path = validate_existing_memory_path(target, &roots).await?;
        let changed = update_memory_document_frontmatter(gcx.gcx.clone(), &path, |frontmatter| {
            let new_tags = normalize_memory_tags(&tags, 16);
            if frontmatter.tags == new_tags {
                return Ok(false);
            }
            frontmatter.tags = new_tags;
            frontmatter.updated = Some(today_string());
            Ok(true)
        })
        .await?;
        if changed {
            paths.push(path);
        }
    }
    if paths.is_empty() {
        Ok(MemoryApplyOutcome::skipped("retag already applied"))
    } else {
        Ok(MemoryApplyOutcome::applied(paths))
    }
}

async fn apply_repair_links(
    gcx: AppState,
    op: &MemoryLifecycleOp,
) -> Result<MemoryApplyOutcome, String> {
    if op.payload.filenames.is_none()
        && op.payload.related_files.is_none()
        && op.payload.links.is_none()
    {
        return Err("repair_links payload has no link fields".to_string());
    }
    let roots = knowledge_roots(gcx.clone()).await;
    let mut paths = Vec::new();
    for target in &op.target_paths {
        let path = validate_existing_memory_path(target, &roots).await?;
        let changed = update_memory_document_frontmatter(gcx.gcx.clone(), &path, |frontmatter| {
            let old = (
                frontmatter.filenames.clone(),
                frontmatter.related_files.clone(),
                frontmatter.links.clone(),
            );
            if let Some(filenames) = &op.payload.filenames {
                frontmatter.filenames = filenames.clone();
            }
            if let Some(related_files) = &op.payload.related_files {
                frontmatter.related_files = related_files.clone();
            }
            if let Some(links) = &op.payload.links {
                frontmatter.links = links.clone();
            }
            let new = (
                frontmatter.filenames.clone(),
                frontmatter.related_files.clone(),
                frontmatter.links.clone(),
            );
            if old == new {
                return Ok(false);
            }
            frontmatter.updated = Some(today_string());
            Ok(true)
        })
        .await?;
        if changed {
            paths.push(path);
        }
    }
    if paths.is_empty() {
        Ok(MemoryApplyOutcome::skipped("links already repaired"))
    } else {
        Ok(MemoryApplyOutcome::applied(paths))
    }
}

async fn apply_review_status(
    gcx: AppState,
    op: &MemoryLifecycleOp,
    status: &str,
) -> Result<MemoryApplyOutcome, String> {
    let roots = knowledge_roots(gcx.clone()).await;
    let status = normalize_memory_status(Some(status));
    let review_after = op.payload.review_after.clone().unwrap_or_else(today_string);
    let mut paths = Vec::new();
    for target in &op.target_paths {
        let path = validate_existing_memory_path(target, &roots).await?;
        let changed = update_memory_document_frontmatter(gcx.gcx.clone(), &path, |frontmatter| {
            if frontmatter.status.as_deref() == Some(status.as_str())
                && frontmatter.review_after.as_deref() == Some(review_after.as_str())
                && (status == "deprecated") == frontmatter.deprecated_at.is_some()
                && (status == "deprecated" || frontmatter.superseded_by.is_none())
            {
                return Ok(false);
            }
            frontmatter.status = Some(status.to_string());
            frontmatter.review_after = Some(review_after.clone());
            if status == "deprecated" {
                frontmatter.deprecated_at = Some(today_string());
            } else {
                frontmatter.deprecated_at = None;
                frontmatter.superseded_by = None;
            }
            frontmatter.updated = Some(today_string());
            Ok(true)
        })
        .await?;
        if changed {
            paths.push(path);
        }
    }
    if paths.is_empty() {
        Ok(MemoryApplyOutcome::skipped("review status already applied"))
    } else {
        Ok(MemoryApplyOutcome::applied(paths))
    }
}

async fn apply_mark_review_needed(
    gcx: AppState,
    op: &MemoryLifecycleOp,
) -> Result<MemoryApplyOutcome, String> {
    let roots = knowledge_roots(gcx.clone()).await;
    let review_after = op.payload.review_after.clone().unwrap_or_else(today_string);
    let mut paths = Vec::new();
    for target in &op.target_paths {
        let path = validate_existing_memory_path(target, &roots).await?;
        let text = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| format!("failed to read memory for review mark: {e}"))?;
        let (frontmatter, body_start) = KnowledgeFrontmatter::parse(&text);
        let body = text.get(body_start..).unwrap_or("").trim().to_string();
        if frontmatter.review_after.as_deref() == Some(review_after.as_str())
            && frontmatter.review_needed == Some(true)
        {
            continue;
        }
        let mut updated = frontmatter;
        updated.review_after = Some(review_after.clone());
        updated.review_needed = Some(true);
        updated.updated = Some(today_string());
        rewrite_memory_document(&path, updated, &body, &[]).await?;
        paths.push(path);
    }
    if paths.is_empty() {
        Ok(MemoryApplyOutcome::skipped("review mark already applied"))
    } else {
        Ok(MemoryApplyOutcome::applied(paths))
    }
}

async fn apply_archive(
    gcx: AppState,
    op: &MemoryLifecycleOp,
    superseded_by: Option<&str>,
) -> Result<MemoryApplyOutcome, String> {
    let roots = knowledge_roots(gcx.clone()).await;
    let mut paths = Vec::new();
    for target in &op.target_paths {
        let path = validate_existing_memory_path(target, &roots).await?;
        let changed = archive_memory_file(
            gcx.clone(),
            &path,
            superseded_by.or(op.payload.superseded_by.as_deref()),
        )
        .await?;
        if changed {
            paths.push(path);
        }
    }
    if paths.is_empty() {
        Ok(MemoryApplyOutcome::skipped("memory already archived"))
    } else {
        Ok(MemoryApplyOutcome::applied(paths))
    }
}

async fn apply_merge_archive(
    gcx: AppState,
    op: &MemoryLifecycleOp,
) -> Result<MemoryApplyOutcome, String> {
    if op.status != MemoryOpStatus::Approved {
        return Err("merge_archive requires approval".to_string());
    }

    let canonical = op
        .payload
        .canonical
        .clone()
        .ok_or_else(|| "merge_archive payload missing canonical memory".to_string())?
        .normalized();
    if canonical.content.trim().is_empty() {
        return Err("merge_archive canonical content is empty".to_string());
    }

    let roots = knowledge_roots(gcx.clone()).await;
    let superseded_targets = if op.payload.superseded_paths.is_empty() {
        op.target_paths.clone()
    } else {
        op.payload.superseded_paths.clone()
    };
    let mut superseded_paths = Vec::new();
    for target in &superseded_targets {
        superseded_paths.push(validate_existing_memory_path(target, &roots).await?);
    }

    let mut tags = canonical.tags.clone();
    if tags.is_empty() {
        tags.push("memory".to_string());
    }
    let mut frontmatter = create_frontmatter(
        canonical.title.as_deref(),
        &tags,
        &canonical.filenames,
        &canonical.links,
        &canonical.kind,
    );
    frontmatter.related_files = canonical.related_files;
    frontmatter.content_hash = Some(compute_content_hash(&canonical.content));
    frontmatter.source_id = op.payload.source_id.clone();
    frontmatter.source_content_hash = op.payload.source_content_hash.clone();
    if let Some(review_after) = canonical.review_after {
        frontmatter.review_after = Some(review_after);
    }
    frontmatter.source_tool = Some(format!("buddy_memory_lifecycle:{}", op.source.as_str()));

    let canonical_path = if let Some(target) = op.target_paths.first() {
        let path = validate_existing_memory_path(target, &roots).await?;
        let existing = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| format!("failed to read canonical memory: {e}"))?;
        let (existing_frontmatter, _) = KnowledgeFrontmatter::parse(&existing);
        frontmatter.id = existing_frontmatter.id.clone();
        frontmatter.status = existing_frontmatter
            .status
            .clone()
            .or(Some("active".to_string()));
        frontmatter.created = existing_frontmatter.created.clone();
        frontmatter.created_at = existing_frontmatter.created_at.clone();
        frontmatter.source_id = frontmatter
            .source_id
            .or(existing_frontmatter.source_id.clone());
        frontmatter.source_content_hash = frontmatter
            .source_content_hash
            .or(existing_frontmatter.source_content_hash.clone());
        // Preserve the canonical's original source identity so exact find_memory_by_source
        // matching keeps working after an in-place merge (do not stamp it as
        // buddy_memory_lifecycle:*).
        if existing_frontmatter.source_tool.is_some() {
            frontmatter.source_tool = existing_frontmatter.source_tool.clone();
        }
        frontmatter.source_trajectory_id = frontmatter
            .source_trajectory_id
            .take()
            .or(existing_frontmatter.source_trajectory_id.clone());
        frontmatter.source_message_range = frontmatter
            .source_message_range
            .take()
            .or(existing_frontmatter.source_message_range.clone());
        frontmatter.source_chat_id = frontmatter
            .source_chat_id
            .take()
            .or(existing_frontmatter.source_chat_id.clone());
        frontmatter.updated = Some(today_string());
        rewrite_memory_document(&path, frontmatter.clone(), canonical.content.trim(), &[]).await?;
        path
    } else {
        memories_add(gcx.gcx.clone(), &frontmatter, canonical.content.trim()).await?
    };
    let canonical_id = frontmatter
        .id
        .clone()
        .unwrap_or_else(|| canonical_path.to_string_lossy().to_string());

    let mut paths = vec![canonical_path];
    for path in superseded_paths {
        if path == paths[0] {
            continue;
        }
        let changed = archive_memory_file(gcx.clone(), &path, Some(&canonical_id)).await?;
        if changed {
            paths.push(path);
        }
    }

    Ok(MemoryApplyOutcome::applied(paths))
}

pub async fn archive_memory_file_checked(
    gcx: AppState,
    path: &Path,
    superseded_by: Option<&str>,
) -> Result<bool, String> {
    let roots = knowledge_roots(gcx.clone()).await;
    reject_unsafe_path(&path.to_string_lossy())?;
    validate_memory_extension(path)?;
    let canonical = canonical_existing_file_no_symlink(path).await?;
    let canonical_roots = canonicalize_existing_roots(&roots).await?;
    if !canonical_roots
        .iter()
        .any(|root| canonical.starts_with(root))
    {
        return Err("memory path is outside knowledge directories".to_string());
    }
    archive_memory_file(gcx, &canonical, superseded_by).await
}

async fn archive_memory_file(
    gcx: AppState,
    path: &Path,
    superseded_by: Option<&str>,
) -> Result<bool, String> {
    update_memory_document_frontmatter(gcx.gcx.clone(), path, |frontmatter| {
        if frontmatter.is_archived() {
            return Ok(false);
        }
        frontmatter.status = Some("archived".to_string());
        frontmatter.deprecated_at = Some(today_string());
        frontmatter.updated = Some(today_string());
        if let Some(superseded_by) = superseded_by {
            frontmatter.superseded_by = Some(superseded_by.to_string());
        }
        Ok(true)
    })
    .await
}

async fn knowledge_roots(gcx: AppState) -> KnowledgeRoots {
    let local = get_project_dirs(gcx.gcx.clone())
        .await
        .into_iter()
        .map(|dir| dir.join(KNOWLEDGE_FOLDER_NAME))
        .collect();
    let global = get_global_knowledge_dir(gcx.gcx.clone()).await;
    KnowledgeRoots { local, global }
}

async fn validate_existing_memory_path(
    path: &str,
    roots: &KnowledgeRoots,
) -> Result<PathBuf, String> {
    let normalized = normalize_path(path).ok_or_else(|| "empty memory path".to_string())?;
    reject_unsafe_path(&normalized)?;
    let candidate = PathBuf::from(&normalized);
    validate_memory_extension(&candidate)?;

    let absolute_candidate = if candidate.is_absolute() {
        candidate
    } else {
        let roots_all = roots.all();
        let mut resolved = None;
        for root in &roots_all {
            if normalized.starts_with(&format!("{}/", KNOWLEDGE_FOLDER_NAME)) {
                if let Some(parent) = root.parent() {
                    let candidate = parent.join(&normalized);
                    if candidate.exists() {
                        resolved = Some(candidate);
                        break;
                    }
                }
            }
            let candidate = root.join(&normalized);
            if candidate.exists() {
                resolved = Some(candidate);
                break;
            }
        }
        resolved.ok_or_else(|| format!("memory path not found: {}", normalized))?
    };

    let canonical = canonical_existing_file_no_symlink(&absolute_candidate).await?;
    let canonical_roots = canonicalize_existing_roots(roots).await?;
    if !canonical_roots
        .iter()
        .any(|root| canonical.starts_with(root))
    {
        return Err("memory path outside knowledge directories".to_string());
    }
    Ok(canonical)
}

fn reject_unsafe_path(path: &str) -> Result<(), String> {
    if path.contains('\0') {
        return Err("memory path contains nul byte".to_string());
    }
    let parsed = Path::new(path);
    for component in parsed.components() {
        if matches!(component, Component::ParentDir) {
            return Err("memory path cannot contain '..'".to_string());
        }
    }
    Ok(())
}

fn validate_memory_extension(path: &Path) -> Result<(), String> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("md") | Some("mdx") => Ok(()),
        _ => Err("memory path must be .md or .mdx".to_string()),
    }
}

async fn canonical_existing_file_no_symlink(path: &Path) -> Result<PathBuf, String> {
    let metadata = tokio::fs::symlink_metadata(path)
        .await
        .map_err(|e| format!("memory path not accessible: {}", e))?;
    if metadata.file_type().is_symlink() {
        return Err("memory path cannot be a symlink".to_string());
    }
    if !metadata.is_file() {
        return Err("memory path must be a file".to_string());
    }
    tokio::fs::canonicalize(path)
        .await
        .map(|path| dunce::simplified(&path).to_path_buf())
        .map_err(|e| format!("failed to canonicalize memory path: {}", e))
}

async fn canonicalize_existing_roots(roots: &KnowledgeRoots) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    for root in roots.all() {
        if !root.exists() {
            continue;
        }
        let metadata = tokio::fs::symlink_metadata(&root)
            .await
            .map_err(|e| format!("knowledge root inaccessible: {}", e))?;
        if metadata.file_type().is_symlink() {
            return Err("knowledge root cannot be a symlink".to_string());
        }
        let canonical = tokio::fs::canonicalize(&root)
            .await
            .map(|path| dunce::simplified(&path).to_path_buf())
            .map_err(|e| format!("failed to canonicalize knowledge root: {}", e))?;
        out.push(canonical);
    }
    if out.is_empty() {
        return Err("no knowledge directories available".to_string());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge_graph::kg_structs::KnowledgeFrontmatter;

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

    fn auto_apply_test_op(
        op_type: MemoryOpType,
        confidence: f32,
        evidence: &str,
    ) -> MemoryLifecycleOp {
        let mut op = MemoryLifecycleOp::pending(
            "op-auto",
            MemorySource::MemoryGarden,
            op_type,
            strings(&[".refact/knowledge/item.md"]),
            evidence,
            confidence,
            "2026-05-02T00:00:00Z",
        );
        op.requires_approval = true;
        op.payload.canonical = Some(MemoryCreatePayload {
            content: "Canonical merged body with plenty of alphabetic text.".to_string(),
            ..Default::default()
        });
        op
    }

    #[test]
    fn exact_duplicate_merge_op_is_auto_apply_eligible() {
        let op = auto_apply_test_op(
            MemoryOpType::MergeArchive,
            MEMORY_OP_AUTO_APPLY_MIN_CONFIDENCE,
            &format!(
                "{}: canonical=a, superseded=b",
                MEMORY_OP_EXACT_DUPLICATE_REASON
            ),
        );

        assert!(memory_op_auto_apply_eligible(&op));
    }

    #[test]
    fn auto_apply_eligibility_rejects_lower_confidence() {
        let op = auto_apply_test_op(
            MemoryOpType::MergeArchive,
            MEMORY_OP_AUTO_APPLY_MIN_CONFIDENCE - 0.01,
            &format!(
                "{}: canonical=a, superseded=b",
                MEMORY_OP_EXACT_DUPLICATE_REASON
            ),
        );

        assert!(!memory_op_auto_apply_eligible(&op));
    }

    #[test]
    fn auto_apply_eligibility_rejects_non_merge_archive() {
        let op = auto_apply_test_op(
            MemoryOpType::ArchiveCandidate,
            MEMORY_OP_AUTO_APPLY_MIN_CONFIDENCE,
            &format!(
                "{}: canonical=a, superseded=b",
                MEMORY_OP_EXACT_DUPLICATE_REASON
            ),
        );

        assert!(!memory_op_auto_apply_eligible(&op));
    }

    #[test]
    fn auto_apply_eligibility_rejects_different_evidence_prefix() {
        let op = auto_apply_test_op(
            MemoryOpType::MergeArchive,
            MEMORY_OP_AUTO_APPLY_MIN_CONFIDENCE,
            "near duplicate: canonical=a, superseded=b",
        );

        assert!(!memory_op_auto_apply_eligible(&op));
    }

    #[test]
    fn auto_apply_eligibility_rejects_degenerate_canonical_body() {
        let mut op = auto_apply_test_op(
            MemoryOpType::MergeArchive,
            MEMORY_OP_AUTO_APPLY_MIN_CONFIDENCE,
            &format!(
                "{}: canonical=a, superseded=b",
                MEMORY_OP_EXACT_DUPLICATE_REASON
            ),
        );
        op.payload.canonical = Some(MemoryCreatePayload {
            content: "247".to_string(),
            ..Default::default()
        });

        assert!(!memory_op_auto_apply_eligible(&op));

        op.payload.canonical = None;
        assert!(!memory_op_auto_apply_eligible(&op));
    }

    fn legacy_test_op(op_id: &str, evidence: &str, status: MemoryOpStatus) -> MemoryLifecycleOp {
        let mut op = MemoryLifecycleOp::default();
        op.op_id = op_id.to_string();
        op.source = MemorySource::MemoryGarden;
        op.op_type = MemoryOpType::CreateMemory;
        op.target_paths = strings(&[".refact/knowledge/item.md"]);
        op.evidence = evidence.to_string();
        op.confidence = 0.91;
        op.requires_approval = false;
        op.status = status;
        op.created_at = "2026-05-02T00:00:00Z".to_string();
        op
    }

    fn explicit_key_test_op(op_id: &str, key: &str, status: MemoryOpStatus) -> MemoryLifecycleOp {
        let mut op = test_op(op_id, key, status);
        op.idempotency_key = key.to_string();
        op
    }

    fn assert_no_raw_secret(text: &str) {
        assert!(!text.contains("password=secret"));
        assert!(!text.contains("secret"));
        assert!(!text.contains("ghp_AbCdEfGhIj1234567890"));
    }

    async fn test_gcx_with_workspace(dir: &Path) -> AppState {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        {
            *crate::app_state::AppState::from_gcx(gcx.clone())
                .await
                .workspace
                .documents_state
                .workspace_folders
                .lock()
                .unwrap() = vec![dir.to_path_buf()];
        }
        AppState::from_gcx(gcx).await
    }

    fn frontmatter_and_body(text: &str) -> (KnowledgeFrontmatter, String) {
        let (frontmatter, content_start) = KnowledgeFrontmatter::parse(text);
        (frontmatter, text[content_start..].trim().to_string())
    }

    async fn write_memory_file(path: &Path, frontmatter: KnowledgeFrontmatter, body: &str) {
        tokio::fs::write(path, format!("{}\n\n{}", frontmatter.to_yaml(), body))
            .await
            .unwrap();
    }

    fn active_frontmatter(title: &str, tags: &[&str]) -> KnowledgeFrontmatter {
        KnowledgeFrontmatter {
            id: Some(title.to_string()),
            title: Some(title.to_string()),
            status: Some("active".to_string()),
            tags: strings(tags),
            created: Some("2026-05-02".to_string()),
            updated: Some("2026-05-02".to_string()),
            kind: Some("domain".to_string()),
            ..Default::default()
        }
    }

    fn fixed_now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-05-02T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn snapshot(
        id: &str,
        title: &str,
        content: &str,
        tags: &[&str],
        files: &[&str],
        status: &str,
        source_class: MemorySourceClass,
    ) -> MemoryDocSnapshot {
        MemoryDocSnapshot {
            id: id.to_string(),
            path: format!("/tmp/{id}.md"),
            title: title.to_string(),
            content: content.to_string(),
            tags: strings(tags),
            filenames: strings(files),
            status: status.to_string(),
            kind: "domain".to_string(),
            source_class: Some(source_class),
            source_confidence: Some(0.8),
            content_hash: compute_content_hash(content),
            created_at: Some("2026-04-01T00:00:00Z".to_string()),
            review_after: Some("2026-08-01".to_string()),
            ..Default::default()
        }
        .normalized()
    }

    fn op_types(ops: &[MemoryLifecycleOp]) -> Vec<MemoryOpType> {
        ops.iter().map(|op| op.op_type).collect()
    }

    fn git_commit(
        oid: &str,
        message: &str,
        classifications: Vec<GitCommitClassification>,
        changes: Vec<crate::git::operations::GitCommitFileChange>,
    ) -> GitCommitSummary {
        GitCommitSummary {
            oid: oid.to_string(),
            short_oid: oid.chars().take(12).collect(),
            time: fixed_now(),
            parent_oids: Vec::new(),
            message: message.to_string(),
            classifications,
            changes,
            file_cap_hit: false,
        }
    }

    fn git_change(path: &str) -> crate::git::operations::GitCommitFileChange {
        crate::git::operations::GitCommitFileChange {
            path: path.to_string(),
            old_path: None,
            status: GitFileChangeStatus::Modified,
            additions: 1,
            deletions: 1,
            binary: false,
        }
    }

    fn git_hotspot(
        path: &str,
        edit_count: usize,
        additions: usize,
        deletions: usize,
        score: u64,
        latest_commit: &str,
    ) -> GitHotspot {
        GitHotspot {
            path: path.to_string(),
            edit_count,
            additions,
            deletions,
            score,
            latest_commit: latest_commit.to_string(),
        }
    }

    fn git_cochange(path_a: &str, path_b: &str, count: usize, commits: &[&str]) -> GitCoChangePair {
        GitCoChangePair {
            path_a: path_a.to_string(),
            path_b: path_b.to_string(),
            count,
            commits: strings(commits),
        }
    }

    #[test]
    fn usefulness_score_monotonicity_prefers_pinned_active_and_proposed_over_stale_duplicate() {
        let now = fixed_now();
        let base = MemoryScoreInput {
            created_at: Some("2026-04-01T00:00:00Z".to_string()),
            source_confidence: Some(0.9),
            tag_overlap: 2,
            ..Default::default()
        };
        let pinned = MemoryScoreInput {
            status: "pinned".to_string(),
            source_class: MemorySourceClass::UserAuthored,
            ..base.clone()
        };
        let active = MemoryScoreInput {
            status: "active".to_string(),
            source_class: MemorySourceClass::AutoGenerated,
            ..base.clone()
        };
        let proposed = MemoryScoreInput {
            status: "proposed".to_string(),
            source_class: MemorySourceClass::AutoGenerated,
            ..base.clone()
        };
        let stale_duplicate = MemoryScoreInput {
            status: "proposed".to_string(),
            source_class: MemorySourceClass::AutoGenerated,
            source_confidence: Some(0.4),
            created_at: Some("2024-01-01T00:00:00Z".to_string()),
            review_after: Some("2024-06-01".to_string()),
            dismissed_count: 4,
            duplicate_penalty: 0.35,
            ..MemoryScoreInput::default()
        };

        let pinned_score = score_memory_usefulness(&pinned, now).score;
        let active_score = score_memory_usefulness(&active, now).score;
        let proposed_score = score_memory_usefulness(&proposed, now).score;
        let stale_duplicate_score = score_memory_usefulness(&stale_duplicate, now).score;

        assert!(pinned_score > active_score);
        assert!(active_score > proposed_score);
        assert!(proposed_score > stale_duplicate_score);
    }

    #[test]
    fn detects_duplicate_by_exact_content_hash() {
        let first = snapshot(
            "a",
            "Memory A",
            "Same body",
            &["buddy"],
            &["src/lib.rs"],
            "active",
            MemorySourceClass::AutoGenerated,
        );
        let mut second = snapshot(
            "b",
            "Memory B",
            "Different spelling",
            &["buddy"],
            &["src/lib.rs"],
            "proposed",
            MemorySourceClass::AutoGenerated,
        );
        second.content_hash = first.content_hash.clone();

        let ops = detect_memory_lifecycle_ops(&[second, first], fixed_now());

        assert!(ops.iter().any(|op| {
            op.op_type == MemoryOpType::MergeArchive
                && op.evidence.contains("exact content_hash duplicate")
        }));
    }

    #[test]
    fn detects_duplicate_by_normalized_title_tags_and_files() {
        let first = snapshot(
            "a",
            "Use Cargo Check",
            "First body",
            &["rust", "buddy"],
            &["refact-agent/engine/src/lib.rs"],
            "active",
            MemorySourceClass::AutoGenerated,
        );
        let second = snapshot(
            "b",
            "use cargo-check",
            "Second body",
            &["buddy"],
            &["refact-agent/engine/src/lib.rs"],
            "proposed",
            MemorySourceClass::AutoGenerated,
        );

        let ops = detect_memory_lifecycle_ops(&[second, first], fixed_now());

        assert!(ops.iter().any(|op| {
            op.op_type == MemoryOpType::MergeArchive
                && op
                    .evidence
                    .contains("same normalized title with overlapping tags/files/entities")
        }));
    }

    #[test]
    fn conflict_precedence_pinned_user_memory_beats_auto_generated_contradiction() {
        let pinned = snapshot(
            "pinned",
            "Use pnpm",
            "User says use pnpm",
            &["package"],
            &[],
            "pinned",
            MemorySourceClass::UserAuthored,
        );
        let auto = snapshot(
            "auto",
            "Avoid pnpm",
            "Generated old advice",
            &["package"],
            &[],
            "proposed",
            MemorySourceClass::AutoGenerated,
        );

        let ops = detect_memory_lifecycle_ops(&[auto, pinned], fixed_now());
        let conflict = ops
            .iter()
            .find(|op| op.evidence.contains("conflict candidate"))
            .expect("conflict op");

        assert_eq!(conflict.op_type, MemoryOpType::MarkReviewNeeded);
        assert_eq!(conflict.target_paths, strings(&["/tmp/auto.md"]));
        assert!(conflict
            .evidence
            .contains("pinned takes precedence over auto"));
    }

    #[test]
    fn usage_metadata_update_tracks_use_injection_and_dismissal() {
        let now = fixed_now();
        let mut frontmatter = active_frontmatter("memory", &["buddy"]);

        assert!(record_memory_usage_metadata(
            &mut frontmatter,
            now,
            true,
            false
        ));
        assert_eq!(frontmatter.use_count, 1);
        assert_eq!(
            frontmatter.last_used_at.as_deref(),
            Some(now.to_rfc3339().as_str())
        );
        assert_eq!(
            frontmatter.last_injected_at.as_deref(),
            Some(now.to_rfc3339().as_str())
        );
        assert!(record_memory_usage_metadata(
            &mut frontmatter,
            now,
            false,
            true
        ));
        assert_eq!(frontmatter.dismissed_count, 1);
        assert_eq!(frontmatter.use_count, 1);
    }

    #[test]
    fn usage_metadata_update_reports_same_timestamp_use_count_change() {
        let now = fixed_now();
        let mut frontmatter = active_frontmatter("memory", &["buddy"]);

        assert!(record_memory_usage_metadata(
            &mut frontmatter,
            now,
            false,
            false
        ));
        assert!(record_memory_usage_metadata(
            &mut frontmatter,
            now,
            false,
            false
        ));

        assert_eq!(frontmatter.use_count, 2);
        assert_eq!(
            frontmatter.last_used_at.as_deref(),
            Some(now.to_rfc3339().as_str())
        );
    }

    #[test]
    fn candidate_output_order_is_deterministic() {
        let a = snapshot(
            "b",
            "Use Y",
            "Body b",
            &["tag"],
            &["file.rs"],
            "proposed",
            MemorySourceClass::AutoGenerated,
        );
        let b = snapshot(
            "a",
            "Use Y",
            "Body a",
            &["tag"],
            &["file.rs"],
            "active",
            MemorySourceClass::AutoGenerated,
        );
        let c = snapshot(
            "c",
            "Avoid Z",
            "Body c",
            &["tag"],
            &[],
            "proposed",
            MemorySourceClass::AutoGenerated,
        );
        let d = snapshot(
            "d",
            "Use Z",
            "Body d",
            &["tag"],
            &[],
            "active",
            MemorySourceClass::AutoGenerated,
        );

        let first =
            detect_memory_lifecycle_ops(&[a.clone(), b.clone(), c.clone(), d.clone()], fixed_now());
        let second = detect_memory_lifecycle_ops(&[d, c, b, a], fixed_now());

        assert_eq!(op_types(&first), op_types(&second));
        assert_eq!(
            first
                .iter()
                .map(|op| op.idempotency_key.clone())
                .collect::<Vec<_>>(),
            second
                .iter()
                .map(|op| op.idempotency_key.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn git_bugfix_commit_creates_lesson_candidate_with_source_sha() {
        let commit = git_commit(
            "abcdef1234567890abcdef1234567890abcdef12",
            "fix parser bug because newline crash",
            vec![
                GitCommitClassification::Bugfix,
                GitCommitClassification::Rationale,
            ],
            vec![git_change("src/parser.rs")],
        );
        let report = GitHistoryReport {
            commits: vec![commit.clone()],
            cochanges: Vec::new(),
            hotspots: Vec::new(),
            commit_cap_hit: false,
        };

        let ops = detect_git_memory_ops(&report, &[], fixed_now());
        let lesson = ops
            .iter()
            .find(|op| {
                op.op_type == MemoryOpType::CreateMemory
                    && op
                        .payload
                        .canonical
                        .as_ref()
                        .map(|payload| payload.kind.as_str() == "lesson")
                        .unwrap_or(false)
            })
            .expect("lesson op");
        let payload = lesson.payload.canonical.as_ref().unwrap();

        assert_eq!(lesson.source, MemorySource::Git);
        assert_eq!(payload.source_commit.as_deref(), Some(commit.oid.as_str()));
        assert_eq!(payload.status.as_deref(), Some("proposed"));
        assert!(payload.content.contains(&commit.short_oid));
    }

    #[test]
    fn git_hotspot_identity_ignores_changed_metrics_and_evidence() {
        let first = git_hotspot_create_op(
            &git_hotspot("src/hot.rs", 3, 10, 5, 100, "aaaaaaaaaaaa"),
            fixed_now(),
        );
        let second = git_hotspot_create_op(
            &git_hotspot("src//hot.rs", 9, 120, 40, 400, "bbbbbbbbbbbb"),
            fixed_now(),
        );

        assert_eq!(first.op_id, second.op_id);
        assert_eq!(first.idempotency_key, second.idempotency_key);
        assert_ne!(first.evidence, second.evidence);
    }

    #[test]
    fn git_cochange_identity_ignores_changed_count_commit_list_and_evidence() {
        let first = git_cochange_create_op(
            &git_cochange("src/a.rs", "src/b.rs", 3, &["aaaaaaaaaaaa", "bbbbbbbbbbbb"]),
            fixed_now(),
        );
        let second = git_cochange_create_op(
            &git_cochange("src//b.rs", "src/a.rs", 9, &["cccccccccccc"]),
            fixed_now(),
        );

        assert_eq!(first.op_id, second.op_id);
        assert_eq!(first.idempotency_key, second.idempotency_key);
        assert_ne!(first.evidence, second.evidence);
    }

    #[test]
    fn git_commit_candidate_idempotency_stays_distinct_by_sha() {
        let first = git_commit(
            "abcdef1234567890abcdef1234567890abcdef12",
            "fix parser bug because newline crash",
            vec![GitCommitClassification::Bugfix],
            vec![git_change("src/parser.rs")],
        );
        let second = git_commit(
            "1234567890abcdef1234567890abcdef12345678",
            "fix parser bug because newline crash",
            vec![GitCommitClassification::Bugfix],
            vec![git_change("src/parser.rs")],
        );
        let first_op = git_commit_memory_create_op(&first, "lesson", fixed_now()).unwrap();
        let second_op = git_commit_memory_create_op(&second, "lesson", fixed_now()).unwrap();

        assert_ne!(first_op.op_id, second_op.op_id);
        assert_ne!(first_op.idempotency_key, second_op.idempotency_key);
    }

    #[test]
    fn git_revert_commit_marks_source_commit_memory_stale() {
        let source = "1234567890abcdef1234567890abcdef12345678";
        let report = GitHistoryReport {
            commits: vec![git_commit(
                "abcdef1234567890abcdef1234567890abcdef12",
                &format!("Revert \"add risky lesson\" This reverts commit {source}."),
                vec![GitCommitClassification::Revert],
                vec![git_change("src/risky.rs")],
            )],
            cochanges: Vec::new(),
            hotspots: Vec::new(),
            commit_cap_hit: false,
        };
        let mut doc = snapshot(
            "git-doc",
            "Risky lesson",
            "Remember risky code",
            &["git"],
            &["src/risky.rs"],
            "proposed",
            MemorySourceClass::AutoGenerated,
        );
        doc.source_commit = Some(source.to_string());

        let ops = detect_git_memory_ops(&report, &[doc], fixed_now());
        let stale = ops
            .iter()
            .find(|op| op.op_type == MemoryOpType::MarkStale)
            .expect("stale op");

        assert_eq!(stale.source, MemorySource::Git);
        assert_eq!(stale.target_paths, strings(&["/tmp/git-doc.md"]));
        assert!(stale.evidence.contains(source));
        assert!(stale.requires_approval);
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
            normalize_path("../outside.md"),
            Some("../outside.md".to_string())
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
    fn lifecycle_status_parser_covers_canonical_statuses_and_aliases() {
        let cases = [
            ("proposed", Some("proposed")),
            ("active", Some("active")),
            ("pinned", Some("pinned")),
            ("archived", Some("archived")),
            ("deprecated", Some("deprecated")),
            ("review", Some("proposed")),
            ("review-needed", Some("proposed")),
            ("review__needed", Some("proposed")),
            ("needs review", Some("proposed")),
            ("needs   review", Some("proposed")),
            ("needs\treview", Some("proposed")),
            ("needs_review", Some("proposed")),
            ("stale", Some("deprecated")),
            ("obsolete", Some("deprecated")),
            ("inactive", Some("archived")),
            ("archive", Some("archived")),
            ("", None),
            ("unknown", None),
        ];

        for (input, expected) in cases {
            assert_eq!(
                parse_memory_lifecycle_status(input),
                expected.map(str::to_string)
            );
        }
    }

    #[test]
    fn lifecycle_status_normalizer_delegates_and_defaults_unknown_to_active() {
        assert_eq!(normalize_memory_status(Some("needs-review")), "proposed");
        assert_eq!(normalize_memory_status(Some("obsolete")), "deprecated");
        assert_eq!(normalize_memory_status(Some("inactive")), "archived");
        assert_eq!(normalize_memory_status(Some("")), "active");
        assert_eq!(normalize_memory_status(Some("unknown")), "active");
        assert_eq!(normalize_memory_status(None), "active");
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
    fn memory_ops_state_same_idempotency_key_with_different_op_id_is_duplicate() {
        let first = explicit_key_test_op("op-1", "semantic-key", MemoryOpStatus::Pending);
        let second = explicit_key_test_op("op-2", "semantic-key", MemoryOpStatus::Applied);

        let state = MemoryOpsState::from_records(vec![
            MemoryOpsRecord::Op { op: first },
            MemoryOpsRecord::Op { op: second.clone() },
        ]);

        assert_eq!(state.ops, vec![second.normalized()]);
        assert_eq!(state.applied_count, 1);
    }

    #[test]
    fn memory_ops_state_missing_incoming_idempotency_key_uses_op_id_fallback() {
        let first = legacy_test_op("op-legacy", "first", MemoryOpStatus::Pending);
        let second = legacy_test_op("op-legacy", "second", MemoryOpStatus::Applied);
        let expected = second.clone().normalized();

        let state = MemoryOpsState::from_records(vec![
            MemoryOpsRecord::Op { op: first },
            MemoryOpsRecord::Op { op: second },
        ]);

        assert_eq!(state.ops, vec![expected]);
        assert_eq!(state.applied_count, 1);
    }

    #[test]
    fn memory_ops_state_different_idempotency_keys_with_same_op_id_are_not_duplicates() {
        let first = explicit_key_test_op("op-collide", "semantic-key-a", MemoryOpStatus::Applied);
        let second = explicit_key_test_op("op-collide", "semantic-key-b", MemoryOpStatus::Pending);

        let state = MemoryOpsState::from_records(vec![
            MemoryOpsRecord::Op { op: first.clone() },
            MemoryOpsRecord::Op { op: second.clone() },
        ]);

        assert_eq!(state.ops, vec![first.normalized(), second.normalized()]);
        assert_eq!(state.applied_count, 1);
        assert_eq!(state.pending_count, 1);
    }

    #[test]
    fn memory_ops_state_existing_finalized_old_key_does_not_suppress_new_key() {
        for status in [MemoryOpStatus::Applied, MemoryOpStatus::Rejected] {
            let first = explicit_key_test_op("op-collide", "old-key", status);
            let second = explicit_key_test_op("op-collide", "new-key", MemoryOpStatus::Pending);

            let state = MemoryOpsState::from_records(vec![
                MemoryOpsRecord::Op { op: first.clone() },
                MemoryOpsRecord::Op { op: second.clone() },
            ]);

            assert_eq!(state.ops, vec![first.normalized(), second.normalized()]);
        }
    }

    #[test]
    fn memory_ops_state_duplicate_pending_does_not_reopen_finalized_or_approved() {
        let statuses = [
            MemoryOpStatus::Applied,
            MemoryOpStatus::Rejected,
            MemoryOpStatus::Skipped,
            MemoryOpStatus::Failed,
            MemoryOpStatus::Approved,
        ];
        for status in statuses {
            let first = test_op(
                &format!("op-{}-first", status.as_str()),
                status.as_str(),
                status,
            );
            let mut pending = test_op(
                &format!("op-{}-pending", status.as_str()),
                "new pending",
                MemoryOpStatus::Pending,
            );
            pending.idempotency_key = first.idempotency_key.clone();

            let state = MemoryOpsState::from_records(vec![
                MemoryOpsRecord::Op { op: first.clone() },
                MemoryOpsRecord::Op { op: pending },
            ]);
            let compacted = MemoryOpsState::from_records(state.canonical_records());

            assert_eq!(state.ops, vec![first.normalized()]);
            assert_eq!(compacted.ops, state.ops);
        }
    }

    #[test]
    fn memory_ops_state_duplicate_pending_replaces_pending() {
        let first = test_op("op-pending-first", "same", MemoryOpStatus::Pending);
        let mut second = test_op("op-pending-second", "new pending", MemoryOpStatus::Pending);
        second.idempotency_key = first.idempotency_key.clone();

        let state = MemoryOpsState::from_records(vec![
            MemoryOpsRecord::Op { op: first },
            MemoryOpsRecord::Op { op: second.clone() },
        ]);

        assert_eq!(state.ops, vec![second.normalized()]);
        assert_eq!(state.pending_count, 1);
    }

    #[test]
    fn memory_ops_state_pending_duplicate_replaced_by_applied() {
        let first = test_op("op-pending-first", "same", MemoryOpStatus::Pending);
        let mut second = test_op("op-applied-second", "applied", MemoryOpStatus::Applied);
        second.idempotency_key = first.idempotency_key.clone();

        let state = MemoryOpsState::from_records(vec![
            MemoryOpsRecord::Op { op: first },
            MemoryOpsRecord::Op { op: second.clone() },
        ]);

        assert_eq!(state.ops, vec![second.normalized()]);
        assert_eq!(state.applied_count, 1);
    }

    #[test]
    fn memory_ops_state_approved_duplicate_replaced_by_finalized() {
        let first = test_op("op-approved-first", "same", MemoryOpStatus::Approved);
        let mut second = test_op("op-rejected-second", "rejected", MemoryOpStatus::Rejected);
        second.idempotency_key = first.idempotency_key.clone();

        let state = MemoryOpsState::from_records(vec![
            MemoryOpsRecord::Op { op: first },
            MemoryOpsRecord::Op { op: second.clone() },
        ]);

        assert_eq!(state.ops, vec![second.normalized()]);
        assert_eq!(state.rejected_count, 1);
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

    #[test]
    fn lifecycle_op_normalizes_evidence_redacts_and_caps() {
        let raw = format!(
            "token ghp_AbCdEfGhIj1234567890 password=secret {}",
            "x".repeat(MEMORY_OP_EVIDENCE_MAX_CHARS * 2)
        );
        let op = MemoryLifecycleOp::pending(
            "op-evidence",
            MemorySource::MemoryGarden,
            MemoryOpType::CreateMemory,
            Vec::new(),
            raw.clone(),
            0.91,
            "2026-05-02T00:00:00Z",
        );

        assert_no_raw_secret(&op.evidence);
        assert!(op.evidence.len() <= MEMORY_OP_EVIDENCE_MAX_CHARS);

        let mut stale = op.clone();
        stale.evidence = raw;
        let normalized = stale.normalized();

        assert_no_raw_secret(&normalized.evidence);
        assert!(normalized.evidence.len() <= MEMORY_OP_EVIDENCE_MAX_CHARS);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn create_memory_op_writes_frontmatter_body_with_normalized_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = test_gcx_with_workspace(dir.path()).await;
        let mut op = MemoryLifecycleOp::pending(
            "op-create",
            MemorySource::BehaviorLearner,
            MemoryOpType::CreateMemory,
            Vec::new(),
            "evidence",
            0.91,
            "2026-05-02T00:00:00Z",
        );
        op.payload = MemoryLifecyclePayload {
            title: Some(" Useful Memory ".to_string()),
            content: Some("# Useful Memory\n\nBody".to_string()),
            tags: Some(strings(&[" Buddy ", "MEMORY", "buddy"])),
            kind: Some("Preference".to_string()),
            filenames: Some(strings(&["src//lib.rs"])),
            related_files: Some(strings(&["src/main.rs"])),
            ..Default::default()
        };

        let outcome = apply_memory_lifecycle_op(gcx, &op).await.unwrap();

        assert_eq!(outcome.status, MemoryOpStatus::Applied);
        assert_eq!(outcome.applied_paths.len(), 1);
        let text = tokio::fs::read_to_string(&outcome.applied_paths[0])
            .await
            .unwrap();
        let (frontmatter, body) = frontmatter_and_body(&text);
        assert_eq!(frontmatter.title.as_deref(), Some("Useful Memory"));
        assert_eq!(frontmatter.tags, strings(&["buddy", "memory"]));
        assert_eq!(frontmatter.kind.as_deref(), Some("preference"));
        assert_eq!(frontmatter.status.as_deref(), Some("active"));
        assert_eq!(frontmatter.filenames, strings(&["src/lib.rs"]));
        assert_eq!(frontmatter.related_files, strings(&["src/main.rs"]));
        assert_eq!(body, "# Useful Memory\n\nBody");
    }

    #[test]
    fn memory_candidate_create_op_normalizes_proposed_metadata() {
        let candidate = MemoryCandidate {
            candidate_id: " candidate-1 ".to_string(),
            source: MemorySource::Trajectory,
            title: " Useful Lesson ".to_string(),
            content: "Body password=secret".to_string(),
            tags: strings(&["Trajectory", "LESSON", "trajectory"]),
            kind: "Decision".to_string(),
            filenames: strings(&["src//lib.rs"]),
            related_files: strings(&["src/main.rs"]),
            source_id: Some(" trajectory-1:0-2 ".to_string()),
            source_message_range: Some(" 0-2 ".to_string()),
            confidence: 0.72,
            status: MemoryCandidateStatus::Proposed,
            ..Default::default()
        };

        let op = candidate.into_create_memory_op(
            "op-candidate",
            "evidence password=secret",
            "2026-05-02T00:00:00Z",
        );

        assert_eq!(op.source, MemorySource::Trajectory);
        assert_eq!(op.op_type, MemoryOpType::CreateMemory);
        assert_eq!(op.status, MemoryOpStatus::Pending);
        assert!(op.requires_approval);
        assert_eq!(op.payload.title.as_deref(), Some("Useful Lesson"));
        assert_eq!(op.payload.kind.as_deref(), Some("decision"));
        assert_eq!(op.payload.review_after.as_deref(), Some("2026-06-01"));
        assert_eq!(op.payload.source_id.as_deref(), Some("trajectory-1:0-2"));
        assert_eq!(op.payload.source_message_range.as_deref(), Some("0-2"));
        assert_eq!(op.payload.filenames.unwrap(), strings(&["src/lib.rs"]));
        assert_eq!(op.payload.related_files.unwrap(), strings(&["src/main.rs"]));
        assert!(op.payload.tags.unwrap().contains(&"memory".to_string()));
        assert!(!op.payload.content.unwrap().contains("secret"));
        assert_no_raw_secret(&op.evidence);
        assert!(op.evidence.len() <= MEMORY_OP_EVIDENCE_MAX_CHARS);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn symlinked_knowledge_root_scan_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let real_root = dir.path().join("real_knowledge");
        tokio::fs::create_dir_all(&real_root).await.unwrap();
        write_memory_file(
            &real_root.join("memory.md"),
            active_frontmatter("memory", &["buddy"]),
            "Body",
        )
        .await;
        let symlink_root = dir.path().join("symlink_knowledge");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real_root, &symlink_root).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&real_root, &symlink_root).unwrap();

        let docs = load_memory_doc_snapshots_from_knowledge_dirs(&[symlink_root]).await;

        assert!(docs.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn popped_symlink_directory_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let scan_root = dir.path().join("scan_root");
        let real_root = dir.path().join("real_root");
        tokio::fs::create_dir_all(&scan_root).await.unwrap();
        tokio::fs::create_dir_all(&real_root).await.unwrap();
        write_memory_file(
            &real_root.join("memory.md"),
            active_frontmatter("memory", &["buddy"]),
            "Body",
        )
        .await;
        let symlink_dir = scan_root.join("linked_dir");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real_root, &symlink_dir).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&real_root, &symlink_dir).unwrap();

        let docs = load_memory_doc_snapshots_from_knowledge_dirs(&[scan_root]).await;

        assert!(docs.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn autonomous_create_memory_defaults_to_proposed_without_approval() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = test_gcx_with_workspace(dir.path()).await;
        let mut op = MemoryLifecycleOp::pending(
            "op-create-proposed",
            MemorySource::MemoryGarden,
            MemoryOpType::CreateMemory,
            Vec::new(),
            "Unapproved autonomous evidence",
            0.50,
            "2026-05-02T00:00:00Z",
        );
        op.requires_approval = false;
        op.payload.content = Some("Autonomous body".to_string());

        let outcome = apply_memory_lifecycle_op(gcx, &op).await.unwrap();
        let text = tokio::fs::read_to_string(&outcome.applied_paths[0])
            .await
            .unwrap();
        let (frontmatter, _) = frontmatter_and_body(&text);

        assert_eq!(frontmatter.status.as_deref(), Some("proposed"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn retag_and_repair_links_preserve_body_and_parseable_yaml() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = test_gcx_with_workspace(dir.path()).await;
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        tokio::fs::create_dir_all(&knowledge_dir).await.unwrap();
        let path = knowledge_dir.join("memory.md");
        let mut frontmatter = active_frontmatter("memory", &["old"]);
        frontmatter.filenames = strings(&["old.rs"]);
        frontmatter.related_files = strings(&["old-related.rs"]);
        frontmatter.links = strings(&["old-link"]);
        write_memory_file(&path, frontmatter, "# Heading\n\nOriginal body").await;

        let mut retag = MemoryLifecycleOp::pending(
            "op-retag",
            MemorySource::MemoryGarden,
            MemoryOpType::Retag,
            vec![path.to_string_lossy().to_string()],
            "retag",
            0.91,
            "2026-05-02T00:00:00Z",
        );
        retag.payload.tags = Some(strings(&["New", "buddy"]));
        apply_memory_lifecycle_op(gcx.clone(), &retag)
            .await
            .unwrap();

        let mut repair = MemoryLifecycleOp::pending(
            "op-repair",
            MemorySource::MemoryGarden,
            MemoryOpType::RepairLinks,
            vec![path.to_string_lossy().to_string()],
            "repair",
            0.91,
            "2026-05-02T00:00:00Z",
        );
        repair.payload.filenames = Some(strings(&["src/lib.rs"]));
        repair.payload.related_files = Some(strings(&["src/main.rs"]));
        repair.payload.links = Some(strings(&["new-link"]));
        apply_memory_lifecycle_op(gcx, &repair).await.unwrap();

        let text = tokio::fs::read_to_string(&path).await.unwrap();
        let (frontmatter, body) = frontmatter_and_body(&text);
        assert_eq!(frontmatter.tags, strings(&["buddy", "new"]));
        assert_eq!(frontmatter.filenames, strings(&["src/lib.rs"]));
        assert_eq!(frontmatter.related_files, strings(&["src/main.rs"]));
        assert_eq!(frontmatter.links, strings(&["new-link"]));
        assert_eq!(body, "# Heading\n\nOriginal body");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn archive_op_preserves_content_and_makes_doc_inactive() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = test_gcx_with_workspace(dir.path()).await;
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        tokio::fs::create_dir_all(&knowledge_dir).await.unwrap();
        let path = knowledge_dir.join("memory.md");
        write_memory_file(
            &path,
            active_frontmatter("memory", &["old"]),
            "Archive body",
        )
        .await;

        let mut op = MemoryLifecycleOp::pending(
            "op-archive",
            MemorySource::MemoryGarden,
            MemoryOpType::Archive,
            vec![path.to_string_lossy().to_string()],
            "archive",
            0.91,
            "2026-05-02T00:00:00Z",
        );
        op.status = MemoryOpStatus::Approved;

        apply_memory_lifecycle_op(gcx.clone(), &op).await.unwrap();

        assert!(path.exists());
        let text = tokio::fs::read_to_string(&path).await.unwrap();
        let (frontmatter, body) = frontmatter_and_body(&text);
        assert_eq!(frontmatter.status.as_deref(), Some("archived"));
        assert_eq!(body, "Archive body");
        let kg = crate::knowledge_graph::build_knowledge_graph(gcx.gcx.clone()).await;
        assert!(kg.active_docs().all(|doc| doc.path != path));
        let found = crate::memories::load_memories_by_tags(gcx.gcx.clone(), &["old"], 10)
            .await
            .unwrap();
        assert!(found.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn review_status_reactivation_clears_archive_metadata() {
        for status in ["active", "proposed", "pinned"] {
            let dir = tempfile::tempdir().unwrap();
            let gcx = test_gcx_with_workspace(dir.path()).await;
            let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
            tokio::fs::create_dir_all(&knowledge_dir).await.unwrap();
            let path = knowledge_dir.join(format!("reactivate-{status}.md"));
            let mut frontmatter = active_frontmatter(status, &["old"]);
            frontmatter.status = Some("deprecated".to_string());
            frontmatter.deprecated_at = Some("2026-05-01".to_string());
            frontmatter.superseded_by = Some("canonical".to_string());
            write_memory_file(&path, frontmatter, "Reactivation body").await;

            let mut op = MemoryLifecycleOp::pending(
                format!("op-reactivate-{status}"),
                MemorySource::MemoryGarden,
                MemoryOpType::MarkReviewNeeded,
                vec![path.to_string_lossy().to_string()],
                "reactivate",
                0.91,
                "2026-05-02T00:00:00Z",
            );
            op.status = MemoryOpStatus::Approved;
            op.payload.review_after = Some("2026-05-03".to_string());

            apply_review_status(gcx, &op, status).await.unwrap();

            let (frontmatter, body) =
                frontmatter_and_body(&tokio::fs::read_to_string(&path).await.unwrap());
            assert_eq!(frontmatter.status.as_deref(), Some(status));
            assert_eq!(frontmatter.deprecated_at, None);
            assert_eq!(frontmatter.superseded_by, None);
            assert_eq!(body, "Reactivation body");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn review_and_stale_ops_persist_canonical_statuses() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = test_gcx_with_workspace(dir.path()).await;
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        tokio::fs::create_dir_all(&knowledge_dir).await.unwrap();
        let review_path = knowledge_dir.join("review.md");
        let stale_path = knowledge_dir.join("stale.md");
        write_memory_file(
            &review_path,
            active_frontmatter("review", &["old"]),
            "Review body",
        )
        .await;
        write_memory_file(
            &stale_path,
            active_frontmatter("stale", &["old"]),
            "Stale body",
        )
        .await;

        let mut review = MemoryLifecycleOp::pending(
            "op-review",
            MemorySource::MemoryGarden,
            MemoryOpType::MarkReviewNeeded,
            vec![review_path.to_string_lossy().to_string()],
            "review",
            0.91,
            "2026-05-02T00:00:00Z",
        );
        review.status = MemoryOpStatus::Approved;
        review.payload.review_after = Some("2026-05-03".to_string());
        apply_memory_lifecycle_op(gcx.clone(), &review)
            .await
            .unwrap();

        let mut stale = MemoryLifecycleOp::pending(
            "op-stale",
            MemorySource::MemoryGarden,
            MemoryOpType::MarkStale,
            vec![stale_path.to_string_lossy().to_string()],
            "stale",
            0.91,
            "2026-05-02T00:00:00Z",
        );
        stale.status = MemoryOpStatus::Approved;
        stale.payload.review_after = Some("2026-05-04".to_string());
        apply_memory_lifecycle_op(gcx, &stale).await.unwrap();

        let (review_frontmatter, review_body) =
            frontmatter_and_body(&tokio::fs::read_to_string(&review_path).await.unwrap());
        assert_eq!(review_frontmatter.status.as_deref(), Some("active"));
        assert_eq!(review_frontmatter.review_needed, Some(true));
        assert_eq!(
            review_frontmatter.review_after.as_deref(),
            Some("2026-05-03")
        );
        assert_eq!(review_frontmatter.deprecated_at, None);
        assert_eq!(review_body, "Review body");

        let (stale_frontmatter, stale_body) =
            frontmatter_and_body(&tokio::fs::read_to_string(&stale_path).await.unwrap());
        assert_eq!(stale_frontmatter.status.as_deref(), Some("deprecated"));
        assert_eq!(
            stale_frontmatter.review_after.as_deref(),
            Some("2026-05-04")
        );
        assert!(stale_frontmatter.deprecated_at.is_some());
        assert_eq!(stale_body, "Stale body");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn invalid_path_traversal_and_symlink_escape_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = test_gcx_with_workspace(dir.path()).await;
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        tokio::fs::create_dir_all(&knowledge_dir).await.unwrap();
        let outside = dir.path().join("outside.md");
        write_memory_file(&outside, active_frontmatter("outside", &["old"]), "Outside").await;
        let link = knowledge_dir.join("link.md");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&outside, &link).unwrap();

        let mut traversal = MemoryLifecycleOp::pending(
            "op-traversal",
            MemorySource::MemoryGarden,
            MemoryOpType::Retag,
            strings(&["../outside.md"]),
            "retag",
            0.91,
            "2026-05-02T00:00:00Z",
        );
        traversal.payload.tags = Some(strings(&["new"]));
        let traversal_err = apply_memory_lifecycle_op(gcx.clone(), &traversal)
            .await
            .unwrap_err();
        assert!(traversal_err.contains(".."));

        let mut symlink = MemoryLifecycleOp::pending(
            "op-symlink",
            MemorySource::MemoryGarden,
            MemoryOpType::Retag,
            vec![link.to_string_lossy().to_string()],
            "retag",
            0.91,
            "2026-05-02T00:00:00Z",
        );
        symlink.payload.tags = Some(strings(&["new"]));
        let symlink_err = apply_memory_lifecycle_op(gcx, &symlink).await.unwrap_err();
        assert!(symlink_err.contains("symlink"));

        let text = tokio::fs::read_to_string(&outside).await.unwrap();
        let (frontmatter, body) = frontmatter_and_body(&text);
        assert_eq!(frontmatter.tags, strings(&["old"]));
        assert_eq!(body, "Outside");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn apply_status_preserves_finalized_ops_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = test_gcx_with_workspace(dir.path()).await;
        for status in [
            MemoryOpStatus::Applied,
            MemoryOpStatus::Rejected,
            MemoryOpStatus::Skipped,
            MemoryOpStatus::Failed,
        ] {
            let mut op = test_op(&format!("op-{}", status.as_str()), status.as_str(), status);
            op.error = Some(format!("{} original", status.as_str()));
            op.applied_at = Some("2026-05-02T00:01:00Z".to_string());

            let updated = apply_memory_lifecycle_op_status(gcx.clone(), &op).await;

            assert_eq!(updated, op.normalized());
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn failed_apply_op_is_direct_noop() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = test_gcx_with_workspace(dir.path()).await;
        let mut op = MemoryLifecycleOp::pending(
            "op-failed",
            MemorySource::BehaviorLearner,
            MemoryOpType::CreateMemory,
            Vec::new(),
            "evidence",
            0.91,
            "2026-05-02T00:00:00Z",
        );
        op.status = MemoryOpStatus::Failed;
        op.error = Some("original failure".to_string());
        op.payload.content = Some("Should not be written".to_string());

        let outcome = apply_memory_lifecycle_op(gcx, &op).await.unwrap();

        assert_eq!(outcome.status, MemoryOpStatus::Failed);
        assert_eq!(outcome.message.as_deref(), Some("original failure"));
        assert!(outcome.applied_paths.is_empty());
        assert!(dir.path().join(KNOWLEDGE_FOLDER_NAME).read_dir().is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pending_approval_required_op_status_remains_pending() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = test_gcx_with_workspace(dir.path()).await;
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        tokio::fs::create_dir_all(&knowledge_dir).await.unwrap();
        let old_path = knowledge_dir.join("old.md");
        write_memory_file(&old_path, active_frontmatter("old", &["old"]), "Old body").await;

        let mut op = MemoryLifecycleOp::pending(
            "op-pending-merge",
            MemorySource::MemoryGarden,
            MemoryOpType::MergeArchive,
            vec![old_path.to_string_lossy().to_string()],
            "merge",
            0.91,
            "2026-05-02T00:00:00Z",
        );
        op.payload.canonical = Some(MemoryCreatePayload {
            title: Some("Canonical".to_string()),
            content: "Canonical body".to_string(),
            tags: strings(&["canonical"]),
            kind: "domain".to_string(),
            ..Default::default()
        });

        let updated = apply_memory_lifecycle_op_status(gcx, &op).await;

        assert_eq!(updated.status, MemoryOpStatus::Pending);
        assert_eq!(updated.error, None);
        assert_eq!(updated.applied_at, None);
        let text = tokio::fs::read_to_string(&old_path).await.unwrap();
        assert_eq!(
            frontmatter_and_body(&text).0.status.as_deref(),
            Some("active")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn failed_apply_leaves_original_file_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = test_gcx_with_workspace(dir.path()).await;
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        tokio::fs::create_dir_all(&knowledge_dir).await.unwrap();
        let path = knowledge_dir.join("memory.md");
        write_memory_file(&path, active_frontmatter("memory", &["old"]), "Body").await;
        let before = tokio::fs::read_to_string(&path).await.unwrap();

        let op = MemoryLifecycleOp::pending(
            "op-fail",
            MemorySource::MemoryGarden,
            MemoryOpType::Retag,
            vec![path.to_string_lossy().to_string()],
            "retag",
            0.91,
            "2026-05-02T00:00:00Z",
        );

        let err = apply_memory_lifecycle_op(gcx, &op).await.unwrap_err();
        assert!(err.contains("missing tags"));
        let after = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(after, before);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn replay_of_applied_op_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = test_gcx_with_workspace(dir.path()).await;
        let mut op = MemoryLifecycleOp::pending(
            "op-applied",
            MemorySource::BehaviorLearner,
            MemoryOpType::CreateMemory,
            Vec::new(),
            "evidence",
            0.91,
            "2026-05-02T00:00:00Z",
        );
        op.status = MemoryOpStatus::Applied;
        op.payload.content = Some("Should not be written".to_string());

        let outcome = apply_memory_lifecycle_op(gcx, &op).await.unwrap();

        assert_eq!(outcome.status, MemoryOpStatus::Applied);
        assert!(dir.path().join(KNOWLEDGE_FOLDER_NAME).read_dir().is_err());
    }
}

#[cfg(test)]
mod buddy_memory_tools_checked_tests {
    use super::*;
    use crate::file_filter::KNOWLEDGE_FOLDER_NAME;

    #[tokio::test]
    async fn archive_memory_file_checked_rejects_dotdot() {
        let dir = tempfile::tempdir().unwrap();
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        tokio::fs::create_dir_all(&knowledge_dir).await.unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        {
            *crate::app_state::AppState::from_gcx(gcx.clone())
                .await
                .workspace
                .documents_state
                .workspace_folders
                .lock()
                .unwrap() = vec![dir.path().to_path_buf()];
        }
        let gcx = AppState::from_gcx(gcx).await;
        let path = knowledge_dir
            .join("..")
            .join("knowledge")
            .join("missing.md");
        let err = archive_memory_file_checked(gcx, &path, None)
            .await
            .unwrap_err();
        assert!(err.contains(".."));
    }
}
