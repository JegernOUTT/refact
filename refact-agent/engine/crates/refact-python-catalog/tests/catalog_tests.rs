use refact_python_catalog::{build_catalog, CatalogOptions, NATIVE_ONLY_TOOLS};
use refact_tool_api::tool_desc::ToolDesc;
use serde_json::{json, Value};

fn tool(name: &str, input: Value, output: Option<Value>) -> ToolDesc {
    serde_json::from_value(json!({"name":name,"description":"Run a command. Full details follow.","input_schema":input,"output_schema":output,"display_name":name,"source":{"source_type":"builtin","config_path":""}})).unwrap()
}
fn fixture() -> Vec<ToolDesc> {
    vec![tool(
        "shell",
        json!({"type":"object","properties":{"command":{"type":"string","description":"Command to execute"},"description":{"type":"string"},"timeout":{"type":"integer"}},"required":["command","description"]}),
        Some(
            json!({"type":"object","properties":{"stdout":{"type":"string"}},"required":["stdout"]}),
        ),
    )]
}
fn catalog(ts: &[ToolDesc]) -> refact_python_catalog::PythonCatalog {
    let c = build_catalog(
        ts,
        &CatalogOptions {
            mcp_lazy: false,
            max_chars: usize::MAX,
            llm: false,
            engine: Vec::new(),
        },
    );
    plausible(&c.description);
    c
}
fn plausible(s: &str) {
    let mut brackets = Vec::new();
    let mut quote = None;
    let mut escaped = false;
    let mut comment = false;
    for c in s.chars() {
        if c == '\n' {
            comment = false;
        }
        if comment {
            continue;
        }
        if escaped {
            escaped = false;
            continue;
        }
        if let Some(q) = quote {
            if c == '\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        if c == '#' {
            comment = true;
            continue;
        }
        if c == '\'' || c == '"' {
            quote = Some(c);
            continue;
        }
        match c {
            '(' | '[' | '{' => brackets.push(c),
            ')' => assert_eq!(brackets.pop(), Some('(')),
            ']' => assert_eq!(brackets.pop(), Some('[')),
            '}' => assert_eq!(brackets.pop(), Some('{')),
            _ => {}
        }
    }
    assert!(brackets.is_empty());
    assert!(quote.is_none());
    for line in s
        .lines()
        .filter(|l| l.starts_with("async def ") || l.starts_with("def "))
    {
        let name = line
            .split("def ")
            .nth(1)
            .unwrap()
            .split('(')
            .next()
            .unwrap();
        assert!(![
            "class", "def", "return", "async", "await", "for", "if", "None", "True", "False", "in",
            "is", "not", "and", "or", "lambda", "from", "import", "pass", "yield", "try", "except",
            "finally", "with", "as", "del", "global", "nonlocal", "raise", "assert", "break",
            "continue", "else", "elif", "while"
        ]
        .contains(&name));
    }
}
#[test]
fn excludes_native_only_tools() {
    let ts: Vec<_> = NATIVE_ONLY_TOOLS
        .iter()
        .map(|n| tool(n, json!({}), None))
        .collect();
    assert!(catalog(&ts).functions.is_empty());
}
#[test]
fn sanitizes_and_deduplicates_names() {
    let ts: Vec<_> = ["a_b", "a-b", "1go", "class", "help", "PROJECT_ROOT"]
        .iter()
        .map(|n| tool(n, json!({}), None))
        .collect();
    let c = catalog(&ts);
    for (py, native) in [
        ("a_b", "a-b"),
        ("a_b_2", "a_b"),
        ("_1go", "1go"),
        ("class_", "class"),
        ("help_2", "help"),
        ("PROJECT_ROOT_2", "PROJECT_ROOT"),
    ] {
        assert_eq!(c.resolve(py).unwrap().tool_name, native);
    }
}
#[test]
fn default_import_names_are_reserved() {
    let ts: Vec<_> = ["asyncio", "os", "re", "json", "Path"]
        .iter()
        .map(|n| tool(n, json!({}), None))
        .collect();
    let c = catalog(&ts);
    for name in ["asyncio", "os", "re", "json", "Path"] {
        assert!(c.resolve(name).is_none(), "{name}");
        assert_eq!(c.resolve(&format!("{name}_2")).unwrap().tool_name, name);
    }
}
#[test]
fn renders_required_params_in_schema_order_then_ellipsis() {
    let c = catalog(&fixture());
    assert!(c
        .description
        .lines()
        .any(|l| l == "shell(command, description, …) -> {stdout}"));
    assert!(c.help_text("shell").unwrap().contains("async def shell(*, command: str, description: str, timeout: int | None = None) -> ToolResult[ShellOutput]:"));
    let only_optional = catalog(&[tool(
        "t",
        json!({"type":"object","properties":{"a":{"type":"string"}}}),
        None,
    )]);
    assert!(only_optional.description.lines().any(|l| l == "t(…)"));
    let none = catalog(&[tool("n", json!({"type":"object","properties":{}}), None)]);
    assert!(none.description.lines().any(|l| l == "n()"));
    let ordered = catalog(&[tool(
        "o",
        json!({"type":"object","properties":{"z":{"type":"string"},"a":{"type":"string"}},"required":["a","z"]}),
        None,
    )]);
    assert!(ordered.description.lines().any(|l| l == "o(z, a)"));
}
#[test]
fn renders_output_typeddict_with_defs_prefixed_by_tool() {
    let c = catalog(&[tool(
        "code_health",
        json!({}),
        Some(
            json!({"type":"object","properties":{"hit":{"$ref":"#/$defs/Hit"}},"$defs":{"Hit":{"type":"object","properties":{"line":{"type":"integer"}}}}}),
        ),
    )]);
    assert!(c
        .description
        .lines()
        .any(|l| l == "code_health() -> {hit?{line?}}"));
    let help = c.help_text("code_health").unwrap();
    assert!(help.contains("CodeHealth_Hit"));
    assert!(help.contains("class CodeHealthOutput(TypedDict)"));
    assert!(help.contains("Full details follow."));
    assert!(!c.description.contains("TypedDict"));
}
#[test]
fn non_identifier_properties_use_unpack_kwargs() {
    let c = catalog(&[tool(
        "odd",
        json!({"type":"object","properties":{"a-b":{"type":"string"},"class":{"type":"integer"}}}),
        None,
    )]);
    assert!(c.description.lines().any(|l| l == "odd(**kwargs)"));
    let help = c.help_text("odd").unwrap();
    assert!(help.contains("**kwargs: Unpack[OddArgs]"));
    assert!(help.contains("\"a-b\": NotRequired[str]"));
}
#[test]
fn description_is_byte_stable_for_shuffled_input() {
    let mut ts = fixture();
    ts.push(tool("z", json!({}), None));
    let a = catalog(&ts).description;
    ts.reverse();
    assert_eq!(a, catalog(&ts).description);
}
#[test]
fn budget_degradation_is_deterministic_and_respects_max_chars() {
    let ts: Vec<_> = (0..100)
        .map(|n| {
            tool(
                &format!("tool_{n}"),
                fixture()[0].input_schema.clone(),
                fixture()[0].output_schema.clone(),
            )
        })
        .collect();
    let full = catalog(&ts).description;
    eprintln!("100-tool catalog: {} characters", full.chars().count());
    for budget in [0, 1, 80, 1000, full.len() - 50, full.len() / 2] {
        let opts = CatalogOptions {
            mcp_lazy: false,
            max_chars: budget,
            llm: false,
            engine: Vec::new(),
        };
        let a = build_catalog(&ts, &opts).description;
        let mut rev = ts.clone();
        rev.reverse();
        assert_eq!(a, build_catalog(&rev, &opts).description);
        assert!(a.chars().count() <= budget);
        plausible(&a);
    }
}
#[test]
fn degradation_steps_apply_in_order() {
    let ts = vec![
        tool(
            "nested",
            json!({"type":"object","properties":{"q":{"type":"string"},"opt":{"type":"string"}},"required":["q"]}),
            Some(
                json!({"type":"object","required":["outer"],"properties":{"outer":{"type":"object","properties":{"inner":{"type":"string"}}}}}),
            ),
        ),
        tool("plain", json!({"type":"object","properties":{}}), None),
    ];
    let at = |max_chars: usize| {
        let opts = CatalogOptions {
            mcp_lazy: false,
            max_chars,
            llm: false,
            engine: Vec::new(),
        };
        let a = build_catalog(&ts, &opts).description;
        let mut rev = ts.clone();
        rev.reverse();
        assert_eq!(a, build_catalog(&rev, &opts).description);
        assert!(a.chars().count() <= max_chars);
        a
    };
    let size = |s: &str| s.chars().count();
    let full = at(usize::MAX);
    assert!(
        full.contains("\nnested(q, …) -> {outer{inner?}}\n"),
        "{full}"
    );
    let top = at(size(&full) - 1);
    assert!(top.contains("\nnested(q, …) -> {outer}\n"), "{top}");
    let no_shape = at(size(&top) - 1);
    assert!(no_shape.contains("\nnested(q, …)\nplain()\n"), "{no_shape}");
    let no_params = at(size(&no_shape) - 1);
    assert!(no_params.contains("\nnested(…)\nplain()\n"), "{no_params}");
    assert_eq!(
        at(size(&no_params) - 1),
        "# Catalog omitted to fit budget; use help(name) and tools().\n"
    );
    assert_eq!(at(10), "");
}

