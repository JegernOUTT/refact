use super::catalog_binding::*;
use crate::tools::{tool_python::ToolPython, tools_description::Tool};
use serde_json::json;
fn descriptions() -> Vec<refact_tool_api::ToolDesc> {
    let mut fixture = ToolPython::default().tool_description();
    fixture.name = "fixture".into();
    fixture.description = "Full fixture description with details.".into();
    fixture.input_schema =
        json!({"type":"object","properties":{"n":{"type":"integer"}},"required":["n"]});
    vec![fixture, ToolPython::default().tool_description()]
}
#[test]
fn python_tool_description_contains_catalog_for_mode_tools() {
    let c = catalog(&descriptions(), false, 10000);
    assert!(c.description.lines().any(|l| l == "fixture(n)"));
    assert!(c
        .help_text("fixture")
        .unwrap()
        .contains("async def fixture(*, n: int) -> ToolResult[None]:"));
}
#[test]
fn catalog_description_identical_across_two_acquisitions() {
    let mut tools = descriptions();
    let a = catalog(&tools, false, 10000).description;
    tools.reverse();
    assert_eq!(a, catalog(&tools, false, 10000).description);
}
#[test]
fn help_returns_full_description() {
    assert!(catalog(&descriptions(), false, 10000)
        .help_text("fixture")
        .unwrap()
        .contains("Full fixture description with details."));
}
#[test]
fn python_is_not_listed_as_its_own_function() {
    assert!(catalog(&descriptions(), false, 10000)
        .resolve("python")
        .is_none());
}
#[test]
fn lazy_mcp_mode_exposes_mcp_helpers_not_typed_mcp_functions() {
    let c = catalog(&descriptions(), true, 10000);
    assert!(c.resolve("mcp_search").is_some());
    assert!(c.resolve("mcp_call").is_some());
}

#[tokio::test]
async fn python_tool_removed_when_disabled_for_project() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(root.join(".refact")).unwrap();
    std::fs::write(
        root.join(".refact/coordination.yaml"),
        "python_tool_enabled: false\n",
    )
    .unwrap();
    let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        dir.path().join("cache"),
        dir.path().join("config"),
    )
    .await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root];
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    let tools = app
        .tool_registry
        .get_tools_for_mode("agent", Some("model"))
        .await;
    assert!(!tools.iter().any(|t| t.name == "python"));
}

#[tokio::test]
async fn project_catalog_description_matches_executor_budget() {
    let (dir, app, _) = crate::tools::tool_python::tests::fixture().await;
    std::fs::write(
        dir.path().join("project/.refact/coordination.yaml"),
        "python_max_catalog_chars: 10000\n",
    )
    .unwrap();
    let app = crate::tools::tool_python::tests::with_tools(app);
    let tools = app
        .tool_registry
        .acquire_tool_catalog("agent", Some("model"), None)
        .await;
    let python = tools
        .index
        .tools
        .iter()
        .find(|tool| tool.name == "python")
        .unwrap();
    assert_eq!(
        python.description,
        format!(
            "{INTRO}{}",
            crate::python_tool::catalog_binding::catalog_with_options(
                &tools.index.tools,
                tools.index.mcp_lazy_mode,
                crate::python_tool::catalog_binding::BindOptions {
                    max_chars: 10000,
                    llm: true,
                    engine: true,
                },
            )
            .description
        )
    );
    assert!(python.description.contains("async def llm("));
    assert!(python
        .description
        .contains("async def engine(name: str, **kwargs)"));
}

#[tokio::test]
async fn real_mode_catalog_sizes_and_prompt_functions() {
    let (_dir, app, _) = crate::tools::tool_python::tests::fixture().await;
    for mode in [
        "agent",
        "ask",
        "buddy",
        "debug",
        "design",
        "explore",
        "plan",
        "review",
        "shell",
        "solo",
        "task_agent",
    ] {
        let tools = app
            .tool_registry
            .get_tools_for_mode(mode, Some("model"))
            .await;
        if !tools.iter().any(|t| t.name == "python") {
            continue;
        }
        let c = catalog(&tools, false, usize::MAX);
        let names: std::collections::HashSet<_> =
            tools.iter().map(|tool| tool.name.clone()).collect();
        let snippet =
            crate::chat::prompt_snippets::python_tool_instructions(&names, false, false, false);
        assert_eq!(
            snippet.contains("await run("),
            c.resolve("run").is_some(),
            "{mode}"
        );
        assert!(c.resolve("git").is_some(), "{mode}");
        for (name, call) in crate::chat::prompt_snippets::PYTHON_EXAMPLE_CALLS {
            if snippet.contains(call) {
                assert!(c.resolve(name).is_some(), "{mode} missing {name}");
                let args: serde_json::Value = match *name {
                    "cat" => json!({"paths":"README.md"}),
                    "glob" => json!({"pattern":"**/*.md"}),
                    "search_pattern" => json!({"pattern":"TODO","scope":"workspace"}),
                    "tree" => json!({}),
                    "knowledge" => json!({"search_key":"release process"}),
                    "web" => json!({"url":"https://example.com"}),
                    _ => unreachable!(),
                };
                refact_tool_api::schema_validate::validate(
                    &c.resolve(name).unwrap().input_schema,
                    &args,
                )
                .unwrap();
            }
        }
        eprintln!(
            "catalog size {mode}: {} characters",
            c.description.chars().count()
        );
        let bounded = catalog(&tools, false, 60000);
        eprintln!(
            "catalog size {mode} budgeted: {} characters",
            bounded.description.chars().count()
        );
        assert!(bounded.description.chars().count() <= 60000, "{mode}");
    }
}

