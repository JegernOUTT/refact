#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    async fn mode_tool_names(mode: &str, codegraph_on: bool) -> HashSet<String> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all(
            &gcx.config_dir,
        )
        .await
        .unwrap();
        if codegraph_on {
            *gcx.codegraph.lock().await = Some(std::sync::Arc::new(
                refact_codegraph::CodeGraphService::open_in_memory().unwrap(),
            ));
        }
        crate::tools::tools_list::get_tools_for_mode(gcx, mode, None)
            .await
            .into_iter()
            .map(|tool| tool.tool_description().name)
            .collect()
    }

    #[tokio::test]
    async fn codegraph_tools_are_mode_gated_by_codegraph() {
        let matrix: &[(&str, &[&str])] = &[
            (
                "agent",
                &[
                    "search_symbol_definition",
                    "codegraph_overview",
                    "code_health",
                    "git_risk",
                    "code_why",
                    "code_duplication",
                    "dead_code",
                    "security_scan",
                    "pr_blast",
                    "code_map",
                ],
            ),
            (
                "ask",
                &[
                    "codegraph_overview",
                    "code_health",
                    "git_risk",
                    "code_why",
                    "code_duplication",
                    "dead_code",
                    "security_scan",
                    "pr_blast",
                    "code_map",
                ],
            ),
            (
                "explore",
                &[
                    "search_symbol_definition",
                    "codegraph_overview",
                    "code_health",
                    "git_risk",
                    "code_why",
                    "code_duplication",
                    "dead_code",
                    "security_scan",
                    "pr_blast",
                    "code_map",
                ],
            ),
            (
                "brainstorming",
                &[
                    "search_symbol_definition",
                    "codegraph_overview",
                    "code_health",
                    "git_risk",
                    "code_why",
                    "code_duplication",
                    "dead_code",
                    "security_scan",
                    "pr_blast",
                    "code_map",
                ],
            ),
            (
                "debug",
                &[
                    "search_symbol_definition",
                    "codegraph_overview",
                    "code_health",
                    "git_risk",
                    "code_why",
                    "code_duplication",
                    "dead_code",
                    "security_scan",
                    "pr_blast",
                    "code_map",
                ],
            ),
            (
                "learn",
                &[
                    "search_symbol_definition",
                    "codegraph_overview",
                    "code_health",
                    "git_risk",
                    "code_why",
                    "code_duplication",
                    "dead_code",
                    "security_scan",
                    "pr_blast",
                    "code_map",
                ],
            ),
            (
                "plan",
                &[
                    "search_symbol_definition",
                    "codegraph_overview",
                    "code_health",
                    "git_risk",
                    "code_why",
                    "code_duplication",
                    "dead_code",
                    "security_scan",
                    "pr_blast",
                    "code_map",
                ],
            ),
            (
                "task_agent",
                &[
                    "search_symbol_definition",
                    "codegraph_overview",
                    "code_health",
                    "git_risk",
                    "code_why",
                    "code_duplication",
                    "dead_code",
                    "security_scan",
                    "pr_blast",
                    "code_map",
                ],
            ),
        ];

        for (mode, expected) in matrix {
            let without_codegraph = mode_tool_names(mode, false).await;
            for name in *expected {
                assert!(
                    !without_codegraph.contains(*name),
                    "{name} must be hidden in {mode} when codegraph is off: {without_codegraph:?}"
                );
            }

            let with_codegraph = mode_tool_names(mode, true).await;
            for name in *expected {
                assert!(
                    with_codegraph.contains(*name),
                    "{name} must be available in {mode} when codegraph is on: {with_codegraph:?}"
                );
            }
        }
    }
}
