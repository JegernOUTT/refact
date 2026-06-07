<div align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="media/logo-dark.svg">
    <source media="(prefers-color-scheme: light)" srcset="media/logo-light.svg">
    <img alt="Refact logo" src="media/logo-light.svg" width="220">
  </picture>

  <h1>Refact</h1>

  <p><strong>Refact — the open-source, local-first agentic coding engine.</strong> Autonomous agents, a task planner, persistent project memory, and a cross-device UI — all running from your IDE with zero cloud.</p>

  <img src="media/hero.gif" alt="Refact agent planning, editing, and running checks" width="900">

  <p>
    <a href="https://github.com/JegernOUTT/refact/stargazers"><img src="https://img.shields.io/github/stars/JegernOUTT/refact?style=for-the-badge&color=blue" alt="GitHub stars"></a>
    <a href="https://github.com/JegernOUTT/refact/issues"><img src="https://img.shields.io/github/issues/JegernOUTT/refact?style=for-the-badge" alt="GitHub issues"></a>
    <a href="https://github.com/JegernOUTT/refact/blob/main/LICENSE"><img src="https://img.shields.io/github/license/JegernOUTT/refact?style=for-the-badge" alt="License"></a>
    <a href="https://github.com/JegernOUTT/refact/wiki"><img src="https://img.shields.io/badge/Documentation-Wiki-2ea44f?style=for-the-badge" alt="Documentation"></a>
  </p>
</div>

Refact is for power users who want an AI development environment they can steer, inspect, and extend: a Rust engine, IDE-native chat, local project state, BYOK providers, MCP integrations, browser automation, task-card agents, and recoverable edits.

## Feature showcase

### Autonomous agents & tools

![Autonomous agents and tools](media/agent-task-planner.gif)

Run agent workflows that inspect code, edit files, apply patches, launch shells or services, delegate to subagents, and pause for tool confirmation when needed.

### Task planner & cards

![Task planner and cards](media/agent-task-planner.gif)

Break large work into planner chats and per-card agent chats, with kanban-style progress and isolated git worktrees for focused execution.

### Persistent memory

![Persistent memory](media/memory.gif)

Keep project knowledge under `.refact/`: static hidden plans, immutable plan updates, knowledge graph facts, VecDB search, trajectories, tasks, and autoinjected context.

### Modes, transitions, compression, and cron

![Modes and transitions](media/chat-modes.gif)

Switch between ask, explore, plan, agent, debug, buddy, task planner, and task-agent modes; preserve hidden mode events, compress long history, and schedule basic cron prompts.

### Modern extension surface

![MCP skills and subagents](media/mcp-skills.gif)

Use MCP over stdio or HTTP/SSE, skills, slash commands, hooks, subagents, and marketplace-installed extensions without flooding every request with every tool schema.

### Multi-provider / BYOK

![Multi-provider setup](media/mcp-skills.gif)

Bring your own keys or local runtimes: Refact connects to hosted providers, OpenAI-compatible endpoints, Ollama, LM Studio, vLLM, Claude Code, GitHub Copilot, and custom backends.

### Rust core + cross-device UI

![Browser-reachable UI](media/any-device.gif)

`refact-lsp` serves the chat UI, HTTP API, and SSE stream locally; open the UI in any browser that can reach the engine.

### Code completion (FIM)

![Code completion](media/code-completion.gif)

Inline fill-in-the-middle completion uses local code context, AST/RAG support, and model capability metadata for fast editing loops.

### Browser automation

![Browser automation](media/browser-tool.gif)

Drive a real browser session: navigate, click, fill forms, inspect accessibility state, run JavaScript, read console logs, and capture screenshots.

### Auto-apply + checkpoints

![Auto-apply and checkpoints](media/auto-apply.gif)

Review patch-like edits, auto-apply safe changes, undo mistakes, preview workspace checkpoints, and roll back when an experiment goes sideways.

## Quickstart & install

