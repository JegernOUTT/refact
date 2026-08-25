use super::*;
use crate::text_safety::{sanitize_tool_inline, sanitize_tool_text};

const MAX_FINDING_ROWS: usize = 6;

#[derive(Debug, Clone)]
pub struct CodeIntelToolCell {
    card: ToolCard,
    report: CodeIntelReport,
    selected: bool,
}

#[derive(Debug, Clone)]
struct CodeIntelReport {
    summary: String,
    data: Value,
}

#[derive(Debug, Clone)]
struct FindingRow {
    title: String,
    detail: String,
    severity: Option<Severity>,
    paths: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

impl CodeIntelToolCell {
    pub fn new(card: &ToolCard, selected: bool) -> Option<Self> {
        let report = CodeIntelReport::parse(&card.name, &card.result)?;
        Some(Self {
            card: card.clone(),
            report,
            selected,
        })
    }

    fn headline(&self) -> &str {
        &self.report.summary
    }
}

impl CodeIntelReport {
    fn parse(tool_name: &str, result: &str) -> Option<Self> {
        let data = serde_json::from_str::<Value>(result).ok()?;
        let object = data.as_object()?;
        if object.get("tool").and_then(Value::as_str) != Some(tool_name) {
            return None;
        }
        let summary = sanitize_tool_inline(object.get("summary").and_then(Value::as_str)?);
        has_structured_data(tool_name, object).then(|| Self { summary, data })
    }

    fn readiness_chip(&self) -> Option<String> {
        let state = self.data.get("index_state")?.as_object()?;
        let ready = state.get("cross_file_ready")?.as_bool()?;
        if ready {
            Some("[index ready]".to_string())
        } else {
            let queued = state
                .get("queued")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            Some(format!("[index building · {queued} queued]"))
        }
    }

    fn warning(&self) -> Option<String> {
        self.data
            .get("warning")
            .and_then(Value::as_str)
            .map(sanitize_tool_text)
    }

    fn markdown(&self) -> Option<&str> {
        (self.data.get("tool").and_then(Value::as_str) == Some("code_map"))
            .then(|| self.data.get("markdown").and_then(Value::as_str))
            .flatten()
    }

    fn metric_cards(&self) -> Vec<String> {
        let object = Some(&self.data);
        match self.data.get("tool").and_then(Value::as_str) {
            Some("codegraph_overview") => {
                let counts = object.and_then(|value| value.get("counts"));
                metric_values(
                    counts,
                    &[("nodes", "nodes"), ("edges", "edges"), ("files", "files")],
                )
                .into_iter()
                .chain(metric_values(object, &[("component_count", "components")]))
                .collect()
            }
            Some("code_health") => metric_values(
                object.and_then(|value| value.get("aggregate")),
                &[
                    ("grade", "grade"),
                    ("function_count", "functions"),
                    ("max_complexity", "max complexity"),
                    ("avg_maintainability_index", "MI"),
                ],
            ),
            Some("git_risk") => metric_values(
                object,
                &[
                    ("commits_analyzed", "commits"),
                    ("agent_authored_pct", "agent-authored"),
                ],
            )
            .into_iter()
            .chain(array_metric(object, "hotspots", "hotspots"))
            .collect(),
            Some("code_why") => metric_values(
                object,
                &[("source_count", "sources"), ("commits_analyzed", "commits")],
            )
            .into_iter()
            .chain(array_metric(object, "decisions", "decisions"))
            .collect(),
            Some("code_duplication") => metric_values(
                object.and_then(|value| value.get("aggregate")),
                &[
                    ("file_count", "files"),
                    ("clone_pair_count", "clone pairs"),
                    ("duplication_percent", "duplication %"),
                ],
            ),
            Some("code_map") => metric_values(
                object,
                &[
                    ("files_count", "files"),
                    ("page_count", "pages"),
                    ("link_count", "links"),
                ],
            ),
            Some("security_scan") => metric_values(
                object,
                &[("finding_count", "findings"), ("omitted", "omitted")],
            ),
            Some("pr_blast") => metric_values(
                object,
                &[
                    ("impacted_file_count", "impacted files"),
                    ("risk_score", "risk"),
                    ("max_depth", "depth"),
                ],
            )
            .into_iter()
            .chain(array_metric(object, "changed_files", "changed files"))
            .collect(),
            Some("dead_code") => metric_values(
                object,
                &[("shown", "shown"), ("total_candidates", "candidates")],
            ),
            _ => Vec::new(),
        }
    }