#[test]
fn lazy_mcp_adds_mcp_search_and_mcp_call_helpers() {
    let c = build_catalog(
        &[],
        &CatalogOptions {
            mcp_lazy: true,
            max_chars: usize::MAX,
            llm: false,
            engine: Vec::new(),
        },
    );
    assert!(c
        .helpers
        .iter()
        .any(|f| f.py_name == "mcp_search" && f.is_async));
    assert!(c
        .helpers
        .iter()
        .any(|f| f.py_name == "mcp_call" && f.is_async));
    assert!(c
        .description
        .contains("async def mcp_search(*, query: str) -> ToolResult[Any]: ..."));
    assert!(c.description.contains(
        "async def mcp_call(*, tool_name: str, args: dict[str, Any]) -> ToolResult[Any]: ..."
    ));
    plausible(&c.description);
}
#[test]
fn envelope_matches_result_envelope_contract() {
    let description = catalog(&[]).description;
    assert!(description.contains("# ToolResult = {ok: bool, data: the tool's structured result or None, text: str, files: list[{path: str, line1: int, line2: int, content: str}], diffs: list[dict], images: list[{image: str, index: int}], images_dropped: int, error: str | None, truncated: bool}."));
}

#[test]
fn only_parent_artifacts_constant_is_nullable() {
    let description = catalog(&[]).description;
    assert!(description.contains("# Constants: PROJECT_ROOT, CHAT_ID, WORKSPACE_DIR, ARTIFACTS_DIR, RUNS_DIR, RUN_ID, RUN_DIR: str; PARENT_ARTIFACTS_DIR: str | None (None without a parent"));
}

