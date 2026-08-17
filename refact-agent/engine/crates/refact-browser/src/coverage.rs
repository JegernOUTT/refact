use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use headless_chrome::Tab;
use headless_chrome::protocol::cdp::types::Event;
use headless_chrome::protocol::cdp::{CSS, DOM, Debugger, Profiler};
use serde::Serialize;

const MAX_SUMMARY_UNUSED_RANGES: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoverageOptions {
    pub js: bool,
    pub css: bool,
    pub reset_on_navigation: bool,
}

impl CoverageOptions {
    pub fn resolve(js: Option<bool>, css: Option<bool>, reset_on_navigation: Option<bool>) -> Self {
        let default_both = js.is_none() && css.is_none();
        Self {
            js: js.unwrap_or(default_both),
            css: css.unwrap_or(default_both),
            reset_on_navigation: reset_on_navigation.unwrap_or(true),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct CoverageRange {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Copy)]
struct CountedCoverageRange {
    start: usize,
    end: usize,
    count: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CoverageSummary {
    pub url: String,
    pub kinds: Vec<String>,
    pub total_bytes: usize,
    pub used_bytes: usize,
    pub used_percentage: f64,
    pub unused_ranges: Vec<CoverageRange>,
    pub unused_ranges_truncated: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CoverageResourceDetail {
    pub url: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub total_bytes: usize,
    pub used_ranges: Vec<CoverageRange>,
    pub unused_ranges: Vec<CoverageRange>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CoverageArtifact {
    pub kind: &'static str,
    pub mime: &'static str,
    pub path: PathBuf,
    pub bytes: usize,
    pub resource_count: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CoverageStopResult {
    pub summaries: Vec<CoverageSummary>,
    pub artifact: CoverageArtifact,
}

#[derive(Default)]
struct CoverageSession {
    options: Option<CoverageOptions>,
    scripts: BTreeMap<String, String>,
    style_sheets: BTreeMap<String, String>,
    listener_installed: bool,
}

#[derive(Default)]
pub struct CoverageManager {
    sessions: Arc<Mutex<HashMap<String, CoverageSession>>>,
}

impl CoverageManager {
    pub fn start(&self, tab: &Tab, options: CoverageOptions) -> Result<(), String> {
        if !options.js && !options.css {
            return Err("Coverage requires js, css, or both".to_string());
        }
        let target_id = tab.get_target_id().to_string();
        {
            let mut sessions = self.sessions.lock().map_err(|error| error.to_string())?;
            let session = sessions.entry(target_id.clone()).or_default();
            if session.options.is_some() {
                return Err("Coverage is already active for this tab".to_string());
            }
            session.options = Some(options);
            session.scripts.clear();
            session.style_sheets.clear();
            if !session.listener_installed {
                let sessions_for_listener = self.sessions.clone();
                let listener_target = target_id.clone();
                tab.add_event_listener(Arc::new(move |event: &Event| {
                    let Ok(mut sessions) = sessions_for_listener.lock() else {
                        return;
                    };
                    let Some(session) = sessions.get_mut(&listener_target) else {
                        return;
                    };
                    let Some(options) = session.options else {
                        return;
                    };
                    match event {
                        Event::DebuggerScriptParsed(event) if options.js => {
                            session
                                .scripts
                                .insert(event.params.script_id.clone(), event.params.url.clone());
                        }
                        Event::CSSStyleSheetAdded(event) if options.css => {
                            if !matches!(
                                event.params.header.origin,
                                CSS::StyleSheetOrigin::Injected
                            ) {
                                session.style_sheets.insert(
                                    event.params.header.style_sheet_id.clone(),
                                    event.params.header.source_url.clone(),
                                );
                            }
                        }
                        Event::RuntimeExecutionContextsCleared(_)
                            if options.reset_on_navigation =>
                        {
                            session.scripts.clear();
                            session.style_sheets.clear();
                        }
                        _ => {}
                    }
                }))
                .map_err(|error| format!("Failed to install coverage listener: {error}"))?;
                session.listener_installed = true;
            }
        }

        let start_result = (|| {
            if options.js {
                tab.call_method(Profiler::Enable(None))
                    .map_err(|error| format!("Failed to enable JavaScript profiler: {error}"))?;
                tab.call_method(Profiler::StartPreciseCoverage {
                    call_count: Some(false),
                    detailed: Some(true),
                    allow_triggered_updates: Some(false),
                })
                .map_err(|error| format!("Failed to start JavaScript coverage: {error}"))?;
                tab.call_method(Debugger::Enable {
                    max_scripts_cache_size: None,
                })
                .map_err(|error| format!("Failed to enable JavaScript debugger: {error}"))?;
            }
            if options.css {
                tab.call_method(DOM::Enable {
                    include_whitespace: None,
                })
                .map_err(|error| format!("Failed to enable DOM for CSS coverage: {error}"))?;
                tab.call_method(CSS::Enable(None))
                    .map_err(|error| format!("Failed to enable CSS coverage: {error}"))?;
                tab.call_method(CSS::StartRuleUsageTracking(None))
                    .map_err(|error| format!("Failed to start CSS coverage: {error}"))?;
            }
            Ok(())
        })();
        if let Err(error) = start_result {
            self.stop_domains(tab, options);
            if let Ok(mut sessions) = self.sessions.lock() {
                if let Some(session) = sessions.get_mut(&target_id) {
                    session.options = None;
                }
            }
            return Err(error);
        }
        Ok(())
    }

    pub fn stop(&self, tab: &Tab, artifacts_dir: &Path) -> Result<CoverageStopResult, String> {
        let target_id = tab.get_target_id().to_string();
        let (options, scripts, style_sheets) = {
            let sessions = self.sessions.lock().map_err(|error| error.to_string())?;
            let session = sessions
                .get(&target_id)
                .ok_or_else(|| "Coverage was not started for this tab".to_string())?;
            let options = session
                .options
                .ok_or_else(|| "Coverage was not started for this tab".to_string())?;
            (
                options,
                session.scripts.clone(),
                session.style_sheets.clone(),
            )
        };

        let collection: Result<Vec<CoverageResourceDetail>, String> = (|| {
            let mut resources = Vec::new();
            if options.js {
                let coverage = tab
                    .call_method(Profiler::TakePreciseCoverage(None))
                    .map_err(|error| format!("Failed to take JavaScript coverage: {error}"))?;
                for script in coverage.result {
                    let source = tab
                        .call_method(Debugger::GetScriptSource {
                            script_id: script.script_id.clone(),
                        })
                        .ok()
                        .map(|result| result.script_source);
                    let total = source
                        .as_ref()
                        .map(String::len)
                        .unwrap_or_else(|| javascript_total_bytes(&script));
                    let counted = script
                        .functions
                        .into_iter()
                        .flat_map(|function| function.ranges)
                        .map(|range| CountedCoverageRange {
                            start: range.start_offset as usize,
                            end: range.end_offset as usize,
                            count: range.count as usize,
                        })
                        .collect::<Vec<_>>();
                    let used = disjoint_used_ranges(&counted, total);
                    resources.push(resource_detail(
                        non_empty_url(
                            &script.url,
                            scripts.get(&script.script_id).map(String::as_str),
                            "anonymous-script",
                            &script.script_id,
                        ),
                        "js",
                        source,
                        total,
                        used,
                    ));
                }
            }
            if options.css {
                let coverage = tab
                    .call_method(CSS::StopRuleUsageTracking(None))
                    .map_err(|error| format!("Failed to stop CSS coverage: {error}"))?;
                let mut ranges_by_sheet: BTreeMap<String, Vec<CoverageRange>> = BTreeMap::new();
                for usage in coverage.rule_usage {
                    if usage.used {
                        ranges_by_sheet
                            .entry(usage.style_sheet_id)
                            .or_default()
                            .push(CoverageRange {
                                start: usage.start_offset.max(0.0) as usize,
                                end: usage.end_offset.max(0.0) as usize,
                            });
                    }
                }
                for (style_sheet_id, url) in style_sheets {
                    let source = tab
                        .call_method(CSS::GetStyleSheetText {
                            style_sheet_id: style_sheet_id.clone(),
                        })
                        .ok()
                        .map(|result| result.text);
                    let total = source.as_ref().map(String::len).unwrap_or_else(|| {
                        ranges_by_sheet
                            .get(&style_sheet_id)
                            .and_then(|ranges| ranges.iter().map(|range| range.end).max())
                            .unwrap_or_default()
                    });
                    resources.push(resource_detail(
                        non_empty_url(&url, None, "inline-style", &style_sheet_id),
                        "css",
                        source,
                        total,
                        ranges_by_sheet.remove(&style_sheet_id).unwrap_or_default(),
                    ));
                }
            }
            Ok(resources)
        })();

        self.stop_domains(tab, options);
        if let Ok(mut sessions) = self.sessions.lock() {
            if let Some(session) = sessions.get_mut(&target_id) {
                session.options = None;
                session.scripts.clear();
                session.style_sheets.clear();
            }
        }
        let resources = collection?;
        let summaries = summarize_by_url(&resources);
        std::fs::create_dir_all(artifacts_dir).map_err(|error| {
            format!(
                "Failed to create browser artifacts directory {}: {error}",
                artifacts_dir.display()
            )
        })?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let path = artifacts_dir.join(format!("coverage-{nonce}.json"));
        let body = serde_json::to_vec_pretty(&serde_json::json!({
            "summaries": summaries,
            "resources": resources,
        }))
        .map_err(|error| format!("Failed to serialize coverage artifact: {error}"))?;
        std::fs::write(&path, &body).map_err(|error| {
            format!(
                "Failed to save coverage artifact {}: {error}",
                path.display()
            )
        })?;
        Ok(CoverageStopResult {
            summaries,
            artifact: CoverageArtifact {
                kind: "coverage",
                mime: "application/json",
                path,
                bytes: body.len(),
                resource_count: resources.len(),
            },
        })
    }

    pub fn cleanup(&self, tabs: &[Arc<Tab>]) {
        let active = self
            .sessions
            .lock()
            .map(|sessions| {
                sessions
                    .iter()
                    .filter_map(|(target, session)| {
                        session.options.map(|options| (target.clone(), options))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for (target, options) in active {
            if let Some(tab) = tabs
                .iter()
                .find(|tab| tab.get_target_id().as_str() == target.as_str())
            {
                if options.css {
                    let _ = tab.call_method(CSS::StopRuleUsageTracking(None));
                }
                self.stop_domains(tab, options);
            }
        }
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.clear();
        }
    }

    fn stop_domains(&self, tab: &Tab, options: CoverageOptions) {
        if options.js {
            let _ = tab.call_method(Profiler::StopPreciseCoverage(None));
            let _ = tab.call_method(Profiler::Disable(None));
            let _ = tab.call_method(Debugger::Disable(None));
        }
        if options.css {
            let _ = tab.call_method(CSS::Disable(None));
            let _ = tab.call_method(DOM::Disable(None));
        }
    }
}

fn non_empty_url(primary: &str, fallback: Option<&str>, prefix: &str, id: &str) -> String {
    if !primary.is_empty() {
        primary.to_string()
    } else if let Some(fallback) = fallback.filter(|value| !value.is_empty()) {
        fallback.to_string()
    } else {
        format!("{prefix}:{id}")
    }
}

fn javascript_total_bytes(script: &Profiler::ScriptCoverage) -> usize {
    script
        .functions
        .iter()
        .flat_map(|function| function.ranges.iter())
        .map(|range| range.end_offset as usize)
        .max()
        .unwrap_or_default()
}

fn resource_detail(
    url: String,
    kind: &str,
    source: Option<String>,
    total: usize,
    used: Vec<CoverageRange>,
) -> CoverageResourceDetail {
    let used_ranges = merge_ranges(&used, total);
    CoverageResourceDetail {
        url,
        kind: kind.to_string(),
        source,
        total_bytes: total,
        unused_ranges: invert_ranges(&used_ranges, total),
        used_ranges,
    }
}

pub fn merge_ranges(ranges: &[CoverageRange], total: usize) -> Vec<CoverageRange> {
    let mut ranges = ranges
        .iter()
        .filter_map(|range| {
            let start = range.start.min(total);
            let end = range.end.min(total);
            (start < end).then_some(CoverageRange { start, end })
        })
        .collect::<Vec<_>>();
    ranges.sort_by_key(|range| (range.start, range.end));
    let mut merged: Vec<CoverageRange> = Vec::new();
    for range in ranges {
        if let Some(last) = merged.last_mut().filter(|last| range.start <= last.end) {
            last.end = last.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    merged
}

fn disjoint_used_ranges(ranges: &[CountedCoverageRange], total: usize) -> Vec<CoverageRange> {
    #[derive(Clone, Copy)]
    struct Point {
        offset: usize,
        start: bool,
        length: usize,
        count: usize,
    }
    let mut points = ranges
        .iter()
        .filter_map(|range| {
            let start = range.start.min(total);
            let end = range.end.min(total);
            (start < end).then_some([
                Point {
                    offset: start,
                    start: true,
                    length: end - start,
                    count: range.count,
                },
                Point {
                    offset: end,
                    start: false,
                    length: end - start,
                    count: range.count,
                },
            ])
        })
        .flatten()
        .collect::<Vec<_>>();
    points.sort_by(|left, right| {
        left.offset
            .cmp(&right.offset)
            .then_with(|| left.start.cmp(&right.start))
            .then_with(|| {
                if left.start {
                    right.length.cmp(&left.length)
                } else {
                    left.length.cmp(&right.length)
                }
            })
    });
    let mut counts = Vec::new();
    let mut results: Vec<CoverageRange> = Vec::new();
    let mut last_offset = 0;
    for point in points {
        if counts.last().copied().unwrap_or_default() > 0 && last_offset < point.offset {
            if let Some(last) = results.last_mut().filter(|last| last.end == last_offset) {
                last.end = point.offset;
            } else {
                results.push(CoverageRange {
                    start: last_offset,
                    end: point.offset,
                });
            }
        }
        last_offset = point.offset;
        if point.start {
            counts.push(point.count);
        } else {
            counts.pop();
        }
    }
    results
}

pub fn invert_ranges(ranges: &[CoverageRange], total: usize) -> Vec<CoverageRange> {
    let mut cursor = 0;
    let mut unused = Vec::new();
    for range in merge_ranges(ranges, total) {
        if cursor < range.start {
            unused.push(CoverageRange {
                start: cursor,
                end: range.start,
            });
        }
        cursor = cursor.max(range.end);
    }
    if cursor < total {
        unused.push(CoverageRange {
            start: cursor,
            end: total,
        });
    }
    unused
}

pub fn summarize_by_url(resources: &[CoverageResourceDetail]) -> Vec<CoverageSummary> {
    #[derive(Default)]
    struct Aggregate {
        total: usize,
        used: Vec<CoverageRange>,
        kinds: Vec<String>,
    }
    let mut aggregates: BTreeMap<String, Aggregate> = BTreeMap::new();
    for resource in resources {
        let aggregate = aggregates.entry(resource.url.clone()).or_default();
        let offset = aggregate.total;
        aggregate
            .used
            .extend(resource.used_ranges.iter().map(|range| CoverageRange {
                start: offset + range.start,
                end: offset + range.end,
            }));
        aggregate.total += resource.total_bytes;
        if !aggregate.kinds.contains(&resource.kind) {
            aggregate.kinds.push(resource.kind.clone());
        }
    }
    aggregates
        .into_iter()
        .map(|(url, aggregate)| {
            let used = merge_ranges(&aggregate.used, aggregate.total);
            let used_bytes = used.iter().map(|range| range.end - range.start).sum();
            let mut unused_ranges = invert_ranges(&used, aggregate.total);
            let unused_ranges_truncated = unused_ranges.len() > MAX_SUMMARY_UNUSED_RANGES;
            unused_ranges.truncate(MAX_SUMMARY_UNUSED_RANGES);
            CoverageSummary {
                url,
                kinds: aggregate.kinds,
                total_bytes: aggregate.total,
                used_bytes,
                used_percentage: if aggregate.total == 0 {
                    0.0
                } else {
                    used_bytes as f64 * 100.0 / aggregate.total as f64
                },
                unused_ranges,
                unused_ranges_truncated,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_math_merges_overlap_and_builds_unused_ranges() {
        let used = merge_ranges(
            &[
                CoverageRange { start: 4, end: 10 },
                CoverageRange { start: 0, end: 5 },
                CoverageRange { start: 15, end: 30 },
            ],
            20,
        );
        assert_eq!(
            used,
            vec![
                CoverageRange { start: 0, end: 10 },
                CoverageRange { start: 15, end: 20 }
            ]
        );
        assert_eq!(
            invert_ranges(&used, 20),
            vec![CoverageRange { start: 10, end: 15 }]
        );
    }

    #[test]
    fn summaries_aggregate_resources_with_the_same_url() {
        let resources = vec![
            resource_detail(
                "https://example.com/app".to_string(),
                "js",
                Some("0123456789".to_string()),
                10,
                vec![CoverageRange { start: 0, end: 5 }],
            ),
            resource_detail(
                "https://example.com/app".to_string(),
                "css",
                Some("abcdefghij".to_string()),
                10,
                vec![CoverageRange { start: 5, end: 10 }],
            ),
        ];
        let summary = summarize_by_url(&resources).pop().unwrap();
        assert_eq!(summary.total_bytes, 20);
        assert_eq!(summary.used_bytes, 10);
        assert_eq!(summary.used_percentage, 50.0);
        assert_eq!(summary.kinds, vec!["js", "css"]);
        assert_eq!(
            summary.unused_ranges,
            vec![CoverageRange { start: 5, end: 15 }]
        );
    }

    #[test]
    fn omitted_coverage_flags_enable_both_domains() {
        assert_eq!(
            CoverageOptions::resolve(None, None, None),
            CoverageOptions {
                js: true,
                css: true,
                reset_on_navigation: true,
            }
        );
        assert_eq!(
            CoverageOptions::resolve(Some(true), None, Some(false)),
            CoverageOptions {
                js: true,
                css: false,
                reset_on_navigation: false,
            }
        );
    }

    #[test]
    fn nested_zero_count_range_overrides_used_parent_range() {
        let used = disjoint_used_ranges(
            &[
                CountedCoverageRange {
                    start: 0,
                    end: 20,
                    count: 1,
                },
                CountedCoverageRange {
                    start: 5,
                    end: 15,
                    count: 0,
                },
            ],
            20,
        );
        assert_eq!(
            used,
            vec![
                CoverageRange { start: 0, end: 5 },
                CoverageRange { start: 15, end: 20 },
            ]
        );
    }
}
