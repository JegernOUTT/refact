use crate::decisions::*;
use serde::{Deserialize, Serialize};

pub const SOURCE_RANK_CLI: u8 = 9;
pub const SOURCE_RANK_ADR: u8 = 8;
pub const SOURCE_RANK_PR: u8 = 7;
pub const SOURCE_RANK_COMMIT: u8 = 6;
pub const SOURCE_RANK_CHANGELOG: u8 = 5;
pub const SOURCE_RANK_INLINE_MARKER: u8 = 4;
pub const SOURCE_RANK_COMMENT: u8 = 3;
pub const SOURCE_RANK_CODE_COMMENT: u8 = 2;
pub const SOURCE_RANK_LLM_INFERRED: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractedDecision {
    pub statement: String,
    pub evidence: String,
    pub source_kind: String,
    pub source_rank: u8,
    pub status: DecisionStatus,
}

const INLINE_MARKERS: &[&str] = &[
    "WHY:",
    "DECISION:",
    "TRADEOFF:",
    "ADR:",
    "RATIONALE:",
    "REJECTED:",
];

const RATIONALE_MARKERS: &[&str] = &[
    "because",
    "so that",
    "in order to",
    "to avoid",
    "instead of",
    "the reason",
    "we need to",
    "due to",
];

fn strip_comment_leader(line: &str) -> (&str, bool) {
    let trimmed = line.trim_start();
    for leader in ["//", "#", "--", "*"] {
        if let Some(rest) = trimmed.strip_prefix(leader) {
            return (rest.trim_start(), true);
        }
    }
    (trimmed, false)
}

fn marker_payload(line: &str) -> Option<String> {
    let (body, _) = strip_comment_leader(line);
    let lower = body.to_ascii_lowercase();
    for marker in INLINE_MARKERS {
        let marker_lower = marker.to_ascii_lowercase();
        if lower.starts_with(&marker_lower) {
            return Some(body[marker.len()..].trim().to_string());
        }
    }
    None
}

fn has_inline_marker(line: &str) -> bool {
    marker_payload(line).is_some()
}

fn decision(
    statement: String,
    evidence: String,
    source_kind: &str,
    source_rank: u8,
    status: DecisionStatus,
) -> ExtractedDecision {
    ExtractedDecision {
        statement,
        evidence,
        source_kind: source_kind.to_string(),
        source_rank,
        status,
    }
}

pub fn mine_inline_markers(text: &str, source_kind: &str) -> Vec<ExtractedDecision> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let Some(mut payload) = marker_payload(lines[i]) else {
            i += 1;
            continue;
        };

        i += 1;
        while i < lines.len() {
            if has_inline_marker(lines[i]) {
                break;
            }
            let original = lines[i];
            let (body, had_leader) = strip_comment_leader(original);
            let is_indented = original.starts_with(' ') || original.starts_with('\t');
            if body.is_empty() || !(had_leader || is_indented) {
                break;
            }
            if !payload.is_empty() {
                payload.push('\n');
            }
            payload.push_str(body.trim());
            i += 1;
        }

        let evidence = payload.trim().to_string();
        if !evidence.is_empty() {
            out.push(decision(
                evidence.clone(),
                evidence,
                source_kind,
                SOURCE_RANK_INLINE_MARKER,
                DecisionStatus::Verified,
            ));
        }
    }

    out
}

fn heading_level_and_title(line: &str) -> Option<(usize, String)> {
    let trimmed = line.trim();
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = trimmed[hashes..].trim();
    if rest.is_empty() {
        None
    } else {
        Some((hashes, rest.to_string()))
    }
}

fn normalize_adr_title(title: &str) -> String {
    let trimmed = title.trim();
    let without_number = trimmed
        .split_once('.')
        .and_then(|(prefix, rest)| {
            if prefix.trim().chars().all(|c| c.is_ascii_digit()) {
                Some(rest.trim())
            } else {
                None
            }
        })
        .unwrap_or(trimmed);
    without_number.to_string()
}

fn section_body(lines: &[&str], start: usize, level: usize) -> String {
    let mut body = Vec::new();
    for line in lines.iter().skip(start + 1) {
        if let Some((next_level, _)) = heading_level_and_title(line) {
            if next_level <= level {
                break;
            }
        }
        body.push(*line);
    }
    body.join("\n").trim().to_string()
}