#[test]
fn block_order_and_helper_signatures() {
    let c = build_catalog(
        &fixture(),
        &CatalogOptions {
            mcp_lazy: true,
            max_chars: usize::MAX,
            llm: true,
            engine: Vec::new(),
        },
    );
    let d = &c.description;
    let pos = |needle: &str| d.find(needle).unwrap_or_else(|| panic!("{needle}\n{d}"));
    let order = [
        pos("# Await every tool call"),
        pos("# ToolResult = "),
        pos("# Constants: "),
        pos("def help(name: str | None = None) -> str: ..."),
        pos("def tools() -> list[str]: ..."),
        pos("def host_path(p: str) -> str: ..."),
        pos("def virtual_path(p: str) -> str: ..."),
        pos("async def git(subcommand: Literal["),
        pos("async def run(cmd: str, *,"),
        pos("async def llm(prompt: str, *,"),
        pos("def show(value: Any, *,"),
        pos("async def mcp_search("),
        pos("# Monty restricted Python."),
        pos("# Tool functions are async"),
        pos("\nshell(command, description, …) -> {stdout}\n"),
    ];
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{d}");
    for rule in [
        "take keyword arguments",
        "same arguments as the direct tool of the same name",
        "Each returns ToolResult",
        "… means optional ones exist",
        "After -> come the keys of data",
        "help(\"name\") prints the full typed signature and data types",
    ] {
        assert!(d.contains(rule), "{rule}");
    }
    assert!(refact_python_catalog::runtime_note()
        .contains("A script can import its own .py files from PROJECT_ROOT (no relative imports, no sys.path)."));
    assert!(refact_python_catalog::runtime_note()
        .contains("paths are not orderable (sort with key=str)"));
}

#[test]
fn renamed_functions_name_their_tool() {
    let c = catalog(&[
        tool("a-b", json!({"type":"object","properties":{}}), None),
        tool("plain", json!({"type":"object","properties":{}}), None),
    ]);
    assert!(c.description.lines().any(|l| l == "a_b()  # tool \"a-b\""));
    assert!(c.description.lines().any(|l| l == "plain()"));
}

fn shape_of(output: Value) -> String {
    let c = catalog(&[tool(
        "f",
        json!({"type":"object","properties":{}}),
        Some(output),
    )]);
    let line = c
        .description
        .lines()
        .find(|l| l.starts_with("f("))
        .unwrap()
        .to_string();
    line.strip_prefix("f() -> ").unwrap_or("").to_string()
}

