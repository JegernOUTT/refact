use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;

use crate::buddy::types::{BuddyDraft, DraftKind};

pub const DRAFT_TTL: Duration = Duration::hours(2);

#[derive(Debug, Clone, Copy)]
pub enum DraftTarget<'a> {
    Any,
    Id(&'a str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftValidationError {
    NotFound,
    KindMismatch {
        expected: DraftKind,
        actual: DraftKind,
    },
    TargetMismatch {
        expected: String,
        actual: String,
    },
    Parse(String),
}

pub fn draft_kind_str(kind: &DraftKind) -> &'static str {
    match kind {
        DraftKind::Skill => "skill",
        DraftKind::Command => "command",
        DraftKind::Delegate => "delegate",
        DraftKind::Mode => "mode",
        DraftKind::AgentsMd => "agents_md",
        DraftKind::DefaultsModel => "defaults_model",
        DraftKind::Hook => "hook",
        DraftKind::PulseReport => "pulse_report",
    }
}

fn yaml_field(value: &serde_yaml::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn frontmatter_field(content: &str, key: &str) -> Option<String> {
    let (frontmatter, _) = crate::ext::slash_commands::parse_frontmatter_and_body(content);
    frontmatter
        .as_mapping()
        .filter(|m| !m.is_empty())
        .and_then(|_| yaml_field(&frontmatter, key))
}

fn yaml_content_field(content: &str, key: &str) -> Result<Option<String>, DraftValidationError> {
    if content.trim().is_empty() {
        return Ok(None);
    }
    let value: serde_yaml::Value =
        serde_yaml::from_str(content).map_err(|e| DraftValidationError::Parse(e.to_string()))?;
    Ok(yaml_field(&value, key))
}

pub fn draft_target_id(
    kind: DraftKind,
    content: &str,
) -> Result<Option<String>, DraftValidationError> {
    match kind {
        DraftKind::Skill | DraftKind::Command => {
            if let Some(name) = frontmatter_field(content, "name") {
                return Ok(Some(name));
            }
            yaml_content_field(content, "name")
        }
        DraftKind::Delegate | DraftKind::Mode => yaml_content_field(content, "id"),
        _ => Ok(None),
    }
}

pub fn validate_draft(
    draft: &BuddyDraft,
    expected_kind: DraftKind,
    target: DraftTarget<'_>,
) -> Result<(), DraftValidationError> {
    if draft.kind != expected_kind {
        return Err(DraftValidationError::KindMismatch {
            expected: expected_kind,
            actual: draft.kind,
        });
    }
    if let DraftTarget::Id(expected) = target {
        if let Some(actual) = draft_target_id(draft.kind, &draft.yaml_or_json)? {
            if actual != expected {
                return Err(DraftValidationError::TargetMismatch {
                    expected: expected.to_string(),
                    actual,
                });
            }
        }
    }
    Ok(())
}

pub struct DraftStore {
    drafts: HashMap<String, BuddyDraft>,
}

impl DraftStore {
    pub fn new() -> Self {
        Self {
            drafts: HashMap::new(),
        }
    }

    pub fn create(
        &mut self,
        kind: DraftKind,
        title: String,
        yaml_or_json: String,
        explanation: String,
    ) -> BuddyDraft {
        let now = Utc::now();
        let draft = BuddyDraft {
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            title,
            yaml_or_json,
            explanation,
            created_at: now,
            expires_at: now + DRAFT_TTL,
        };
        self.drafts.insert(draft.id.clone(), draft.clone());
        draft
    }

    pub fn get(&self, id: &str) -> Option<&BuddyDraft> {
        self.drafts.get(id)
    }

    pub fn get_validated(
        &self,
        id: &str,
        expected_kind: DraftKind,
        target: DraftTarget<'_>,
    ) -> Result<&BuddyDraft, DraftValidationError> {
        let draft = self.get(id).ok_or(DraftValidationError::NotFound)?;
        validate_draft(draft, expected_kind, target)?;
        Ok(draft)
    }

    pub fn delete(&mut self, id: &str) -> Option<BuddyDraft> {
        self.drafts.remove(id)
    }

    pub fn insert(&mut self, draft: BuddyDraft) {
        self.drafts.insert(draft.id.clone(), draft);
    }

    pub fn consume(&mut self, id: &str) -> Option<BuddyDraft> {
        self.drafts.remove(id)
    }

    pub fn consume_validated(
        &mut self,
        id: &str,
        expected_kind: DraftKind,
        target: DraftTarget<'_>,
    ) -> Result<BuddyDraft, DraftValidationError> {
        self.get_validated(id, expected_kind, target)?;
        self.consume(id).ok_or(DraftValidationError::NotFound)
    }

    pub fn expire_old(&mut self, now: DateTime<Utc>) {
        self.drafts.retain(|_, d| d.expires_at > now);
    }

    pub fn snapshot(&self) -> Vec<BuddyDraft> {
        self.drafts.values().cloned().collect()
    }
}

impl Default for DraftStore {
    fn default() -> Self {
        Self::new()
    }
}
