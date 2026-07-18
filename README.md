<div align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="media/logo-dark.svg">
    <source media="(prefers-color-scheme: light)" srcset="media/logo-light.svg">
    <img alt="Refact logo" src="media/logo-light.svg" width="220">
  </picture>

  <h1>Refact</h1>

  <p><strong>Refact — the open-source, local-first agentic coding engine.</strong> A standalone <code>refact</code> binary runs the resident daemon, warm project workers, TUI, browser GUI, LSP, IDE clients, autonomous agents, task planning, and memory — all on your machine.</p>

  <img src="media/hero.png#gh-light-mode-only" alt="Refact agent planning, editing, and running checks" width="900">
  <img src="media/hero-dark.png#gh-dark-mode-only" alt="Refact agent planning, editing, and running checks" width="900">

  <p>
    <a href="https://github.com/JegernOUTT/refact/stargazers"><img src="https://img.shields.io/github/stars/JegernOUTT/refact?style=for-the-badge&color=blue" alt="GitHub stars"></a>
    <a href="https://github.com/JegernOUTT/refact/issues"><img src="https://img.shields.io/github/issues/JegernOUTT/refact?style=for-the-badge" alt="GitHub issues"></a>
    <a href="https://github.com/JegernOUTT/refact/blob/main/LICENSE"><img src="https://img.shields.io/github/license/JegernOUTT/refact?style=for-the-badge" alt="License"></a>
    <a href="https://github.com/JegernOUTT/refact/wiki"><img src="https://img.shields.io/badge/Documentation-Wiki-2ea44f?style=for-the-badge" alt="Documentation"></a>
  </p>
</div>

**10+ agent modes · 50+ tools · 20+ provider families · worktree-isolated agent fleets · MCP · skills · subagents · BYOK · 100% local — zero cloud**

## Install

**Unix/macOS**

```sh
curl -fsSL https://raw.githubusercontent.com/JegernOUTT/refact/main/install.sh | sh
refact ui
```

**Windows PowerShell**

```powershell
irm https://raw.githubusercontent.com/JegernOUTT/refact/main/install.ps1 | iex
refact ui
```

> [!WARNING]
> The resident **daemon** and **TUI** are an early preview and may be buggy or change without notice. The IDE plugins (VS Code/JetBrains) and core engine are the stable surfaces. Please report issues.

Refact is a local-first agentic coding engine shipped as a standalone `refact` binary. Start it once and a resident daemon keeps the control plane warm: one daemon supervises project workers, serves the in-browser GUI, brokers the full-screen TUI, exposes LSP/HTTP for editor clients, and gives autonomous tool-using agents the same concrete surfaces you use every day — files, shells, browser automation, AST search, patches, checkpoints, integrations, and verification loops. It is not just an IDE sidebar: the task planner can break big work into cards, launch fleets of task agents in isolated git worktrees, and let Buddy keep a nosy little watch over project memory, diagnostics, docs, dependencies, and opportunities without handing your repo to a hosted control plane.

What makes Refact different is ownership. You bring the models and keys, choose hosted or local providers, wire MCP, skills, hooks, and subagents into your own workflows, and keep every byte of project state under `<project>/.refact/` where it can be inspected, backed up, deleted, or versioned on your terms. The same daemon-backed stack is steerable and hackable from any client: modes are YAML, plans are durable, memory is project-scoped, and the Rust engine serves a browser-reachable UI from your machine — powerful enough for multi-agent coding runs, transparent enough to debug when the gremlin gets spicy.

## Autonomous agents & tools

Refact is not a one-shot autocomplete box; it is a live agent loop that can inspect a repo, change it, run the result, read the fallout, and try again. The fun part: the same brain that streams an answer can pause for permission, fan out tool calls, preserve signed thinking blocks, and keep working like a tiny dev team in a trench coat.

