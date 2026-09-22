use std::collections::BTreeSet;

use crate::app_state::AppState;
use crate::subchat::ToolsPolicy;
use crate::tools::tools_list::get_tools_for_mode;

pub(crate) fn canonical(name: &str) -> String {
    let name = name.trim();
    let resolved = crate::llm::adapters::claude_code_compat::cc_normalize_internal_tool_name(name);
    if resolved == "grep" {
        "search_pattern".into()
    } else {
        crate::llm::adapters::claude_code_compat::CC_TOOL_RENAMES
            .iter()
            .find_map(|(original, renamed)| (*renamed == resolved).then(|| original.to_string()))
            .unwrap_or(resolved)
    }
}

/// Keep the strictest of two supervised budgets field by field. `None` is
/// "unlimited", so any concrete limit always wins over it; a longer cooldown
/// and a lower no-progress threshold are the tighter side.
fn tightest_budget(
    left: &crate::chat::types::GoalBudget,
    right: &crate::chat::types::GoalBudget,
) -> crate::chat::types::GoalBudget {
    fn tightest<T: Ord + Copy>(left: Option<T>, right: Option<T>) -> Option<T> {
        match (left, right) {
            (Some(left), Some(right)) => Some(left.min(right)),
            (Some(only), None) | (None, Some(only)) => Some(only),
            (None, None) => None,
        }
    }
    crate::chat::types::GoalBudget {
        max_turns: tightest(left.max_turns, right.max_turns),
        max_minutes: tightest(left.max_minutes, right.max_minutes),
        max_tokens: tightest(left.max_tokens, right.max_tokens),
        max_cost_cents: tightest(left.max_cost_cents, right.max_cost_cents),
        cooldown_ms: left.cooldown_ms.max(right.cooldown_ms),
        no_progress_token_threshold: left
            .no_progress_token_threshold
            .min(right.no_progress_token_threshold),
        no_progress_turns: tightest(left.no_progress_turns, right.no_progress_turns),
        explicit: left.explicit || right.explicit,
    }
}

/// A supervised policy stays supervised through narrowing: degrading it to a
/// plain `Only` would silently drop the budget the native mandate attached to
/// it, which is a widening of authority, not a narrowing.
fn supervised_narrowed(
    tools: &[String],
    allowed: &[String],
    budget: crate::chat::types::GoalBudget,
) -> ToolsPolicy {
    ToolsPolicy::Supervised {
        tools: tools
            .iter()
            .filter(|name| allowed.contains(name))
            .cloned()
            .collect(),
        budget,
    }
}

pub(crate) fn intersect(policy: &ToolsPolicy, ceiling: &ToolsPolicy) -> ToolsPolicy {
    match (policy, ceiling) {
        (ToolsPolicy::None, _) | (_, ToolsPolicy::None) => ToolsPolicy::None,
        (ToolsPolicy::All, other) | (other, ToolsPolicy::All) => other.clone(),
        (ToolsPolicy::Only(names), ToolsPolicy::Only(allowed)) => ToolsPolicy::from_option(Some(
            names
                .iter()
                .filter(|name| allowed.contains(name))
                .cloned()
                .collect(),
        )),
        // `Supervised` is a concrete, bounded tool set minted by the native
        // authority-guarded spawn path. It is never "all tools": intersecting
        // it can only remove names, never add any.
        (
            ToolsPolicy::Supervised { tools, budget },
            ToolsPolicy::Supervised {
                tools: allowed,
                budget: ceiling_budget,
            },
        ) => supervised_narrowed(tools, allowed, tightest_budget(budget, ceiling_budget)),
        (ToolsPolicy::Supervised { tools, budget }, ToolsPolicy::Only(allowed))
        | (ToolsPolicy::Only(allowed), ToolsPolicy::Supervised { tools, budget }) => {
            supervised_narrowed(tools, allowed, budget.clone())
        }
    }
}

