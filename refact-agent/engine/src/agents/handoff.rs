use std::sync::Arc;

use refact_chat_api::handoff::{HandoffFinding, HandoffLimits, HandoffMetric, HandoffReport};

use crate::agents::types::BackgroundAgent;
use crate::app_state::AppState;
use crate::call_validation::ChatMessage;
use crate::caps::BaseModelRecord;
use crate::global_context::GlobalContext;

pub const WITHHELD_DETAIL: &str =
    "[withheld: the parent destination is not permitted to receive this path]";

pub fn limits_from(settings: &refact_work_core::CoordinationSettings) -> HandoffLimits {
    HandoffLimits {
        max_entries: settings.handoff_report_max_entries.max(1),
        max_chars: settings.handoff_report_max_chars.max(1),
    }
}

pub fn current_limits() -> HandoffLimits {
    limits_from(&crate::runtime_settings::current().coordination)
}

pub fn metric_key(profile_name: Option<&str>, config_name: &str) -> String {
    match profile_name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => format!("profile:{name}"),
        None => format!("config:{config_name}"),
    }
}

pub fn record_key(record: &BackgroundAgent) -> String {
    metric_key(record.profile_name.as_deref(), &record.config_name)
}

/// Every declared path is checked against the privacy policy for the model the
/// parent chat talks to, exactly like any other content bound for that model.
/// An unresolvable destination withholds rather than discloses.
pub fn redact_for_destination(
    gcx: &Arc<GlobalContext>,
    destination: Option<&BaseModelRecord>,
    report: &mut HandoffReport,
) {
    let mut withheld = 0usize;
    for gap in &mut report.gaps {
        let Some(path) = gap.path.clone() else {
            continue;
        };
        if destination.is_some_and(|model| path_permitted(gcx, &path, model)) {
            continue;
        }
        gap.path = None;
        gap.detail = WITHHELD_DETAIL.to_string();
        withheld += 1;
    }
    if withheld > 0 {
        report.notes.push(format!(
            "{withheld} declared path(s) were withheld by the privacy policy for the parent destination"
        ));
    }
}

/// Suggestions are content too: a path the destination may not receive is
/// dropped and counted, never rendered.
pub fn redact_metric(
    gcx: &Arc<GlobalContext>,
    destination: Option<&BaseModelRecord>,
    metric: &mut HandoffMetric,
) {
    let before = metric.suggested_sources.len();
    metric
        .suggested_sources
        .retain(|path| destination.is_some_and(|model| path_permitted(gcx, path, model)));
    metric.withheld_sources = before.saturating_sub(metric.suggested_sources.len()) as u32;
}

fn path_permitted(gcx: &Arc<GlobalContext>, path: &str, model: &BaseModelRecord) -> bool {
    let Ok(records) =
        crate::privacy::records::declared_file_records(gcx, [std::path::PathBuf::from(path)])
    else {
        return false;
    };
    let mut probe = ChatMessage::default();
    crate::privacy::records::merge_records(&mut probe, records);
    crate::agents::launch_spec::clear_current_context(gcx, std::slice::from_ref(&probe), model)
        .is_ok()
}

pub async fn parent_destination(app: &AppState, parent_chat_id: &str) -> Option<BaseModelRecord> {
    let session = app
        .chat
        .sessions
        .read()
        .await
        .get(parent_chat_id)
        .cloned()?;
    let model_id = session.lock().await.thread.model.clone();
    if model_id.trim().is_empty() {
        return None;
    }
    let caps = crate::global_context::try_load_caps_quickly_if_not_present(app.gcx.clone(), 0)
        .await
        .ok()?;
    crate::caps::resolve_chat_model(caps, &model_id)
        .ok()
        .map(|model| model.base.clone())
}

/// Bounded aggregation over finished children, newest first. Absent reports are
/// counted as `unknown`, never as a sufficient handoff.
pub fn aggregate_records(
    records: &[BackgroundAgent],
    limits: &HandoffLimits,
    max_records: usize,
) -> Vec<HandoffMetric> {
    aggregate_since(records, limits, max_records, None)
}

