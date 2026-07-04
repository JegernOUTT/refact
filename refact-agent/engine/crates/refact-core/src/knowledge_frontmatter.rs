use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KnowledgeFrontmatter {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub updated: Option<String>,
    #[serde(default)]
    pub filenames: Vec<String>,
    #[serde(default)]
    pub links: Vec<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub superseded_by: Option<String>,
    #[serde(default)]
    pub deprecated_at: Option<String>,
    #[serde(default)]
    pub review_after: Option<String>,
    #[serde(default)]
    pub source_chat_id: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub entities: Vec<String>,
    #[serde(default)]
    pub related_files: Vec<String>,
    #[serde(default)]
    pub related_entities: Vec<String>,
    #[serde(default)]
    pub content_hash: Option<String>,
    #[serde(default)]
    pub source_tool: Option<String>,
    #[serde(default)]
    pub source_confidence: Option<f32>,
    #[serde(default)]
    pub source_trajectory_id: Option<String>,
    #[serde(default)]
    pub source_message_range: Option<String>,
    #[serde(default)]
    pub source_commit: Option<String>,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub last_used_at: Option<String>,
    #[serde(default)]
    pub use_count: u32,
    #[serde(default)]
    pub last_injected_at: Option<String>,
    #[serde(default)]
    pub dismissed_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_content_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_needed: Option<bool>,
    #[serde(default)]
    pub occurrences: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_observed: Option<String>,
    #[serde(
        flatten,
        default,
        skip_serializing_if = "std::collections::BTreeMap::is_empty"
    )]
    pub extra: BTreeMap<String, serde_yaml::Value>,
}

impl KnowledgeFrontmatter {
    pub fn parse(content: &str) -> (Self, usize) {
        if !content.starts_with("---") {
            return (Self::default(), 0);
        }
        let rest = &content[3..];
        let end_marker = rest.find("\n---");
        let Some(end_idx) = end_marker else {
            return (Self::default(), 0);
        };
        let yaml_content = &rest[..end_idx];
        let mut end_offset = 3 + end_idx + 4;
        if content.len() > end_offset && content.as_bytes().get(end_offset) == Some(&b'\n') {
            end_offset += 1;
        }
        match serde_yaml::from_str::<KnowledgeFrontmatter>(yaml_content) {
            Ok(fm) => (fm, end_offset),
            Err(_) => (Self::default(), 0),
        }
    }

