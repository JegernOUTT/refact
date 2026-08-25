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
const MAX_SOURCE_CHARS: usize = 64;

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
    Git,
    Review,
    #[serde(other)]
    Unknown,
}

impl ToolEnrichmentKind {
    fn is_known(self) -> bool {
        self != Self::Unknown
    }

    fn requires_workspace_path(self) -> bool {
        matches!(self, Self::Path | Self::Diff | Self::Review)
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ToolEnrichmentReferenceDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rename_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hunk_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_available: Option<bool>,
}

impl ToolEnrichmentReferenceDetails {
    fn normalized(mut self, truncated: &mut bool) -> Option<Self> {
        self.action = normalize_optional_text(self.action, MAX_STATUS_CHARS, truncated);
        self.rename_to = self
            .rename_to
            .and_then(|path| normalize_workspace_relative_path(&path));
        self.short_sha = self.short_sha.and_then(|sha| normalize_short_sha(&sha));
        self.scope = normalize_optional_text(self.scope, MAX_STATUS_CHARS, truncated);
        self.parent_chat_id =
            normalize_optional_text(self.parent_chat_id, MAX_TARGET_CHARS, truncated);
        self.child_chat_id =
            normalize_optional_text(self.child_chat_id, MAX_TARGET_CHARS, truncated);
        Some(self)
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line1: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line2: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub redacted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<ToolEnrichmentReferenceDetails>,
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
            line1: None,
            line2: None,
            count: None,
            source: None,
            truncated: false,
            redacted: false,
            details: None,
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
        self.source = normalize_optional_text(self.source, MAX_SOURCE_CHARS, &mut self.truncated);
        self.line1 = self.line1.filter(|line| *line > 0);
        self.line2 = self.line2.filter(|line| *line > 0);
        if self
            .line2
            .is_some_and(|line2| self.line1.is_none_or(|line1| line2 < line1))
        {
            self.line2 = None;
        }
        self.count = self.count.filter(|count| *count > 0);
        self.details = self
            .details
            .and_then(|details| details.normalized(&mut self.truncated));
        self.confidence = self
            .confidence
            .filter(|confidence| confidence.is_finite() && (0.0..=1.0).contains(confidence));
        Some(self)
    }

    fn dedup_key(&self) -> (ToolEnrichmentKind, String, Option<usize>, Option<usize>) {
        (self.kind, self.target.clone(), self.line1, self.line2)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolEnrichment {
    pub schema_version: u8,
    #[serde(default)]
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
        if self.privacy.redacted || self.privacy.restricted {
            self.references.clear();
            self.truncated = true;
        }
        Some(self)
    }
}

pub fn tool_enrichment_from_extra(extra: &Map<String, Value>) -> Option<ToolEnrichment> {
    serde_json::from_value::<ToolEnrichment>(extra.get(TOOL_ENRICHMENT_EXTRA_KEY)?.clone())
        .ok()?
        .normalized()
}

pub fn attach_tool_enrichment(message: &mut ChatMessage, enrichment: ToolEnrichment) -> bool {
    attach_tool_enrichment_to_extra(&mut message.extra, enrichment)
}

pub fn attach_tool_enrichment_to_extra(
    extra: &mut Map<String, Value>,
    enrichment: ToolEnrichment,
) -> bool {
    let Some(mut enrichment) = enrichment.normalized() else {
        return false;
    };
    if enrichment.references.is_empty()
        && !enrichment.privacy.redacted
        && !enrichment.privacy.restricted
    {
        return false;
    }
    if let Some(existing) = extra
        .get(TOOL_ENRICHMENT_EXTRA_KEY)
        .cloned()
        .and_then(|value| serde_json::from_value::<ToolEnrichment>(value).ok())
        .and_then(ToolEnrichment::normalized)
    {
        if existing.privacy.redacted || existing.privacy.restricted {
            return replace_tool_enrichment(extra, existing);
        }
        enrichment = merge_tool_enrichment(existing, enrichment);
    }
    replace_tool_enrichment(extra, enrichment)
}

pub fn redact_tool_enrichment(message: &mut ChatMessage) -> bool {
    let mut enrichment = message
        .extra
        .get(TOOL_ENRICHMENT_EXTRA_KEY)
        .cloned()
        .and_then(|value| serde_json::from_value::<ToolEnrichment>(value).ok())
        .and_then(ToolEnrichment::normalized)
        .unwrap_or_default();
    enrichment.privacy.redacted = true;
    enrichment.privacy.restricted = true;
    attach_tool_enrichment(message, enrichment)
}

fn merge_tool_enrichment(mut existing: ToolEnrichment, incoming: ToolEnrichment) -> ToolEnrichment {
    if existing.privacy.redacted || existing.privacy.restricted {
        return existing;
    }
    if incoming.privacy.redacted || incoming.privacy.restricted {
        return incoming;
    }
    existing.truncated |= incoming.truncated;
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
    let incoming_has_higher_provenance =
        incoming.provenance.priority() > existing.provenance.priority();
    if incoming_has_higher_provenance {
        existing.provenance = incoming.provenance;
        existing.label = incoming.label;
        existing.summary = incoming.summary;
        existing.status = incoming.status;
        existing.source = incoming.source;
        existing.details = incoming.details;
    } else {
        existing.label = existing.label.take().or(incoming.label);
        existing.summary = existing.summary.take().or(incoming.summary);
        existing.status = existing.status.take().or(incoming.status);
        existing.source = existing.source.take().or(incoming.source);
        existing.details = existing.details.take().or(incoming.details);
    }
    existing.count = match (existing.count, incoming.count) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (left @ Some(_), None) => left,
        (None, right) => right,
    };
    existing.confidence = match (existing.confidence, incoming.confidence) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (left @ Some(_), None) => left,
        (None, right) => right,
    };
    existing.truncated |= incoming.truncated;
    existing.redacted |= incoming.redacted;
}