#[test]
fn shapes_follow_the_output_schema() {
    assert_eq!(
        shape_of(json!({
            "type":"object",
            "required":["a","n","items","rows","map","nested","any","one","maybe"],
            "properties":{
                "a":{"type":"string"},
                "n":{"type":["string","null"]},
                "opt":{"type":"integer"},
                "items":{"type":"array","items":{"type":"string"}},
                "rows":{"type":"array","items":{"$ref":"#/$defs/Row"}},
                "map":{"type":"object","additionalProperties":{"type":"integer"}},
                "nested":{"type":"object","required":["x"],"properties":{"x":{"type":"object","properties":{"deep":{"type":"object","properties":{"z":{"type":"integer"}}}}}}},
                "any":{"anyOf":[{"type":"string"},{"type":"integer"}]},
                "one":{"oneOf":[{"$ref":"#/$defs/Row"},{"type":"null"}]},
                "maybe":{"type":"string","nullable":true}
            },
            "$defs":{"Row":{"type":"object","required":["id"],"properties":{"id":{"type":"integer"},"tags":{"type":"array","items":{"type":"string"}}}}}
        })),
        "{a, n?, opt?, items[], rows[{id, tags?[]}], map{*}, nested{x{…}}, any, one?{id, tags?[]}, maybe?}"
    );
    assert_eq!(
        shape_of(json!({"type":"object","additionalProperties":{"type":"string"}})),
        "{*}"
    );
    assert_eq!(
        shape_of(
            json!({"type":"array","items":{"type":"object","properties":{"k":{"type":"string"}},"required":["k"]}})
        ),
        "[{k}]"
    );
    assert_eq!(shape_of(json!({"type":"object","properties":{}})), "{}");
    assert_eq!(
        shape_of(json!({"type":"object","properties":{"self":{"$ref":"#"}}})),
        "{self?{self?{…}}}"
    );
}

#[test]
fn long_shapes_fall_back_to_top_level_keys_then_cut() {
    let nested = json!({"type":"object","properties":{"inner_field_one":{"type":"string"},"inner_field_two":{"type":"string"},"inner_field_three":{"type":"string"}}});
    let mut props = serde_json::Map::new();
    for i in 0..5 {
        props.insert(format!("key{i}"), nested.clone());
    }
    let top = shape_of(json!({"type":"object","properties":props}));
    assert_eq!(top, "{key0?, key1?, key2?, key3?, key4?}");
    let mut many = serde_json::Map::new();
    for i in 0..40 {
        many.insert(
            format!("a_rather_long_key_{i:02}"),
            json!({"type":"string"}),
        );
    }
    let cut = shape_of(json!({"type":"object","properties":many}));
    assert!(cut.chars().count() <= 160, "{cut}");
    assert!(cut.ends_with(", …}"), "{cut}");
    assert!(
        cut.starts_with("{a_rather_long_key_00?, a_rather_long_key_01?"),
        "{cut}"
    );
    let none = catalog(&[tool("f", json!({}), None)]);
    assert!(none.description.lines().any(|l| l == "f()"));
}

#[test]
fn golden_description_for_fixture_catalog() {
    let actual = catalog(&fixture()).description;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/golden_description.txt");
    if std::env::var_os("UPDATE_CATALOG_GOLDEN").is_some() {
        std::fs::write(&path, &actual).unwrap();
    }
    let expected = std::fs::read_to_string(path).unwrap().replace("\r\n", "\n");
    assert_eq!(actual,expected,"Regenerate with UPDATE_CATALOG_GOLDEN=1 cargo test -p refact-python-catalog golden_description_for_fixture_catalog");
}

#[test]
fn schema_types_cover_unions_enums_maps_and_unknowns() {
    let c = catalog(&[tool(
        "types",
        json!({"type":"object","properties":{
        "choice":{"enum":["a","b"]},
        "nullable":{"anyOf":[{"type":"integer"},{"type":"null"}]},
        "legacy_nullable":{"enum":["a"],"nullable":true},
        "mapping":{"type":"object","additionalProperties":{"type":"number"}},
        "rows":{"type":"array","items":{"type":"boolean"}},
        "unsupported":{"allOf":[{"type":"string"}]},
        "unknown_ref":{"$ref":"https://invalid/schema"}
    },"required":["choice","nullable","legacy_nullable","mapping","rows","unsupported","unknown_ref"]}),
        None,
    )]);
    for expected in [
        "choice: Literal[\"a\", \"b\"]",
        "nullable: int | None",
        "legacy_nullable: Literal[\"a\"] | None",
        "mapping: dict[str, Any]",
        "rows: list[bool]",
        "unsupported: Any",
        "unknown_ref: Any",
    ] {
        assert!(
            c.help_text("types").unwrap().contains(expected),
            "{expected}\n{}",
            c.help_text("types").unwrap()
        );
    }
}