    pub fn to_yaml(&self) -> String {
        let mut lines = vec!["---".to_string()];
        if let Some(id) = &self.id {
            lines.push(format!("id: \"{}\"", id));
        }
        if let Some(title) = &self.title {
            lines.push(format!("title: \"{}\"", title.replace('"', "\\\"")));
        }
        if let Some(kind) = &self.kind {
            lines.push(format!("kind: {}", kind));
        }
        if let Some(created) = &self.created {
            lines.push(format!("created: {}", created));
        }
        if let Some(updated) = &self.updated {
            lines.push(format!("updated: {}", updated));
        }
        if let Some(review_after) = &self.review_after {
            lines.push(format!("review_after: {}", review_after));
        }
        if let Some(status) = &self.status {
            lines.push(format!("status: {}", status));
        }
        if !self.tags.is_empty() {
            let tags_str = self
                .tags
                .iter()
                .map(|t| format!("\"{}\"", t))
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!("tags: [{}]", tags_str));
        }
        if !self.filenames.is_empty() {
            let files_str = self
                .filenames
                .iter()
                .map(|f| format!("\"{}\"", f))
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!("filenames: [{}]", files_str));
        }
        if !self.links.is_empty() {
            let links_str = self
                .links
                .iter()
                .map(|l| format!("\"{}\"", l))
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!("links: [{}]", links_str));
        }
        if let Some(superseded_by) = &self.superseded_by {
            lines.push(format!("superseded_by: \"{}\"", superseded_by));
        }
        if let Some(deprecated_at) = &self.deprecated_at {
            lines.push(format!("deprecated_at: {}", deprecated_at));
        }
        if let Some(source_chat_id) = &self.source_chat_id {
            lines.push(format!(
                "source_chat_id: \"{}\"",
                source_chat_id.replace('"', "\\\"")
            ));
        }
        if let Some(created_at) = &self.created_at {
            lines.push(format!(
                "created_at: \"{}\"",
                created_at.replace('"', "\\\"")
            ));
        }
        if let Some(summary) = &self.summary {
            lines.push(format!("summary: \"{}\"", summary.replace('"', "\\\"")));
        }
        if let Some(description) = &self.description {
            lines.push(format!(
                "description: \"{}\"",
                description.replace('"', "\\\"")
            ));
        }
        if !self.entities.is_empty() {
            let entities_str = self
                .entities
                .iter()
                .map(|t| format!("\"{}\"", t.replace('"', "\\\"")))
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!("entities: [{}]", entities_str));
        }
        if !self.related_files.is_empty() {
            let files_str = self
                .related_files
                .iter()
                .map(|f| format!("\"{}\"", f.replace('"', "\\\"")))
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!("related_files: [{}]", files_str));
        }
        if !self.related_entities.is_empty() {
            let entities_str = self
                .related_entities
                .iter()
                .map(|t| format!("\"{}\"", t.replace('"', "\\\"")))
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!("related_entities: [{}]", entities_str));
        }
        if let Some(content_hash) = &self.content_hash {
            lines.push(format!(
                "content_hash: \"{}\"",
                content_hash.replace('"', "\\\"")
            ));
        }
        if let Some(source_tool) = &self.source_tool {
            lines.push(format!(
                "source_tool: \"{}\"",
                source_tool.replace('"', "\\\"")
            ));
        }
        if let Some(source_confidence) = self.source_confidence {
            lines.push(format!("source_confidence: {:.3}", source_confidence));
        }
        if let Some(source_trajectory_id) = &self.source_trajectory_id {
            lines.push(format!(
                "source_trajectory_id: \"{}\"",
                source_trajectory_id.replace('"', "\\\"")
            ));
        }
        if let Some(source_message_range) = &self.source_message_range {
            lines.push(format!(
                "source_message_range: \"{}\"",
                source_message_range.replace('"', "\\\"")
            ));
        }
        if let Some(source_commit) = &self.source_commit {
            lines.push(format!(
                "source_commit: \"{}\"",
                source_commit.replace('"', "\\\"")
            ));
        }
        if let Some(topic) = &self.topic {
            lines.push(format!("topic: \"{}\"", topic.replace('"', "\\\"")));
        }
        if let Some(last_used_at) = &self.last_used_at {
            lines.push(format!(
                "last_used_at: \"{}\"",
                last_used_at.replace('"', "\\\"")
            ));
        }
        if self.use_count > 0 {
            lines.push(format!("use_count: {}", self.use_count));
        }
        if let Some(last_injected_at) = &self.last_injected_at {
            lines.push(format!(
                "last_injected_at: \"{}\"",
                last_injected_at.replace('"', "\\\"")
            ));
        }
        if self.dismissed_count > 0 {
            lines.push(format!("dismissed_count: {}", self.dismissed_count));
        }
        if let Some(source_id) = &self.source_id {
            lines.push(format!("source_id: \"{}\"", source_id.replace('"', "\\\"")));
        }
        if let Some(source_content_hash) = &self.source_content_hash {
            lines.push(format!(
                "source_content_hash: \"{}\"",
                source_content_hash.replace('"', "\\\"")
            ));
        }
        if let Some(review_needed) = self.review_needed {
            lines.push(format!("review_needed: {}", review_needed));
        }
        if self.occurrences > 0 {
            lines.push(format!("occurrences: {}", self.occurrences));
        }
        if let Some(signal_key) = &self.signal_key {
            lines.push(format!(
                "signal_key: \"{}\"",
                signal_key.replace('"', "\\\"")
            ));
        }
        if let Some(last_observed) = &self.last_observed {
            lines.push(format!("last_observed: \"{}\"", last_observed));
        }
        for (key, value) in &self.extra {
            if let Ok(serialized) = serde_yaml::to_string(value) {
                let rendered = serialized.trim_end_matches('\n');
                if !rendered.contains('\n') {
                    lines.push(format!("{}: {}", key, rendered));
                } else {
                    lines.push(format!("{}:", key));
                    for line in rendered.lines() {
                        lines.push(format!("  {}", line));
                    }
                }
            }
        }
        lines.push("---".to_string());
        lines.join("\n")
    }

    pub fn is_active(&self) -> bool {
        !self.is_archived() && !self.is_deprecated()
    }

    pub fn is_deprecated(&self) -> bool {
        self.status.as_deref() == Some("deprecated")
    }

    pub fn is_archived(&self) -> bool {
        self.status.as_deref() == Some("archived")
    }

    pub fn is_pinned(&self) -> bool {
        self.status.as_deref() == Some("pinned")
    }

    pub fn kind_or_default(&self) -> &str {
        self.kind
            .as_deref()
            .unwrap_or(if self.filenames.is_empty() {
                "domain"
            } else {
                "code"
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_frontmatter_fields_roundtrip_through_to_yaml() {
        let doc = "---\ntitle: \"Known\"\ntags: [\"buddy\"]\nunknown_scalar: keep-me\nunknown_list:\n  - alpha\n  - beta\n---\nBody";
        let (frontmatter, _) = KnowledgeFrontmatter::parse(doc);

        assert_eq!(frontmatter.title.as_deref(), Some("Known"));
        assert_eq!(frontmatter.tags, vec!["buddy".to_string()]);
        assert_eq!(
            frontmatter.extra.get("unknown_scalar"),
            Some(&serde_yaml::Value::String("keep-me".to_string()))
        );
        assert!(matches!(
            frontmatter.extra.get("unknown_list"),
            Some(serde_yaml::Value::Sequence(values)) if values.len() == 2
        ));

        let rendered = frontmatter.to_yaml();
        let (roundtripped, _) = KnowledgeFrontmatter::parse(&format!("{}\nBody", rendered));

        assert_eq!(roundtripped.title.as_deref(), Some("Known"));
        assert_eq!(roundtripped.tags, vec!["buddy".to_string()]);
        assert_eq!(roundtripped.extra, frontmatter.extra);
    }

    #[test]
    fn frontmatter_without_extra_serializes_without_artifacts() {
        let frontmatter = KnowledgeFrontmatter {
            id: Some("id-1".to_string()),
            title: Some("Title".to_string()),
            tags: vec!["buddy".to_string()],
            ..Default::default()
        };

        assert_eq!(
            frontmatter.to_yaml(),
            "---\nid: \"id-1\"\ntitle: \"Title\"\ntags: [\"buddy\"]\n---"
        );
    }
}