- Real session runtime: chats move through `Idle → Generating → ExecutingTools → WaitingUserInput/Paused → Completed/Error`, so a request can become an iterative edit-test-debug cycle instead of a single reply.
- Search stack with teeth: `tree`, `cat`, regex search, AST `symbol_def`, semantic search, trajectory/knowledge lookup, and project memory let the agent triangulate code by path, structure, and meaning.
- Editing is first-class and guarded: `apply_patch`, `create_textdoc`, `update_textdoc*`, `mv`, `rm`, and `undo_textdoc` produce concrete workspace mutations, with confirmation gates and limited auto-approval for patch-like edits.
- Runtime tools close the loop: `shell`, `process_start`, `process_read`, `process_wait`, `process_kill`, PTY `process_write_stdin`, `sleep`, and `cron_*` let agents run commands, monitor services, drive interactive processes, and schedule follow-ups.
- Delegation is built in: `subagent`, `delegate`, `strategic_planning`, `deep_research`, and `code_review` can split exploration, review, and implementation work across specialized agent passes.
- Web and browser reach: web fetch/search plus Chrome automation tools can inspect external docs, reproduce UI flows, and bring live evidence back into the chat loop.
- Streaming stays structured: content, reasoning, thinking blocks, citations, server content blocks, and tool calls are merged as typed deltas; compatible tools can run in parallel and report back without flattening the transcript into mush.

| Need       | Tool families                                              |
| ---------- | ---------------------------------------------------------- |
| Understand | `tree`, `cat`, regex, AST, semantic search, knowledge      |
| Change     | `apply_patch`, `create/update_textdoc`, `mv`, `rm`, `undo` |
| Prove      | `shell`, `process_*`, PTY stdin, browser, cron             |

