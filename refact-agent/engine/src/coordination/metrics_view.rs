use refact_privacy::{FileRecord, PrivacyAuditError, PrivacyAudited};
use serde::Serialize;

use crate::app_state::AppState;

use super::error::WorkError;
use super::metrics::{self, Explanation, MetricsSnapshot, Sample, SeriesKey};
use super::paging::ViewBinding;
use super::scope::WorkCaller;

#[derive(Debug, Clone, Default)]
pub struct MetricsRequest {
    pub rows: Option<usize>,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MetricSeries {
    pub name: String,
    pub labels: Vec<MetricLabel>,
    pub sample: Sample,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MetricLabel {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MetricEntry {
    Series { series: MetricSeries },
    Explanation { explanation: Explanation },
}

#[derive(Debug, Clone, Serialize)]
pub struct MetricsPage {
    pub snapshot_id: String,
    pub seq: u64,
    pub project: String,
    pub entries: Vec<MetricEntry>,
    pub total: usize,
    pub series_total: usize,
    pub explanations_total: usize,
    pub returned: usize,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub generated_at_ms: i64,
}

struct MetricsAudit;

impl PrivacyAudited for MetricsAudit {
    fn privacy_records(&self) -> Result<Vec<(usize, FileRecord)>, PrivacyAuditError> {
        Ok(Vec::new())
    }
}

fn redacted_series(key: &SeriesKey, sample: Sample, max_chars: usize) -> MetricSeries {
    MetricSeries {
        name: key.name.to_string(),
        labels: key
            .labels
            .iter()
            .map(|(name, value)| MetricLabel {
                name: (*name).to_string(),
                value: metrics::redact(value, max_chars),
            })
            .collect(),
        sample,
    }
}

fn redacted_explanation(explanation: &Explanation, max_chars: usize) -> Explanation {
    Explanation {
        decision: explanation.decision,
        outcome: metrics::redact(&explanation.outcome, max_chars),
        rule: metrics::redact(&explanation.rule, max_chars),
        inputs: explanation
            .inputs
            .iter()
            .map(|(name, value)| {
                (
                    metrics::redact(name, max_chars),
                    metrics::redact(value, max_chars),
                )
            })
            .collect(),
        seq: explanation.seq,
        at_ms: explanation.at_ms,
    }
}

pub fn entries(snapshot: &MetricsSnapshot) -> Vec<MetricEntry> {
    let max_chars = metrics::caps().max_label_chars;
    let mut entries = snapshot
        .keys
        .iter()
        .filter_map(|key| {
            snapshot
                .series
                .get(&MetricsSnapshot::render(key))
                .copied()
                .map(|sample| MetricEntry::Series {
                    series: redacted_series(key, sample, max_chars),
                })
        })
        .collect::<Vec<_>>();
    entries.extend(snapshot.explanations.iter().rev().map(|explanation| {
        MetricEntry::Explanation {
            explanation: redacted_explanation(explanation, max_chars),
        }
    }));
    entries
}

fn fingerprint(entries: &[MetricEntry]) -> Result<String, WorkError> {
    serde_json::to_string(entries)
        .map(|value| crate::file_activity::digest_of(&value))
        .map_err(|_| WorkError::unavailable())
}

pub fn page(
    app: &AppState,
    caller: &WorkCaller,
    request: MetricsRequest,
    snapshot: MetricsSnapshot,
) -> Result<MetricsPage, WorkError> {
    let settings = crate::file_activity::settings();
    let disclosure = match &caller.model {
        Some(model) => {
            let before = super::privacy_guard::policy_key(&app.gcx, model)?;
            crate::privacy::destinations::clear_for_model(&app.gcx, MetricsAudit, model)
                .map_err(|_| WorkError::policy_unavailable())?;
            if super::privacy_guard::policy_key(&app.gcx, model)? != before {
                return Err(WorkError::policy_unavailable());
            }
            before
        }
        None => "operator".to_string(),
    };
    let series_total = snapshot.keys.len();
    let explanations_total = snapshot.explanations.len();
    let entries = entries(&snapshot);
    let window = app.work_views.page(
        ViewBinding {
            project: super::roots::project_identity(&caller.source_root),
            principal: caller.principal.clone(),
            disclosure,
            filter: "metrics".to_string(),
        },
        fingerprint(&entries)?,
        entries.len(),
        request.rows.unwrap_or(settings.overview_default_rows),
        request.cursor.as_deref(),
        &settings,
        super::service::now_ms(),
    )?;
    let page = entries[window.offset..window.end].to_vec();
    Ok(MetricsPage {
        snapshot_id: window.snapshot_id,
        seq: window.seq,
        project: super::roots::project_identity(&caller.source_root),
        total: entries.len(),
        series_total,
        explanations_total,
        returned: page.len(),
        entries: page,
        truncated: window.next_cursor.is_some(),
        next_cursor: window.next_cursor,
        generated_at_ms: refact_work_core::now_ms(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordination::metrics::{Decision, MetricsRecorder, EVIDENCE_VERDICTS};

    fn snapshot() -> MetricsSnapshot {
        let recorder = MetricsRecorder::new();
        for verdict in ["failed", "blocked", "unavailable", "not_run"] {
            recorder.count(EVIDENCE_VERDICTS, vec![("verdict", verdict.to_string())]);
        }
        recorder.explain(Explanation::new(
            Decision::Denial,
            "denied",
            "policy.no_secret",
            vec![(
                "reason".to_string(),
                "sk-live-ZZQ2ab3Ccd4Eef5Ggh6Iij7Kkl8M".to_string(),
            )],
        ));
        recorder.snapshot()
    }

    #[test]
    fn read_entries_preserve_distinct_verdict_series_and_redact_again() {
        let entries = entries(&snapshot());
        let verdicts = entries
            .iter()
            .filter_map(|entry| match entry {
                MetricEntry::Series { series } if series.name == EVIDENCE_VERDICTS => series
                    .labels
                    .iter()
                    .find(|label| label.name == "verdict")
                    .map(|label| label.value.as_str()),
                _ => None,
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            verdicts,
            ["blocked", "failed", "not_run", "unavailable"].into()
        );
        let MetricEntry::Explanation { explanation } = entries.last().unwrap() else {
            panic!("the explanation must be readable")
        };
        assert_eq!(explanation.decision, Decision::Denial);
        assert_eq!(explanation.rule, "policy.no_secret");
        assert_eq!(explanation.outcome, "denied");
        assert_eq!(explanation.inputs[0].0, "reason");
        assert!(explanation.inputs[0].1.starts_with("redacted:"));
    }
}