pub fn mine_adr(text: &str) -> Vec<ExtractedDecision> {
    let lines: Vec<&str> = text.lines().collect();
    let mut title = None;
    let mut status = None;
    let mut decision_body = None;
    let mut context_body = None;

    for (idx, line) in lines.iter().enumerate() {
        let Some((level, heading)) = heading_level_and_title(line) else {
            continue;
        };
        let lower = heading.to_ascii_lowercase();
        if level == 1 && title.is_none() {
            title = Some(normalize_adr_title(&heading));
        } else if lower == "status" {
            let body = section_body(&lines, idx, level);
            status = body
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .map(|line| {
                    line.trim_matches(|c: char| c == '*' || c == '-' || c.is_whitespace())
                        .to_string()
                });
        } else if lower == "decision" {
            decision_body = Some(section_body(&lines, idx, level));
        } else if lower == "context" {
            context_body = Some(section_body(&lines, idx, level));
        }
    }

    let Some(title) = title else {
        return Vec::new();
    };
    let evidence = decision_body
        .filter(|body| !body.trim().is_empty())
        .or_else(|| context_body.filter(|body| !body.trim().is_empty()))
        .unwrap_or_else(|| title.clone());
    let normalized_status = status.as_deref().map(str::to_ascii_lowercase);
    let statement = match (status.as_deref(), normalized_status.as_deref()) {
        (Some(status), Some("accepted" | "proposed" | "superseded" | "deprecated")) => {
            format!("{} ({})", title, status)
        }
        _ => title,
    };
    let status = classify_evidence(&evidence, text);

    vec![decision(
        statement,
        evidence,
        "adr",
        SOURCE_RANK_ADR,
        status,
    )]
}

pub fn mine_changelog(text: &str) -> Vec<ExtractedDecision> {
    let mut out = Vec::new();
    let mut in_target_section = false;

    for line in text.lines() {
        let trimmed = line.trim();
        if let Some((level, heading)) = heading_level_and_title(trimmed) {
            if level <= 2 {
                in_target_section = false;
            } else if level == 3 {
                let lower = heading.to_ascii_lowercase();
                in_target_section = matches!(lower.as_str(), "changed" | "removed" | "deprecated");
            }
            continue;
        }

        if in_target_section {
            let bullet = trimmed
                .strip_prefix("- ")
                .or_else(|| trimmed.strip_prefix("* "));
            if let Some(bullet) = bullet {
                let item = bullet.trim().to_string();
                if !item.is_empty() {
                    out.push(decision(
                        item.clone(),
                        item,
                        "changelog",
                        SOURCE_RANK_CHANGELOG,
                        DecisionStatus::Verified,
                    ));
                }
            }
        }
    }

    out
}

pub fn mine_pr_body(text: &str) -> Vec<ExtractedDecision> {
    let lower = text.to_ascii_lowercase();
    let gated = [
        "## why",
        "## motivation",
        "## rationale",
        "closes #",
        "fixes #",
    ]
    .iter()
    .any(|marker| lower.contains(marker));
    if !gated {
        return Vec::new();
    }

    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        let Some((level, heading)) = heading_level_and_title(line) else {
            continue;
        };
        if level != 2 {
            continue;
        }
        let lower_heading = heading.to_ascii_lowercase();
        if matches!(lower_heading.as_str(), "why" | "motivation" | "rationale") {
            let body = section_body(&lines, idx, level);
            if !body.is_empty() {
                out.push(decision(
                    body.clone(),
                    body,
                    "pr",
                    SOURCE_RANK_PR,
                    DecisionStatus::Verified,
                ));
            }
        }
    }

    out
}

