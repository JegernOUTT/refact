use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};

use crate::operator_types::{NativeSourceIdentity, ProjectIdentity, ResourceRef, WorkerIdentity};
use crate::types::BuddyRuntimeEvent;

pub const OPERATOR_SIGNAL_SCHEMA_VERSION_V1: u16 = 1;
const MAX_PROJECT_SIGNALS: usize = 128;
const MAX_SOURCE_WATERMARKS_PER_PROJECT: usize = 128;
const MAX_PROJECTS: usize = 32;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperatorSignalKind {
    Observation,
    Warning,
    DecisionRef,
    OperationState,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperatorSignalAuthority {
    Hint,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct OperatorSignal {
    pub schema_version: u16,
    pub kind: OperatorSignalKind,
    pub authority: OperatorSignalAuthority,
    pub project: ProjectIdentity,
    pub worker: Option<WorkerIdentity>,
    pub source: NativeSourceIdentity,
    pub source_sequence: Option<u64>,
    pub chat: Option<ResourceRef>,
    pub tool: Option<ResourceRef>,
    pub goal: Option<ResourceRef>,
    pub runtime_event: BuddyRuntimeEvent,
}

#[derive(Default)]
pub struct OperatorSignalProjection {
    signals_by_project: HashMap<String, VecDeque<OperatorSignal>>,
    highest_source_sequences: HashMap<String, HashMap<SourceKey, u64>>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct SourceKey {
    source_kind: String,
    source_id: String,
    source_version: String,
    worker_id: Option<String>,
    engine_instance_id: Option<String>,
    worker_epoch: Option<u64>,
}

impl OperatorSignalProjection {
    pub fn accept(&mut self, signal: OperatorSignal) -> Result<bool, String> {
        validate_signal(&signal)?;
        let project_id = signal.project.project_id.clone();
        if !self.signals_by_project.contains_key(&project_id)
            && self.signals_by_project.len() == MAX_PROJECTS
        {
            return Err("operator signal project capacity reached".to_string());
        }
        let source_key = SourceKey {
            source_kind: format!("{:?}", signal.source.source_kind),
            source_id: signal.source.source_id.clone(),
            source_version: signal.source.source_version.clone(),
            worker_id: signal
                .worker
                .as_ref()
                .map(|worker| worker.worker_id.clone()),
            engine_instance_id: signal
                .worker
                .as_ref()
                .map(|worker| worker.engine_instance_id.clone()),
            worker_epoch: signal.worker.as_ref().map(|worker| worker.epoch),
        };
        if let Some(sequence) = signal.source_sequence {
            let watermarks = self
                .highest_source_sequences
                .entry(project_id.clone())
                .or_default();
            if watermarks
                .get(&source_key)
                .is_some_and(|highest| sequence <= *highest)
            {
                return Ok(false);
            }
            if !watermarks.contains_key(&source_key)
                && watermarks.len() == MAX_SOURCE_WATERMARKS_PER_PROJECT
            {
                return Err("operator signal source watermark capacity reached".to_string());
            }
            watermarks.insert(source_key, sequence);
        }

        let signals = self.signals_by_project.entry(project_id).or_default();
        if signals.len() == MAX_PROJECT_SIGNALS {
            signals.pop_front();
        }
        signals.push_back(signal);
        Ok(true)
    }

    pub fn signals_for_project(&self, project_id: &str) -> Vec<OperatorSignal> {
        self.signals_by_project
            .get(project_id)
            .map(|signals| signals.iter().cloned().collect())
            .unwrap_or_default()
    }
}

fn validate_signal(signal: &OperatorSignal) -> Result<(), String> {
    if signal.schema_version != OPERATOR_SIGNAL_SCHEMA_VERSION_V1 {
        return Err("unsupported operator signal schema version".to_string());
    }
    if signal.project.project_id.is_empty() || signal.project.canonical_root.is_empty() {
        return Err("operator signal project identity is required".to_string());
    }
    if signal.source.source_id.is_empty() || signal.source.source_version.is_empty() {
        return Err("operator signal native source identity is required".to_string());
    }
    if let Some(chat) = &signal.chat {
        if chat.resource_kind != "chat" || chat.resource_id.is_empty() || chat.version.is_empty() {
            return Err("operator signal chat reference is invalid".to_string());
        }
        if signal
            .runtime_event
            .chat_id
            .as_deref()
            .is_some_and(|id| id != chat.resource_id)
        {
            return Err("operator signal chat reference does not match runtime event".to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operator_types::NativeSourceKind;

    fn signal(project_id: &str, source_id: &str, sequence: Option<u64>) -> OperatorSignal {
        OperatorSignal {
            schema_version: OPERATOR_SIGNAL_SCHEMA_VERSION_V1,
            kind: OperatorSignalKind::OperationState,
            authority: OperatorSignalAuthority::Hint,
            project: ProjectIdentity {
                project_id: project_id.to_string(),
                canonical_root: format!("/{project_id}"),
            },
            worker: Some(WorkerIdentity {
                worker_id: "engine".to_string(),
                engine_instance_id: "instance".to_string(),
                epoch: 2,
            }),
            source: NativeSourceIdentity {
                source_kind: NativeSourceKind::Chat,
                source_id: source_id.to_string(),
                source_version: "1".to_string(),
            },
            source_sequence: sequence,
            chat: Some(ResourceRef {
                resource_kind: "chat".to_string(),
                resource_id: source_id.to_string(),
                version: "1".to_string(),
            }),
            tool: None,
            goal: None,
            runtime_event: serde_json::from_value(serde_json::json!({
                "id": "event-1", "signal_type": "paused", "title": "Paused",
                "source": "test", "status": "paused", "priority": "normal",
                "created_at": "2026-01-01T00:00:00Z"
            }))
            .unwrap(),
        }
    }

    #[test]
    fn projection_rejects_duplicate_and_out_of_order_per_source_only() {
        let mut projection = OperatorSignalProjection::default();
        assert!(projection
            .accept(signal("project-a", "chat-a", Some(2)))
            .unwrap());
        assert!(!projection
            .accept(signal("project-a", "chat-a", Some(2)))
            .unwrap());
        assert!(!projection
            .accept(signal("project-a", "chat-a", Some(1)))
            .unwrap());
        assert!(projection
            .accept(signal("project-a", "chat-b", Some(1)))
            .unwrap());
        assert!(projection
            .accept(signal("project-b", "chat-a", Some(1)))
            .unwrap());
        assert_eq!(projection.signals_for_project("project-a").len(), 2);
        assert_eq!(projection.signals_for_project("project-b").len(), 1);
    }

    #[test]
    fn projection_rejects_invalid_project_or_chat_correlation() {
        let mut projection = OperatorSignalProjection::default();
        let mut invalid = signal("project-a", "chat-a", None);
        invalid.project.project_id.clear();
        assert!(projection.accept(invalid).is_err());

        let mut mismatched = signal("project-a", "chat-a", None);
        mismatched.runtime_event.chat_id = Some("different-chat".to_string());
        assert!(projection.accept(mismatched).is_err());
    }
}