#[test]
fn generated_types_do_not_shadow_tools_or_shared_types() {
    let c = catalog(&[
        tool(
            "file_chunk",
            json!({}),
            Some(json!({"type":"object","properties":{"x":{"type":"string"}}})),
        ),
        tool("FileChunkOutput", json!({}), None),
        tool(
            "a",
            json!({"type":"object","properties":{"v":{"$ref":"#/$defs/Hit"}},"$defs":{"Hit":{"type":"object","properties":{"a":{"type":"integer"}}}}}),
            Some(
                json!({"type":"object","properties":{"v":{"$ref":"#/$defs/Hit"}},"$defs":{"Hit":{"type":"object","properties":{"b":{"type":"string"}}}}}),
            ),
        ),
    ]);
    assert!(c
        .help_text("file_chunk")
        .unwrap()
        .contains("class FileChunkOutput_2(TypedDict)"));
    assert!(c.help_text("a").unwrap().contains("A_Hit"));
}

#[test]
fn nested_input_objects_are_plain_dicts() {
    let c = catalog(&[tool(
        "nested",
        json!({"type":"object","properties":{"items":{"type":"array","items":{"type":"object","properties":{"x":{"type":"integer"}}}},"obj":{"type":"object","properties":{"x":{"type":"integer"}}}},"required":["items","obj"]}),
        None,
    )]);
    let help = c.help_text("nested").unwrap();
    assert!(help.contains("items: list[dict[str, Any]]"));
    assert!(help.contains("obj: dict[str, Any]"));
    assert!(!help.contains("class Nested"));
    assert!(help.contains("\"x\""));
}

#[test]
fn help_has_the_full_typed_stub_with_parameter_docs() {
    let c = catalog(&fixture());
    let help = c.help_text("shell").unwrap();
    assert!(help.starts_with("class ShellOutput(TypedDict):\n    stdout: str\n\n# \"command\": Command to execute\nasync def shell(*, command: str, description: str, timeout: int | None = None) -> ToolResult[ShellOutput]:\n    \"Run a command.\"\n    ...\n"), "{help}");
    assert!(help.contains("Run a command. Full details follow."));
    assert!(help.contains("Input schema: {"));
    assert!(!c.description.contains("Command to execute"));
    assert!(c.help_text("show").unwrap().contains("one-line note"));
    assert!(c.help_text("missing").is_none());
}

#[test]
fn help_index_lists_functions_within_budget() {
    let ts: Vec<_> = (0..50)
        .map(|n| tool(&format!("tool_{n:02}"), json!({}), None))
        .collect();
    let c = catalog(&ts);
    let all = c.help_index(usize::MAX);
    assert!(all.contains("tool_00: Run a command.\n"));
    assert!(all.contains("tool_49: Run a command.\n"));
    assert!(all.contains("show: "));
    assert!(!all.contains("more; tools()"));
    let small = c.help_index(400);
    assert!(small.chars().count() <= 400, "{small}");
    assert!(
        small.ends_with("more; tools() lists every name.\n"),
        "{small}"
    );
}
#[test]
fn llm_helper_is_bound_and_described_only_when_enabled() {
    let off = catalog(&[]);
    assert!(off.resolve("llm").is_none());
    assert!(!off.description.contains("llm("));
    let on = build_catalog(
        &[],
        &CatalogOptions {
            mcp_lazy: false,
            max_chars: usize::MAX,
            llm: true,
            engine: Vec::new(),
        },
    );
    let helper = on.resolve("llm").unwrap();
    assert!(helper.is_async);
    assert!(on.description.contains("async def llm(prompt: str, *, system: str | None = None, schema: dict[str, Any] | None = None, model: Literal[\"light\", \"default\", \"thinking\", \"chat\"] | None = None, max_tokens: int | None = None) -> LlmResult: ..."));
    assert!(on.description.contains("LlmResult = {ok: bool, text: str, data: Any | None, model: str, usage: {input_tokens: int, output_tokens: int}, error: str | None}"));
    let help = on.help_text("llm").unwrap();
    assert!(help.contains("no tools, no chat history"));
    assert!(help.contains("PermissionError"));
    assert!(help.contains("8192 when omitted, capped at the model's output limit"));
    let tool = tool("llm", json!({"type":"object","properties":{}}), None);
    let shadow = build_catalog(
        &[tool],
        &CatalogOptions {
            mcp_lazy: false,
            max_chars: usize::MAX,
            llm: true,
            engine: Vec::new(),
        },
    );
    assert!(shadow.functions.iter().all(|f| f.py_name != "llm"));
    plausible(&on.description);
}