→ Deep dive: [Agent Modes](https://github.com/JegernOUTT/refact/wiki/Agent-Modes), [Agent Tools](https://github.com/JegernOUTT/refact/wiki/Agent-Tools)

## Buddy — your proactive project companion

![Buddy proactively surfacing opportunities](media/buddy.png#gh-light-mode-only)
![Buddy proactively surfacing opportunities](media/buddy-dark.png#gh-dark-mode-only)
Buddy is the little project gremlin that keeps watching while you step away: it reads real project state, spots decision-ready opportunities, and comes back with a nudge plus the next safe move. It can investigate, draft, navigate, and escalate — but stays suggest-first instead of silently mutating your code.

- Runs named proactive workflows including Daily Digest, Idle Suggester, Test Coverage Watcher, Dependency Radar, Docs Gardener, Architecture Drift Watcher, Error Detective, and Security Whisperer.
- Watches practical signals such as stuck tasks, default model gaps, MCP auth trouble, memory clutter, git pressure, diagnostics clusters, dependency risk, documentation drift, and coverage gaps.
- Launches focused investigation chats when a signal deserves deeper analysis, keeping the main thread calm while the gremlin does the digging.
- Opens relevant Buddy and project views so you can jump straight from an opportunity to the page that explains or resolves it.
- Creates editable drafts for skills, commands, subagents, modes, AGENTS.md updates, and defaults instead of forcing changes straight into the tree.
- Files GitHub issues for confirmed product bugs, after gathering enough diagnostic context to avoid noisy drive-by reports.
- Speaks through the GUI as opportunities, speech bubbles, recent error panels, autonomous chat history, and reviewable draft cards.
- Stays guarded by default: Buddy mode has `auto_approve_editing_tools: false` and `auto_approve_dangerous_commands: false`, so code changes and risky commands remain under your control.

→ Deep dive: [Buddy](https://github.com/JegernOUTT/refact/wiki/Buddy)

## Task planner & agent fleets

![Task planner board with per-card agents](media/agent-task-planner.png#gh-light-mode-only)
![Task planner board with per-card agents](media/agent-task-planner-dark.png#gh-dark-mode-only)
Describe a feature once; the planner cracks it into cards, launches a tiny swarm, and keeps every agent in its own git worktree so experiments can collide with reality without colliding with each other. Watch the board light up as sandboxed agents build, test, report, race, and get squash-merged back in dependency order.

- Planner chats turn broad goals into executable cards with target files, priorities, assumptions, follow-ups, and structured final reports instead of one mega-prompt trying to hold the whole quest in its paws.
- The board is real Kanban: `planned`, `doing`, `done`, `failed`, and `regressed` columns, plus `depends_on` edges that separate ready cards from blocked cards so the fleet runs in the right order.
- Each spawned card agent gets its own branch and isolated worktree under the worktree registry; edits, tests, crashes, and spicy gremlin detours stay quarantined until the planner chooses to merge.
- Agents self-verify before finishing, then a verifier can re-check the card, capture command results, concerns, and recommendations, and leave a planner-visible report on the card.
- The planner owns the merge loop: inspect the agent diff, merge or squash the branch, run post-merge checks, auto-mark regressions, and preserve dirty worktrees when a run needs inspection or rescue.
- A/B card racing lets the planner spawn two variants for one card, compare their worktrees, then pick the winner instead of arguing with vibes in a comment thread.
- Live steering is built in: pause, resume, cancel, restart fresh or resume from a retained worktree, broadcast guidance, answer agent questions, and inspect pulses for state, last activity, tool calls, edits, and blockers.
- The payoff is ridiculous in the best way: describe the feature, watch a fleet of sandboxed agents build and test the pieces in parallel, then let the planner merge the clean winners into one coherent change.

→ Deep dive: [Task Planner and Cards](https://github.com/JegernOUTT/refact/wiki/Task-Planner-and-Cards), [Worktrees](https://github.com/JegernOUTT/refact/wiki/Worktrees)

## Persistent project memory

![Knowledge save and autoinjection](media/memory.png#gh-light-mode-only)
![Knowledge save and autoinjection](media/memory-dark.png#gh-dark-mode-only)
Refact turns your repo into a local memory palace: plans, notes, trajectories, code semantics, and background signals live under `<project>/.refact/`, ready for the next session without leaving your machine.
Static plans are the uncrushable spine—`set_plan` pins the base, `update_plan` appends deltas, and `get_plan` synthesizes the current truth while compression is forbidden from erasing it.

- **Local by design:** project knowledge, trajectories, task memory, integrations, schedules, and VecDB state are scoped to `.refact/`, with global memory only added explicitly through the configured knowledge directory.
- **Plans that survive the squeeze:** hidden `plan` messages and `event(plan_delta)` updates are marked `Never` in chat-history compression, so long agent runs can trim noise without eating the mission briefing.
- **Graph + vector recall:** a petgraph knowledge graph links documents to tags, file refs, entities, links, and supersession edges, while SQLite + vec0 embeddings index code, Markdown, and trajectory chunks for semantic search.
- **Autoinjected context, not paste spam:** @-commands and memory injection turn recall into `context_file` messages, then token-aware postprocessing resolves paths, AST-marks useful regions, and keeps only the sharpest context slices.
- **Typed operational memory:** decisions, specs, gotchas, risks, handoffs, progress notes, postmortems, briefs, and task memories become searchable project artifacts instead of disappearing into chat scrollback.
- **Trajectories become fuel:** saved conversations under `.refact/trajectories/` are chunked into metadata plus message windows, making past tool results and decisions discoverable without replaying the whole transcript.
- **Background indexers keep watch:** AST, VecDB, knowledge cleanup, and Buddy memory observers refresh the map while respecting shutdown and project scope, so future chats start warm instead of blank.

→ Deep dive: [Memory and Knowledge](https://github.com/JegernOUTT/refact/wiki/Memory-and-Knowledge), [Hidden Roles and Plans](https://github.com/JegernOUTT/refact/wiki/Hidden-Roles-and-Plans)

## Local-first & endlessly customizable — zero cloud

Refact runs where your code already lives: on your machine, pointed only at the models, runtimes, and integrations you choose. No bundled cloud is required, no account is mandatory, and local/BYOK usage keeps the whole AI stack inspectable, forkable, and yours.

- The engine runs as local `refact` daemon and worker processes; network traffic is limited to configured providers and integrations, from hosted APIs to local Ollama/LM Studio-style runtimes.
- Project state is plain project state: trajectories, knowledge, tasks, integration settings, schedules, and VecDB indexes live under `<project>/.refact/`.
- User-level knobs stay local too: privacy rules and provider definitions live under `~/.config/refact/`, while caches, logs, telemetry artifacts, shadow repos, and integration runtime state live under `~/.cache/refact/`.
- Bring your own keys and keep credentials on your machine; BYOK provider settings can target cloud APIs or fully offline local inference with no Refact-hosted control plane in the loop.
- Agent modes, subagents, toolbox/slash commands, code-lens prompts, system prompts, and provider defaults are YAML-configurable from `refact-agent/engine/yaml_configs/defaults/` and its packaged defaults.
- Tool behavior is policy, not magic: tune tool parameters, privacy filters, and confirmation rules so reads, edits, shells, integrations, and autonomous actions match your trust boundary.
- The payoff is ownership: audit every prompt, fork every workflow, swap every model, and shape the assistant around your repo instead of renting a sealed black box.

→ Deep dive: [Privacy](https://github.com/JegernOUTT/refact/wiki/Privacy), [Agent Modes](https://github.com/JegernOUTT/refact/wiki/Agent-Modes)

## Modes, transitions, compression & scheduling

![Switching modes and compacting context](media/chat-modes.png#gh-light-mode-only)
![Switching modes and compacting context](media/chat-modes-dark.png#gh-dark-mode-only)
Refact does not run one generic chatbot forever; it shifts between purpose-built operating systems for thought, code, review, planning, execution, memory, and time. The wild part: those shifts become durable context, so the agent can hand itself a plan, compact the past, and wake up later without losing the plot.

- **Mode as behavior, not branding:** `ask`, `explore`, `plan`, `agent`, `quick_agent`, `debug`, `buddy`, `task_planner`, and `task_agent` ship as separate defaults with distinct prompts, toolsets, subagent policies, and execution discipline.
- **Read-only when you need bearings, sharp tools when you need motion:** Explore gathers context without editing; Plan turns decisions into implementation shape; Agent and Quick Agent can operate autonomously with file, shell, process, MCP, knowledge, and task tools.
- **Task handoffs carry their own gravity:** switching or handing off modes records hidden `mode_switch` events instead of smearing instructions into chat text, and a Task Planner handoff with an `initial_plan` auto-creates a pinned task document so the board starts with the plan already attached.
- **Context is actively re-shaped mid-flight:** before model calls, history goes through a four-stage pressure valve—deduplicate context files, compress bulky tool results, repair tool-call/tool-result ordering, then trim to the budget.
- **Compression stays visible and auditable:** deterministic and reactive compaction create first-class `compression_report` messages with token accounting, while LLM segment summaries stay as source-preserving assistant artifacts instead of invisible prompt surgery.
- **Long sessions stay coherent across provider weirdness:** hidden plan/event roles are preserved in trajectories, exempted from destructive compression where needed, and lowered into provider-safe context frames instead of leaking fake roles onto the wire.
- **The agent can schedule its own future:** `cron_create`, `cron_list`, and `cron_delete` manage session-only or durable project jobs, with deterministic jitter, missed durable one-shot catch-up, 30-day recurring auto-expire, and `cron_fire` events that enqueue the saved prompt.
- **Waiting is a first-class move too:** interruptible `sleep` avoids blocking shell processes and can emit tick events, so “check again later” becomes part of the same observable runtime instead of a sticky-note outside the system.

→ Deep dive: [Agent Modes](https://github.com/JegernOUTT/refact/wiki/Agent-Modes), [Context Compression](https://github.com/JegernOUTT/refact/wiki/Context-Compression), [Scheduler and Cron](https://github.com/JegernOUTT/refact/wiki/Scheduler-and-Cron)

## Modern extension surface — MCP, skills, slash commands, hooks, subagents & marketplace

![MCP, skills, and subagents](media/mcp-skills.png#gh-light-mode-only)
![MCP, skills, and subagents](media/mcp-skills-dark.png#gh-dark-mode-only)
Plug in almost anything — local CLIs, HTTP/SSE services, project rituals, specialist agents, reusable prompts — without dumping the whole universe into the model. Refact keeps the surface sharp: discover late, load only what matters, and fan out focused workers when the job wants a swarm.

- **MCP without context spam:** connect stdio servers or Streamable HTTP/SSE endpoints, including OAuth-capable configs, then let lazy discovery do the heavy lifting: `tool_search` finds the right capability and `mcp_call` runs it only when needed.
- **Skills on demand:** focused instruction packs stay out of the prompt until the task calls for them; load a skill with `load_skill`, work inside its guidance, then `deload_skill` to compact the run back into a clean report.
- **Slash-command workflows:** project and installed commands turn repeatable rituals into one-keystroke launches — reviews, migrations, diagnostics, release prep, or whatever your team keeps retyping.
- **Hooks for automation seams:** pre/post-tool and lifecycle hooks let extensions react around tool calls, sessions, and subagent runs, so policy, logging, formatting, and handoff glue can live beside the workflow instead of inside the chat.
- **Subagents as real tools:** project-defined subagents can expose schemas, run as first-class agentic tools, and execute in parallel when marked safe — perfect for investigation swarms, code reading, research, and scoped background work.
- **Marketplace-installed powers:** Skill, Command, and Subagent extensions can be browsed and installed from the marketplace, with the GUI Extensions page managing creation, editing, sources, and installed packs.
- **The payoff:** huge extension catalogs stay calm, because Refact surfaces the exact capability at the exact moment: search it, load it, run it, summarize it, and keep moving.

→ Deep dive: [MCP](https://github.com/JegernOUTT/refact/wiki/MCP), [Skills, Commands & Hooks](https://github.com/JegernOUTT/refact/wiki/Skills-Commands-Hooks), [Subagents](https://github.com/JegernOUTT/refact/wiki/Subagents), [Marketplace](https://github.com/JegernOUTT/refact/wiki/Marketplace)

## Bring your own models — any provider, any runtime

![Bring your own models](media/byok.png#gh-light-mode-only)
![Bring your own models](media/byok-dark.png#gh-dark-mode-only)
Refact is BYOK all the way down: hosted frontier APIs, local runtimes, self-hosted OpenAI-compatible stacks, and OAuth-backed coding agents can all sit in the same cockpit. Pick the right brain per task, keep credentials and policy local, and let capability-aware routing do the boring glue work.

- 20+ provider families are first-class citizens: Anthropic, OpenAI, OpenAI Responses, OpenAI Codex, OpenRouter, Groq, DeepSeek, Doubao, xAI, xAI Responses, Google Gemini, Qwen, Kimi, Zhipu, MiniMax, GitHub Copilot, Claude Code, plus custom endpoints and local runtimes.
- Run fully local or self-hosted when you want the keys off the internet: Ollama, LM Studio, vLLM, and any OpenAI-compatible `/chat/completions`, `/responses`, `/completions`, or embeddings route can be wired in.
- Mix models by role instead of marrying one vendor: chat, agent, task planner, light chat, thinking, Buddy, code completion/FIM, and embeddings each have their own defaults.
- Capability metadata keeps selections honest: tool use, agent mode, reasoning, multimodality, context windows, cache control, tokenizer/FIM settings, and embeddings are resolved before a model is offered for work.
- OAuth is built into the provider surface for OpenAI Codex and Claude Code, while classic API-key providers stay plain BYOK through local provider config.
- Adding a new provider is intentionally tiny: create one YAML template for endpoints, wire format, defaults, and model caps, then add one entry to the provider template list.
- The result is model freedom without config soup: swap paid frontier models, cheap routers, local coders, and private deployments per task while Refact keeps the same agentic tool loop.

→ Deep dive: [BYOK](https://github.com/JegernOUTT/refact/wiki/BYOK), [Providers](https://github.com/JegernOUTT/refact/wiki/Providers), [Supported Models](https://github.com/JegernOUTT/refact/wiki/Supported-Models)

## Architecture: resident daemon + thin clients

One `refact` binary becomes the whole control room. Running `refact` opens the full-screen TUI and starts or reuses a resident local daemon; the daemon supervises a warm worker per opened project, serves the in-browser GUI, and gives IDE plugins a shared backend instead of asking every client to boot its own engine.

- The daemon owns the control plane: `/daemon/v1/*` handles status, workers, projects, logs, events, shutdown, and health checks, while `/p/{id}/v1/*` proxies each registered project's worker API.
- Project workers run the Rust engine surfaces you already expect — Axum HTTP, tower-lsp, chat commands, caps, tools, checkpoints, SSE snapshots, background indexers, provider routing, and exec runtime.
- TUI, browser GUI, VS Code, and JetBrains are all thin clients to the same resident daemon. The UI you choose changes the shell around the agent, not the sessions, tools, memory, or project state underneath.
- `refact projects open .` registers the current workspace and starts its worker; `refact ps`, `refact status`, `refact logs`, `refact events`, and `refact doctor` inspect the same local runtime.
- `refact daemon --foreground` is available for debugging or service supervision, while normal `refact`/`refact tui` usage auto-starts the daemon when needed.
- IDE plugins provide file context and editor actions through LSP, HTTP, and a postMessage bridge; they attach to the daemon-backed project instead of being the only way to run Refact.
- Local intelligence stays local: tree-sitter AST indexing, SQLite+vec0 semantic search, project memory, Buddy, scheduler, and task-agent worktrees live on your machine and feed the same agent/tool pipeline.

→ Deep dive: [Architecture](https://github.com/JegernOUTT/refact/wiki/Architecture), [GUI Architecture](https://github.com/JegernOUTT/refact/wiki/GUI-Architecture)

**Also included:** [Code Completion (FIM)](https://github.com/JegernOUTT/refact/wiki/Code-Completion-FIM), browser automation via [Chrome integrations](https://github.com/JegernOUTT/refact/wiki/Integrations-Chrome), and [Checkpoints &amp; Git](https://github.com/JegernOUTT/refact/wiki/Checkpoints-and-Git).

## Quickstart & install

Use the one-line installer at the top of this README, or download a binary manually from [GitHub Releases](https://github.com/JegernOUTT/refact/releases). The standalone binary gives you the daemon, TUI, in-browser GUI, worker supervisor, and CLI controls in one local package.

After installation, open the Refact dashboard:

```sh
refact ui
```

That starts or reuses the local daemon and opens the dashboard in your default browser. To open a project's workspace directly, pass its path:

```sh
refact ui .
```

IDE plugins are optional clients for the same daemon-backed projects:

- [VS Code](https://github.com/JegernOUTT/refact/wiki/Installation-VS-Code)
- [JetBrains](https://github.com/JegernOUTT/refact/wiki/Installation-JetBrains)

After installation, configure a provider or local runtime with [BYOK](https://github.com/JegernOUTT/refact/wiki/BYOK), then start with the [Quickstart](https://github.com/JegernOUTT/refact/wiki/Quickstart) or the full [Installation](https://github.com/JegernOUTT/refact/wiki/Installation) guide.

## Daily use: daemon, TUI, and worker

Refact now has one user-facing binary with a resident daemon behind it:

- `refact` and `refact tui` are the primary interactive entrypoints. They open the full-screen TUI and start or reuse the local daemon.
- `refact daemon` is the resident control plane. It supervises project workers, logs, events, project registration, health checks, and browser/IDE attachment.
- The project worker, also called the engine, is the per-project backend. It runs chat, tools, LSP/HTTP, indexing, checkpoints, scheduler work, and SSE streams; normal users usually do not start it directly.
- The TUI, browser GUI, VS Code, and JetBrains all attach to the same daemon-managed project, so they share the same sessions, tools, memory, and project state.

Fastest start from a repo:

```sh
refact projects open .
refact
```

Most useful day-to-day commands:

| Command | Use it for |
| --- | --- |
| `refact` | Open the TUI for the current project |
| `refact tui --project .` | Open the TUI for an explicit project path |
| `refact run --project . "…"` | Run one headless agent turn through the daemon |
| `refact projects open .` | Register or wake the current project worker |
| `refact ps` | List daemon-managed workers |
| `refact status` | Check daemon health |
| `refact logs --daemon -f` | Follow daemon logs |
| `refact logs . -f` | Follow this project's worker logs |
| `refact events -f` | Follow daemon events |
| `refact doctor` | Diagnose daemon setup and worker reachability |
| `refact restart --daemon` | Restart the daemon after config or binary changes |
| `refact self-update` | Update the installed binary from GitHub Releases |

Use daemon-backed `refact`, `refact tui`, and `refact run` for normal work. Use direct `refact worker` only when you are debugging the low-level engine process itself.

Gotcha: `--tui` is not a top-level flag; use `refact` or `refact tui`.

Full guide: [CLI and daemon installation guide](https://github.com/JegernOUTT/refact/wiki/Installation-CLI).

## Comparison

| Capability                        | Refact (this fork)                                                                    | Typical AI assistant                       |
| --------------------------------- | ------------------------------------------------------------------------------------- | ------------------------------------------ |
| Local-first / no-cloud            | Local engine, local project state, zero bundled cloud requirement                     | Often service-hosted by default            |
| BYOK providers                    | Broad hosted, local, OpenAI-compatible, and custom provider support                   | Usually one vendor or a small provider set |
| Autonomous agents                 | Tool-using agent modes with shell, file, browser, MCP, and delegation support         | Often chat-first with limited autonomy     |
| Task planner + cards              | Planner chats, task boards, per-card agents, and worktree isolation                   | Usually external project tracking          |
| Persistent memory + autoinjection | `.refact/` knowledge, trajectories, tasks, integrations, VecDB, and context injection | Often ephemeral or account-cloud memory    |
| Hidden static plans               | `set_plan`, `update_plan`, and `get_plan` preserve base plans plus append-only deltas | Rarely supported                           |
| MCP / skills / subagents          | MCP lazy discovery, skills, slash commands, hooks, subagents, and marketplaces        | Varies by vendor                           |
| Worktree isolation                | Per-card isolated git worktrees for task-agent execution                              | Rarely built in                            |
| Scheduler / cron                  | Basic `cron_create`, `cron_list`, `cron_delete`, and `sleep` tick events              | Usually absent or external                 |
| Cross-device UI                   | Browser UI works anywhere that can reach the local engine                             | Usually app-specific                       |
| Telemetry-free                    | Local/BYOK usage does not require a Refact cloud account                              | Often account and service telemetry based  |

## Supported providers & models

Refact supports provider families including Anthropic, OpenAI, OpenAI Responses, OpenAI Codex, OpenRouter, Ollama, LM Studio, vLLM, Groq, DeepSeek, Doubao, xAI, xAI Responses, Google Gemini, Qwen, Kimi, Zhipu, MiniMax, GitHub Copilot, Claude Code, and custom OpenAI-compatible providers.

Model availability, pricing, quotas, and data policy come from the provider or runtime you configure. See [Supported Models](https://github.com/JegernOUTT/refact/wiki/Supported-Models) and [BYOK](https://github.com/JegernOUTT/refact/wiki/BYOK).

## Contributing & community

Contributions are welcome. Start with [CONTRIBUTING.md](CONTRIBUTING.md) or open a focused [Issue](https://github.com/JegernOUTT/refact/issues).

Docs live in the [GitHub Wiki](https://github.com/JegernOUTT/refact/wiki), including setup guides, supported models, BYOK configuration, and agent tooling notes.

## License + attribution

Refact is distributed under the BSD-3-Clause license. See [LICENSE](LICENSE) for details.

Refact is the actively maintained fork of the archived [`smallcloudai/refact`](https://github.com/smallcloudai/refact).
