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
- "Find all usages of symbol X" → subagent with search_symbol_definition, search_pattern, cat, knowledge
- "Understand how module Y works" → subagent with cat, tree, search_pattern, knowledge
- "Find files matching pattern Z" → subagent with glob, tree
- "Trace data flow from A to B" → subagent with search_symbol_definition, cat, knowledge
- "Find the usage of a lib in the web" → subagent with web, knowledge
- "Find similar past work" → subagent with search_trajectories, get_trajectory_context
- "Check project knowledge" → subagent with knowledge

**Tools available for subagents**:
- `tree()` - project structure; add `use_ast=true` for symbols
- `cat()` - read files; supports line ranges like `file.rs:10-50`
- `glob()` - find files by path pattern
- `search_symbol_definition()` - trace code flow
- `search_pattern()` - regex search inside file contents
- `web()`, `web_search()` - external documentation
- `knowledge()` - search project knowledge base
- `search_trajectories()` - find relevant past conversations
- `get_trajectory_context()` - retrieve messages from a trajectory"#;

pub const CODEGRAPH_INSTRUCTIONS: &str = r#"## CodeGraph Tools

Use `tree`, `cat`, `search_pattern`, and `search_symbol_definition` for exact files, text, and symbols. Use CodeGraph for project-wide or cross-file questions:
- `codegraph_overview` / `code_map` — architecture, entry points, modules, and important files.
- `code_health` / `security_scan` — one file; pass paths in the same workspace-relative, absolute, or active-worktree forms accepted by `cat`.
- `git_risk` / `code_why` — history, ownership, hotspots, and implementation rationale.
- `code_duplication` / `dead_code` — project-wide clone and reachability candidates.
- `pr_blast` — likely downstream impact of changed files.

Treat index-readiness warnings as partial results and corroborate important conclusions with `cat` or search tools."#;

pub const AGENT_EXECUTION_INSTRUCTIONS: &str = r#"3. Plan or Execute a Plan
  - **No plan yet**: for creative or ambiguous work, switch to Brainstorm/Plan before editing.
  - **Plan exists**: treat it as ground truth. Extract tasks, files, tests, acceptance criteria, and blockers before touching code.
  - **Plan is incomplete**: ask questions or switch back to Plan mode instead of inventing missing requirements.
  - **Significant changes**: present a short execution summary and ask for confirmation before the first edit.

4. Execute with Plan Discipline
  - For small/trivial changes, implement directly and verify.
  - For plan-based work, execute task-by-task; mark progress with `tasks_set` and keep one active task at a time.
  - Use subagents for focused investigation, test runs, spec-compliance review, and code-quality review.
  - When delegating implementation work, provide exact task text, files, constraints, allowed side effects, verification command, and expected status report. Never dispatch multiple editing subagents against overlapping files.
  - If a subagent asks for context, answer or re-dispatch with better context. If it reports blocked, change the approach, split the task, or ask the user.

5. Validate and Review
  - Run targeted verification after each meaningful task.
  - For plan-based work, check spec compliance before code quality: first confirm the change matches the plan, then review maintainability.
  - For significant changes, run independent adversarial reviews with `subagent()` using different concrete models on the same pinned diff and explicit domains; challenge findings once, adjudicate by evidence rather than majority, and report unchecked areas.
  - Iterate until checks pass or the blocker is evidenced and clearly reported."#;

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

pub const HANDOFF_ONLY_INSTRUCTIONS: &str = r#"## Chat Management Tools

**handoff_to_mode(target_mode, reason, ...)** — Transition to a different mode when the workflow changes (e.g., brainstorming/design, implementation planning, task-board execution, explore-only, or quick Q&A). When handing a complete plan to Task Planner, pass the full plan in `initial_plan`."#;

pub const ARTIFACTS_INSTRUCTIONS_FULL: &str = r#"## Artifacts for the user

Artifacts are files you create only for the user to view, read or download: reports, HTML pages, charts and images, CSV or JSON exports, rendered diagrams, generated documents. Save every artifact in this chat's artifacts folder, never in the project tree:
`{ARTIFACTS_DIR}`

- Write artifacts with your file tools using absolute paths inside this folder, or with Python, where `ARTIFACTS_DIR` is the same folder. File tools create missing folders automatically; in Python, create missing parent folders with `Path(...).mkdir(parents=True, exist_ok=True)`.
- Source code, tests, configs, repository documentation, AGENTS.md, skills, knowledge and task documents are project files, not artifacts: keep them in their usual project locations. Copy an artifact into the project only when the user asks.
- Create an artifact only when a file serves the user better than an inline answer: large or downloadable results, standalone reports, data exports, images. Show small visuals inline with mermaid, svg or html fenced blocks instead. Do not create summary documents nobody asked for.
- Use short descriptive kebab-case names with the correct extension and only letters, digits, `-`, `_`, `.` and `/` (for example `q3-sales-report.html`, `charts/latency-p95.png`). Group multi-file results in a subfolder.
- When the user asks for changes, update the existing artifact in place. Write a new versioned file such as `q3-sales-report-v2.html` only when the user wants to keep the previous version.
- Make HTML artifacts self-contained: inline all CSS and JavaScript and embed images as data URIs. The preview blocks network requests and does not load neighbouring files.
- Delete temporary or intermediate files you created in this folder. Keep artifacts the user has already seen unless they ask you to remove them. The folder is deleted together with this chat.
- Link every artifact you create or update in your reply: `[Short title](refact://artifact/{CHAT_ID}/<path inside the artifacts folder>)`, for example `[Q3 sales report](refact://artifact/{CHAT_ID}/q3-sales-report.html)`. A link on its own line shows a preview card, a link inside a sentence shows a compact file chip, and the user opens the file by clicking it. Never link artifacts by absolute path or `file://` URL.
- When a subagent or tool reports artifact links, keep them unchanged in your reply so the user can open them.
"#;

pub const ARTIFACTS_INSTRUCTIONS_READ_ONLY: &str = r#"## Artifacts for the user

Files created for the user in this chat are stored in its artifacts folder: `{ARTIFACTS_DIR}`. You cannot create files in this mode. Link an existing artifact as `[Short title](refact://artifact/{CHAT_ID}/<path inside the artifacts folder>)` on its own line to show a preview card the user can open by clicking. When a subagent or tool reports artifact links, keep them unchanged in your reply.
"#;

pub const ARTIFACTS_INSTRUCTIONS_UNAVAILABLE: &str = r#"## Artifacts for the user

This chat has no artifacts folder, so give results inline instead of creating files for the user. When a subagent or tool reports artifact links, keep them unchanged in your reply.
"#;