#[test]
fn show_helper_is_always_bound_described_and_reserved() {
    let c = catalog(&[]);
    let helper = c.resolve("show").unwrap();
    assert!(!helper.is_async);
    assert!(c.description.contains("def show(value: Any, *, kind: Literal[\"table\", \"markdown\", \"bar\", \"line\", \"area\", \"scatter\", \"pie\"] | None = None, title: str | None = None, x: str | None = None, y: str | list[str] | None = None, columns: list[str] | None = None) -> None: ..."));
    let help = c.help_text("show").unwrap();
    for needle in [
        "list of dicts",
        "dict of lists",
        "pie",
        "10000 rows",
        "200 columns",
        "TypeError",
        "one-line note",
    ] {
        assert!(help.contains(needle), "{needle}: {help}");
    }
    let shadow = catalog(&[tool("show", json!({"type":"object","properties":{}}), None)]);
    assert!(shadow.functions.iter().all(|f| f.py_name != "show"));
    plausible(&c.description);
}

fn engine_docs() -> Vec<refact_python_catalog::EngineFunctionDoc> {
    vec![
        refact_python_catalog::EngineFunctionDoc {
            name: "settings.get".into(),
            area: "settings".into(),
            class: "read".into(),
            summary: "Read runtime and coordination settings.".into(),
            input_schema: json!({"type":"object","additionalProperties":false,"properties":{"keys":{"type":"array","items":{"type":"string"},"description":"Keys to read; all when absent."},"scope":{"enum":["effective","global","project"]}}}),
            output_schema: Some(
                json!({"type":"object","properties":{"settings":{"type":"array"}}}),
            ),
        },
        refact_python_catalog::EngineFunctionDoc {
            name: "settings.set".into(),
            area: "settings".into(),
            class: "change".into(),
            summary: "Change settings after the user approves.".into(),
            input_schema: json!({"type":"object","additionalProperties":false,"required":["values"],"properties":{"values":{"type":"object"},"scope":{"enum":["project","global"]}}}),
            output_schema: None,
        },
        refact_python_catalog::EngineFunctionDoc {
            name: "system.info".into(),
            area: "system".into(),
            class: "redacted_read".into(),
            summary: "Version, ports and directories.".into(),
            input_schema: json!({"type":"object","properties":{}}),
            output_schema: None,
        },
    ]
}

#[test]
fn engine_is_bound_described_and_helped_only_with_docs() {
    let off = catalog(&[]);
    assert!(off.resolve("engine").is_none());
    assert!(!off.description.contains("engine("));
    assert!(off.help_text("engine").is_none());
    let on = build_catalog(
        &[tool(
            "engine",
            json!({"type":"object","properties":{}}),
            None,
        )],
        &CatalogOptions {
            mcp_lazy: false,
            max_chars: usize::MAX,
            llm: false,
            engine: engine_docs(),
        },
    );
    let helper = on.resolve("engine").unwrap();
    assert!(helper.is_async);
    assert_eq!(helper.tool_name, "engine");
    assert!(on.functions.iter().all(|f| f.py_name != "engine"));
    assert!(on.description.contains("engine_2()  # tool \"engine\""));
    let engine_lines: Vec<_> = on
        .description
        .lines()
        .filter(|l| l.contains("engine") && !l.contains("# tool"))
        .collect();
    assert_eq!(engine_lines.len(), 2, "{engine_lines:?}");
    assert!(on
        .description
        .contains("async def engine(name: str, **kwargs) -> ToolResult[Any]: ..."));
    assert!(on
        .description
        .contains("# engine areas: settings, system; help(\"engine\") lists their functions\n"));
    assert!(!on.description.contains("settings.get"));
    let areas = on.help_text("engine").unwrap();
    assert!(areas.contains("settings: get, set"), "{areas}");
    assert!(areas.contains("system: info"), "{areas}");
    let area = on.help_text("engine.settings").unwrap();
    assert!(
        area.contains("settings.get [read]: Read runtime and coordination settings."),
        "{area}"
    );
    assert!(
        area.contains("settings.set [change]: Change settings after the user approves."),
        "{area}"
    );
    assert!(!area.contains("system.info"));
    let function = on.help_text("engine.settings.get").unwrap();
    assert!(
        function.contains("await engine(\"settings.get\", keys=None, scope=None)"),
        "{function}"
    );
    assert!(function.contains("Class: read"), "{function}");
    assert!(function.contains("keys: list[str] | None"), "{function}");
    assert!(
        function.contains("Keys to read; all when absent."),
        "{function}"
    );
    assert!(function.contains("Output (data):"), "{function}");
    let set = on.help_text("engine.settings.set").unwrap();
    assert!(
        set.contains("await engine(\"settings.set\", values=..., scope=None)"),
        "{set}"
    );
    assert!(set.contains("asks the user"), "{set}");
    assert!(on.help_text("engine.nope").is_none());
    assert!(on.help_text("engine.settings.nope").is_none());
    assert!(on.help_index(100_000).contains("engine: "));
    plausible(&on.description);
}