fn aggregate_since(
    records: &[BackgroundAgent],
    limits: &HandoffLimits,
    max_records: usize,
    oldest_ms: Option<i64>,
) -> Vec<HandoffMetric> {
    let mut finished = records
        .iter()
        .filter(|record| record.status.is_terminal())
        .filter(|record| {
            oldest_ms.is_none_or(|oldest| record.last_update_at.timestamp_millis() >= oldest)
        })
        .collect::<Vec<_>>();
    finished.sort_by(|left, right| right.last_update_at.cmp(&left.last_update_at));
    let keyed = finished
        .into_iter()
        .take(max_records.max(1))
        .map(|record| (record_key(record), record.handoff.clone()))
        .collect::<Vec<_>>();
    refact_chat_api::handoff::aggregate(
        keyed
            .iter()
            .map(|(key, report)| (key.as_str(), report.as_ref())),
        limits,
    )
}

/// The bounded slice of finished children one finding may read: a record cap
/// and a retention window, so this stays a metric rather than a transcript
/// store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FindingWindow {
    pub now_ms: i64,
    pub retention_ms: i64,
    pub max_records: usize,
    pub min_samples: u32,
}

impl FindingWindow {
    pub fn current(now_ms: i64) -> Self {
        let settings = crate::runtime_settings::current().coordination;
        FindingWindow {
            now_ms,
            retention_ms: settings.handoff_finding_retention_ms as i64,
            max_records: settings.handoff_metric_max_records,
            min_samples: settings.handoff_finding_min_samples as u32,
        }
    }

    pub fn now() -> Self {
        Self::current(chrono::Utc::now().timestamp_millis())
    }

    fn oldest_ms(&self) -> i64 {
        self.now_ms.saturating_sub(self.retention_ms.max(1))
    }
}

/// The review signal for every launch key inside the window. Advisory: it names
/// what recurred and never edits a profile or launch specification.
pub fn findings(
    records: &[BackgroundAgent],
    limits: &HandoffLimits,
    window: FindingWindow,
) -> Vec<HandoffFinding> {
    aggregate_since(
        records,
        limits,
        window.max_records,
        Some(window.oldest_ms()),
    )
    .iter()
    .map(|metric| metric.finding(window.min_samples))
    .collect()
}

/// A key nothing was recorded for is `insufficient_data` with a zero sample,
/// never a clean bill of health.
pub fn finding_for(
    records: &[BackgroundAgent],
    key: &str,
    limits: &HandoffLimits,
    window: FindingWindow,
) -> HandoffFinding {
    findings(records, limits, window)
        .into_iter()
        .find(|finding| finding.key() == key)
        .unwrap_or_else(|| HandoffFinding::InsufficientData {
            key: key.to_string(),
            samples: 0,
            required: window.min_samples.max(1),
        })
}

