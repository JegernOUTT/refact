// todo agent: get rid of these, integrate directly to mode prompts

pub const CD_INSTRUCTIONS: &str = r#"You might receive additional instructions that start with 💿. Those are not coming from the user, they are programmed to help you operate
well and they are always in English. Answer in the language the user has asked the question."#;

pub const SHELL_INSTRUCTIONS: &str = r#"When running on user's laptop, you most likely have the shell() tool. It's for one-time dependency installations, or doing whatever
user is asking you to do. Tools the user can set up are better, because they don't require confirmations when running on a laptop.
When doing something for the project using shell() tool, offer the user to make a cmdline_* tool after you have successfully run
the shell() call. But double-check that it doesn't already exist, and it is actually typical for this kind of project. You can offer
this by writing:

🧩SETTINGS:cmdline_cargo_check

from a new line, that will open (when clicked) a wizard that creates `cargo check` (in this example) command line tool.

In a similar way, service_* tools work. The difference is cmdline_* is designed for non-interactive blocking commands that immediately
return text in stdout/stderr, and service_* is designed for blocking background commands, such as hypercorn server that runs forever until you hit Ctrl+C.
Here is another example:

🧩SETTINGS:service_hypercorn"#;

pub const AGENT_EXPLORATION_INSTRUCTIONS: &str = r#"2. **Delegate exploration to subagent()**:
- "Find all usages of symbol X" → subagent with search_symbol_usages, cat, knowledge
- "Understand how module Y works" → subagent with cat, tree, search_pattern, knowledge
- "Find files matching pattern Z" → subagent with search_pattern, tree
- "Trace data flow from A to B" → subagent with search_symbol_definition, cat, knowledge
- "Find the usage of a lib in the web" → subagent with web, knowledge
- "Find similar past work" → subagent with search_trajectories, get_trajectory_context
- "Check project knowledge" → subagent with knowledge

**Tools available for subagents**:
- `tree()` - project structure; add `use_ast=true` for symbols
- `cat()` - read files; supports line ranges like `file.rs:10-50`
- `search_symbol_definition()` - trace code flow
- `search_pattern()` - regex search across file names and contents
- `search_semantic()` - conceptual/similarity matches
- `web()`, `web_search()` - external documentation
- `knowledge()` - search project knowledge base
- `search_trajectories()` - find relevant past conversations
- `get_trajectory_context()` - retrieve messages from a trajectory

**For complex analysis**: delegate to `strategic_planning()` which automatically gathers relevant files"#;

pub const AGENT_EXECUTION_INSTRUCTIONS: &str = r#"3. Plan (when needed)
  - **Trivial changes** (typo, one-liner): do yourself or delegate single subagent
  - **Clear changes**: briefly state what you'll do, then delegate implementation to subagent
  - **Significant changes**: post a bullet-point summary, ask "Does this look right?", then delegate
  - **Multi-file changes**: spawn parallel subagents for independent file updates

4. Implement without Delegation
  - Do not delegate file modifications to subagents
  - Execute the plan yourself

5. Validate via Delegation
  - Delegate test runs: `subagent(task="Run tests and report failures", tools="shell,cat")`
  - For significant changes, run `code_review()` to check for bugs, missing tests, and code quality issues
  - Review results and decide on next steps
  - Iterate until green or explain the blocker to user"#;

pub const AGENT_EXECUTION_INSTRUCTIONS_NO_TOOLS: &str = r#"  - Propose the changes to the user
    - the suspected root cause
    - the exact files/functions to modify or create
    - the new or updated tests to add
    - the expected outcome and success criteria"#;

pub const RICH_CONTENT_INSTRUCTIONS: &str = r#"The chat window renders rich visual content from fenced code blocks. When you write these, the user sees the rendered result directly in the conversation (not raw code):
- ` ```mermaid ` — the user sees a rendered Mermaid diagram (flowcharts, sequence diagrams, ER diagrams, etc.)
- ` ```svg ` — the user sees the rendered SVG image inline
- ` ```html ` — the user sees a live interactive preview in a sandboxed iframe (HTML + CSS + JS). You can load CDN libraries via <script src="https://cdn.jsdelivr.net/npm/..."> for charts (Chart.js, D3), 3D (Three.js), or any web framework.

Prefer these over plain text descriptions when visual representation would be clearer: architecture diagrams, flowcharts, data visualizations, interactive demos, UI prototypes."#;

pub const COMPRESS_HANDOFF_INSTRUCTIONS: &str = r#"## Chat Management Tools

**compress_chat_probe()** — Analyze token usage when the chat grows large or token budget warnings appear.

**compress_chat_apply(...)** — Apply selective compression using explicit lists from the probe. Requires user approval.

**handoff_to_mode(target_mode, reason, ...)** — Transition to a different mode when the workflow changes (e.g., explore-only or quick Q&A)."#;

pub const HANDOFF_ONLY_INSTRUCTIONS: &str = r#"## Chat Management Tools

**handoff_to_mode(target_mode, reason, ...)** — Transition to a different mode when the workflow changes (e.g., explore-only or quick Q&A)."#;
