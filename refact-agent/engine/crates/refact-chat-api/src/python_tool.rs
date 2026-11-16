use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct PythonCallRecord {
    pub seq: u32,
    pub call_id: String,
    pub tool: String,
    pub args_preview: String,
    pub status: PythonCallStatus,
    pub summary: String,
    pub started_at_ms: u64,
    pub duration_ms: Option<u64>,
    pub approval: PythonCallApproval,
    pub diffs: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub col: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_in: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_out: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PythonCallStatus {
    Running,
    Ok,
    Failed,
    Denied,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PythonCallApproval {
    NotNeeded,
    Auto,
    Allowed,
    AllowedForChat,
    AllowedForRun,
    Denied,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PythonRunUpdate {
    Started { background: bool },
    CallStarted { record: PythonCallRecord },
    CallFinished { record: PythonCallRecord },
    Stdout { text: String },
    Detached {},
    Display { display: PythonDisplay },
    Finished { status: String },
}

/// One table, chart or markdown block a script showed with `show()`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct PythonDisplay {
    pub index: u32,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<Vec<Vec<serde_json::Value>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows_total: Option<u64>,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub y: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct PythonApprovalRequest {
    pub request_id: String,
    pub tool_call_id: String,
    pub run_id: String,
    pub call_id: String,
    pub tool_name: String,
    pub args_preview: String,
    pub command: Option<String>,
    pub rule: String,
    pub editing_tool: bool,
    pub created_at_ms: u64,
    /// Only a person may answer: no run or chat permit covers this request.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub needs_person: bool,
    /// Present when a script asks to change the engine itself (`engine("area.fn")`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<PythonEngineRequest>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PythonEngineClass {
    Change,
    Risky,
    Destructive,
}

/// One engine value a change touches; absent `old` means it was not set, absent `new` means removed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct PythonEngineItem {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
    /// Never on the wire: the label is built from a name a user chose (a zone, provider, model,
    /// tool or path), not a configuration field name, so it does not make the value a secret.
    #[serde(skip)]
    pub label_is_name: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct PythonEngineRequest {
    pub function: String,
    pub class: PythonEngineClass,
    pub title: String,
    pub items: Vec<PythonEngineItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

/// An applied engine change, as listed in `extra.python.engine_changes`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct PythonEngineChange {
    pub seq: u32,
    pub function: String,
    pub class: PythonEngineClass,
    pub title: String,
    pub items: Vec<PythonEngineItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    pub at_ms: u64,
    /// Omitted when the change was applied; `unknown` when the handler timed out or panicked after it started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<PythonEngineOutcome>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PythonEngineOutcome {
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PythonApprovalDecisionKind {
    AllowOnce,
    AllowForChat,
    AllowForRun,
    Deny,
    Stop,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PythonRunControlAction {
    Stop,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct PythonRunSnapshot {
    pub tool_call_id: String,
    pub run_id: String,
    pub status: PythonRunLiveStatus,
    pub started_at_ms: u64,
    pub calls: Vec<PythonCallRecord>,
    pub stdout_tail: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub displays: Vec<PythonDisplay>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PythonRunLiveStatus {
    Running,
    Background,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn call_record_call_site_is_optional_on_the_wire() {
        let base = serde_json::json!({"seq":1,"call_id":"py_r_1","tool":"cat","args_preview":"{}","status":"ok","summary":"","started_at_ms":1,"duration_ms":2,"approval":"not_needed","diffs":[]});
        let record: PythonCallRecord = serde_json::from_value(base.clone()).unwrap();
        assert_eq!((record.line, record.col), (None, None));
        assert_eq!(serde_json::to_value(&record).unwrap(), base);
        let mut with = base;
        with["line"] = 3.into();
        with["col"] = 4.into();
        let record: PythonCallRecord = serde_json::from_value(with.clone()).unwrap();
        assert_eq!((record.line, record.col), (Some(3), Some(4)));
        assert_eq!(serde_json::to_value(&record).unwrap(), with);
    }
    #[test]
    fn run_scoped_approval_wire_values() {
        assert_eq!(
            serde_json::to_value(PythonApprovalDecisionKind::AllowForRun).unwrap(),
            "allow_for_run"
        );
        assert_eq!(
            serde_json::from_value::<PythonApprovalDecisionKind>("allow_for_run".into()).unwrap(),
            PythonApprovalDecisionKind::AllowForRun
        );
        assert_eq!(
            serde_json::to_value(PythonCallApproval::AllowedForRun).unwrap(),
            "allowed_for_run"
        );
        assert_eq!(
            serde_json::from_value::<PythonCallApproval>("allowed_for_run".into()).unwrap(),
            PythonCallApproval::AllowedForRun
        );
    }
    #[test]
    fn call_record_llm_fields_are_optional_on_the_wire() {
        let base = serde_json::json!({"seq":1,"call_id":"py_r_1","tool":"llm","args_preview":"{}","status":"ok","summary":"","started_at_ms":1,"duration_ms":2,"approval":"not_needed","diffs":[]});
        let record: PythonCallRecord = serde_json::from_value(base.clone()).unwrap();
        assert_eq!(
            (record.model.as_deref(), record.tokens_in, record.tokens_out),
            (None, None, None)
        );
        assert_eq!(serde_json::to_value(&record).unwrap(), base);
        let mut with = base;
        with["model"] = "openai/gpt-5-mini".into();
        with["tokens_in"] = 12.into();
        with["tokens_out"] = 3.into();
        let record: PythonCallRecord = serde_json::from_value(with.clone()).unwrap();
        assert_eq!(record.model.as_deref(), Some("openai/gpt-5-mini"));
        assert_eq!((record.tokens_in, record.tokens_out), (Some(12), Some(3)));
        assert_eq!(serde_json::to_value(&record).unwrap(), with);
    }
    #[test]
    fn engine_wire_shapes_and_older_frames() {
        let old = serde_json::json!({"request_id":"q","tool_call_id":"t","run_id":"r","call_id":"c","tool_name":"cat","args_preview":"{}","command":null,"rule":"ask","editing_tool":false,"created_at_ms":1});
        let request: PythonApprovalRequest = serde_json::from_value(old.clone()).unwrap();
        assert!(request.engine.is_none());
        assert_eq!(serde_json::to_value(&request).unwrap(), old);
        let mut with = old;
        with["tool_name"] = "engine.settings.set".into();
        with["engine"] = serde_json::json!({"function":"settings.set","class":"risky","title":"change engine settings","items":[{"label":"python_max_nested_calls","old":1000,"new":5},{"label":"x","new":{"a":1},"truncated":true},{"label":"gone","old":"v"}],"scope":"project","warning":"Scripts may make more nested calls."});
        let request: PythonApprovalRequest = serde_json::from_value(with.clone()).unwrap();
        let engine = request.engine.as_ref().unwrap();
        assert_eq!(engine.class, PythonEngineClass::Risky);
        assert!(engine.items[1].truncated && !engine.items[0].truncated);
        assert_eq!(engine.items[2].new, None);
        assert_eq!(serde_json::to_value(&request).unwrap(), with);
        let change = serde_json::json!({"seq":3,"function":"settings.set","class":"change","title":"change engine settings","items":[{"label":"k","old":1,"new":2}],"at_ms":9});
        let parsed: PythonEngineChange = serde_json::from_value(change.clone()).unwrap();
        assert_eq!(parsed.scope, None);
        assert_eq!(serde_json::to_value(&parsed).unwrap(), change);
        for class in ["change", "risky", "destructive"] {
            let value: PythonEngineClass = serde_json::from_value(class.into()).unwrap();
            assert_eq!(serde_json::to_value(value).unwrap(), class);
        }
    }
    #[test]
    fn approval_request_needs_person_is_optional_on_the_wire() {
        let base = serde_json::json!({"request_id":"q","tool_call_id":"t","run_id":"r","call_id":"c","tool_name":"create_textdoc","args_preview":"{}","command":null,"rule":"x","editing_tool":true,"created_at_ms":1});
        let request: PythonApprovalRequest = serde_json::from_value(base.clone()).unwrap();
        assert!(!request.needs_person);
        assert_eq!(serde_json::to_value(&request).unwrap(), base);
        let mut with = base;
        with["needs_person"] = true.into();
        let request: PythonApprovalRequest = serde_json::from_value(with.clone()).unwrap();
        assert!(request.needs_person);
        assert_eq!(serde_json::to_value(&request).unwrap(), with);
    }
    #[test]
    fn display_wire_shape_and_older_frames() {
        let snapshot: PythonRunSnapshot = serde_json::from_value(serde_json::json!({"tool_call_id":"t","run_id":"r","status":"running","started_at_ms":1,"calls":[],"stdout_tail":""})).unwrap();
        assert!(snapshot.displays.is_empty());
        assert!(serde_json::to_value(&snapshot)
            .unwrap()
            .get("displays")
            .is_none());
        let minimal = PythonDisplay {
            index: 1,
            kind: "markdown".into(),
            title: None,
            columns: vec![],
            rows: None,
            rows_total: None,
            truncated: false,
            x: None,
            y: vec![],
            text: Some("hi".into()),
            path: "/d/1.json".into(),
        };
        assert_eq!(
            serde_json::to_value(&minimal).unwrap(),
            serde_json::json!({"index":1,"kind":"markdown","truncated":false,"text":"hi","path":"/d/1.json"})
        );
        let full = serde_json::json!({"index":2,"kind":"bar","title":"T","columns":["a","b"],"rows":[["x",1],[null,2.5],[true,"s"]],"rows_total":900,"truncated":true,"x":"a","y":["b"],"path":"/d/2.json"});
        let display: PythonDisplay = serde_json::from_value(full.clone()).unwrap();
        assert_eq!(serde_json::to_value(&display).unwrap(), full);
        let update = PythonRunUpdate::Display {
            display,
        };
        let wire = serde_json::to_value(&update).unwrap();
        assert_eq!(wire["kind"], "display");
        assert_eq!(wire["display"]["rows_total"], 900);
        assert_eq!(
            serde_json::from_value::<PythonRunUpdate>(wire).unwrap(),
            update
        );
    }
}
