pub use refact_chat_history::prompt_snippets::*;

pub const PYTHON_TOOL_INSTRUCTIONS: &str = r#"## Python tool (Code Mode)
`python` runs Python 3 in this chat's persistent session, with every other tool available as an async function. The runtime is Monty, a restricted Python: no third-party packages, no subprocess, no network, and only part of the standard library (the python tool description lists what is available).
Use it when the work is mechanical: counting, filtering, parsing or aggregating data, loops over many files, bulk edits, and independent tool calls that can run together. For such work prefer one python call over many separate tool calls or a subagent. For one quick action, call the tool directly.
- Call tools with keyword arguments and await them: %PYTHON_SINGLE_EXAMPLE%. Leave out optional arguments you do not need. %PYTHON_PARALLEL_EXAMPLE% (`asyncio` is already imported).
- Pass `description` with every python call: one short sentence saying what the script does. The user sees it as the title of the run.
- Each call returns a dict: `ok`, `data` (the structured result; its keys are listed in the python tool description), `text` (what the tool would print), `files`, `diffs`, `images`, `error`, `truncated`. Check `r["ok"]` and read `r["data"]`.
- A call blocked by policy or rejected by the user raises PermissionError; bad arguments raise TypeError. A tool that fails returns `ok: false`.
%PYTHON_LLM_BULLET%%PYTHON_ENGINE_BULLET%- `show(rows, title="...")` displays a table to the user in the chat without spending your context; `kind="bar"`, `"line"`, `"area"`, `"scatter"` or `"pie"` with `x=` and `y=` draws a chart, and a string is shown as markdown. You get back only a one-line note, so use it for results the user should see in full.
- You get back the value of the last expression and the print() output, both truncated. Return compact summaries; keep large data in variables or files.
- Variables persist between python calls; `restart=true` clears them and keeps files.
- open() and pathlib work on real files; relative paths resolve against PROJECT_ROOT. Save outputs under ARTIFACTS_DIR; link user-facing outputs there as described in the Artifacts section. Full outputs of earlier runs are under RUNS_DIR. On Windows, convert paths with host_path()/virtual_path().
- The python tool description lists every function with its required arguments and the keys of `data`; `help("name")` gives the full signature and types; `tools()` lists the functions.
- A run longer than about 60 s continues in the background and its result arrives later; in chats whose tools always run in the foreground (task agents, supervised subagents) the call waits for the run to finish instead. Use run_in_background=true to start in the background on purpose. Only one run per chat at a time.
"#;

pub const PYTHON_LLM_BULLET: &str = "- `r = await llm(\"question\", schema={...})` asks a model one question from inside a script: no tools, no chat history, the cheap model by default. Read `r[\"text\"]`, or `r[\"data\"]` when you pass a JSON schema. Use it with `asyncio.gather` to classify, extract or summarise many items and return only the compact result. Calls and tokens are budgeted per run.\n";

pub const PYTHON_ENGINE_BULLET: &str = "- `await engine(\"settings.get\")` reads and changes the engine itself: settings, this chat, models, tools, indexes, agents, usage and more. `help(\"engine\")` lists the areas and functions. Reads run freely; every change asks the user first.\n";

pub const PYTHON_EXAMPLE_CALLS: &[(&str, &str)] = &[
    ("cat", "cat(paths=\"README.md\")"),
    ("glob", "glob(pattern=\"**/*.md\")"),
    (
        "search_pattern",
        "search_pattern(pattern=\"TODO\", scope=\"workspace\")",
    ),
    ("tree", "tree()"),
    ("knowledge", "knowledge(search_key=\"release process\")"),
    ("web", "web(url=\"https://example.com\")"),
];

/// `llm` adds the `llm()` helper bullet (`python_max_llm_calls` above 0); `engine` adds the
/// `engine()` bullet (`python_engine_read_enabled`).
pub fn python_tool_instructions(
    names: &std::collections::HashSet<String>,
    llm: bool,
    engine: bool,
) -> String {
    let calls: Vec<_> = PYTHON_EXAMPLE_CALLS
        .iter()
        .filter(|(name, _)| names.contains(*name))
        .collect();
    let single = format!(
        "`r = await {}`",
        calls
            .first()
            .map_or("tool_name(arg=value)", |(_, call)| *call)
    );
    let parallel = if calls.len() >= 2 {
        format!(
            "Run independent calls together: `a, b = await asyncio.gather({}, {})`",
            calls[0].1, calls[1].1
        )
    } else {
        "Run independent calls together with `await asyncio.gather(...)`".into()
    };
    PYTHON_TOOL_INSTRUCTIONS
        .replace("%PYTHON_SINGLE_EXAMPLE%", &single)
        .replace("%PYTHON_PARALLEL_EXAMPLE%", &parallel)
        .replace(
            "%PYTHON_LLM_BULLET%",
            if llm { PYTHON_LLM_BULLET } else { "" },
        )
        .replace(
            "%PYTHON_ENGINE_BULLET%",
            if engine { PYTHON_ENGINE_BULLET } else { "" },
        )
}

