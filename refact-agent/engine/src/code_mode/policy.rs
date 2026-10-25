//! What orchestration code is allowed to call, derived from the authority the
//! assignment already carries.
//!
//! There is no second authority model here. The callable set is the declared
//! catalog intersected with the run's `ToolsPolicy` ceiling, so code mode can
//! only ever be narrower than the tools the same run could call directly.

use serde_json::Value;

use crate::agents::tool_authority::allows_dispatch;
use crate::subchat::ToolsPolicy;

use super::hosts::{declared_names, operation, HostCall, HostOperation, HOST_OPERATIONS};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Capabilities {
    granted: Vec<&'static str>,
}

/// An open ceiling is not a ceiling. `ToolsPolicy::All` means the run's
/// authority was never frozen to concrete names, and code mode refuses rather
/// than granting everything the catalog happens to list.
pub fn capabilities(ceiling: &ToolsPolicy) -> Result<Capabilities, String> {
    if matches!(ceiling, ToolsPolicy::All) {
        return Err(
            "code mode requires a concrete authority ceiling; open tool authority is not a ceiling"
                .to_string(),
        );
    }
    Ok(Capabilities {
        granted: HOST_OPERATIONS
            .iter()
            .filter(|op| allows_dispatch(ceiling, op.tool))
            .map(|op| op.name)
            .collect(),
    })
}

impl Capabilities {
    pub fn granted(&self) -> &[&'static str] {
        &self.granted
    }

    pub fn is_empty(&self) -> bool {
        self.granted.is_empty()
    }

    pub fn allows(&self, op: &str) -> bool {
        self.granted.contains(&op)
    }

    /// The schemas handed to the model: exactly the granted operations, so the
    /// authored code is never told about authority it does not have.
    pub fn schemas(&self) -> Vec<Value> {
        HOST_OPERATIONS
            .iter()
            .filter(|op| self.allows(op.name))
            .map(HostOperation::schema)
            .collect()
    }

    pub fn authorize(&self, call: &HostCall) -> Result<&'static HostOperation, String> {
        let Some(op) = operation(&call.op) else {
            return Err(format!(
                "host operation `{}` is not declared; code mode declares only {}",
                call.op,
                declared_names()
            ));
        };
        if !self.allows(op.name) {
            return Err(format!(
                "host operation `{}` is outside the assignment authority ceiling (tool `{}`)",
                op.name, op.tool
            ));
        }
        op.validate(&call.arguments)?;
        Ok(op)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ceiling(tools: &[&str]) -> ToolsPolicy {
        ToolsPolicy::Only(tools.iter().map(|tool| tool.to_string()).collect())
    }

    #[test]
    fn nothing_is_callable_until_the_ceiling_grants_the_backing_tool() {
        let none = capabilities(&ceiling(&["agent_wait"])).unwrap();

        assert!(none.is_empty());
        assert!(none.schemas().is_empty());
        assert!(!none.allows("report"));

        let narrow = capabilities(&ceiling(&["progress_report"])).unwrap();
        assert_eq!(narrow.granted(), &["report"]);
        assert!(!narrow.allows("spawn"));
    }

    #[test]
    fn an_open_authority_is_refused_rather_than_read_as_everything() {
        assert!(capabilities(&ToolsPolicy::All)
            .unwrap_err()
            .contains("concrete authority ceiling"));
    }

    #[test]
    fn a_call_outside_the_ceiling_is_denied_even_though_it_is_declared() {
        let capabilities = capabilities(&ceiling(&["progress_report", "wait"])).unwrap();

        let denied = capabilities
            .authorize(&HostCall::new(
                "spawn",
                json!({"task": "t", "expected_result": "r"}),
            ))
            .unwrap_err();

        assert!(denied.contains("outside the assignment authority ceiling"));
        assert!(denied.contains("subagent"));
        capabilities
            .authorize(&HostCall::new("report", json!({"status_line": "on it"})))
            .unwrap();
    }

    #[test]
    fn an_undeclared_operation_cannot_be_reached_by_any_ceiling() {
        let capabilities = capabilities(&ceiling(&["shell", "apply", "subagent"])).unwrap();

        let denied = capabilities
            .authorize(&HostCall::new("shell", json!({"command": "rm -rf /"})))
            .unwrap_err();

        assert!(denied.contains("is not declared"));
        assert_eq!(capabilities.granted(), &["spawn"]);
    }

    #[test]
    fn granted_schemas_describe_only_granted_operations() {
        let capabilities = capabilities(&ceiling(&["subagent", "wait"])).unwrap();

        let names: Vec<String> = capabilities
            .schemas()
            .iter()
            .map(|schema| schema["op"].as_str().unwrap().to_string())
            .collect();

        assert_eq!(names, vec!["spawn".to_string(), "wait".to_string()]);
    }
}