#[test]
fn file_helpers_are_bound_described_and_reserved() {
    let c = catalog(&[]);
    for name in ["read", "grep", "replace"] {
        let helper = c.resolve(name).unwrap();
        assert!(!helper.is_async, "{name}");
        assert_eq!(helper.tool_name, name);
    }
    assert_eq!(
        refact_python_catalog::FILE_HELPERS,
        &["read", "grep", "replace"]
    );
    for stub in [
        "def read(path: str, start: int = 1, end: int | None = None, *, numbered: bool = True) -> str: ...  # lines start..end of a text file (1-based, inclusive; start=-N: last N lines) as \"  12 | text\"; numbered=False: the exact text\n",
        "def grep(pattern: str, path: str = \".\", *, context: int = 0, glob: str | None = None, ignore_case: bool = False, limit: int = 200, data: bool = False) -> str | dict: ...  # regex search in a file or directory (search_pattern syntax and ignore rules): \"path:line: text\" lines\n",
        "def replace(path: str, old: str, new: str, *, count: int = 1) -> str: ...  # exact-text edit; ValueError unless exactly count matches (0: all, at least one); returns a unified diff\n",
    ] {
        assert!(c.description.contains(stub), "{stub}");
    }
    for (name, needles) in [
        (
            "read",
            &[
                "  12 | text",
                "read(p, -40)",
                "ValueError",
                "UnicodeDecodeError",
                "os.chdir()",
                "files_read",
            ][..],
        ),
        (
            "grep",
            &[
                "Rust regex",
                "privacy",
                "files_skipped",
                "truncated",
                "20,000",
                "256 MiB",
                "without matches",
            ][..],
        ),
        (
            "replace",
            &[
                "exactly",
                "\"aa\" occurs once in \"aaa\"",
                "60 lines",
                "hard link",
                "read-only",
                "in a read-only chat a symlinked path is refused",
                "one without file-editing, rm, mv, shell or process_start tools",
                "unsaved editor buffers",
                "undo_textdoc",
            ][..],
        ),
    ] {
        let help = c.help_text(name).unwrap();
        for needle in needles {
            assert!(help.contains(needle), "{name}: {needle}\n{help}");
        }
        assert!(help.contains("Input schema:"), "{name}");
    }
    let index = c.help_index(usize::MAX);
    for line in [
        "read: Read lines of a text file with line numbers.\n",
        "grep: Search files for a regex.\n",
        "replace: Replace exact text in a file and return the diff.\n",
    ] {
        assert!(index.contains(line), "{line}");
    }
    let shadow = catalog(
        &["read", "grep", "replace"]
            .map(|n| tool(n, json!({"type":"object","properties":{}}), None)),
    );
    for name in ["read", "grep", "replace"] {
        assert_eq!(
            shadow.resolve(&format!("{name}_2")).unwrap().tool_name,
            name
        );
        assert!(shadow.functions.iter().all(|f| f.py_name != name));
    }
    plausible(&c.description);
}

