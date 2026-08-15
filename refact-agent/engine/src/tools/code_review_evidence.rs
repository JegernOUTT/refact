use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::files_in_workspace::get_file_text_from_memory_or_disk;
use crate::global_context::GlobalContext;
use crate::tools::code_review_scope::ReviewScope;
use crate::tools::code_review_types::{ReviewEvidence, ReviewFinding};

const EXCERPT_CONTEXT_LINES: u32 = 5;
const MAX_EXCERPT_BYTES: usize = 1024;
const MAX_DIFF_HUNK_BYTES: usize = 640;
const MAX_SYMBOL_BYTES: usize = 384;
const MAX_SYMBOLS_PER_FINDING: usize = 8;
const TRUNCATION_MARKER: &str = "\n[evidence truncated]";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceRejection {
    pub finding_id: String,
    pub index: usize,
    pub reason: String,
}

impl EvidenceRejection {
    pub fn check_name(&self) -> String {
        let key = if self.finding_id.is_empty() {
            self.index.to_string()
        } else {
            self.finding_id.clone()
        };
        format!("evidence_reject:{key}:{}", self.reason)
    }
}

#[derive(Default)]
struct SymbolGraphFacts {
    ids_by_name_and_path: HashMap<(String, String), Vec<i64>>,
    incoming_by_id: HashMap<i64, usize>,
}

impl SymbolGraphFacts {
    fn from_cached(cached: &refact_codegraph::CachedGraphAnalytics) -> Self {
        let mut facts = Self::default();
        for (id, name, path) in &cached.data.nodes {
            facts
                .ids_by_name_and_path
                .entry((name.clone(), normalize_path(path)))
                .or_default()
                .push(*id);
        }
        for (_src, dst, kind) in &cached.data.edges {
            if kind != "defined_in" {
                *facts.incoming_by_id.entry(*dst).or_default() += 1;
            }
        }
        facts
    }

    fn usage_count(&self, name: &str, path: &str) -> usize {
        self.ids_by_name_and_path
            .get(&(name.to_string(), normalize_path(path)))
            .into_iter()
            .flatten()
            .map(|id| self.incoming_by_id.get(id).copied().unwrap_or(0))
            .sum()
    }
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
        .trim_start_matches("./")
        .trim_end_matches('/')
        .to_string()
}

fn path_matches(candidate: &str, scope_path: &Path) -> bool {
    let candidate = normalize_path(candidate);
    let scope_path = normalize_path(&scope_path.to_string_lossy());
    if Path::new(&candidate).is_absolute() {
        return candidate == scope_path;
    }
    scope_path == candidate || scope_path.ends_with(&format!("/{candidate}"))
}

fn resolve_scope_path(scope: &ReviewScope, candidate: &str) -> Option<PathBuf> {
    let matches = scope
        .files
        .iter()
        .filter(|path| path_matches(candidate, path))
        .collect::<Vec<_>>();
    (matches.len() == 1).then(|| matches[0].clone())
}

fn truncate_content(content: String, max_bytes: usize) -> String {
    if content.len() <= max_bytes {
        return content;
    }
    let content_budget = max_bytes.saturating_sub(TRUNCATION_MARKER.len());
    let mut end = content_budget;
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = content[..end].to_string();
    truncated.push_str(TRUNCATION_MARKER);
    truncated
}

fn excerpt_for_range(text: &str, line1: u32, line2: u32) -> (u32, u32, String) {
    let lines = text.lines().collect::<Vec<_>>();
    let context_line1 = line1.saturating_sub(EXCERPT_CONTEXT_LINES).max(1);
    let context_line2 = line2
        .saturating_add(EXCERPT_CONTEXT_LINES)
        .min(lines.len() as u32);
    let mut content = String::new();
    for line_number in context_line1..=context_line2 {
        content.push_str(&format!(
            "{line_number}: {}\n",
            lines[line_number.saturating_sub(1) as usize]
        ));
    }
    (context_line1, context_line2, content)
}

fn patch_header_path(line: &str) -> Option<&str> {
    line.strip_prefix("diff --git ")?
        .split_whitespace()
        .nth(1)
        .map(|path| path.trim_matches('"').trim_start_matches("b/"))
}