pub fn metric_for(
    records: &[BackgroundAgent],
    key: &str,
    limits: &HandoffLimits,
    max_records: usize,
) -> Option<HandoffMetric> {
    aggregate_records(records, limits, max_records)
        .into_iter()
        .find(|metric| metric.key == key)
        .filter(|metric| metric.completions > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_chat_api::handoff::{HandoffGap, HandoffGapCategory, HandoffSufficiency};

    fn report(path: &str) -> HandoffReport {
        HandoffReport {
            sufficiency: HandoffSufficiency::GapsReported,
            source: refact_chat_api::HANDOFF_SOURCE_CHILD_DECLARATION.to_string(),
            claimed_by: "agent-1".to_string(),
            claimed_at_ms: 1,
            gaps: vec![HandoffGap {
                category: HandoffGapCategory::MissingFile,
                detail: "needed the retry rule".to_string(),
                path: Some(path.to_string()),
                extra_steps: Some(2),
            }],
            notes: Vec::new(),
        }
    }

    #[tokio::test]
    async fn declared_paths_are_filtered_for_the_parent_destination_before_they_reach_a_model() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let workspace = tempfile::tempdir().unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![workspace.path().into()];
        let allowed = workspace.path().join("retry.rs");
        let blocked = workspace.path().join("secret.env");
        std::fs::write(&allowed, "fn retry() {}\n").unwrap();
        std::fs::write(&blocked, "SECRET=1\n").unwrap();
        gcx.privacy_policy_load.write().unwrap().policy = Arc::new(refact_privacy::PrivacyPolicy {
            blocked: vec!["*.env".into()],
            ..Default::default()
        });
        let model = BaseModelRecord {
            id: "allowed/model".to_string(),
            ..Default::default()
        };

        let mut permitted = report(&allowed.to_string_lossy());
        redact_for_destination(&gcx, Some(&model), &mut permitted);
        assert_eq!(
            permitted.gaps[0].path.as_deref(),
            Some(allowed.to_string_lossy().as_ref())
        );
        assert!(permitted.notes.is_empty());

        let mut denied = report(&blocked.to_string_lossy());
        redact_for_destination(&gcx, Some(&model), &mut denied);
        assert!(denied.gaps[0].path.is_none());
        assert_eq!(denied.gaps[0].detail, WITHHELD_DETAIL);
        assert!(denied.notes[0].contains("withheld by the privacy policy"));
        assert_eq!(denied.gaps[0].category, HandoffGapCategory::MissingFile);

        let mut unresolved = report(&allowed.to_string_lossy());
        redact_for_destination(&gcx, None, &mut unresolved);
        assert!(unresolved.gaps[0].path.is_none());
    }

    fn record(agent_id: &str, status: &str, last_update: &str) -> BackgroundAgent {
        serde_json::from_value(serde_json::json!({
            "schema_version": 1,
            "agent_id": agent_id,
            "parent_chat_id": "parent",
            "parent_root_chat_id": null,
            "parent_tool_call_id": null,
            "child_chat_id": null,
            "kind": "subagent",
            "config_name": "subagent",
            "title": agent_id,
            "prompt": "prompt",
            "target_files": [],
            "status": status,
            "progress": null,
            "step_count": 0,
            "last_activity": null,
            "result_summary": null,
            "result_payload_path": null,
            "error": null,
            "edited_files": [],
            "diff_summary": null,
            "conflict_summary": null,
            "completion_message_id": null,
            "completion_pushed_at": null,
            "model": "model",
            "created_at": "2026-01-01T00:00:00Z",
            "started_at": null,
            "finished_at": null,
            "last_update_at": last_update,
            "change_seq": 1
        }))
        .unwrap()
    }

    fn gap(category: HandoffGapCategory) -> HandoffGap {
        HandoffGap {
            category,
            detail: "d".to_string(),
            path: None,
            extra_steps: None,
        }
    }

    fn declared(agent_id: &str, at: &str, gaps: Vec<HandoffGap>) -> BackgroundAgent {
        let mut record = record(agent_id, "completed", at);
        record.profile_name = Some("reviewer".to_string());
        record.handoff = Some(HandoffReport {
            sufficiency: if gaps.is_empty() {
                HandoffSufficiency::Sufficient
            } else {
                HandoffSufficiency::GapsReported
            },
            source: refact_chat_api::HANDOFF_SOURCE_CHILD_DECLARATION.to_string(),
            claimed_by: agent_id.to_string(),
            claimed_at_ms: 1,
            gaps,
            notes: Vec::new(),
        });
        record
    }

    fn window(min_samples: u32) -> FindingWindow {
        FindingWindow {
            now_ms: chrono::DateTime::parse_from_rfc3339("2026-01-10T00:00:00Z")
                .unwrap()
                .timestamp_millis(),
            retention_ms: 7 * 24 * 60 * 60 * 1_000,
            max_records: 64,
            min_samples,
        }
    }
}
