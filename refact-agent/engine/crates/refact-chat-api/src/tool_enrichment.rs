use std::collections::HashMap;
use std::path::{Component, Path};

use refact_core::chat_types::ChatMessage;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use url::Url;

pub const TOOL_ENRICHMENT_EXTRA_KEY: &str = "tool_enrichment";
pub const TOOL_ENRICHMENT_SCHEMA_VERSION: u8 = 1;
const MAX_REFERENCES: usize = 32;
const MAX_TARGET_CHARS: usize = 512;
const MAX_LABEL_CHARS: usize = 160;
const MAX_SUMMARY_CHARS: usize = 320;
const MAX_STATUS_CHARS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolEnrichmentKind {
    Path,
    Symbol,
    Url,
    Citation,
    Process,
    Query,
    Diff,
    Artifact,
    Diagnostic,
    Agent,
    Test,
    #[serde(other)]
    Unknown,
}

impl ToolEnrichmentKind {
    fn is_known(self) -> bool {
        self != Self::Unknown
    }

    fn requires_workspace_path(self) -> bool {
        matches!(self, Self::Path | Self::Diff)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolEnrichmentProvenance {
    #[default]
    Native,
    Derived,
    Heuristic,
    #[serde(other)]
    Unknown,
}

impl ToolEnrichmentProvenance {
    fn priority(self) -> u8 {
        match self {
            Self::Native => 3,
            Self::Derived => 2,
            Self::Heuristic => 1,
            Self::Unknown => 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ToolEnrichmentPrivacy {
    #[serde(default, skip_serializing_if = "is_false")]
    pub redacted: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub restricted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolEnrichmentReference {
    pub kind: ToolEnrichmentKind,
    pub target: String,
    #[serde(default)]
    pub provenance: ToolEnrichmentProvenance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub redacted: bool,
}

impl ToolEnrichmentReference {
    pub fn new(kind: ToolEnrichmentKind, target: impl Into<String>) -> Self {
        Self {
            kind,
            target: target.into(),
            provenance: ToolEnrichmentProvenance::Native,
            label: None,
            summary: None,
            confidence: None,
            status: None,
            truncated: false,
            redacted: false,
        }
    }

    fn normalized(mut self) -> Option<Self> {
        if !self.kind.is_known() || self.provenance == ToolEnrichmentProvenance::Unknown {
            return None;
        }
        self.target = normalize_target(self.kind, &self.target)?;
        self.label = normalize_optional_text(self.label, MAX_LABEL_CHARS, &mut self.truncated);
        self.summary =
            normalize_optional_text(self.summary, MAX_SUMMARY_CHARS, &mut self.truncated);
        self.status = normalize_optional_text(self.status, MAX_STATUS_CHARS, &mut self.truncated);
        self.confidence = self
            .confidence
            .filter(|confidence| confidence.is_finite() && (0.0..=1.0).contains(confidence));
        Some(self)
    }

    fn dedup_key(&self) -> (ToolEnrichmentKind, String) {
        (self.kind, self.target.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolEnrichment {
    pub schema_version: u8,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<ToolEnrichmentReference>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "is_default")]
    pub privacy: ToolEnrichmentPrivacy,
}

impl Default for ToolEnrichment {
    fn default() -> Self {
        Self {
            schema_version: TOOL_ENRICHMENT_SCHEMA_VERSION,
            references: Vec::new(),
            truncated: false,
            privacy: ToolEnrichmentPrivacy::default(),
        }
    }
}

impl ToolEnrichment {
    pub fn normalized(mut self) -> Option<Self> {
        if self.schema_version != TOOL_ENRICHMENT_SCHEMA_VERSION {
            return None;
        }
        let mut deduped = Vec::new();
        let mut positions = HashMap::new();
        for reference in self
            .references
            .drain(..)
            .filter_map(ToolEnrichmentReference::normalized)
        {
            if let Some(index) = positions.get(&reference.dedup_key()).copied() {
                merge_reference(&mut deduped[index], reference);
            } else if deduped.len() < MAX_REFERENCES {
                positions.insert(reference.dedup_key(), deduped.len());
                deduped.push(reference);
            } else {
                self.truncated = true;
            }
        }
        self.references = deduped;
        Some(self)
    }
}

pub fn tool_enrichment_from_extra(extra: &Map<String, Value>) -> Option<ToolEnrichment> {
    serde_json::from_value::<ToolEnrichment>(extra.get(TOOL_ENRICHMENT_EXTRA_KEY)?.clone())
        .ok()?
        .normalized()
}

pub fn attach_tool_enrichment(message: &mut ChatMessage, enrichment: ToolEnrichment) -> bool {
    let Some(mut enrichment) = enrichment.normalized() else {
        return false;
    };
    if let Some(existing_value) = message.extra.get(TOOL_ENRICHMENT_EXTRA_KEY) {
        let Ok(existing) = serde_json::from_value::<ToolEnrichment>(existing_value.clone()) else {
            return false;
        };
        let Some(existing) = existing.normalized() else {
            return false;
        };
        enrichment = merge_tool_enrichment(existing, enrichment);
    }
    message.extra.insert(
        TOOL_ENRICHMENT_EXTRA_KEY.to_string(),
        serde_json::to_value(enrichment).expect("tool enrichment should serialize"),
    );
    true
}

pub fn redact_tool_enrichment(message: &mut ChatMessage) -> bool {
    let Some(value) = message.extra.get(TOOL_ENRICHMENT_EXTRA_KEY).cloned() else {
        return false;
    };
    let Ok(enrichment) = serde_json::from_value::<ToolEnrichment>(value) else {
        message.extra.remove(TOOL_ENRICHMENT_EXTRA_KEY);
        return true;
    };
    let Some(mut enrichment) = enrichment.normalized() else {
        message.extra.remove(TOOL_ENRICHMENT_EXTRA_KEY);
        return true;
    };
    enrichment.references.clear();
    enrichment.truncated = true;
    enrichment.privacy.redacted = true;
    enrichment.privacy.restricted = true;
    message.extra.insert(
        TOOL_ENRICHMENT_EXTRA_KEY.to_string(),
        serde_json::to_value(enrichment).expect("tool enrichment should serialize"),
    );
    true
}

fn merge_tool_enrichment(mut existing: ToolEnrichment, incoming: ToolEnrichment) -> ToolEnrichment {
    existing.truncated |= incoming.truncated;
    existing.privacy.redacted |= incoming.privacy.redacted;
    existing.privacy.restricted |= incoming.privacy.restricted;
    for reference in incoming.references {
        if let Some(existing_reference) = existing
            .references
            .iter_mut()
            .find(|candidate| candidate.dedup_key() == reference.dedup_key())
        {
            merge_reference(existing_reference, reference);
        } else if existing.references.len() < MAX_REFERENCES {
            existing.references.push(reference);
        } else {
            existing.truncated = true;
        }
    }
    existing
}

fn merge_reference(existing: &mut ToolEnrichmentReference, incoming: ToolEnrichmentReference) {
    if incoming.provenance.priority() > existing.provenance.priority() {
        existing.provenance = incoming.provenance;
    }
    existing.label = existing.label.take().or(incoming.label);
    existing.summary = existing.summary.take().or(incoming.summary);
    existing.status = existing.status.take().or(incoming.status);
    existing.confidence = match (existing.confidence, incoming.confidence) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (left @ Some(_), None) => left,
        (None, right) => right,
    };
    existing.truncated |= incoming.truncated;
    existing.redacted |= incoming.redacted;
}

fn normalize_target(kind: ToolEnrichmentKind, target: &str) -> Option<String> {
    if kind == ToolEnrichmentKind::Url {
        return normalize_http_url(target);
    }
    if kind.requires_workspace_path() {
        return normalize_workspace_relative_path(target);
    }
    if kind == ToolEnrichmentKind::Artifact {
        return normalize_artifact_target(target);
    }
    normalize_safe_text(target, MAX_TARGET_CHARS).map(|(text, _)| text)
}

fn normalize_http_url(value: &str) -> Option<String> {
    let mut url = Url::parse(value.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return None;
    }
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    let normalized = url.to_string();
    normalize_safe_text(&normalized, MAX_TARGET_CHARS).map(|(text, _)| text)
}

fn normalize_workspace_relative_path(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.contains('\\') {
        return None;
    }
    let path = Path::new(value);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return None;
    }
    let normalized = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => part.to_str(),
            Component::CurDir => None,
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    normalize_safe_text(&normalized, MAX_TARGET_CHARS).map(|(text, _)| text)
}

fn normalize_artifact_target(value: &str) -> Option<String> {
    if let Some(id) = value.trim().strip_prefix("artifact:") {
        let id = id.trim();
        if id.is_empty()
            || id.chars().count() > MAX_TARGET_CHARS
            || !id.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-' | ':')
            })
        {
            return None;
        }
        return Some(format!("artifact:{id}"));
    }
    normalize_workspace_relative_path(value)
}

fn normalize_optional_text(
    value: Option<String>,
    max_chars: usize,
    truncated: &mut bool,
) -> Option<String> {
    let value = value?;
    let (text, was_truncated) = normalize_safe_text(&value, max_chars)?;
    *truncated |= was_truncated;
    Some(text)
}

fn normalize_safe_text(value: &str, max_chars: usize) -> Option<(String, bool)> {
    let value = value.trim();
    if value.is_empty() || value.chars().any(char::is_control) {
        return None;
    }
    if refact_core::string_utils::redact_sensitive(value) != value {
        return None;
    }
    let mut chars = value.chars();
    let normalized = chars.by_ref().take(max_chars).collect::<String>();
    Some((normalized, chars.next().is_some()))
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_default(value: &ToolEnrichmentPrivacy) -> bool {
    value == &ToolEnrichmentPrivacy::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn reference(kind: ToolEnrichmentKind, target: &str) -> ToolEnrichmentReference {
        ToolEnrichmentReference::new(kind, target)
    }

    #[test]
    fn legacy_messages_remain_identical_without_an_envelope() {
        let mut message = ChatMessage::new("tool".to_string(), "raw fallback".to_string());
        let before = serde_json::to_value(&message).unwrap();
        assert!(!attach_tool_enrichment(
            &mut message,
            ToolEnrichment {
                schema_version: 2,
                ..Default::default()
            }
        ));
        assert_eq!(serde_json::to_value(&message).unwrap(), before);
    }

    #[test]
    fn unknown_versions_and_kinds_are_ignored_safely() {
        let unknown_version = Map::from_iter([(
            TOOL_ENRICHMENT_EXTRA_KEY.to_string(),
            json!({"schema_version": 2, "references": []}),
        )]);
        assert!(tool_enrichment_from_extra(&unknown_version).is_none());

        let unknown_kind = Map::from_iter([(
            TOOL_ENRICHMENT_EXTRA_KEY.to_string(),
            json!({
                "schema_version": 1,
                "references": [
                    {"kind": "future_kind", "target": "ignored", "provenance": "native"},
                    {"kind": "symbol", "target": "crate::item", "provenance": "native"}
                ]
            }),
        )]);
        let parsed = tool_enrichment_from_extra(&unknown_kind).unwrap();
        assert_eq!(parsed.references.len(), 1);
        assert_eq!(parsed.references[0].kind, ToolEnrichmentKind::Symbol);
    }

    #[test]
    fn normalization_bounds_arrays_and_marks_truncation() {
        let mut enrichment = ToolEnrichment::default();
        enrichment.references = (0..(MAX_REFERENCES + 2))
            .map(|index| reference(ToolEnrichmentKind::Symbol, &format!("crate::item_{index}")))
            .collect();
        enrichment.references[0].label = Some("x".repeat(MAX_LABEL_CHARS + 1));

        let normalized = enrichment.normalized().unwrap();
        assert_eq!(normalized.references.len(), MAX_REFERENCES);
        assert!(normalized.truncated);
        assert!(normalized.references[0].truncated);
        assert_eq!(
            normalized.references[0]
                .label
                .as_ref()
                .unwrap()
                .chars()
                .count(),
            MAX_LABEL_CHARS
        );
    }

    #[test]
    fn paths_urls_and_artifacts_are_privacy_scoped() {
        let url = reference(
            ToolEnrichmentKind::Url,
            "https://user:secret@example.test/a?token=secret#fragment",
        )
        .normalized()
        .unwrap();
        assert_eq!(url.target, "https://example.test/a");
        assert!(reference(ToolEnrichmentKind::Url, "file:///private.txt")
            .normalized()
            .is_none());
        assert_eq!(
            reference(ToolEnrichmentKind::Path, "src/lib.rs")
                .normalized()
                .unwrap()
                .target,
            "src/lib.rs"
        );
        assert!(reference(ToolEnrichmentKind::Path, "../secret.txt")
            .normalized()
            .is_none());
        assert!(reference(ToolEnrichmentKind::Path, "/private/secret.txt")
            .normalized()
            .is_none());
        assert_eq!(
            reference(ToolEnrichmentKind::Artifact, "artifact:chart-1")
                .normalized()
                .unwrap()
                .target,
            "artifact:chart-1"
        );
        assert!(
            reference(ToolEnrichmentKind::Artifact, "artifact:../../secret")
                .normalized()
                .is_none()
        );
    }

    #[test]
    fn merging_deduplicates_and_prefers_native_provenance() {
        let mut message = ChatMessage::new("tool".to_string(), "raw result".to_string());
        let mut heuristic = reference(ToolEnrichmentKind::Symbol, "crate::thing");
        heuristic.provenance = ToolEnrichmentProvenance::Heuristic;
        heuristic.confidence = Some(0.2);
        let mut native = reference(ToolEnrichmentKind::Symbol, "crate::thing");
        native.provenance = ToolEnrichmentProvenance::Native;
        native.confidence = Some(0.9);
        native.status = Some("found".to_string());

        assert!(attach_tool_enrichment(
            &mut message,
            ToolEnrichment {
                references: vec![heuristic],
                ..Default::default()
            }
        ));
        assert!(attach_tool_enrichment(
            &mut message,
            ToolEnrichment {
                references: vec![native],
                ..Default::default()
            }
        ));

        let enrichment = tool_enrichment_from_extra(&message.extra).unwrap();
        assert_eq!(enrichment.references.len(), 1);
        assert_eq!(
            enrichment.references[0].provenance,
            ToolEnrichmentProvenance::Native
        );
        assert_eq!(enrichment.references[0].confidence, Some(0.9));
        assert_eq!(enrichment.references[0].status.as_deref(), Some("found"));
        assert_eq!(message.content.content_text_only(), "raw result");
    }

    #[test]
    fn redaction_removes_references_without_changing_raw_content() {
        let mut message = ChatMessage::new("tool".to_string(), "raw result".to_string());
        attach_tool_enrichment(
            &mut message,
            ToolEnrichment {
                references: vec![reference(ToolEnrichmentKind::Path, "src/lib.rs")],
                ..Default::default()
            },
        );

        assert!(redact_tool_enrichment(&mut message));
        let enrichment = tool_enrichment_from_extra(&message.extra).unwrap();
        assert!(enrichment.references.is_empty());
        assert!(enrichment.privacy.redacted);
        assert!(enrichment.privacy.restricted);
        assert_eq!(message.content.content_text_only(), "raw result");
    }
}
