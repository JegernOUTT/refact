use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tokio::sync::Mutex as AMutex;

use crate::app_state::AppState;
use crate::chat::types::{ChatSession, ToolDecisionItem};
use crate::ext::hooks_runner::{
    apply_permission_decision, get_project_dir_string, PermissionDecisionRequest,
};

pub(crate) struct PermissionHookOutcome {
    pub decisions: Vec<ToolDecisionItem>,
    pub denial_reasons: HashMap<String, String>,
}

struct PermissionHookTarget {
    tool_call_id: String,
    tool_name: String,
    tool_input: serde_json::Value,
    accepted: bool,
    source: String,
    reason_type: String,
}

pub(crate) async fn apply_confirmation_permission_hooks(
    app: &AppState,
    session_arc: &Arc<AMutex<ChatSession>>,
    decisions: &[ToolDecisionItem],
) -> PermissionHookOutcome {
    let (session_id, targets) = {
        let session = session_arc.lock().await;
        let mut seen = HashSet::new();
        let mut targets = Vec::new();
        for decision in decisions {
            if !seen.insert(decision.tool_call_id.as_str()) {
                continue;
            }
            let Some(reason) = session
                .runtime
                .pause_reasons
                .iter()
                .find(|reason| reason.tool_call_id == decision.tool_call_id)
            else {
                continue;
            };
            if reason.reason_type != "confirmation" {
                continue;
            }
            let Some(call) = session.messages.iter().find_map(|message| {
                message
                    .tool_calls
                    .as_ref()?
                    .iter()
                    .find(|call| call.id == decision.tool_call_id)
            }) else {
                continue;
            };
            if call.function.name == "cache_guard"
                || call.function.name == "agent_approval"
                || session
                    .approval_wrapper_calls
                    .contains_key(&decision.tool_call_id)
            {
                continue;
            }
            let tool_input = serde_json::to_value(call.function.parse_args().unwrap_or_default())
                .unwrap_or_else(|_| serde_json::json!({}));
            targets.push(PermissionHookTarget {
                tool_call_id: decision.tool_call_id.clone(),
                tool_name: call.function.name.clone(),
                tool_input,
                accepted: decision.accepted
                    && !decisions.iter().any(|other| {
                        other.tool_call_id == decision.tool_call_id && !other.accepted
                    }),
                source: if crate::chat::tools::is_shell_privacy_approval(reason) {
                    "shell_privacy_approval".to_string()
                } else {
                    "user_confirmation".to_string()
                },
                reason_type: reason.reason_type.clone(),
            });
        }
        (session.chat_id.clone(), targets)
    };

    let mut hooked_decisions = decisions.to_vec();
    let mut denial_reasons = HashMap::new();
    if targets.is_empty() {
        return PermissionHookOutcome {
            decisions: hooked_decisions,
            denial_reasons,
        };
    }
    let project_dir = get_project_dir_string(app.clone()).await;
    for target in targets {
        let result = apply_permission_decision(
            app.clone(),
            session_id.clone(),
            project_dir.clone(),
            PermissionDecisionRequest {
                tool_name: target.tool_name,
                tool_input: target.tool_input,
                accepted: target.accepted,
                source: target.source,
                request_id: None,
                tool_call_id: target.tool_call_id.clone(),
                reason_type: target.reason_type,
            },
        )
        .await;
        if target.accepted && !result.accepted {
            for decision in hooked_decisions
                .iter_mut()
                .filter(|decision| decision.tool_call_id == target.tool_call_id)
            {
                decision.accepted = false;
            }
            if let Some(reason) = result.denial_reason {
                denial_reasons.insert(target.tool_call_id, reason);
            }
        }
    }

    PermissionHookOutcome {
        decisions: hooked_decisions,
        denial_reasons,
    }
}