    fn finding_rows(&self) -> Vec<FindingRow> {
        let keys = match self.data.get("tool").and_then(Value::as_str) {
            Some("codegraph_overview") => &["top_pagerank", "dead_code", "execution_flows"][..],
            Some("code_health") => &["findings", "health_impact"][..],
            Some("git_risk") => &["hotspots", "findings", "recent_commit_risks"][..],
            Some("code_why") => &["decisions", "related"][..],
            Some("code_duplication") => &["clones", "dry_violations", "test_smells"][..],
            Some("code_map") => &["pages", "top_files", "backlink_hubs"][..],
            Some("security_scan") => &["findings"][..],
            Some("pr_blast") => &["directly_impacted", "transitively_impacted"][..],
            Some("dead_code") => &["entries"][..],
            _ => &[],
        };
        let mut rows = Vec::new();
        for key in keys {
            collect_rows_for_key(&self.data, key, &[], &mut rows);
            if rows.len() >= MAX_FINDING_ROWS {
                break;
            }
        }
        rows.truncate(MAX_FINDING_ROWS);
        rows
    }
}

impl HistoryCell for CodeIntelToolCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Search
    }

    fn render_raw(&self, width: usize) -> Vec<Line<'static>> {
        let mut meta = self.report.readiness_chip().into_iter().collect::<Vec<_>>();
        let duration = rendered_duration(&self.card);
        if !duration.is_empty() {
            meta.push(duration);
        }
        let mut lines = vec![role_line(
            if self.selected {
                "code intel selected"
            } else {
                "code intel"
            },
            default_theme_style(ThemeRole::Highlight),
        )];
        lines.push(tool_summary_line(
            &self.card,
            self.headline().to_string(),
            meta.join(" · "),
        ));
        lines.extend(subchat_lines(&self.card, width));

        if self.card.expanded {
            let metrics = self.report.metric_cards();
            if !metrics.is_empty() {
                lines.extend(wrap_with_prefix(
                    &metrics.join(" "),
                    width,
                    dim_span("  "),
                    dim_span("  "),
                    default_theme_style(ThemeRole::Muted),
                ));
            }
            if let Some(warning) = self.report.warning() {
                lines.extend(wrap_with_prefix(
                    &format!("warning: {warning}"),
                    width,
                    dim_span("  "),
                    dim_span("  "),
                    default_theme_style(ThemeRole::Warning),
                ));
            }
            let rows = self.report.finding_rows();
            let prefix = common_path_prefix(
                &rows
                    .iter()
                    .flat_map(|row| row.paths.iter())
                    .cloned()
                    .collect::<Vec<_>>(),
            );
            if let Some(prefix) = &prefix {
                lines.extend(wrap_with_prefix(
                    &format!("[paths: {prefix}]"),
                    width,
                    dim_span("  "),
                    dim_span("  "),
                    default_theme_style(ThemeRole::Muted),
                ));
            }
            for row in rows {
                lines.extend(finding_lines(row, prefix.as_deref(), width));
            }
            if let Some(markdown) = self.report.markdown() {
                let renderer = MarkdownRenderer::new(Some(width.saturating_sub(4).max(1)));
                lines.extend(prefix_lines(
                    renderer.render(&sanitize_tool_text(markdown)),
                    dim_span("  └ "),
                    dim_span("    "),
                ));
            }
        }
        finish(lines)
    }

    fn is_final(&self) -> bool {
        self.card.status.is_final()
    }

    fn revision(&self) -> u64 {
        revision(&(self.kind(), &self.card, &self.report.summary, self.selected))
    }
}