fn codegraph_descriptions() -> Vec<refact_tool_api::ToolDesc> {
    use crate::tools::tool_codegraph::*;
    let tools: Vec<Box<dyn Tool + Send>> = vec![
        Box::new(ToolCodegraphOverview {
            config_path: String::new(),
        }),
        Box::new(ToolCodeHealth {
            config_path: String::new(),
        }),
        Box::new(ToolGitRisk {
            config_path: String::new(),
        }),
        Box::new(ToolCodeWhy {
            config_path: String::new(),
        }),
        Box::new(ToolCodeDuplication {
            config_path: String::new(),
        }),
        Box::new(ToolDeadCode {
            config_path: String::new(),
        }),
        Box::new(ToolSecurityScan {
            config_path: String::new(),
        }),
        Box::new(ToolPrBlast {
            config_path: String::new(),
        }),
        Box::new(ToolCodeMap {
            config_path: String::new(),
        }),
    ];
    tools.iter().map(|t| t.tool_description()).collect()
}

#[tokio::test]
async fn real_mode_python_description_sizes_are_bounded() {
    let (_dir, app, _) = crate::tools::tool_python::tests::fixture().await;
    let codegraph = codegraph_descriptions();
    assert_eq!(codegraph.len(), 9);
    let mut sizes = Vec::new();
    for (mode, bound, bound_with_codegraph) in [("agent", 16000, 18000), ("explore", 9500, 11000)] {
        let mut tools = app
            .tool_registry
            .get_tools_for_mode(mode, Some("model"))
            .await;
        assert!(tools.iter().any(|t| t.name == "python"), "{mode}");
        assert!(!tools.iter().any(|t| t.name == "code_map"), "{mode}");
        let size = |tools: &[refact_tool_api::ToolDesc]| {
            format!(
                "{INTRO}{}",
                catalog_with_llm(tools, false, 60000, true).description
            )
            .chars()
            .count()
        };
        let base = size(&tools);
        tools.extend(codegraph.clone());
        let full = size(&tools);
        eprintln!("python description {mode}: {base} characters, {full} with CodeGraph");
        sizes.push((mode, base, bound, full, bound_with_codegraph));
    }
    for (mode, base, bound, full, bound_with_codegraph) in sizes {
        assert!(base <= bound, "{mode}: {base}");
        assert!(full <= bound_with_codegraph, "{mode}: {full}");
    }
}

