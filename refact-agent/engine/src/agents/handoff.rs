use std::sync::Arc;

use refact_chat_api::handoff::{HandoffLimits, HandoffMetric, HandoffReport};

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
    let mut finished = records
        .iter()
        .filter(|record| record.status.is_terminal())
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

    #[test]
    fn records_group_by_profile_then_config_and_stay_bounded() {
        let mut profiled = record("profiled", "completed", "2026-01-03T00:00:00Z");
        profiled.profile_name = Some("reviewer".to_string());
        profiled.handoff = Some(report("src/a.rs"));
        let unprofiled = record("unprofiled", "completed", "2026-01-02T00:00:00Z");
        let mut running = record("running", "running", "2026-01-04T00:00:00Z");
        running.profile_name = Some("reviewer".to_string());

        let limits = HandoffLimits::default();
        let records = vec![profiled.clone(), unprofiled, running];
        let metrics = aggregate_records(&records, &limits, 64);
        assert_eq!(metrics.len(), 2);
        let reviewer = metric_for(&records, "profile:reviewer", &limits, 64).unwrap();
        assert_eq!(reviewer.completions, 1);
        assert_eq!(reviewer.with_gaps, 1);
        assert_eq!(reviewer.suggested_sources, vec!["src/a.rs"]);
        let config = metric_for(&records, "config:subagent", &limits, 64).unwrap();
        assert_eq!(config.completions, 1);
        assert_eq!(config.unknown, 1);
        assert!(config.suggested_sources.is_empty());
        assert!(metric_for(&records, "profile:missing", &limits, 64).is_none());
        assert_eq!(aggregate_records(&records, &limits, 1).len(), 1);
        assert_eq!(record_key(&profiled), "profile:reviewer");
    }
}
