pub use refact_chat_history::prompt_snippets::*;

pub const PYTHON_TOOL_INSTRUCTIONS: &str = r#"## Python tool (Code Mode)
`python` runs Python 3 in this chat's persistent session, with every other tool available as an async function. The runtime is Monty, a restricted Python: no third-party packages, no subprocess, no network, and only part of the standard library (the python tool description lists what is available). `os`, `re`, `json`, `Path` and `asyncio` are already imported.
Use it when the work is mechanical: counting, filtering, parsing or aggregating data, loops over many files, bulk edits, and independent tool calls that can run together. For such work prefer one python call over many separate tool calls or a subagent. For one quick action, call the tool directly.
- Call tools with keyword arguments and await them: %PYTHON_SINGLE_EXAMPLE%. Leave out optional arguments you do not need. %PYTHON_PARALLEL_EXAMPLE%.
- Files: `read(path, start, end)` returns numbered lines (`read(p, -40)` is the tail), `grep(pattern, path, context=2)` searches a file or a folder, `replace(path, old, new)` swaps text that occurs exactly once and returns the diff. These three are plain calls, no `await`.
- Git: `await git("status")`, `git("diff")`, `git("log", n=10)` and `git("show", rev, path)` return data. Use them instead of shell git when you only read.
%PYTHON_RUN_BULLET%- Pass `description` with every python call: one short sentence saying what the script does. The user sees it as the title of the run.
- Each call returns a dict: `ok`, `data` (the structured result; its keys are listed in the python tool description), `text` (what the tool would print), `files`, `diffs`, `images`, `error`, `truncated`. Check `r["ok"]` and read `r["data"]`.
- A call blocked by policy or rejected by the user raises PermissionError; bad arguments raise TypeError. A tool that fails returns `ok: false`.
%PYTHON_LLM_BULLET%%PYTHON_ENGINE_BULLET%- A list, a comparison or numbers for the user with more than about eight rows? `show(rows, title="...")` displays the table in the chat (charts with `kind=`, `x=`, `y=`) and you get back one line. Do this instead of writing a long table yourself.
- You get back the value of the last expression and the print() output, both truncated. Return compact summaries; keep large data in variables or files.
- Variables persist between python calls; `restart=true` clears them and keeps files.
- open() and pathlib work on real files; relative paths resolve against PROJECT_ROOT. Save outputs under ARTIFACTS_DIR; link user-facing outputs there as described in the Artifacts section. Full outputs of earlier runs are under RUNS_DIR. On Windows, convert paths with host_path()/virtual_path().
%PYTHON_READ_ONLY_BULLET%- The python tool description lists every function with its required arguments and the keys of `data`; `help("name")` gives the full signature and types; `tools()` lists the functions.
- A run longer than about 60 s continues in the background. Its result arrives in this chat by itself and starts your next turn, so do not poll for it; in chats whose tools always run in the foreground (task agents, supervised subagents) the call waits for the run to finish instead. Use run_in_background=true to start in the background on purpose. Only one run per chat at a time.
"#;

pub const PYTHON_LLM_BULLET: &str = "- More than about ten items to read and judge (files, log chunks, search hits)? Ask a model per item: `r = await llm(\"question\", schema={...})`, fanned out with `asyncio.gather`, and keep only the compact answers. No tools, no chat history, the cheap model by default; calls and tokens are budgeted per run.\n";

pub const PYTHON_ENGINE_BULLET: &str = "- `await engine(\"settings.get\")` reads and changes the engine itself: settings, this chat, models, tools, indexes, agents, usage and more. `help(\"engine\")` lists the areas and functions. Reads run freely; every change asks the user first.\n";

pub const PYTHON_RUN_BULLET: &str = "- Builds and tests: `r = await run(\"cargo test -p x\")` waits for the command and returns `exit_code`, `tail`, `problems` and the path of the full `log`. Use it instead of starting a process and polling it.\n";