async fn kernel_eval(code: &str, names: &[&str]) -> serde_json::Value {
    use refact_python_kernel::{client::*, protocol::*, hostfs::HostFs};
    use std::time::Duration;
    let mut client =
        KernelClient::spawn_in_thread(Box::new(HostFs::default()), ClientOptions::default())
            .unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(30), async {
        client.next_event().await;
        client
            .send(KernelCommand::Feed {
                run_id: "probe".into(),
                code: code.into(),
                globals: names
                    .iter()
                    .map(|name| GlobalBinding {
                        name: (*name).into(),
                        kind: GlobalKind::AsyncFn,
                    })
                    .collect(),
                limits: RunLimits::default(),
            })
            .await
            .unwrap();
        loop {
            match client.next_event().await {
                ClientEvent::Kernel(KernelEvent::Completed {
                    value, ..
                }) => break value,
                ClientEvent::Kernel(KernelEvent::Raised {
                    ..
                }) => panic!("probe failed: {code}"),
                ClientEvent::Kernel(KernelEvent::SyncCall {
                    call_id,
                    function,
                    ..
                }) => {
                    // Mirrors the engine: unknown synchronous names are a TypeError.
                    client
                        .send(KernelCommand::SyncResult {
                            call_id,
                            result: HostResult::Err {
                                exc_type: "TypeError".into(),
                                message: format!("unknown function: {function}"),
                            },
                        })
                        .await
                        .unwrap();
                }
                ClientEvent::Kernel(KernelEvent::NeedResults {
                    pending, ..
                }) => {
                    client
                        .send(KernelCommand::Results {
                            run_id: "probe".into(),
                            results: pending
                                .into_iter()
                                .map(|call_id| {
                                    (call_id, HostResult::Ok(json!({"ok":true,"data":[]})))
                                })
                                .collect(),
                        })
                        .await
                        .unwrap();
                }
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    client.send(KernelCommand::Shutdown).await.unwrap();
    outcome
}

#[tokio::test]
async fn runtime_module_inventory_and_common_path_methods() {
    for (modules, available) in [
        (refact_python_catalog::AVAILABLE_MODULES, true),
        (refact_python_catalog::MISSING_MODULES, false),
    ] {
        for module in modules {
            let code = format!("try:\n    import {module}\n    imported = True\nexcept ImportError:\n    imported = False\nimported");
            assert_eq!(kernel_eval(&code, &[]).await, json!(available), "{module}");
        }
    }
    for expression in [
        "Path('/tmp/a').relative_to('/tmp')",
        "Path('/tmp/a.txt').with_suffix('.md')",
        "list(Path('/nonexistent-pycm-probe').rglob('*.txt'))",
        "list(os.walk('/nonexistent-pycm-probe'))",
        "os.path.join('a', 'b')",
        "os.path.relpath('/tmp/a', '/tmp')",
        "os.path.splitext('a.txt')",
    ] {
        let code = format!("from pathlib import Path\nimport os\ntry:\n    method = {expression}\n    present = True\nexcept AttributeError:\n    present = False\npresent");
        let present = kernel_eval(&code, &[]).await;
        eprintln!("runtime method {expression}: {present}");
        assert_eq!(
            present,
            json!(!expression.contains("relative_to")),
            "{expression}"
        );
    }
}

#[tokio::test]
async fn runtime_note_gaps_for_bound_methods_and_globals_hold() {
    let note = refact_python_catalog::runtime_note();
    assert!(note.contains("globals() is missing"));
    assert!(note.contains("key=lambda k: d[k]"));
    let probe = |expression: &str| {
        format!("d = {{'a': 1, 'b': 2}}\ntry:\n    value = {expression}\n    works = True\nexcept Exception:\n    works = False\nworks")
    };
    for (expression, works) in [
        ("max(d, key=d.get)", false),
        ("globals()", false),
        ("max(d, key=lambda k: d[k])", true),
        ("d.get('c', 0)", true),
    ] {
        assert_eq!(
            kernel_eval(&probe(expression), &[]).await,
            json!(works),
            "{expression}"
        );
    }
}

#[tokio::test]
async fn prompt_examples_execute_with_fixture_tools() {
    let result = kernel_eval("import asyncio\nr = await cat(paths=\"README.md\")\na, b = await asyncio.gather(cat(paths=\"README.md\"), glob(pattern=\"**/*.md\"))\n[r['ok'], a['ok'], b['ok']]", &["cat", "glob", "search_pattern"]).await;
    assert_eq!(result, json!([true, true, true]));
}

/// The description is built from the active project's settings; a chat worktree with other
/// settings still gets the described helpers bound, with its own budgets.
async fn run_in_worktree(project: &str, worktree: &str, code: &str) -> (bool, serde_json::Value) {
    let (dir, app, _) = crate::tools::tool_python::tests::fixture().await;
    let source = dir.path().join("project");
    std::fs::write(source.join(".refact/coordination.yaml"), project).unwrap();
    let root = dir.path().join("worktree");
    std::fs::create_dir_all(root.join(".refact")).unwrap();
    std::fs::write(root.join(".refact/coordination.yaml"), worktree).unwrap();
    app.chat.sessions.read().await["python-test"]
        .lock()
        .await
        .thread
        .worktree = Some(crate::worktrees::types::WorktreeMeta {
        id: "python-worktree".into(),
        kind: "chat".into(),
        root,
        source_workspace_root: source.clone(),
        repo_root: source,
        branch: Some("feature".into()),
        base_branch: Some("main".into()),
        base_commit: Some("base".into()),
        task_id: None,
        card_id: None,
        agent_id: None,
        enforce: true,
    });
    let app = crate::tools::tool_python::tests::with_tools(app);
    let described = app
        .tool_registry
        .acquire_tool_catalog("agent", Some("model"), None)
        .await
        .index
        .tools
        .iter()
        .find(|tool| tool.name == "python")
        .unwrap()
        .description
        .contains("async def llm(");
    let messages = crate::tools::tool_python::tests::execute(
        app,
        &[crate::tools::tool_python::tests::call("bind", code, false)],
    )
    .await;
    let python = messages[0].extra["python"].clone();
    let result = std::fs::read(
        std::path::Path::new(python["run_dir"].as_str().unwrap()).join("result.json"),
    )
    .map(|bytes| serde_json::from_slice(&bytes).unwrap())
    .unwrap_or_else(|_| panic!("{python}"));
    (described, result)
}

#[tokio::test]
async fn run_binds_the_helpers_the_description_advertises() {
    let probe = "try:\n    await llm('q')\n    out = 'called'\nexcept NameError as e:\n    out = 'NameError: ' + str(e)\nexcept RuntimeError as e:\n    out = 'RuntimeError: ' + str(e)\nout";
    let (described, result) = run_in_worktree("", "python_max_llm_calls: 0\n", probe).await;
    assert!(described);
    assert_eq!(
        result,
        "RuntimeError: llm() is switched off for this project (python_max_llm_calls is 0)"
    );
    let (described, result) = run_in_worktree("python_max_llm_calls: 0\n", "", probe).await;
    assert!(!described);
    assert!(
        result
            .as_str()
            .is_some_and(|text| text.starts_with("NameError") && text.contains("llm")),
        "{result}"
    );
}
