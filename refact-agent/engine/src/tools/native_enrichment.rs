use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use refact_chat_api::{
    attach_tool_enrichment, ToolEnrichment, ToolEnrichmentKind, ToolEnrichmentProvenance,
    ToolEnrichmentReference,
};

use crate::call_validation::{ChatMessage, ContextFile};
use crate::global_context::GlobalContext;
use crate::worktrees::scope::ExecutionScope;

const MAX_REFERENCES: usize = 32;

pub(crate) fn workspace_roots(
    gcx: &GlobalContext,
    execution_scope: Option<&ExecutionScope>,
) -> Vec<PathBuf> {
    let mut roots = if execution_scope.is_some_and(ExecutionScope::is_enforced) {
        vec![execution_scope.unwrap().effective_root().to_path_buf()]
    } else {
        gcx.documents_state
            .workspace_folders
            .lock()
            .unwrap()
            .clone()
    };
    roots.sort_by(|left, right| {
        right
            .components()
            .count()
            .cmp(&left.components().count())
            .then(left.cmp(right))
    });
    roots.dedup();
    roots
}

pub(crate) struct NativeReferences {
    references: Vec<ToolEnrichmentReference>,
    seen: HashSet<(ToolEnrichmentKind, String, Option<usize>, Option<usize>)>,
    truncated: bool,
}

impl NativeReferences {
    pub(crate) fn new() -> Self {
        Self {
            references: Vec::new(),
            seen: HashSet::new(),
            truncated: false,
        }
    }

    pub(crate) fn mark_truncated(&mut self) {
        self.truncated = true;
    }

    pub(crate) fn add_query(&mut self, query: &str, count: usize, status: &str, source: &str) {
        let mut reference = native_reference(ToolEnrichmentKind::Query, query);
        reference.count = (count > 0).then_some(count);
        reference.status = Some(status.to_string());
        reference.source = Some(source.to_string());
        self.push(reference);
    }

    pub(crate) fn add_symbol(&mut self, symbol: &str, status: &str, source: &str) {
        let mut reference = native_reference(ToolEnrichmentKind::Symbol, symbol);
        reference.status = Some(status.to_string());
        reference.source = Some(source.to_string());
        self.push(reference);
    }

    pub(crate) fn add_artifact(
        &mut self,
        id: &str,
        label: Option<String>,
        status: Option<String>,
        source: Option<String>,
    ) {
        let mut reference =
            native_reference(ToolEnrichmentKind::Artifact, format!("artifact:{id}"));
        reference.label = label;
        reference.status = status;
        reference.source = source;
        self.push(reference);
    }

    pub(crate) fn add_path(
        &mut self,
        path: &Path,
        roots: &[PathBuf],
        line1: usize,
        line2: usize,
        status: &str,
        confidence: Option<f32>,
        source: &str,
    ) {
        let Some(target) = workspace_relative_path(path, roots) else {
            return;
        };
        let mut reference = native_reference(ToolEnrichmentKind::Path, target.clone());
        reference.label = Some(path_label(&target, line1, line2));
        reference.status = Some(status.to_string());
        reference.confidence = confidence;
        reference.line1 = (line1 > 0).then_some(line1);
        reference.line2 = (line2 > 0).then_some(line2);
        reference.source = Some(source.to_string());
        self.push(reference);
    }

    pub(crate) fn add_context_file(
        &mut self,
        file: &ContextFile,
        roots: &[PathBuf],
        status: &str,
        source: &str,
    ) {
        self.add_path(
            Path::new(&file.file_name),
            roots,
            file.line1,
            file.line2,
            status,
            (file.usefulness > 0.0).then_some((file.usefulness / 100.0).clamp(0.0, 1.0)),
            source,
        );
        for symbol in &file.symbols {
            self.add_symbol(symbol, status, source);
        }
    }

    pub(crate) fn attach(self, message: &mut ChatMessage) {
        if self.references.is_empty() {
            return;
        }
        attach_tool_enrichment(
            message,
            ToolEnrichment {
                references: self.references,
                truncated: self.truncated,
                ..Default::default()
            },
        );
    }

    fn push(&mut self, reference: ToolEnrichmentReference) {
        let key = (
            reference.kind,
            reference.target.clone(),
            reference.line1,
            reference.line2,
        );
        if !self.seen.insert(key) {
            return;
        }
        if self.references.len() == MAX_REFERENCES {
            self.truncated = true;
            return;
        }
        self.references.push(reference);
    }
}

fn native_reference(
    kind: ToolEnrichmentKind,
    target: impl Into<String>,
) -> ToolEnrichmentReference {
    ToolEnrichmentReference {
        provenance: ToolEnrichmentProvenance::Native,
        ..ToolEnrichmentReference::new(kind, target)
    }
}

fn workspace_relative_path(path: &Path, roots: &[PathBuf]) -> Option<String> {
    let relative = if path.is_absolute() {
        roots.iter().find_map(|root| path.strip_prefix(root).ok())?
    } else {
        path
    };
    if relative.as_os_str().is_empty()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return None;
    }
    let normalized = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => part.to_str(),
            Component::CurDir => None,
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    (!normalized.is_empty()).then_some(normalized)
}

fn path_label(path: &str, line1: usize, line2: usize) -> String {
    if line1 == 0 || line2 == 0 {
        return path.to_string();
    }
    format!("{path}:{line1}-{line2}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_relative_path_requires_a_safe_workspace_path() {
        let root = crate::test_paths::abs("workspace");
        let source = crate::test_paths::abs("workspace/src/lib.rs");
        assert_eq!(
            workspace_relative_path(&source, &[root.clone()]),
            Some("src/lib.rs".to_string())
        );
        assert_eq!(
            workspace_relative_path(Path::new("../secret.rs"), &[root]),
            None
        );
        assert_eq!(
            workspace_relative_path(Path::new("/outside/secret.rs"), &[]),
            None
        );
    }

    #[test]
    fn native_references_are_bounded_and_keep_the_first_range() {
        let root = crate::test_paths::abs("workspace");
        let source = crate::test_paths::abs("workspace/src/lib.rs");
        let mut references = NativeReferences::new();
        references.add_path(&source, &[root.clone()], 2, 4, "match", Some(0.9), "test");
        references.add_path(&source, &[root], 6, 8, "match", Some(0.8), "test");
        assert_eq!(references.references.len(), 2);
        for index in 0..MAX_REFERENCES {
            references.add_symbol(&format!("symbol_{index}"), "definition", "test");
        }

        let mut message = ChatMessage::new("tool".to_string(), "raw result".to_string());
        references.attach(&mut message);
        let enrichment = refact_chat_api::tool_enrichment_from_extra(&message.extra).unwrap();

        assert_eq!(enrichment.references.len(), MAX_REFERENCES);
        assert!(enrichment.truncated);
        assert_eq!(enrichment.references[0].target, "src/lib.rs");
        assert_eq!(
            enrichment.references[0].label.as_deref(),
            Some("src/lib.rs:2-4")
        );
        assert_eq!(
            enrichment.references[1].label.as_deref(),
            Some("src/lib.rs:6-8")
        );
        assert_eq!(message.content.content_text_only(), "raw result");
    }
}