fn replace_tool_enrichment(extra: &mut Map<String, Value>, enrichment: ToolEnrichment) -> bool {
    let Ok(value) = serde_json::to_value(enrichment) else {
        return false;
    };
    extra.insert(TOOL_ENRICHMENT_EXTRA_KEY.to_string(), value);
    true
}

fn normalize_target(kind: ToolEnrichmentKind, target: &str) -> Option<String> {
    if matches!(kind, ToolEnrichmentKind::Url | ToolEnrichmentKind::Citation) {
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

pub fn sanitize_http_url(value: &str) -> Option<String> {
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

fn normalize_http_url(value: &str) -> Option<String> {
    sanitize_http_url(value)
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

fn normalize_short_sha(value: &str) -> Option<String> {
    let value = value.trim();
    (7..=12)
        .contains(&value.len())
        .then_some(value)
        .filter(|value| value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(|value| value.to_ascii_lowercase())
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
    fn sanitize_http_url_strips_redirect_query_and_credentials() {
        assert_eq!(
            sanitize_http_url("https://user:token@example.test/final?token=secret#fragment"),
            Some("https://example.test/final".to_string())
        );
        assert!(sanitize_http_url("file:///private/final").is_none());
    }

    #[test]
    fn merging_deduplicates_and_prefers_native_provenance() {
        let mut message = ChatMessage::new("tool".to_string(), "raw result".to_string());
        let mut heuristic = reference(ToolEnrichmentKind::Symbol, "crate::thing");
        heuristic.provenance = ToolEnrichmentProvenance::Heuristic;
        heuristic.confidence = Some(0.2);
        heuristic.label = Some("stale label".to_string());
        heuristic.summary = Some("stale summary".to_string());
        heuristic.status = Some("stale status".to_string());
        heuristic.source = Some("stale source".to_string());
        heuristic.details = Some(ToolEnrichmentReferenceDetails {
            action: Some("stale action".to_string()),
            ..Default::default()
        });
        let mut native = reference(ToolEnrichmentKind::Symbol, "crate::thing");
        native.provenance = ToolEnrichmentProvenance::Native;
        native.confidence = Some(0.9);
        native.label = Some("native label".to_string());
        native.summary = Some("native summary".to_string());
        native.status = Some("found".to_string());
        native.source = Some("native source".to_string());
        native.details = Some(ToolEnrichmentReferenceDetails {
            action: Some("native action".to_string()),
            ..Default::default()
        });

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
        assert_eq!(enrichment.references[0].count, None);
        assert_eq!(
            enrichment.references[0].label.as_deref(),
            Some("native label")
        );
        assert_eq!(
            enrichment.references[0].summary.as_deref(),
            Some("native summary")
        );
        assert_eq!(enrichment.references[0].status.as_deref(), Some("found"));
        assert_eq!(
            enrichment.references[0].source.as_deref(),
            Some("native source")
        );
        assert_eq!(
            enrichment.references[0]
                .details
                .as_ref()
                .and_then(|details| details.action.as_deref()),
            Some("native action")
        );
        assert_eq!(message.content.content_text_only(), "raw result");
    }

    #[test]
    fn empty_enrichment_does_not_create_a_key() {
        let mut message = ChatMessage::new("tool".to_string(), "raw result".to_string());

        assert!(!attach_tool_enrichment(
            &mut message,
            ToolEnrichment::default(),
        ));
        assert!(!message.extra.contains_key(TOOL_ENRICHMENT_EXTRA_KEY));
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

    #[test]
    fn redacted_enrichment_remains_terminal_after_later_attachment() {
        let mut message = ChatMessage::new("tool".to_string(), "raw result".to_string());
        assert!(attach_tool_enrichment(
            &mut message,
            ToolEnrichment {
                references: vec![reference(ToolEnrichmentKind::Path, "src/lib.rs")],
                ..Default::default()
            },
        ));
        assert!(redact_tool_enrichment(&mut message));

        assert!(attach_tool_enrichment(
            &mut message,
            ToolEnrichment {
                references: vec![reference(ToolEnrichmentKind::Path, "src/readded.rs")],
                ..Default::default()
            },
        ));

        let enrichment = tool_enrichment_from_extra(&message.extra).unwrap();
        assert!(enrichment.references.is_empty());
        assert!(enrichment.privacy.redacted);
        assert!(enrichment.privacy.restricted);
    }

    #[test]
    fn redaction_installs_a_terminal_envelope_without_existing_metadata() {
        let mut message = ChatMessage::new("tool".to_string(), "raw result".to_string());

        assert!(redact_tool_enrichment(&mut message));
        assert!(attach_tool_enrichment(
            &mut message,
            ToolEnrichment {
                references: vec![reference(ToolEnrichmentKind::Path, "src/readded.rs")],
                ..Default::default()
            },
        ));

        let enrichment = tool_enrichment_from_extra(&message.extra).unwrap();
        assert!(enrichment.references.is_empty());
        assert!(enrichment.privacy.redacted);
        assert!(enrichment.privacy.restricted);
        assert_eq!(
            message.extra[TOOL_ENRICHMENT_EXTRA_KEY]["references"],
            json!([])
        );
    }

    #[test]
    fn malformed_existing_value_is_replaced_by_valid_enrichment() {
        let mut message = ChatMessage::new("tool".to_string(), "raw result".to_string());
        message
            .extra
            .insert(TOOL_ENRICHMENT_EXTRA_KEY.to_string(), json!(null));

        assert!(attach_tool_enrichment(
            &mut message,
            ToolEnrichment {
                references: vec![reference(ToolEnrichmentKind::Path, "src/lib.rs")],
                ..Default::default()
            },
        ));
        let enrichment = tool_enrichment_from_extra(&message.extra).unwrap();
        assert_eq!(enrichment.references[0].target, "src/lib.rs");
        assert_ne!(message.extra[TOOL_ENRICHMENT_EXTRA_KEY], Value::Null);
    }

    #[test]
    fn thin_metadata_details_remain_bounded_and_path_scoped() {
        let mut diff_reference = reference(ToolEnrichmentKind::Diff, "src/old.rs");
        diff_reference.details = Some(ToolEnrichmentReferenceDetails {
            action: Some("rename".to_string()),
            rename_to: Some("src/new.rs".to_string()),
            hunk_count: Some(2),
            short_sha: Some("ABC1234".to_string()),
            ..Default::default()
        });

        let normalized = diff_reference.normalized().unwrap();
        let details = normalized.details.unwrap();
        assert_eq!(details.rename_to.as_deref(), Some("src/new.rs"));
        assert_eq!(details.short_sha.as_deref(), Some("abc1234"));

        let mut unsafe_reference = reference(ToolEnrichmentKind::Review, "src/lib.rs");
        unsafe_reference.details = Some(ToolEnrichmentReferenceDetails {
            rename_to: Some("../private.rs".to_string()),
            ..Default::default()
        });
        assert!(unsafe_reference
            .normalized()
            .unwrap()
            .details
            .unwrap()
            .rename_to
            .is_none());
    }
}