#[test]
fn git_helper_is_always_bound_described_and_reserved() {
    let c = catalog(&[]);
    let helper = c.resolve("git").unwrap();
    assert!(helper.is_async);
    assert_eq!(helper.tool_name, "git");
    assert_eq!(refact_python_catalog::GIT_HELPER, "git");
    assert_eq!(
        refact_python_catalog::GIT_POSITIONAL,
        &["subcommand", "#1", "#2"]
    );
    assert!(c.description.contains("async def git(subcommand: Literal[\"status\", \"diff\", \"log\", \"show\", \"branches\"], *args, **kwargs) -> ToolResult[dict]: ...  # read-only git data, no shell needed: git(\"status\"), git(\"diff\", rev=None, path=None, staged=False), git(\"log\", n=20, rev=None, path=None), git(\"show\", rev, path=None), git(\"branches\")\n"));
    assert!(!c.description.contains("blame"));
    let help = c.help_text("git").unwrap();
    for needle in [
        "repository that holds PROJECT_ROOT",
        "workspace folders",
        "runs no git program",
        "read-only chats",
        "\"A...B\": the merge base of A and B to B",
        "withheld_count",
        "Untracked files and submodules are left out",
        "Blob (X:path), index, reflog (@{...}) and peel (^{...}) forms",
        "300 files per diff",
        "512 KiB per call",
        "PermissionError",
        "privacy records",
        "renames are detected only among at most 200 added and deleted files",
        "log reads at most 50,000 commits (scan_truncated)",
        "branches: [{name, is_head, upstream, tip_oid, ahead, behind, upstream_gone}], remote_branches: [{name, remote, short_name, tip_oid}], tags: [{name, target_oid}]",
        "at most 1,000 branches, 1,000 remote branches and 1,000 tags, the first by name (truncated)",
        "A call over a very large change set can take long; Stop is honoured between files.",
        "on each side of a range too",
        "listed with link=True and has no patch and no text; its target is never returned",
        "naming it as path in diff, log or show raises PermissionError, whether or not it exists",
        "with path, only commits that changed a file under it that git() may show",
        "checked as written and as the file it resolves to now through links; both must pass",
        "which says so when the repository is shallow",
        "borrows objects from another one (objects/info/alternates) is not read",
    ] {
        assert!(help.contains(needle), "{needle}: {help}");
    }
    assert!(!help.contains("blame"));
    assert!(c
        .help_index(usize::MAX)
        .contains("git: Read git status, diffs, history, files at a revision and branches.\n"));
    let shadow = catalog(&[tool("git", json!({"type":"object","properties":{}}), None)]);
    assert_eq!(shadow.resolve("git_2").unwrap().tool_name, "git");
    assert_eq!(shadow.resolve("git").unwrap().tool_name, "git");
    assert!(!shadow.resolve("git").unwrap().input_schema["properties"]
        .as_object()
        .unwrap()
        .is_empty());
    plausible(&c.description);
}

#[test]
fn run_helper_is_bound_and_described_only_with_shell() {
    let without = catalog(&[tool("cat", json!({"type":"object","properties":{}}), None)]);
    assert!(without.resolve("run").is_none());
    assert!(!without.description.contains("def run("));
    assert!(without.help_text("run").is_none());
    let with = catalog(&fixture());
    let helper = with.resolve("run").unwrap();
    assert!(helper.is_async);
    assert_eq!(helper.tool_name, "run");
    assert_eq!(refact_python_catalog::RUN_TOOL, "shell");
    assert_eq!(refact_python_catalog::RUN_POSITIONAL, &["cmd"]);
    assert!(with.description.contains("async def run(cmd: str, *, timeout: int | None = None, cwd: str | None = None, tail: int = 60) -> RunResult: ...  # runs a build or test command to its end through the shell tool (same approvals and rules). RunResult = {ok: bool, exit_code: int | None, status: str, duration_s: float, tail: str, problems: list[str], log: str | None, truncated: bool, error: str | None}\n"));
    let help = with.help_text("run").unwrap();
    for needle in [
        "same approvals, hooks and rules as calling shell",
        "instead of starting a process and polling it",
        "RUN_DIR/logs",
        "truncated is True",
        "\"withheld\"",
        "background one makes run() raise RuntimeError",
        "warnings are left out",
        "pass a timeout",
    ] {
        assert!(help.contains(needle), "{needle}: {help}");
    }
    let shadow = catalog(&[
        tool("run", json!({"type":"object","properties":{}}), None),
        tool("shell", json!({"type":"object","properties":{}}), None),
    ]);
    assert_eq!(shadow.resolve("run_2").unwrap().tool_name, "run");
    assert_eq!(shadow.resolve("run").unwrap().tool_name, "run");
    let unbound = catalog(&[tool("run", json!({"type":"object","properties":{}}), None)]);
    assert!(unbound.resolve("run").is_none());
    assert_eq!(unbound.resolve("run_2").unwrap().tool_name, "run");
    plausible(&with.description);
}

#[test]
fn the_pre_imported_modules_are_named() {
    assert!(refact_python_catalog::runtime_note()
        .contains("os, re, json, Path and asyncio are imported before every run."));
    assert!(catalog(&[])
        .description
        .contains("os, re, json, Path and asyncio are imported before every run."));
}

#[test]
fn the_io_module_is_listed_with_what_it_lacks() {
    assert!(refact_python_catalog::AVAILABLE_MODULES.contains(&"io"));
    let note = refact_python_catalog::runtime_note();
    assert!(
        note.contains(
            "io has StringIO, BytesIO and open only (no TextIOWrapper; newline=None does not translate)."
        ),
        "{note}"
    );
}
