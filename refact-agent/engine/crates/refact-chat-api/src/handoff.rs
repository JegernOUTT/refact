use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const HANDOFF_BLOCK_TAG: &str = "handoff";
pub const HANDOFF_SOURCE_CHILD_DECLARATION: &str = "child_declaration";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffGapCategory {
    MissingFile,
    MissingDecision,
    MissingMessageContext,
    AmbiguousRequirement,
    MissingVerificationCommand,
    Other,
}

impl HandoffGapCategory {
    pub const ALL: [HandoffGapCategory; 6] = [
        HandoffGapCategory::MissingFile,
        HandoffGapCategory::MissingDecision,
        HandoffGapCategory::MissingMessageContext,
        HandoffGapCategory::AmbiguousRequirement,
        HandoffGapCategory::MissingVerificationCommand,
        HandoffGapCategory::Other,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::MissingFile => "missing_file",
            Self::MissingDecision => "missing_decision",
            Self::MissingMessageContext => "missing_message_context",
            Self::AmbiguousRequirement => "ambiguous_requirement",
            Self::MissingVerificationCommand => "missing_verification_command",
            Self::Other => "other",
        }
    }

    pub fn parse(value: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|category| category.as_str().eq_ignore_ascii_case(value.trim()))
            .unwrap_or(Self::Other)
    }

    /// A gap the parent can close mechanically next time by naming the source in
    /// the launch specification. Everything else needs a human or planner edit.
    pub fn mechanically_fixable(self) -> bool {
        matches!(self, Self::MissingFile)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffSufficiency {
    #[default]
    Unknown,
    Sufficient,
    GapsReported,
}

impl HandoffSufficiency {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Sufficient => "sufficient",
            Self::GapsReported => "gaps_reported",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffGap {
    pub category: HandoffGapCategory,
    #[serde(default)]
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_steps: Option<u32>,
}

/// The child's claim about the brief it was given. It is recorded with
/// provenance and never rewrites the parent's launch specification.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffReport {
    #[serde(default)]
    pub sufficiency: HandoffSufficiency,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub claimed_by: String,
    #[serde(default)]
    pub claimed_at_ms: i64,
    #[serde(default)]
    pub gaps: Vec<HandoffGap>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandoffLimits {
    pub max_entries: usize,
    pub max_chars: usize,
}

impl Default for HandoffLimits {
    fn default() -> Self {
        Self {
            max_entries: 16,
            max_chars: 512,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HandoffDeclaration {
    #[serde(default)]
    gaps: Vec<GapDeclaration>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GapDeclaration {
    #[serde(default)]
    category: String,
    #[serde(default)]
    detail: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    extra_steps: Option<u32>,
}

impl HandoffReport {
    pub fn summary_lines(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "handoff_sufficiency: {} (claimed by {}, {})",
            self.sufficiency.as_str(),
            self.claimed_by,
            self.source
        )];
        for gap in &self.gaps {
            let mut line = format!("- {}: {}", gap.category.as_str(), gap.detail);
            if let Some(path) = &gap.path {
                line.push_str(&format!(" [{path}]"));
            }
            if let Some(steps) = gap.extra_steps {
                line.push_str(&format!(" (~{steps} extra steps)"));
            }
            lines.push(line);
        }
        lines.extend(self.notes.iter().map(|note| format!("- note: {note}")));
        lines
    }
}

pub fn render_instructions(limits: &HandoffLimits) -> String {
    let categories = HandoffGapCategory::ALL
        .into_iter()
        .map(HandoffGapCategory::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "# Handoff sufficiency (optional)\nIf your brief was missing something you had to discover or guess, you MAY end your final message with exactly one ```{HANDOFF_BLOCK_TAG} block:\n```{HANDOFF_BLOCK_TAG}\n{{\"gaps\":[{{\"category\":\"missing_file\",\"detail\":\"had to read the retry helper to learn the backoff rule\",\"path\":\"src/auth/retry.rs\",\"extra_steps\":3}}]}}\n```\nCategories: {categories}. At most {} entries and {} characters per field; anything longer is truncated with an explicit marker. Omitting the block is not a failure and is recorded as `unknown`, never as `sufficient`. This measures the brief you were given, not your work, and it grants no tools, paths or budget.",
        limits.max_entries, limits.max_chars
    )
}

/// Reads the child's optional declaration out of its final message. `None` means
/// the child said nothing, which is `unknown` and never blocks completion.
pub fn parse_report(
    final_text: &str,
    claimed_by: &str,
    claimed_at_ms: i64,
    limits: &HandoffLimits,
) -> Option<HandoffReport> {
    let blocks = tagged_blocks(final_text, HANDOFF_BLOCK_TAG);
    let first = blocks.first()?;
    let mut report = HandoffReport {
        sufficiency: HandoffSufficiency::Unknown,
        source: HANDOFF_SOURCE_CHILD_DECLARATION.to_string(),
        claimed_by: claimed_by.to_string(),
        claimed_at_ms,
        gaps: Vec::new(),
        notes: Vec::new(),
    };
    if blocks.len() > 1 {
        report.notes.push(format!(
            "{} handoff blocks were declared; only the first was read",
            blocks.len()
        ));
    }
    let declaration = match serde_json::from_str::<HandoffDeclaration>(first) {
        Ok(declaration) => declaration,
        Err(error) => {
            report.notes.push(format!(
                "handoff declaration was not readable and stays unknown: {error}"
            ));
            return Some(report);
        }
    };
    let max_entries = limits.max_entries.max(1);
    let max_chars = limits.max_chars.max(1);
    let declared = declaration.gaps.len();
    let mut truncated_fields = 0usize;
    for raw in declaration.gaps.into_iter().take(max_entries) {
        let (detail, detail_capped) = cap(&raw.detail, max_chars);
        let path = raw
            .path
            .map(|path| cap(&path, max_chars))
            .filter(|(path, _)| !path.trim().is_empty());
        truncated_fields += usize::from(detail_capped);
        truncated_fields += usize::from(path.as_ref().is_some_and(|(_, capped)| *capped));
        report.gaps.push(HandoffGap {
            category: HandoffGapCategory::parse(&raw.category),
            detail,
            path: path.map(|(path, _)| path),
            extra_steps: raw.extra_steps,
        });
    }
    if declared > report.gaps.len() {
        report.notes.push(format!(
            "handoff gaps truncated: kept {} of {declared} declared entries",
            report.gaps.len()
        ));
    }
    if truncated_fields > 0 {
        report.notes.push(format!(
            "{truncated_fields} declared field(s) exceeded {max_chars} characters and were truncated"
        ));
    }
    report.sufficiency = if report.gaps.is_empty() {
        HandoffSufficiency::Sufficient
    } else {
        HandoffSufficiency::GapsReported
    };
    Some(report)
}

fn tagged_blocks(text: &str, tag: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        match &mut current {
            Some(block) => {
                if trimmed.starts_with("```") {
                    blocks.push(std::mem::take(block));
                    current = None;
                } else {
                    block.push_str(line);
                    block.push('\n');
                }
            }
            None => {
                if trimmed
                    .strip_prefix("```")
                    .is_some_and(|rest| rest.trim().eq_ignore_ascii_case(tag))
                {
                    current = Some(String::new());
                }
            }
        }
    }
    blocks
}

fn cap(text: &str, max: usize) -> (String, bool) {
    let total = text.chars().count();
    if total <= max {
        return (text.to_string(), false);
    }
    let kept: String = text.chars().take(max).collect();
    (format!("{kept}… [truncated, {total} chars]"), true)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffMetric {
    pub key: String,
    pub completions: u32,
    pub reported: u32,
    pub unknown: u32,
    pub sufficient: u32,
    pub with_gaps: u32,
    pub gaps: u32,
    pub extra_steps: u32,
    #[serde(default)]
    pub gaps_by_category: BTreeMap<String, u32>,
    #[serde(default)]
    pub suggested_sources: Vec<String>,
    /// Suggestions dropped because the destination may not receive the path.
    #[serde(default)]
    pub withheld_sources: u32,
}

impl HandoffMetric {
    pub fn summary_lines(&self) -> Vec<String> {
        let mut lines = vec![
            format!("- key: {}", self.key),
            format!(
                "- completions: {} (reported {}, unknown {})",
                self.completions, self.reported, self.unknown
            ),
        ];
        if self.gaps > 0 {
            lines.push(format!(
                "- gaps: {}",
                self.gaps_by_category
                    .iter()
                    .map(|(category, count)| format!("{category} {count}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !self.suggested_sources.is_empty() {
            lines.push(format!(
                "- suggested launch_spec additions (suggestion only; this launch's scope and authority are unchanged): {}",
                self.suggested_sources.join(", ")
            ));
        }
        if self.withheld_sources > 0 {
            lines.push(format!(
                "- {} suggestion(s) withheld by the privacy policy for this destination",
                self.withheld_sources
            ));
        }
        lines
    }
}

/// Bounded aggregation over finished children. `None` is an absent declaration,
/// which counts as `unknown` rather than as a sufficient handoff.
pub fn aggregate<'a, I>(entries: I, limits: &HandoffLimits) -> Vec<HandoffMetric>
where
    I: IntoIterator<Item = (&'a str, Option<&'a HandoffReport>)>,
{
    let mut by_key: BTreeMap<String, HandoffMetric> = BTreeMap::new();
    for (key, report) in entries {
        let metric = by_key
            .entry(key.to_string())
            .or_insert_with(|| HandoffMetric {
                key: key.to_string(),
                ..Default::default()
            });
        metric.completions = metric.completions.saturating_add(1);
        let Some(report) =
            report.filter(|report| report.sufficiency != HandoffSufficiency::Unknown)
        else {
            metric.unknown = metric.unknown.saturating_add(1);
            continue;
        };
        metric.reported = metric.reported.saturating_add(1);
        if report.gaps.is_empty() {
            metric.sufficient = metric.sufficient.saturating_add(1);
        } else {
            metric.with_gaps = metric.with_gaps.saturating_add(1);
        }
        for gap in &report.gaps {
            metric.gaps = metric.gaps.saturating_add(1);
            metric.extra_steps = metric
                .extra_steps
                .saturating_add(gap.extra_steps.unwrap_or(0));
            *metric
                .gaps_by_category
                .entry(gap.category.as_str().to_string())
                .or_default() += 1;
            let Some(path) = gap
                .path
                .as_deref()
                .map(str::trim)
                .filter(|path| !path.is_empty())
            else {
                continue;
            };
            if gap.category.mechanically_fixable()
                && metric.suggested_sources.len() < limits.max_entries.max(1)
                && !metric.suggested_sources.iter().any(|known| known == path)
            {
                metric.suggested_sources.push(path.to_string());
            }
        }
    }
    by_key.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(body: &str) -> String {
        format!("Status: DONE\n\n```{HANDOFF_BLOCK_TAG}\n{body}\n```\n")
    }

    #[test]
    fn child_entries_are_typed_and_stored_with_provenance() {
        let report = parse_report(
            &block(
                r#"{"gaps":[{"category":"missing_file","detail":"had to find the retry rule","path":"src/auth/retry.rs","extra_steps":3},{"category":"made_up","detail":"unclassified"}]}"#,
            ),
            "agent-1",
            42,
            &HandoffLimits::default(),
        )
        .expect("declared report");
        assert_eq!(report.sufficiency, HandoffSufficiency::GapsReported);
        assert_eq!(report.source, HANDOFF_SOURCE_CHILD_DECLARATION);
        assert_eq!(report.claimed_by, "agent-1");
        assert_eq!(report.claimed_at_ms, 42);
        assert_eq!(report.gaps[0].category, HandoffGapCategory::MissingFile);
        assert_eq!(report.gaps[0].path.as_deref(), Some("src/auth/retry.rs"));
        assert_eq!(report.gaps[0].extra_steps, Some(3));
        assert_eq!(report.gaps[1].category, HandoffGapCategory::Other);
        assert!(report.notes.is_empty());
        let round_trip: HandoffReport =
            serde_json::from_value(serde_json::to_value(&report).unwrap()).unwrap();
        assert_eq!(round_trip, report);
    }

    #[test]
    fn an_absent_or_empty_declaration_is_unknown_or_sufficient_and_never_blocks() {
        assert!(parse_report(
            "Status: DONE\nNo handoff block at all.",
            "agent-1",
            0,
            &HandoffLimits::default()
        )
        .is_none());
        let empty = parse_report(
            &block(r#"{"gaps":[]}"#),
            "agent-1",
            0,
            &HandoffLimits::default(),
        )
        .unwrap();
        assert_eq!(empty.sufficiency, HandoffSufficiency::Sufficient);
        let malformed =
            parse_report(&block("not json"), "agent-1", 0, &HandoffLimits::default()).unwrap();
        assert_eq!(malformed.sufficiency, HandoffSufficiency::Unknown);
        assert!(malformed.notes[0].contains("stays unknown"));
        let legacy: HandoffReport = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy.sufficiency, HandoffSufficiency::Unknown);
        assert!(legacy.gaps.is_empty());
    }

    #[test]
    fn oversized_and_over_count_declarations_are_truncated_with_an_explicit_marker() {
        let gaps = (0..5)
            .map(|index| {
                format!(
                    r#"{{"category":"missing_file","detail":"{}","path":"{}"}}"#,
                    "d".repeat((index + 1) * 4),
                    "p".repeat(20)
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let report = parse_report(
            &block(&format!(r#"{{"gaps":[{gaps}]}}"#)),
            "agent-1",
            0,
            &HandoffLimits {
                max_entries: 2,
                max_chars: 5,
            },
        )
        .unwrap();
        assert_eq!(report.gaps.len(), 2);
        assert_eq!(report.gaps[0].detail, "dddd");
        assert!(report.gaps[1].detail.contains("[truncated, 8 chars]"));
        assert!(report.gaps[1]
            .path
            .as_deref()
            .unwrap()
            .contains("[truncated, 20 chars]"));
        assert!(report
            .notes
            .iter()
            .any(|note| note.contains("kept 2 of 5 declared entries")));
        assert!(report
            .notes
            .iter()
            .any(|note| note.contains("were truncated")));
        let mut twice = block(r#"{"gaps":[]}"#);
        twice.push_str(&block(r#"{"gaps":[]}"#));
        let both = parse_report(&twice, "agent-1", 0, &HandoffLimits::default()).unwrap();
        assert!(both.notes[0].contains("only the first was read"));
    }

    #[test]
    fn aggregation_produces_a_per_key_metric_across_completions() {
        let reported = parse_report(
            &block(
                r#"{"gaps":[{"category":"missing_file","detail":"a","path":"src/a.rs","extra_steps":2},{"category":"ambiguous_requirement","detail":"b"}]}"#,
            ),
            "agent-1",
            0,
            &HandoffLimits::default(),
        )
        .unwrap();
        let repeated = parse_report(
            &block(r#"{"gaps":[{"category":"missing_file","detail":"a","path":"src/a.rs"}]}"#),
            "agent-2",
            0,
            &HandoffLimits::default(),
        )
        .unwrap();
        let sufficient = parse_report(
            &block(r#"{"gaps":[]}"#),
            "agent-3",
            0,
            &HandoffLimits::default(),
        )
        .unwrap();
        let metrics = aggregate(
            [
                ("profile:reviewer", Some(&reported)),
                ("profile:reviewer", Some(&repeated)),
                ("profile:reviewer", Some(&sufficient)),
                ("profile:reviewer", None),
                ("config:subagent", Some(&reported)),
            ],
            &HandoffLimits::default(),
        );
        assert_eq!(metrics.len(), 2);
        let reviewer = &metrics[1];
        assert_eq!(reviewer.key, "profile:reviewer");
        assert_eq!(reviewer.completions, 4);
        assert_eq!(reviewer.reported, 3);
        assert_eq!(reviewer.unknown, 1);
        assert_eq!(reviewer.sufficient, 1);
        assert_eq!(reviewer.with_gaps, 2);
        assert_eq!(reviewer.gaps, 3);
        assert_eq!(reviewer.extra_steps, 2);
        assert_eq!(reviewer.gaps_by_category["missing_file"], 2);
        assert_eq!(reviewer.gaps_by_category["ambiguous_requirement"], 1);
        assert_eq!(reviewer.suggested_sources, vec!["src/a.rs"]);
        assert_eq!(metrics[0].key, "config:subagent");
        assert_eq!(metrics[0].completions, 1);
    }

    #[test]
    fn a_sufficiency_entry_cannot_widen_scope_or_authority() {
        for widening in [
            r#"{"gaps":[],"tools":["shell"]}"#,
            r#"{"gaps":[],"target_files":["src/a.rs"]}"#,
            r#"{"gaps":[{"category":"missing_file","detail":"a","grant_tools":["shell"]}]}"#,
            r#"{"gaps":[{"category":"missing_file","detail":"a","max_steps":1000}]}"#,
        ] {
            let report =
                parse_report(&block(widening), "agent-1", 0, &HandoffLimits::default()).unwrap();
            assert_eq!(
                report.sufficiency,
                HandoffSufficiency::Unknown,
                "{widening}"
            );
            assert!(report.gaps.is_empty(), "{widening}");
            assert!(report.notes[0].contains("stays unknown"), "{widening}");
        }
        let metric = aggregate(
            [(
                "profile:reviewer",
                Some(
                    &parse_report(
                        &block(
                            r#"{"gaps":[{"category":"missing_file","detail":"a","path":"src/a.rs"}]}"#,
                        ),
                        "agent-1",
                        0,
                        &HandoffLimits::default(),
                    )
                    .unwrap(),
                ),
            )],
            &HandoffLimits::default(),
        );
        assert_eq!(metric[0].suggested_sources, vec!["src/a.rs"]);
        assert!(metric[0].summary_lines().iter().any(|line| line
            .contains("suggestion only; this launch's scope and authority are unchanged")));
    }
}