pub const PYTHON_READ_ONLY_BULLET: &str = "- In this chat scripts cannot change files outside ARTIFACTS_DIR; tools called from a script follow their own rules.\n";

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
/// `engine()` bullet (`python_engine_read_enabled`); `run()` is described when `names` has the
/// tool it calls; `read_only` adds the line for chats whose scripts may write only artifacts.
pub fn python_tool_instructions(
    names: &std::collections::HashSet<String>,
    llm: bool,
    engine: bool,
    read_only: bool,
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
        .replace(
            "%PYTHON_RUN_BULLET%",
            if names.contains(refact_python_catalog::RUN_TOOL) {
                PYTHON_RUN_BULLET
            } else {
                ""
            },
        )
        .replace(
            "%PYTHON_READ_ONLY_BULLET%",
            if read_only {
                PYTHON_READ_ONLY_BULLET
            } else {
                ""
            },
        )
}

#[cfg(test)]
mod python_examples_tests {
    use super::*;
    use std::collections::HashSet;

    /// The section as it was before C24, with the examples, `llm()` and `engine()` bullets.
    const PREVIOUS_FULL_SECTION_CHARS: usize = 3377;

    #[test]
    fn examples_are_deterministic_and_have_generic_fallbacks() {
        let names = HashSet::from(["cat".into(), "glob".into(), "search_pattern".into()]);
        let instructions = python_tool_instructions(&names, false, false, false);
        assert!(instructions.contains("`r = await cat(paths=\"README.md\")`"));
        assert!(instructions.contains(
            "a, b = await asyncio.gather(cat(paths=\"README.md\"), glob(pattern=\"**/*.md\"))"
        ));
        assert!(!instructions.contains("%PYTHON_"));
        assert!(instructions.contains("lists every function with its required arguments and the keys of `data`; `help(\"name\")` gives the full signature and types"));
        let one = python_tool_instructions(&HashSet::from(["web".into()]), false, false, false);
        assert!(one.contains("`r = await web(url=\"https://example.com\")`"));
        assert!(one.contains("Run independent calls together with `await asyncio.gather(...)`"));
        let none = python_tool_instructions(&Default::default(), false, false, false);
        assert!(none.contains("`r = await tool_name(arg=value)`"));
        assert!(none.contains("in chats whose tools always run in the foreground (task agents, supervised subagents) the call waits for the run to finish instead"));
        assert!(!none.contains("task-agent chats"));
        assert!(none.contains("(the python tool description lists what is available). `os`, `re`, `json`, `Path` and `asyncio` are already imported.\n"));
        assert!(none.contains("Run independent calls together with `await asyncio.gather(...)`.\n- Files: `read(path, start, end)` returns numbered lines (`read(p, -40)` is the tail), `grep(pattern, path, context=2)` searches a file or a folder, `replace(path, old, new)` swaps text that occurs exactly once and returns the diff. These three are plain calls, no `await`.\n- Git: `await git(\"status\")`, `git(\"diff\")`, `git(\"log\", n=10)` and `git(\"show\", rev, path)` return data. Use them instead of shell git when you only read.\n- Pass `description` with every python call: one short sentence saying what the script does. The user sees it as the title of the run.\n- Each call returns a dict"));
        assert!(!none.contains("llm("));
        assert!(!none.contains("%PYTHON_LLM_BULLET%"));
        assert!(!none.contains("%PYTHON_ENGINE_BULLET%"));
        assert!(!none.contains("engine("));
        assert!(!none.contains("blame"));
    }
    #[test]
    fn llm_bullet_is_present_only_when_enabled() {
        let on = python_tool_instructions(&Default::default(), true, false, false);
        assert!(on.contains("rejected by the user raises PermissionError; bad arguments raise TypeError. A tool that fails returns `ok: false`.\n- More than about ten items to read and judge (files, log chunks, search hits)? Ask a model per item: `r = await llm(\"question\", schema={...})`, fanned out with `asyncio.gather`, and keep only the compact answers. No tools, no chat history, the cheap model by default; calls and tokens are budgeted per run.\n- A list, a comparison"));
        let off = python_tool_instructions(&Default::default(), false, false, false);
        assert_eq!(on.replace(PYTHON_LLM_BULLET, ""), off);
    }
    #[test]
    fn show_bullet_is_always_present() {
        for llm in [false, true] {
            let text = python_tool_instructions(&Default::default(), llm, false, false);
            assert!(text.contains("- A list, a comparison or numbers for the user with more than about eight rows? `show(rows, title=\"...\")` displays the table in the chat (charts with `kind=`, `x=`, `y=`) and you get back one line. Do this instead of writing a long table yourself.\n- You get back the value"), "{text}");
        }
    }
    #[test]
    fn engine_bullet_is_present_only_when_reads_are_enabled() {
        for llm in [false, true] {
            let on = python_tool_instructions(&Default::default(), llm, true, false);
            assert!(on.contains("- `await engine(\"settings.get\")` reads and changes the engine itself: settings, this chat, models, tools, indexes, agents, usage and more. `help(\"engine\")` lists the areas and functions. Reads run freely; every change asks the user first.\n- A list, a comparison"), "{on}");
            let off = python_tool_instructions(&Default::default(), llm, false, false);
            assert_eq!(on.replace(PYTHON_ENGINE_BULLET, ""), off);
            assert!(!off.contains("engine("));
        }
    }
    #[test]
    fn run_is_described_only_with_shell() {
        let with = python_tool_instructions(&HashSet::from(["shell".into()]), false, false, false);
        assert!(with.contains("return data. Use them instead of shell git when you only read.\n- Builds and tests: `r = await run(\"cargo test -p x\")` waits for the command and returns `exit_code`, `tail`, `problems` and the path of the full `log`. Use it instead of starting a process and polling it.\n- Pass `description`"), "{with}");
        let without = python_tool_instructions(&HashSet::from(["cat".into()]), false, false, false);
        assert_eq!(
            with.replace(PYTHON_RUN_BULLET, ""),
            without.replace("cat(paths=\"README.md\")", "tool_name(arg=value)")
        );
        assert!(!without.contains("run("));
        assert!(!without.contains("cargo"));
    }
    #[test]
    fn read_only_chats_get_the_artifacts_line_only() {
        let read_only = python_tool_instructions(&Default::default(), false, false, true);
        assert!(read_only.contains("Full outputs of earlier runs are under RUNS_DIR. On Windows, convert paths with host_path()/virtual_path().\n- In this chat scripts cannot change files outside ARTIFACTS_DIR; tools called from a script follow their own rules.\n- The python tool description lists"), "{read_only}");
        let writable = python_tool_instructions(&Default::default(), false, false, false);
        assert_eq!(read_only.replace(PYTHON_READ_ONLY_BULLET, ""), writable);
        assert!(!writable.contains("cannot change files"));
    }
    #[test]
    fn background_runs_are_not_polled() {
        let text = python_tool_instructions(&Default::default(), false, false, false);
        assert!(text.contains("- A run longer than about 60 s continues in the background. Its result arrives in this chat by itself and starts your next turn, so do not poll for it; in chats whose tools always run in the foreground (task agents, supervised subagents) the call waits for the run to finish instead. Use run_in_background=true to start in the background on purpose. Only one run per chat at a time.\n"));
        assert!(!text.contains("arrives later"));
    }
    #[test]
    fn the_section_grows_by_at_most_900_characters() {
        let names = HashSet::from(["cat".into(), "glob".into(), "shell".into()]);
        for read_only in [false, true] {
            let text = python_tool_instructions(&names, true, true, read_only);
            assert!(
                text.chars().count() <= PREVIOUS_FULL_SECTION_CHARS + 900,
                "{} characters",
                text.chars().count()
            );
        }
    }
}