fn has_structured_data(tool_name: &str, object: &serde_json::Map<String, Value>) -> bool {
    match tool_name {
        "codegraph_overview" => object.get("counts").is_some_and(Value::is_object),
        "code_health" => object.get("aggregate").is_some_and(Value::is_object),
        "git_risk" => object.get("hotspots").is_some_and(Value::is_array),
        "code_why" => object.get("decisions").is_some_and(Value::is_array),
        "code_duplication" => object.get("clones").is_some_and(Value::is_array),
        "code_map" => object.get("pages").is_some_and(Value::is_array),
        "security_scan" => object.get("findings").is_some_and(Value::is_array),
        "pr_blast" => object.get("directly_impacted").is_some_and(Value::is_array),
        "dead_code" => object.get("entries").is_some_and(Value::is_array),
        _ => false,
    }
}

fn metric_values(value: Option<&Value>, fields: &[(&str, &str)]) -> Vec<String> {
    let Some(object) = value.and_then(Value::as_object) else {
        return Vec::new();
    };
    fields
        .iter()
        .filter_map(|(key, label)| {
            object
                .get(*key)
                .map(|value| format!("[{label}: {}]", value_text(value)))
        })
        .collect()
}

fn array_metric(value: Option<&Value>, key: &str, label: &str) -> Option<String> {
    value
        .and_then(|value| value.get(key))
        .and_then(Value::as_array)
        .map(|items| format!("[{label}: {}]", items.len()))
}

fn value_text(value: &Value) -> String {
    sanitize_tool_inline(
        value
            .as_str()
            .map(ToString::to_string)
            .unwrap_or_else(|| value.to_string()),
    )
}