fn patch_path_matches(candidate: &Path, patch_path: &str) -> bool {
    let candidate = normalize_path(&candidate.to_string_lossy());
    let patch_path = normalize_path(patch_path);
    candidate == patch_path || candidate.ends_with(&format!("/{patch_path}"))
}

fn parse_new_hunk_range(header: &str) -> Option<(u32, u32)> {
    if header.trim() == "@@" {
        return Some((1, u32::MAX));
    }
    let range = header
        .split_whitespace()
        .find(|part| part.starts_with('+'))?
        .trim_start_matches('+');
    let (start, count) = match range.split_once(',') {
        Some((start, count)) => (start.parse::<u32>().ok()?, count.parse::<u32>().ok()?),
        None => (range.parse::<u32>().ok()?, 1),
    };
    let end = start.saturating_add(count.saturating_sub(1));
    Some((start, end))
}

fn ranges_overlap(line1: u32, line2: u32, other_line1: u32, other_line2: u32) -> bool {
    line1 <= other_line2 && other_line1 <= line2
}

fn overlapping_diff_hunks(patch: &str, file: &Path, line1: u32, line2: u32) -> Option<String> {
    let lines = patch.lines().collect::<Vec<_>>();
    let mut output = String::new();
    let mut current_file_matches = false;
    let mut index = 0;
    while index < lines.len() {
        if let Some(path) = patch_header_path(lines[index]) {
            current_file_matches = patch_path_matches(file, path);
            index += 1;
            continue;
        }
        if !current_file_matches || !lines[index].starts_with("@@") {
            index += 1;
            continue;
        }
        let hunk_start = index;
        index += 1;
        while index < lines.len()
            && !lines[index].starts_with("@@")
            && !lines[index].starts_with("diff --git ")
            && !lines[index].starts_with("## ")
        {
            index += 1;
        }
        let Some((hunk_line1, hunk_line2)) = parse_new_hunk_range(lines[hunk_start]) else {
            continue;
        };
        if !ranges_overlap(line1, line2, hunk_line1, hunk_line2) {
            continue;
        }
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&lines[hunk_start..index].join("\n"));
        output.push('\n');
    }
    (!output.is_empty()).then_some(output)
}

fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(first) if first == '_' || first.is_ascii_alphabetic())
        && chars.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn is_camel_or_snake(value: &str) -> bool {
    value.contains('_')
        || value
            .chars()
            .skip(1)
            .any(|character| character.is_ascii_uppercase())
}

fn symbol_tokens(claim: &str, excerpt: &str) -> Vec<String> {
    let mut symbols = BTreeSet::new();
    let mut remainder = claim;
    while let Some(start) = remainder.find('`') {
        remainder = &remainder[start + 1..];
        let Some(end) = remainder.find('`') else {
            break;
        };
        let value = &remainder[..end];
        if is_identifier(value) {
            symbols.insert(value.to_string());
        }
        remainder = &remainder[end + 1..];
    }
    for value in
        claim.split(|character: char| !(character == '_' || character.is_ascii_alphanumeric()))
    {
        if is_identifier(value) && is_camel_or_snake(value) && excerpt.contains(value) {
            symbols.insert(value.to_string());
        }
    }
    symbols.into_iter().take(MAX_SYMBOLS_PER_FINDING).collect()
}

async fn symbol_evidence(
    service: &crate::codegraph::CodeGraphService,
    graph: &SymbolGraphFacts,
    claim: &str,
    excerpt: &str,
) -> Result<Option<String>, String> {
    let symbols = symbol_tokens(claim, excerpt);
    if symbols.is_empty() {
        return Ok(None);
    }
    let mut lines = Vec::with_capacity(symbols.len());
    for symbol in symbols {
        let mut definitions = service.definitions(&symbol).await?;
        definitions.sort_by_key(|definition| {
            (
                normalize_path(&definition.cpath),
                definition.full_line1(),
                definition.path(),
            )
        });
        let Some(definition) = definitions.first() else {
            lines.push(format!("symbol {symbol}: NOT FOUND"));
            continue;
        };
        let usage_count = definitions
            .iter()
            .map(|definition| graph.usage_count(&definition.name(), &definition.cpath))
            .sum::<usize>();
        lines.push(format!(
            "symbol {symbol}: defined at {}:{}, {usage_count} usages",
            definition.cpath,
            definition.full_line1()
        ));
    }
    Ok(Some(lines.join("\n")))
}