pub(crate) async fn current_ceiling(
    app: &AppState,
    parent_id: &str,
    model: &str,
    child_mode: &str,
) -> Result<ToolsPolicy, String> {
    let parent = app.chat.sessions.read().await.get(parent_id).cloned();
    let (mode, parent_model, command_tools) = if let Some(parent) = parent {
        let parent = parent.lock().await;
        if parent.closed {
            return Err("parent tool authority is unavailable".into());
        }
        (
            parent.thread.mode.clone(),
            parent.thread.model.clone(),
            parent.active_command.allowed_tools.clone(),
        )
    } else {
        (child_mode.to_string(), model.to_string(), Vec::new())
    };
    let names: BTreeSet<String> = get_tools_for_mode(app.gcx.clone(), &mode, Some(model))
        .await
        .into_iter()
        .map(|tool| tool.tool_description().name)
        .collect();
    let mut ceiling = ToolsPolicy::from_option(Some(names.into_iter().collect()));
    if !parent_model.is_empty() && parent_model != model {
        let parent_names = get_tools_for_mode(app.gcx.clone(), &mode, Some(&parent_model))
            .await
            .into_iter()
            .map(|tool| tool.tool_description().name)
            .collect();
        ceiling = intersect(&ceiling, &ToolsPolicy::from_option(Some(parent_names)));
    }
    if !command_tools.is_empty() {
        ceiling = intersect(
            &ceiling,
            &ToolsPolicy::Only(command_tools.iter().map(|name| canonical(name)).collect()),
        );
    }
    if let Some(parent_agent) = app.agents.find_agent_id_by_child_chat_id(parent_id).await {
        let config = app
            .agent_executions
            .saved_config(&parent_agent)
            .ok_or("parent child tool authority unavailable")?;
        if matches!(config.tools, ToolsPolicy::All) {
            return Err("parent child tool authority requires migration".into());
        }
        ceiling = intersect(&ceiling, &config.tools);
    }
    Ok(ceiling)
}

pub(crate) async fn freeze(
    app: &AppState,
    parent_id: &str,
    model: &str,
    mode: &str,
    requested: &ToolsPolicy,
) -> Result<ToolsPolicy, String> {
    let ceiling = current_ceiling(app, parent_id, model, mode).await?;
    if let ToolsPolicy::Only(names) = requested {
        let requested = ToolsPolicy::Only(
            names
                .iter()
                .map(|name| canonical(name))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
        );
        let narrowed = intersect(&requested, &ceiling);
        if let ToolsPolicy::Only(names) = &requested {
            if names.iter().any(|name| !narrowed.allows_tool(name)) {
                return Err("requested child tools exceed the current parent authority".into());
            }
        }
        Ok(narrowed)
    } else {
        Ok(intersect(requested, &ceiling))
    }
}

pub(crate) fn allows_dispatch(policy: &ToolsPolicy, tool_name: &str) -> bool {
    let name = canonical(tool_name);
    if policy.allows_tool(&name) || policy.allows_tool(&format!("mcp_{tool_name}")) {
        return true;
    }
    matches!(name.as_str(), "mcp_call" | "mcp_tool_search")
        && matches!(policy, ToolsPolicy::Only(names) if names.iter().any(|name| name.starts_with("mcp_") && !matches!(name.as_str(), "mcp_call" | "mcp_tool_search")))
}