fn collect_rows_for_key(
    value: &Value,
    key: &str,
    inherited_paths: &[String],
    rows: &mut Vec<FindingRow>,
) {
    if rows.len() >= MAX_FINDING_ROWS {
        return;
    }
    match value {
        Value::Object(object) => {
            if let Some(items) = object.get(key).and_then(Value::as_array) {
                let mut row_context_paths = inherited_paths.to_vec();
                row_context_paths.extend(row_paths(object));
                row_context_paths.sort();
                row_context_paths.dedup();
                rows.extend(
                    items
                        .iter()
                        .filter_map(|item| finding_row(item, &row_context_paths))
                        .take(MAX_FINDING_ROWS.saturating_sub(rows.len())),
                );
            }
            for (field, nested) in object {
                if field != key {
                    let mut nested_paths = inherited_paths.to_vec();
                    nested_paths.extend(row_paths(object));
                    nested_paths.sort();
                    nested_paths.dedup();
                    collect_rows_for_key(nested, key, &nested_paths, rows);
                    if rows.len() >= MAX_FINDING_ROWS {
                        return;
                    }
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_rows_for_key(item, key, inherited_paths, rows);
                if rows.len() >= MAX_FINDING_ROWS {
                    return;
                }
            }
        }
        _ => {}
    }
}

fn finding_row(value: &Value, inherited_paths: &[String]) -> Option<FindingRow> {
    let object = value.as_object()?;
    let title = [
        "title",
        "name",
        "symbol",
        "rule",
        "biomarker",
        "summary",
        "label",
        "path",
    ]
    .iter()
    .find_map(|key| object.get(*key).and_then(Value::as_str))?
    .to_string();
    let detail = ["detail", "reason", "snippet", "via", "source_kind"]
        .iter()
        .filter_map(|key| object.get(*key).and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join(" · ");
    let detail = if detail.is_empty() {
        ["distance", "risk", "score", "confidence", "count"]
            .iter()
            .filter_map(|key| {
                object
                    .get(*key)
                    .map(|value| format!("{key} {}", value_text(value)))
            })
            .collect::<Vec<_>>()
            .join(" · ")
    } else {
        detail
    };
    let mut paths = inherited_paths.to_vec();
    paths.extend(row_paths(object));
    paths.sort();
    paths.dedup();
    Some(FindingRow {
        title: sanitize_tool_inline(title),
        detail: sanitize_tool_text(detail),
        severity: object.get("severity").and_then(parse_severity),
        paths,
    })
}

fn parse_severity(value: &Value) -> Option<Severity> {
    match value.as_str()? {
        "Low" => Some(Severity::Low),
        "Medium" => Some(Severity::Medium),
        "High" => Some(Severity::High),
        "Critical" => Some(Severity::Critical),
        _ => None,
    }
}

fn row_paths(object: &serde_json::Map<String, Value>) -> Vec<String> {
    let mut paths = ["path", "path_a", "path_b", "target_path", "source_ref"]
        .iter()
        .filter_map(|key| object.get(*key).and_then(Value::as_str))
        .map(sanitize_tool_inline)
        .collect::<Vec<_>>();
    for key in ["paths", "changed_files"] {
        if let Some(items) = object.get(key).and_then(Value::as_array) {
            paths.extend(
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(sanitize_tool_inline),
            );
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

fn finding_lines(row: FindingRow, prefix: Option<&str>, width: usize) -> Vec<Line<'static>> {
    let mut text = String::new();
    if let Some(severity) = row.severity {
        text.push_str(&format!("[{}] ", severity_label(severity)));
    }
    text.push_str(&row.title);
    if !row.detail.is_empty() {
        text.push_str(" — ");
        text.push_str(&row.detail);
    }
    if !row.paths.is_empty() {
        text.push_str(" · ");
        text.push_str(
            &row.paths
                .iter()
                .map(|path| {
                    path.strip_prefix(prefix.unwrap_or_default())
                        .unwrap_or(path)
                })
                .filter(|path| !path.is_empty())
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    wrap_with_prefix(
        &sanitize_tool_text(text),
        width,
        dim_span("  └ "),
        dim_span("    "),
        default_theme_style(ThemeRole::Text),
    )
}

fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Low => "Low",
        Severity::Medium => "Medium",
        Severity::High => "High",
        Severity::Critical => "Critical",
    }
}

fn common_path_prefix(paths: &[String]) -> Option<String> {
    let paths = paths
        .iter()
        .filter(|path| path.starts_with('/'))
        .collect::<Vec<_>>();
    if paths.is_empty() {
        return None;
    }
    let components = paths
        .iter()
        .map(|path| {
            path.split('/')
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut shared = components.first().cloned().unwrap_or_default();
    for path in components.iter().skip(1) {
        let count = shared
            .iter()
            .zip(path)
            .take_while(|(left, right)| left == right)
            .count();
        shared.truncate(count);
    }
    if components.len() == 1 && !shared.is_empty() {
        shared.pop();
    }
    if shared.is_empty() {
        Some("/".to_string())
    } else {
        Some(format!("/{}{}", shared.join("/"), "/"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::cells::test_support::{text, tool_card};
    use serde_json::json;

    fn ready_index_state() -> Value {
        json!({"queued": 0, "cross_file_edges": 42, "cross_file_ready": true})
    }

    fn dead_code_index_state() -> Value {
        json!({
            "queued": 0,
            "dirty_paths": 0,
            "pending_refs": 0,
            "cross_file_edges": 42,
            "cross_file_ready": true
        })
    }

    fn fixture(name: &str) -> Value {
        let summary = format!("{name} summary");
        match name {
            "codegraph_overview" => json!({
                "tool": name,
                "summary": summary,
                "counts": {"nodes": 4, "edges": 5, "files": 2},
                "index_state": ready_index_state(),
                "scc_count": 0,
                "largest_scc": 1,
                "component_count": 2,
                "top_pagerank": [{"symbol": "main", "path": "/workspace/repo/src/main.rs", "score": 0.9}],
                "top_betweenness": [],
                "file_centrality": {"top_pagerank": [], "top_betweenness": []},
                "community_count": 1,
                "dead_code_count": 0,
                "partial": false,
                "warning": null,
                "communities": [],
                "execution_flows": [],
                "dead_code": [],
                "entry_points": [],
                "api_contract_files": []
            }),
            "code_health" => json!({
                "tool": name,
                "summary": summary,
                "index_state": ready_index_state(),
                "aggregate": {
                    "file_count": 1,
                    "function_count": 2,
                    "avg_score": 0.8,
                    "grade": "A",
                    "max_complexity": 4,
                    "avg_maintainability": 0.9,
                    "avg_maintainability_index": 91.0,
                    "avg_maintainability_signal": 0.8,
                    "avg_duplication_pct": 0.0,
                    "biomarker_count": 1,
                    "refactoring_count": 0
                },
                "files": [],
                "file_category": "module",
                "call_graph": [],
                "coverage": null,
                "warm_cache": false
            }),
            "git_risk" => json!({
                "tool": name,
                "summary": summary,
                "commits_analyzed": 12,
                "agent_authored_pct": 0.2,
                "hotspots": [{"path": "/workspace/repo/src/lib.rs", "risk": 0.7}],
                "ownership": [],
                "co_change": [],
                "coupling": [],
                "reviewers": [],
                "findings": [],
                "recent_commit_risks": []
            }),
            "code_why" => json!({
                "tool": name,
                "summary": summary,
                "query": "cache",
                "source_count": 2,
                "commits_analyzed": 12,
                "decisions": [{"kind": "commit", "confidence": 0.8, "corroboration": 1, "source_kind": "git", "source_ref": "/workspace/repo/README.md", "summary": "Keep cache", "provenance_tags": []}],
                "related": []
            }),
            "code_duplication" => json!({
                "tool": name,
                "summary": summary,
                "aggregate": {"file_count": 2, "clone_pair_count": 1, "duplication_pct": 0.1, "duplication_percent": 10.0},
                "clones": [{"path_a": "/workspace/repo/src/a.rs", "path_b": "/workspace/repo/src/b.rs", "lines": 10}],
                "dry_violations": [],
                "test_smells": []
            }),
            "code_map" => json!({
                "tool": name,
                "summary": summary,
                "files_count": 2,
                "page_count": 1,
                "link_count": 3,
                "query": null,
                "index_state": ready_index_state(),
                "partial": false,
                "warning": null,
                "top_files": [],
                "backlink_hubs": [],
                "pages": [],
                "markdown": null
            }),
            "security_scan" => json!({
                "tool": name,
                "summary": summary,
                "path": "/workspace/repo/src/config.rs",
                "lang": "rust",
                "finding_count": 1,
                "counts": {"Critical": 1},
                "findings": [{"rule": "hardcoded_secret", "severity": "Critical", "line": 4, "snippet": "token = value"}],
                "omitted": 0
            }),
            "pr_blast" => json!({
                "tool": name,
                "summary": summary,
                "changed_files": ["/workspace/repo/src/a.rs"],
                "directly_impacted": [{"path": "/workspace/repo/src/b.rs", "symbol": "parse", "distance": 1, "via": "calls", "kind": "behavioral"}],
                "transitively_impacted": [{"path": "/workspace/repo/tests/a.rs", "symbol": "test_parse", "distance": 2, "via": "calls", "kind": "behavioral"}],
                "impacted_file_count": 2,
                "risk_score": 0.71,
                "suggested_reviewers": [],
                "index_state": ready_index_state(),
                "partial": false,
                "max_depth": 3
            }),
            "dead_code" => json!({
                "tool": name,
                "summary": summary,
                "entries": [{"name": "unused", "path": "/workspace/repo/src/lib.rs", "line": 9, "reason": "no callers", "confidence": 0.9, "git_recency": "old", "incoming_edges": 0}],
                "index_state": dead_code_index_state(),
                "partial": false,
                "warning": null,
                "shown": 1,
                "total_candidates": 1
            }),
            _ => panic!("unknown fixture: {name}"),
        }
    }

    fn card(name: &str, result: Value) -> ToolCard {
        let mut card = tool_card(name, json!({}), &result.to_string());
        card.expanded = false;
        card.duration_ms = None;
        card
    }

    const CODE_INTEL_TOOLS: &[&str] = &[
        "codegraph_overview",
        "code_health",
        "git_risk",
        "code_why",
        "code_duplication",
        "code_map",
        "security_scan",
        "pr_blast",
        "dead_code",
    ];

    #[test]
    fn every_code_intel_fixture_uses_summary_as_collapsed_headline() {
        for name in CODE_INTEL_TOOLS {
            let result = fixture(name);
            let expected = result["summary"].as_str().unwrap().to_string();
            let card = card(name, result);
            let cell = CodeIntelToolCell::new(&card, false).unwrap();
            assert_eq!(cell.headline(), expected, "{name}");
            let rendered = text(&cell.render(160));
            assert!(rendered.contains(&expected), "{name}: {rendered}");
            let dispatched = cell_from_tool_card(card, false);
            assert_eq!(dispatched.kind(), HistoryCellKind::Search, "{name}");
            assert!(
                text(&dispatched.render(160)).starts_with("code intel\n"),
                "{name}"
            );
        }
    }

    #[test]
    fn malformed_code_intel_payload_uses_the_generic_tool_card() {
        let mut card = tool_card("code_health", json!({}), "[]");
        card.expanded = false;
        card.set_result("[]");
        assert!(CodeIntelToolCell::new(&card, false).is_none());
        let cell = cell_from_tool_card(card, false);
        assert_eq!(cell.kind(), HistoryCellKind::Tool);
        assert!(text(&cell.render(80)).starts_with("tool\n"));
    }

    #[test]
    fn code_map_markdown_uses_the_markdown_renderer() {
        let mut result = fixture("code_map");
        result["markdown"] = json!("## Architecture\n\n- **entrypoint**");
        let mut card = card("code_map", result);
        card.expanded = true;
        let rendered = text(&CodeIntelToolCell::new(&card, false).unwrap().render(100));
        assert!(rendered.contains("Architecture"));
        assert!(rendered.contains("entrypoint"));
    }

    #[test]
    fn index_state_and_absolute_path_prefix_render_once() {
        let mut card = card("pr_blast", fixture("pr_blast"));
        card.expanded = true;
        let rendered = text(&CodeIntelToolCell::new(&card, false).unwrap().render(120));
        assert!(rendered.contains("[index ready]"));
        assert_eq!(rendered.matches("[paths: /workspace/repo/]").count(), 1);
        assert!(rendered.contains("src/b.rs"));
        assert!(rendered.contains("tests/a.rs"));
    }

    #[test]
    fn code_intel_text_is_sanitized_and_wrapped() {
        let mut result = fixture("security_scan");
        result["summary"] = json!("scan\u{1b}[31m result");
        result["findings"][0]["snippet"] =
            json!("/not/a/path\u{1b}]8;;https://example.com\u{7} text");
        let mut card = card("security_scan", result);
        card.expanded = true;
        let rendered = text(&CodeIntelToolCell::new(&card, false).unwrap().render(30));
        assert!(!rendered.contains('\u{1b}'));
        assert!(rendered.lines().all(|line| line.chars().count() <= 30));
    }

    #[test]
    fn warning_is_rendered_from_its_json_field() {
        let mut result = fixture("codegraph_overview");
        result["warning"] = json!("cross-file analysis is partial");
        let mut card = card("codegraph_overview", result);
        card.expanded = true;
        let rendered = text(&CodeIntelToolCell::new(&card, false).unwrap().render(120));
        assert!(rendered.contains("warning: cross-file analysis is partial"));
        assert!(!rendered.contains('⚠'));
    }

    #[test]
    fn only_source_severity_variants_are_rendered() {
        assert_eq!(parse_severity(&json!("Low")), Some(Severity::Low));
        assert_eq!(parse_severity(&json!("Medium")), Some(Severity::Medium));
        assert_eq!(parse_severity(&json!("High")), Some(Severity::High));
        assert_eq!(parse_severity(&json!("Critical")), Some(Severity::Critical));
        assert_eq!(parse_severity(&json!("Info")), None);
    }
}