fn strip_code_comment(line: &str, in_block: &mut bool) -> Option<String> {
    if *in_block {
        if let Some(end) = line.find("*/") {
            let comment = &line[..end];
            *in_block = false;
            return Some(comment.trim_start_matches('*').trim().to_string());
        }
        return Some(line.trim().trim_start_matches('*').trim().to_string());
    }

    let trimmed = line.trim_start();
    if let Some(rest) = trimmed.strip_prefix("//") {
        return Some(rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix('#') {
        return Some(rest.trim().to_string());
    }
    if let Some(start) = trimmed.find("/*") {
        let after = &trimmed[start + 2..];
        if let Some(end) = after.find("*/") {
            return Some(after[..end].trim().to_string());
        }
        *in_block = true;
        return Some(after.trim().to_string());
    }
    None
}

fn is_filtered_comment(comment: &str) -> bool {
    let lower = comment.to_ascii_lowercase();
    if comment.contains("SPDX-")
        || comment.contains("Copyright")
        || lower.contains("licensed under")
    {
        return true;
    }
    let contains_code_punct =
        comment.contains(';') || comment.contains('{') || comment.contains('}');
    let contains_rationale = RATIONALE_MARKERS
        .iter()
        .any(|marker| lower.contains(marker));
    contains_code_punct && !contains_rationale
}

pub fn harvest_rationale_comments(text: &str) -> Vec<ExtractedDecision> {
    let mut out = Vec::new();
    let mut in_block = false;

    for line in text.lines() {
        let Some(comment) = strip_code_comment(line, &mut in_block) else {
            continue;
        };
        let evidence = comment.trim();
        if evidence.is_empty() || is_filtered_comment(evidence) {
            continue;
        }
        let lower = evidence.to_ascii_lowercase();
        if RATIONALE_MARKERS
            .iter()
            .any(|marker| lower.contains(marker))
        {
            out.push(decision(
                evidence.to_string(),
                evidence.to_string(),
                "code_comment",
                SOURCE_RANK_CODE_COMMENT,
                DecisionStatus::Verified,
            ));
        }
    }

    out
}

fn push_deduped(out: &mut Vec<ExtractedDecision>, candidate: ExtractedDecision) {
    if let Some(existing) = out
        .iter_mut()
        .find(|item| item.statement == candidate.statement)
    {
        if candidate.source_rank > existing.source_rank {
            *existing = candidate;
        }
    } else {
        out.push(candidate);
    }
}

pub fn extract_all(sources: &[DecisionSource]) -> Vec<ExtractedDecision> {
    let mut out = Vec::new();

    for source in sources {
        let mut extracted = match source.kind.as_str() {
            "adr" => mine_adr(&source.text),
            "changelog" => mine_changelog(&source.text),
            "pr" => mine_pr_body(&source.text),
            "commit" | "comment" => {
                let mut decisions = mine_inline_markers(&source.text, &source.kind);
                decisions.extend(harvest_rationale_comments(&source.text));
                decisions
            }
            _ => mine_inline_markers(&source.text, &source.kind),
        };

        for candidate in extracted.drain(..) {
            push_deduped(&mut out, candidate);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_decision_marker_is_found_at_rank_4() {
        let decisions = mine_inline_markers("# DECISION: use sqlite", "comment");
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].statement, "use sqlite");
        assert_eq!(decisions[0].source_rank, SOURCE_RANK_INLINE_MARKER);
        assert_eq!(decisions[0].status, DecisionStatus::Verified);
    }

    #[test]
    fn nygard_adr_decision_is_found_at_rank_8() {
        let text = "# 1. Use X\n\n## Status\nAccepted\n\n## Decision\nWe will use X\n";
        let decisions = mine_adr(text);
        assert_eq!(decisions.len(), 1);
        assert!(decisions[0].statement.contains("Use X"));
        assert!(decisions[0].statement.contains("Accepted"));
        assert_eq!(decisions[0].evidence, "We will use X");
        assert_eq!(decisions[0].source_rank, SOURCE_RANK_ADR);
    }

    #[test]
    fn changelog_changed_bullets_are_found_but_added_is_not() {
        let text =
            "## Unreleased\n\n### Added\n- shiny thing\n\n### Changed\n- switched to tree-sitter\n";
        let decisions = mine_changelog(text);
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].statement, "switched to tree-sitter");
        assert_eq!(decisions[0].source_rank, SOURCE_RANK_CHANGELOG);
    }

    #[test]
    fn pr_why_body_is_found_at_rank_7() {
        let decisions = mine_pr_body("## Why\nbecause it is faster\n");
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].statement, "because it is faster");
        assert_eq!(decisions[0].source_rank, SOURCE_RANK_PR);
    }

    #[test]
    fn rationale_comment_is_found_and_copyright_is_filtered() {
        let text = "// Copyright 2020\n// we use a cache because lookups are hot\n";
        let decisions = harvest_rationale_comments(text);
        assert_eq!(decisions.len(), 1);
        assert_eq!(
            decisions[0].evidence,
            "we use a cache because lookups are hot"
        );
        assert_eq!(decisions[0].source_rank, SOURCE_RANK_CODE_COMMENT);
    }

    #[test]
    fn extract_all_dedupes_duplicate_statement_keeping_higher_rank() {
        let sources = vec![
            DecisionSource {
                kind: "commit".to_string(),
                text: "// we use a cache because lookups are hot".to_string(),
            },
            DecisionSource {
                kind: "changelog".to_string(),
                text: "### Changed\n- we use a cache because lookups are hot".to_string(),
            },
        ];

        let decisions = extract_all(&sources);
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].source_rank, SOURCE_RANK_CHANGELOG);
        assert_eq!(decisions[0].source_kind, "changelog");
    }
}