1. Install Refact for your IDE:
   - [VS Code](https://github.com/JegernOUTT/refact/wiki/Installation-VS-Code)
   - [JetBrains](https://github.com/JegernOUTT/refact/wiki/Installation-JetBrains)
2. Open a workspace and launch the Refact sidebar or tool window.
3. Configure a provider or local runtime with [BYOK](https://github.com/JegernOUTT/refact/wiki/BYOK).
4. Pick chat, agent, reasoning, and embedding defaults where applicable.
5. Start with the [Quickstart](https://github.com/JegernOUTT/refact/wiki/Quickstart), then explore the full [Installation](https://github.com/JegernOUTT/refact/wiki/Installation) guide.

## Comparison

| Capability | Refact (this fork) | Upstream archive | Typical AI assistant |
| --- | --- | --- | --- |
| Local-first / no-cloud | Local engine, local project state, zero bundled cloud requirement | Earlier local-first foundation, no longer active | Often service-hosted by default |
| BYOK providers | Broad hosted, local, OpenAI-compatible, and custom provider support | Older provider coverage | Usually one vendor or a small provider set |
| Autonomous agents | Tool-using agent modes with shell, file, browser, MCP, and delegation support | Earlier agent workflows | Often chat-first with limited autonomy |
| Task planner + cards | Planner chats, task boards, per-card agents, and worktree isolation | Not the active focus | Usually external project tracking |
| Persistent memory + autoinjection | `.refact/` knowledge, trajectories, tasks, integrations, VecDB, and context injection | Earlier project state concepts | Often ephemeral or account-cloud memory |
| Hidden static plans | `set_plan`, `update_plan`, and `get_plan` preserve base plans plus append-only deltas | Not a primary public surface | Rarely supported |
| MCP / skills / subagents | MCP lazy discovery, skills, slash commands, hooks, subagents, and marketplaces | More limited extension story | Varies by vendor |
| Worktree isolation | Per-card isolated git worktrees for task-agent execution | Not a central workflow | Rarely built in |
| Scheduler / cron | Basic `cron_create`, `cron_list`, `cron_delete`, and `sleep` tick events | Not a central workflow | Usually absent or external |
| Cross-device UI | Browser UI works anywhere that can reach the local engine | Earlier web UI foundation | Usually app-specific |
| Telemetry-free | Local/BYOK usage does not require a Refact cloud account | Cloud-era integrations existed historically | Often account and service telemetry based |

## Supported providers & models

Refact supports provider families including Anthropic, OpenAI, OpenAI Responses, OpenAI Codex, OpenRouter, Ollama, LM Studio, vLLM, Groq, DeepSeek, Doubao, xAI, xAI Responses, Google Gemini, Qwen, Kimi, Zhipu, MiniMax, GitHub Copilot, Claude Code, and custom OpenAI-compatible providers.

Model availability, pricing, quotas, and data policy come from the provider or runtime you configure. See [Supported Models](https://github.com/JegernOUTT/refact/wiki/Supported-Models) and [BYOK](https://github.com/JegernOUTT/refact/wiki/BYOK).

## Contributing & community

Contributions are welcome. Start with [CONTRIBUTING.md](CONTRIBUTING.md), open a focused [Issue](https://github.com/JegernOUTT/refact/issues), or join [Discussions](https://github.com/JegernOUTT/refact/discussions).

Docs live in the [GitHub Wiki](https://github.com/JegernOUTT/refact/wiki), including setup guides, supported models, BYOK configuration, and agent tooling notes.

## Roadmap / project status

Editable near-term focus:

- PTY stdin workflows through `process_write_stdin` for richer interactive terminal sessions.
- Scheduler growth beyond basic cron while preserving durable/session-scoped job semantics.
- Marketplace expansion for skills, commands, subagents, and extension discovery.
- More polished task-agent review loops around isolated worktrees, checkpoints, and auto-apply.

## License + attribution

Refact is distributed under the BSD-3-Clause license. See [LICENSE](LICENSE) for details.

Refact is the actively maintained fork of the archived [`smallcloudai/refact`](https://github.com/smallcloudai/refact).