fn rejection(finding: &ReviewFinding, index: usize, reason: &str) -> EvidenceRejection {
    EvidenceRejection {
        finding_id: finding.id.clone(),
        index: index + 1,
        reason: reason.to_string(),
    }
}

pub async fn collect_evidence(
    gcx: Arc<GlobalContext>,
    scope: &ReviewScope,
    findings: &mut Vec<ReviewFinding>,
) -> Vec<EvidenceRejection> {
    let codegraph = gcx.codegraph.lock().await.clone();
    let graph_facts = match codegraph.as_ref() {
        Some(service) => service
            .cached_graph_analytics()
            .await
            .ok()
            .map(|cached| SymbolGraphFacts::from_cached(&cached)),
        None => None,
    };
    let mut surviving = Vec::with_capacity(findings.len());
    let mut rejections = Vec::new();

    for (index, mut finding) in std::mem::take(findings).into_iter().enumerate() {
        let Some(file) = resolve_scope_path(scope, &finding.file) else {
            rejections.push(rejection(&finding, index, "file_not_in_scope"));
            continue;
        };
        let text = match get_file_text_from_memory_or_disk(gcx.clone(), &file).await {
            Ok(text) => text,
            Err(_) => {
                rejections.push(rejection(&finding, index, "file_unreadable"));
                continue;
            }
        };
        let line_count = text.lines().count() as u32;
        let normalized_line1 = finding.line1.max(1);
        if normalized_line1 > line_count {
            rejections.push(rejection(&finding, index, "range_out_of_bounds"));
            continue;
        }
        finding.line1 = normalized_line1;
        finding.line2 = finding.line2.max(normalized_line1).min(line_count);
        let (excerpt_line1, excerpt_line2, excerpt) =
            excerpt_for_range(&text, finding.line1, finding.line2);
        finding.evidence.push(ReviewEvidence {
            kind: "excerpt".to_string(),
            path: Some(file.to_string_lossy().to_string()),
            line1: Some(excerpt_line1),
            line2: Some(excerpt_line2),
            content: truncate_content(excerpt.clone(), MAX_EXCERPT_BYTES),
        });
        finding.checks_performed.push("excerpt_ok".to_string());

        if scope.diff_base.is_none() {
            finding
                .checks_performed
                .push("diff_hunk_skipped:no_diff_base".to_string());
        } else if !scope.changed_files.iter().any(|path| path == &file) {
            finding
                .checks_performed
                .push("diff_hunk_skipped:file_unchanged".to_string());
        } else if let Some(patch) = scope.diff_patch.as_deref() {
            if let Some(content) =
                overlapping_diff_hunks(patch, &file, finding.line1, finding.line2)
            {
                finding.evidence.push(ReviewEvidence {
                    kind: "diff_hunk".to_string(),
                    path: Some(file.to_string_lossy().to_string()),
                    line1: Some(finding.line1),
                    line2: Some(finding.line2),
                    content: truncate_content(content, MAX_DIFF_HUNK_BYTES),
                });
                finding
                    .checks_performed
                    .push("diff_hunk_attached".to_string());
            } else {
                finding
                    .checks_performed
                    .push("diff_hunk_skipped:no_overlap".to_string());
            }
        } else {
            finding
                .checks_performed
                .push("diff_hunk_skipped:patch_unavailable".to_string());
        }

        match (codegraph.as_deref(), graph_facts.as_ref()) {
            (None, _) => finding
                .checks_performed
                .push("symbols_skipped:codegraph_unavailable".to_string()),
            (Some(_), None) => finding
                .checks_performed
                .push("symbols_skipped:codegraph_error".to_string()),
            (Some(service), Some(graph)) => {
                match symbol_evidence(service, graph, &finding.claim, &excerpt).await {
                    Ok(content) => {
                        finding.checks_performed.push("symbols_checked".to_string());
                        if let Some(content) = content {
                            finding.evidence.push(ReviewEvidence {
                                kind: "symbol".to_string(),
                                path: None,
                                line1: None,
                                line2: None,
                                content: truncate_content(content, MAX_SYMBOL_BYTES),
                            });
                        }
                    }
                    Err(_) => finding
                        .checks_performed
                        .push("symbols_skipped:codegraph_error".to_string()),
                }
            }
        }
        surviving.push(finding);
    }

    *findings = surviving;
    rejections
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::code_review_scope::ReviewBudgets;
    use crate::tools::code_review_types::{ReviewSeverity, VerificationStatus};

    fn finding(file: &Path, line1: u32, line2: u32, claim: &str) -> ReviewFinding {
        ReviewFinding {
            id: String::new(),
            category: "correctness".to_string(),
            severity: ReviewSeverity::High,
            confidence: 0.8,
            verification_status: VerificationStatus::Unverified,
            file: file.to_string_lossy().to_string(),
            line1,
            line2,
            claim: claim.to_string(),
            evidence: vec![],
            impact: None,
            remediation: None,
            checks_performed: vec![],
        }
    }

    fn scope(file: PathBuf) -> ReviewScope {
        ReviewScope {
            files: vec![file],
            seed_files: vec![],
            focus: None,
            diff_base: None,
            changed_files: vec![],
            diff_patch: None,
            budgets: ReviewBudgets {
                max_files: 10,
                tokens_budget: 10_000,
                max_candidates: 30,
            },
        }
    }

    async fn gcx_for(root: &Path) -> Arc<GlobalContext> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.to_path_buf()];
        gcx
    }

    #[tokio::test]
    async fn tool_code_review_evidence_attaches_excerpt_with_context_and_clamps_end() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("sample.rs");
        std::fs::write(
            &file,
            (1..=12)
                .map(|line| format!("line {line}"))
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        let gcx = gcx_for(temp.path()).await;
        let mut findings = vec![finding(&file, 8, 30, "The branch fails.")];

        let rejected = collect_evidence(gcx, &scope(file.clone()), &mut findings).await;

        assert!(rejected.is_empty());
        assert_eq!(findings[0].line2, 12);
        assert_eq!(findings[0].evidence[0].kind, "excerpt");
        assert_eq!(findings[0].evidence[0].line1, Some(3));
        assert_eq!(findings[0].evidence[0].line2, Some(12));
        assert!(findings[0].evidence[0].content.contains("8: line 8"));
        assert!(findings[0]
            .checks_performed
            .contains(&"symbols_skipped:codegraph_unavailable".to_string()));
    }

    #[tokio::test]
    async fn tool_code_review_evidence_rejects_out_of_range_and_out_of_scope_findings() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("sample.rs");
        let other = temp.path().join("other.rs");
        std::fs::write(&file, "one\ntwo\n").unwrap();
        std::fs::write(&other, "one\n").unwrap();
        let gcx = gcx_for(temp.path()).await;
        let mut findings = vec![
            finding(&file, 3, 3, "Outside lines."),
            finding(&other, 1, 1, "Outside scope."),
        ];

        let rejected = collect_evidence(gcx, &scope(file), &mut findings).await;

        assert!(findings.is_empty());
        assert_eq!(rejected[0].reason, "range_out_of_bounds");
        assert_eq!(
            rejected[0].check_name(),
            "evidence_reject:1:range_out_of_bounds"
        );
        assert_eq!(rejected[1].reason, "file_not_in_scope");
        assert_eq!(
            rejected[1].check_name(),
            "evidence_reject:2:file_not_in_scope"
        );
    }

    #[tokio::test]
    async fn tool_code_review_evidence_attaches_overlapping_precomputed_diff_hunk() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("src").join("sample.rs");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "one\ntwo changed\nthree\n").unwrap();
        let gcx = gcx_for(temp.path()).await;
        let mut review_scope = scope(file.clone());
        review_scope.diff_base = Some("base".to_string());
        review_scope.changed_files = vec![file.clone()];
        review_scope.diff_patch = Some(
            "diff --git a/src/sample.rs b/src/sample.rs\n--- a/src/sample.rs\n+++ b/src/sample.rs\n@@ -1,3 +1,3 @@\n one\n-two\n+two changed\n three\n"
                .to_string(),
        );
        let mut findings = vec![finding(&file, 2, 2, "Changed line.")];

        collect_evidence(gcx, &review_scope, &mut findings).await;

        let diff = findings[0]
            .evidence
            .iter()
            .find(|evidence| evidence.kind == "diff_hunk")
            .unwrap();
        assert!(diff.content.contains("+two changed"));
        assert!(findings[0]
            .checks_performed
            .contains(&"diff_hunk_attached".to_string()));
    }

    #[tokio::test]
    async fn tool_code_review_evidence_silently_records_missing_patch_and_codegraph() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("sample.rs");
        std::fs::write(&file, "fn sample() {}\n").unwrap();
        let gcx = gcx_for(temp.path()).await;
        let mut review_scope = scope(file.clone());
        review_scope.diff_base = Some("base".to_string());
        review_scope.changed_files = vec![file.clone()];
        let mut findings = vec![finding(&file, 1, 1, "`sample` can fail.")];

        collect_evidence(gcx, &review_scope, &mut findings).await;

        assert_eq!(findings[0].evidence.len(), 1);
        assert!(findings[0]
            .checks_performed
            .contains(&"diff_hunk_skipped:patch_unavailable".to_string()));
        assert!(findings[0]
            .checks_performed
            .contains(&"symbols_skipped:codegraph_unavailable".to_string()));
    }

    #[tokio::test]
    async fn tool_code_review_evidence_attaches_codegraph_symbol_facts() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("sample.rs");
        let text = "pub struct TargetSymbol;\npub fn caller() { let _ = TargetSymbol; }\n";
        std::fs::write(&file, text).unwrap();
        let gcx = gcx_for(temp.path()).await;
        let service = Arc::new(crate::codegraph::CodeGraphService::open_in_memory().unwrap());
        service
            .index_file(&file.to_string_lossy(), text, "rust")
            .await
            .unwrap();
        service.connect_usages().await.unwrap();
        *gcx.codegraph.lock().await = Some(service);
        let mut findings = vec![finding(
            &file,
            1,
            2,
            "`TargetSymbol` is referenced incorrectly.",
        )];

        collect_evidence(gcx, &scope(file), &mut findings).await;

        let symbol = findings[0]
            .evidence
            .iter()
            .find(|evidence| evidence.kind == "symbol")
            .unwrap();
        assert!(symbol.content.contains("symbol TargetSymbol: defined at"));
        assert!(symbol.content.contains("usages"));
        assert!(findings[0]
            .checks_performed
            .contains(&"symbols_checked".to_string()));
    }

    #[tokio::test]
    async fn tool_code_review_evidence_caps_total_content_with_marker() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("sample.rs");
        std::fs::write(&file, format!("{}\n", "x".repeat(5000))).unwrap();
        let gcx = gcx_for(temp.path()).await;
        let mut findings = vec![finding(&file, 1, 1, "Long line.")];

        collect_evidence(gcx, &scope(file), &mut findings).await;

        let total = findings[0]
            .evidence
            .iter()
            .map(|evidence| evidence.content.len())
            .sum::<usize>();
        assert!(total <= MAX_EXCERPT_BYTES + MAX_DIFF_HUNK_BYTES + MAX_SYMBOL_BYTES);
        assert!(findings[0].evidence[0]
            .content
            .contains("[evidence truncated]"));
    }

    #[test]
    fn tool_code_review_evidence_module_has_no_process_spawns() {
        let source = include_str!("code_review_evidence.rs");
        let std_process = ["process", "::Command"].concat();
        let tokio_process = ["tokio", "::process"].concat();

        assert!(!source.contains(&std_process));
        assert!(!source.contains(&tokio_process));
    }
}
