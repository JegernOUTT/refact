use std::path::PathBuf;
use std::sync::Arc;

use crate::files_in_workspace::get_file_text_from_memory_or_disk;
use crate::global_context::GlobalContext;
use crate::tools::review_scope::{paths_match, ReviewScope};
use crate::tools::review_types::ReviewFinding;

const WINDOW_LINES: u32 = 3;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EvidenceOutcome {
    pub checked: usize,
    pub present: usize,
    pub relocated: usize,
    pub unreadable: usize,
}

fn strip_line_number_prefix(line: &str) -> &str {
    let trimmed = line.trim_start();
    let digits: String = trimmed.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() || digits.len() > 7 {
        return trimmed;
    }
    let rest = &trimmed[digits.len()..];
    for marker in [": ", ":", "| ", "|", " "] {
        if let Some(stripped) = rest.strip_prefix(marker) {
            return stripped;
        }
    }
    trimmed
}

pub fn normalize_snippet(text: &str) -> String {
    text.lines()
        .map(strip_line_number_prefix)
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn window_text(text: &str, line_start: u32, line_end: u32) -> String {
    let first = line_start.saturating_sub(WINDOW_LINES).max(1) as usize;
    let last = line_end.saturating_add(WINDOW_LINES) as usize;
    text.lines()
        .enumerate()
        .filter(|(index, _)| {
            let line = index + 1;
            line >= first && line <= last
        })
        .map(|(_, line)| line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn locate_snippet(text: &str, needle: &str) -> Option<(u32, u32)> {
    let needle_lines: Vec<String> = normalize_snippet(needle)
        .lines()
        .map(str::to_string)
        .collect();
    if needle_lines.is_empty() {
        return None;
    }
    let file_lines: Vec<String> = text
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect();
    let first = needle_lines.first()?;
    for (index, line) in file_lines.iter().enumerate() {
        if !line.contains(first.as_str()) {
            continue;
        }
        let mut cursor = index;
        let mut matched = 0;
        for needle_line in &needle_lines {
            let mut found = false;
            while cursor < file_lines.len() {
                if file_lines[cursor].contains(needle_line.as_str()) {
                    found = true;
                    cursor += 1;
                    break;
                }
                if !file_lines[cursor].trim().is_empty() {
                    break;
                }
                cursor += 1;
            }
            if !found {
                break;
            }
            matched += 1;
        }
        if matched == needle_lines.len() {
            return Some((index as u32 + 1, cursor as u32));
        }
    }
    None
}

pub fn resolve_scope_path(scope: &ReviewScope, candidate: &str) -> Option<PathBuf> {
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return None;
    }
    let direct = PathBuf::from(candidate);
    if direct.is_absolute() && direct.exists() {
        return Some(direct);
    }
    let known = scope
        .files
        .iter()
        .chain(scope.changed_files.iter())
        .find(|path| paths_match(&path.to_string_lossy(), candidate));
    if let Some(path) = known {
        return Some(path.clone());
    }
    if let Some(root) = scope.repo_root.as_ref() {
        let joined = root.join(candidate);
        if joined.exists() {
            return Some(joined);
        }
    }
    direct.exists().then_some(direct)
}

pub async fn verify_evidence(
    gcx: Arc<GlobalContext>,
    scope: &ReviewScope,
    findings: &mut [ReviewFinding],
) -> EvidenceOutcome {
    let mut outcome = EvidenceOutcome::default();
    for finding in findings.iter_mut() {
        outcome.checked += 1;
        if finding.evidence.trim().is_empty() {
            continue;
        }
        let Some(path) = resolve_scope_path(scope, &finding.file) else {
            outcome.unreadable += 1;
            continue;
        };
        let Ok(text) = get_file_text_from_memory_or_disk(gcx.clone(), &path).await else {
            outcome.unreadable += 1;
            continue;
        };
        finding.file = path.to_string_lossy().to_string();
        let window = window_text(&text, finding.line_start, finding.line_end);
        let needle = normalize_snippet(&finding.evidence);
        if !needle.is_empty() && normalize_snippet(&window).contains(&needle) {
            finding.evidence_present = true;
            outcome.present += 1;
            continue;
        }
        if let Some((start, end)) = locate_snippet(&text, &finding.evidence) {
            finding.line_start = start;
            finding.line_end = end.max(start);
            finding.evidence_present = true;
            outcome.present += 1;
            outcome.relocated += 1;
        }
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::review_scope::DiffHunks;
    use crate::tools::review_types::{ReviewSeverity, ScopeMode};

    const FILE: &str = "fn one() {\n    let value = 1;\n}\n\nfn two() {\n    return Ok(());\n}\n";

    fn scope_for(root: &std::path::Path, file: &std::path::Path) -> ReviewScope {
        ReviewScope {
            mode: ScopeMode::Strict,
            requested: vec![file.to_path_buf()],
            files: vec![file.to_path_buf()],
            dropped_files: vec![],
            changed_files: vec![],
            focus: None,
            plan: None,
            base: None,
            head: None,
            diff_patch: None,
            patch_total_bytes: 0,
            hunks: DiffHunks::default(),
            repo_root: Some(root.to_path_buf()),
            expansion: None,
        }
    }

    fn finding(file: &str, line_start: u32, line_end: u32, evidence: &str) -> ReviewFinding {
        ReviewFinding {
            id: String::new(),
            stage: "diff".to_string(),
            model: None,
            title: "title".to_string(),
            severity: ReviewSeverity::High,
            file: file.to_string(),
            line_start,
            line_end,
            claim: "claim".to_string(),
            evidence: evidence.to_string(),
            evidence_present: false,
            reproduction: None,
            fix: None,
            introduced_by_diff: false,
            out_of_scope: false,
            reported_by: vec![],
            locations: vec![],
            disputed: None,
        }
    }

    #[tokio::test]
    async fn review_evidence_confirms_quote_inside_the_window() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("lib.rs");
        std::fs::write(&path, FILE).unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let scope = scope_for(temp.path(), &path);
        let mut findings = vec![finding("lib.rs", 6, 6, "return Ok(());")];

        let outcome = verify_evidence(gcx, &scope, &mut findings).await;

        assert!(findings[0].evidence_present);
        assert_eq!(findings[0].line_start, 6);
        assert_eq!(outcome.present, 1);
        assert_eq!(outcome.relocated, 0);
    }

    #[tokio::test]
    async fn review_evidence_relocates_a_quote_found_elsewhere_in_the_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("lib.rs");
        std::fs::write(&path, FILE).unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let scope = scope_for(temp.path(), &path);
        let mut findings = vec![finding("lib.rs", 120, 124, "    return Ok(());")];

        let outcome = verify_evidence(gcx, &scope, &mut findings).await;

        assert!(findings[0].evidence_present);
        assert_eq!(findings[0].line_start, 6);
        assert_eq!(findings[0].line_end, 6);
        assert_eq!(outcome.relocated, 1);
    }

    #[tokio::test]
    async fn review_evidence_rejects_a_quote_that_is_not_in_the_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("lib.rs");
        std::fs::write(&path, FILE).unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let scope = scope_for(temp.path(), &path);
        let mut findings = vec![
            finding("lib.rs", 2, 2, "let value = 999;"),
            finding("nowhere.rs", 1, 1, "let value = 1;"),
        ];

        let outcome = verify_evidence(gcx, &scope, &mut findings).await;

        assert!(!findings[0].evidence_present);
        assert!(!findings[1].evidence_present);
        assert_eq!(outcome.present, 0);
        assert_eq!(outcome.unreadable, 1);
    }

    #[tokio::test]
    async fn review_evidence_accepts_quotes_with_line_number_prefixes() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("lib.rs");
        std::fs::write(&path, FILE).unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let scope = scope_for(temp.path(), &path);
        let mut findings = vec![finding(
            "lib.rs",
            5,
            7,
            "5: fn two() {\n6:     return Ok(());",
        )];

        verify_evidence(gcx, &scope, &mut findings).await;

        assert!(findings[0].evidence_present);
    }

    #[test]
    fn review_evidence_normalizes_whitespace_and_drops_blank_lines() {
        assert_eq!(
            normalize_snippet("  let   a = 1;  \n\n\tlet b = 2;\n"),
            "let a = 1;\nlet b = 2;"
        );
    }
}