pub(crate) async fn ensure_context_tool(
    context: &std::sync::Arc<
        tokio::sync::Mutex<crate::at_commands::at_commands::AtCommandsContext>,
    >,
    registered_name: &str,
) -> Result<(), String> {
    let (app, agent_id, mut ceiling) = {
        let context = context.lock().await;
        (
            context.app.clone(),
            context.background_agent_id.clone(),
            context.child_tool_ceiling.clone(),
        )
    };
    if let Some(agent_id) = agent_id {
        let config = app
            .agent_executions
            .saved_config(&agent_id)
            .ok_or("child tool authority unavailable")?;
        if matches!(config.tools, ToolsPolicy::All) {
            return Err("child tool authority requires migration".into());
        }
        ceiling = Some(ceiling.as_ref().map_or(config.tools.clone(), |ceiling| {
            intersect(ceiling, &config.tools)
        }));
    }
    let allowed = ceiling.as_ref().is_none_or(|ceiling| {
        ceiling.allows_tool(registered_name)
            || (matches!(registered_name, "mcp_call" | "mcp_tool_search")
                && allows_dispatch(ceiling, registered_name))
    });
    if allowed {
        Ok(())
    } else {
        Err("tool is outside accepted child authority".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn registered_dispatch_enforces_deny_all_and_exact_alias_target() {
        let (_gcx, app, parent, _workspace) =
            crate::agents::tests::app_with_parent_session("dispatch-parent").await;
        let context = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::at_commands::at_commands::AtCommandsContext::new_from_app(
                app.clone(),
                4096,
                1,
                false,
                vec![],
                "dispatch-parent".into(),
                None,
                "model".into(),
                None,
                None,
            )
            .await,
        ));
        let catalog = app
            .tool_registry
            .acquire_tool_catalog("agent", Some("model"), None)
            .await;
        let pool = app
            .tool_registry
            .acquire_turn_tool_pool("agent", Some("model"), None, &catalog)
            .await;
        for ceiling in [ToolsPolicy::None, ToolsPolicy::Only(vec!["cat".into()])] {
            context.lock().await.child_tool_ceiling = Some(ceiling);
            for with_pool in [false, true] {
                let result = app
                    .tool_registry
                    .execute_tool_with_catalog_and_pool(
                        &context,
                        &catalog,
                        if with_pool { pool.as_ref() } else { None },
                        "agent",
                        Some("model"),
                        "denied",
                        "t_set_tasks",
                        serde_json::json!({"tasks":[]}).as_object().unwrap().clone(),
                    )
                    .await;
                assert!(result
                    .err()
                    .is_some_and(|error| error.contains("authority")));
            }
        }
        assert!(parent.lock().await.messages.is_empty());
        context.lock().await.child_tool_ceiling = Some(ToolsPolicy::Only(vec!["get_goal".into()]));
        let result = app
            .tool_registry
            .execute_tool_with_catalog_and_pool(
                &context,
                &catalog,
                pool.as_ref(),
                "agent",
                Some("model"),
                "allowed",
                "t_get_goal",
                Default::default(),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.messages[0].tool_call_id, "allowed");
        assert_ne!(result.messages[0].tool_failed, Some(true));
        context.lock().await.child_tool_ceiling = Some(ToolsPolicy::None);
        let thread = parent.lock().await.thread.clone();
        let call = crate::call_validation::ChatToolCall {
            id: "denied-through-helper".into(),
            function: crate::call_validation::ChatToolFunction {
                name: "get_goal".into(),
                arguments: "{}".into(),
            },
            index: Some(0),
            tool_type: "function".into(),
            extra_content: None,
            started_at_ms: None,
            completed_at_ms: None,
        };
        let (messages, _) = crate::chat::tools::execute_tools(
            app.clone(),
            &[call],
            &[],
            &thread,
            "agent",
            Some("model"),
            crate::chat::tools::ExecuteToolsOptions {
                execution_context: Some(context),
                ..Default::default()
            },
        )
        .await;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tool_call_id, "denied-through-helper");
        assert_eq!(messages[0].tool_failed, Some(true));
    }

    #[test]
    fn canonical_aliases_and_mcp_transport_are_not_ambient_authority() {
        assert_eq!(canonical("t_apply"), "apply_patch");
        assert_eq!(canonical("regex_search"), "search_pattern");
        assert_eq!(canonical("cat"), "cat");
        assert_eq!(canonical("mcp_fixture_read"), "mcp_fixture_read");
        let authority = ToolsPolicy::Only(vec!["mcp_fixture_read".into()]);
        assert!(allows_dispatch(&authority, "fixture_read"));
        assert!(!allows_dispatch(&authority, "fixture_write"));
        assert!(!ToolsPolicy::Only(vec!["mcp_call".into()]).allows_tool("mcp_fixture_read"));
    }

    #[tokio::test]
    async fn parent_command_and_mode_are_independent_ceilings() {
        let (_gcx, app, parent, _workspace) =
            crate::agents::tests::app_with_parent_session("tools-parent").await;
        parent.lock().await.thread.mode = "agent".into();
        let frozen = freeze(&app, "tools-parent", "model", "agent", &ToolsPolicy::All)
            .await
            .unwrap();
        assert!(matches!(frozen, ToolsPolicy::Only(_)));
        assert!(frozen.allows_tool("cat"));
        assert!(frozen.allows_tool("shell"));
        parent.lock().await.active_command.allowed_tools = vec!["cat".into()];
        let ceiling = current_ceiling(&app, "tools-parent", "model", "agent")
            .await
            .unwrap();
        let narrowed = intersect(&frozen, &ceiling);
        assert!(narrowed.allows_tool("cat"));
        assert!(!narrowed.allows_tool("shell"));
        assert!(!narrowed.allows_tool("agent_message"));
        assert!(freeze(
            &app,
            "tools-parent",
            "model",
            "agent",
            &ToolsPolicy::Only(vec!["shell".into()])
        )
        .await
        .is_err());
        parent.lock().await.active_command.allowed_tools.clear();
        parent.lock().await.thread.mode = "NO_TOOLS".into();
        assert!(freeze(
            &app,
            "tools-parent",
            "model",
            "agent",
            &ToolsPolicy::Only(vec!["shell".into()])
        )
        .await
        .is_err());
        let restored: ToolsPolicy =
            serde_json::from_str(&serde_json::to_string(&narrowed).unwrap()).unwrap();
        let widened = current_ceiling(&app, "tools-parent", "model", "agent")
            .await
            .unwrap();
        assert!(!intersect(&restored, &widened).allows_tool("agent_message"));
    }

    #[tokio::test]
    async fn registered_parent_ceiling_is_unavailable_legacy_or_concrete() {
        use crate::agents::types::{BgAgentKind, CreateAgentRequest};
        let (_gcx, app, _parent, _workspace) =
            crate::agents::tests::app_with_parent_session("nested-root").await;
        let (record, _, _) = app
            .agents
            .create(CreateAgentRequest {
                parent_chat_id: "nested-root".into(),
                parent_root_chat_id: Some("nested-root".into()),
                parent_tool_call_id: None,
                kind: BgAgentKind::Subagent,
                config_name: "test".into(),
                title: "middle".into(),
                prompt: "test".into(),
                target_files: vec![],
                model: "model".into(),
                model_type: None,
                goal_summary: None,
                plan_present: false,
                worktree_id: None,
                worktree_branch: None,
            })
            .await
            .unwrap();
        let middle = "subchat-nested-middle";
        app.agents
            .mark_running(&record.agent_id, middle.into())
            .await
            .unwrap();
        assert!(current_ceiling(&app, middle, "model", "agent")
            .await
            .unwrap_err()
            .contains("unavailable"));
        let mut config = crate::subchat::test_support::minimal_config(middle, "nested-root");
        config.model = "model".into();
        config.tools = ToolsPolicy::All;
        app.agent_executions
            .remember_config(&record.agent_id, config.clone());
        assert!(current_ceiling(&app, middle, "model", "agent")
            .await
            .unwrap_err()
            .contains("migration"));
        assert!(freeze(&app, middle, "model", "agent", &ToolsPolicy::All)
            .await
            .is_err());
        config.tools = ToolsPolicy::Only(vec!["cat".into()]);
        app.agent_executions
            .remember_config(&record.agent_id, config.clone());
        let ceiling = current_ceiling(&app, middle, "model", "agent")
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&ceiling).unwrap(),
            serde_json::json!({"Only": ["cat"]})
        );
        let inherited = freeze(&app, middle, "model", "agent", &ToolsPolicy::All)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&inherited).unwrap(),
            serde_json::json!({"Only": ["cat"]})
        );
        assert!(freeze(
            &app,
            middle,
            "model",
            "agent",
            &ToolsPolicy::Only(vec!["cat".into(), "shell".into()])
        )
        .await
        .is_err());
        config.tools = ToolsPolicy::None;
        app.agent_executions
            .remember_config(&record.agent_id, config);
        assert!(matches!(
            freeze(&app, middle, "model", "agent", &ToolsPolicy::All)
                .await
                .unwrap(),
            ToolsPolicy::None
        ));
        assert!(freeze(
            &app,
            middle,
            "model",
            "agent",
            &ToolsPolicy::Only(vec!["cat".into()])
        )
        .await
        .is_err());
    }

    #[test]
    fn empty_and_inherited_ceilings_never_expand_concrete_authority() {
        let accepted = ToolsPolicy::Only(vec!["cat".into(), "shell".into()]);
        assert!(matches!(
            intersect(&accepted, &ToolsPolicy::None),
            ToolsPolicy::None
        ));
        let current = ToolsPolicy::Only(vec!["cat".into(), "new_tool".into()]);
        let narrowed = intersect(&accepted, &current);
        assert!(narrowed.allows_tool("cat"));
        assert!(!narrowed.allows_tool("new_tool"));
        assert!(!narrowed.allows_tool("shell"));
    }

    #[test]
    fn proxy_dispatch_is_not_underlying_tool_authority() {
        let authority = ToolsPolicy::Only(vec!["mcp_one_read".into()]);
        assert!(allows_dispatch(&authority, "mcp_call"));
        assert!(allows_dispatch(&authority, "mcp_tool_search"));
        assert!(!authority.allows_tool("mcp_other_write"));
        assert!(!allows_dispatch(&ToolsPolicy::None, "mcp_call"));
        assert!(!allows_dispatch(
            &ToolsPolicy::Only(vec!["cat".into()]),
            "mcp_call"
        ));
    }
}