#[cfg(test)]
mod python_examples_tests {
    use super::*;
    #[test]
    fn examples_are_deterministic_and_have_generic_fallbacks() {
        let names =
            std::collections::HashSet::from(["cat".into(), "glob".into(), "search_pattern".into()]);
        let instructions = python_tool_instructions(&names, false, false);
        assert!(instructions.contains("`r = await cat(paths=\"README.md\")`"));
        assert!(instructions.contains(
            "a, b = await asyncio.gather(cat(paths=\"README.md\"), glob(pattern=\"**/*.md\"))"
        ));
        assert!(!instructions.contains("%PYTHON_"));
        assert!(instructions.contains("lists every function with its required arguments and the keys of `data`; `help(\"name\")` gives the full signature and types"));
        let one = python_tool_instructions(
            &std::collections::HashSet::from(["web".into()]),
            false,
            false,
        );
        assert!(one.contains("`r = await web(url=\"https://example.com\")`"));
        assert!(one.contains("Run independent calls together with `await asyncio.gather(...)`"));
        let none = python_tool_instructions(&Default::default(), false, false);
        assert!(none.contains("`r = await tool_name(arg=value)`"));
        assert!(none.contains("in chats whose tools always run in the foreground (task agents, supervised subagents) the call waits for the run to finish instead"));
        assert!(!none.contains("task-agent chats"));
        assert!(none.contains("(`asyncio` is already imported).\n- Pass `description` with every python call: one short sentence saying what the script does. The user sees it as the title of the run.\n- Each call returns a dict"));
        assert!(!none.contains("llm("));
        assert!(!none.contains("%PYTHON_LLM_BULLET%"));
        assert!(!none.contains("%PYTHON_ENGINE_BULLET%"));
        assert!(!none.contains("engine("));
    }
    #[test]
    fn llm_bullet_is_present_only_when_enabled() {
        let on = python_tool_instructions(&Default::default(), true, false);
        assert!(on.contains("rejected by the user raises PermissionError; bad arguments raise TypeError. A tool that fails returns `ok: false`.\n- `r = await llm(\"question\", schema={...})` asks a model one question from inside a script: no tools, no chat history, the cheap model by default. Read `r[\"text\"]`, or `r[\"data\"]` when you pass a JSON schema. Use it with `asyncio.gather` to classify, extract or summarise many items and return only the compact result. Calls and tokens are budgeted per run.\n- `show(rows"));
        let off = python_tool_instructions(&Default::default(), false, false);
        assert_eq!(on.replace(PYTHON_LLM_BULLET, ""), off);
    }
    #[test]
    fn show_bullet_is_always_present() {
        for llm in [false, true] {
            let text = python_tool_instructions(&Default::default(), llm, false);
            assert!(text.contains("- `show(rows, title=\"...\")` displays a table to the user in the chat without spending your context; `kind=\"bar\"`, `\"line\"`, `\"area\"`, `\"scatter\"` or `\"pie\"` with `x=` and `y=` draws a chart, and a string is shown as markdown. You get back only a one-line note, so use it for results the user should see in full.\n- You get back the value"), "{text}");
        }
    }
    #[test]
    fn engine_bullet_is_present_only_when_reads_are_enabled() {
        for llm in [false, true] {
            let on = python_tool_instructions(&Default::default(), llm, true);
            assert!(on.contains("- `await engine(\"settings.get\")` reads and changes the engine itself: settings, this chat, models, tools, indexes, agents, usage and more. `help(\"engine\")` lists the areas and functions. Reads run freely; every change asks the user first.\n- `show(rows"), "{on}");
            let off = python_tool_instructions(&Default::default(), llm, false);
            assert_eq!(on.replace(PYTHON_ENGINE_BULLET, ""), off);
            assert!(!off.contains("engine("));
        }
    }
}
