<a name="readme-top"></a>

<div align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://docs.refact.ai/_astro/logo-dark.CCzD55EA.svg">
    <source media="(prefers-color-scheme: light)" srcset="https://docs.refact.ai/_astro/logo-light.CblxRz3x.svg">
    <img alt="Refact logo" src="https://docs.refact.ai/_astro/logo-dark.CCzD55EA.svg" width="200">
  </picture>
  <h1 align="center">Refact</h1>
  <p align="center">Open-source, local-first AI coding assistant for IDE chat, autonomous agent workflows, and tool-powered development.</p>
</div>

<div align="center">
  <a href="https://github.com/smallcloudai/refact/stargazers"><img src="https://img.shields.io/github/stars/smallcloudai/refact?style=for-the-badge&color=blue" alt="GitHub stars"></a>
  <a href="https://docs.refact.ai"><img src="https://img.shields.io/badge/documentation-blue?logo=googledocs&logoColor=FFE165&style=for-the-badge" alt="Documentation"></a>
  <a href="https://github.com/smallcloudai/refact/issues"><img src="https://img.shields.io/badge/issues-github?style=for-the-badge" alt="GitHub issues"></a>
</div>

Refact runs a local Rust engine (`refact-lsp`) from your IDE and connects only to the model providers, local runtimes, and integrations you configure. It brings together chat, codebase search, autonomous agents, browser automation, and tool integrations while keeping project state, indexes, trajectories, and task data on your machine.

> Refact Cloud has been retired. Read the announcement: [Refact Cloud is shutting down](https://refact.ai/blog/2026/refact-cloud-is-shutting-down/).

## Table Of Contents

- [Why Refact](#why-refact)
- [Core Features](#core-features)
- [What You Can Ask Refact To Do](#what-you-can-ask-refact-to-do)
- [Providers And Local Runtimes](#providers-and-local-runtimes)
- [Getting Started](#getting-started)
- [Repository Map](#repository-map)
- [Developer Commands](#developer-commands)
- [Documentation And Support](#documentation-and-support)

## Why Refact

- **Local-first by design**: the IDE extension starts a local engine and stores workspace context, checkpoints, tasks, knowledge, and trajectories locally.
- **Bring your own models**: connect hosted providers, OpenAI-compatible endpoints, or local runtimes instead of relying on a bundled model service.
- **Deep codebase awareness**: combine open files, selections, project tree, AST symbols, semantic search, git state, and previous work into useful model context.
- **Agent workflows inside the IDE**: let the agent inspect files, edit code, run checks, use integrations, and report progress without leaving VS Code or JetBrains.
- **Extensible tools**: use built-in tools, command-line integrations, browser automation, databases, code hosting integrations, and MCP servers.

No bundled inference endpoint or Refact-issued API key is required for local/BYOK usage.

## Core Features

### Local Engine Runtime

`refact-lsp` is the local HTTP/LSP engine behind the IDE experience. It serves the chat UI, tracks open workspaces, exposes model capability and tool APIs, manages shutdown and background tasks, and keeps project state in local Refact directories.

### Chat Sessions And Agent Modes

The engine runs persistent chat threads with command queues, SSE streaming, pause/confirmation states, retries, regeneration, message editing, and trajectory storage. Modes such as Ask, Explore, Debug, Review, Plan, Agent, and task workflows decide which tools and context sources are available.

### Workspace Understanding

Refact builds local context from project files, selected code, open editors, git state, AST indexes, and vector search. The agent can inspect project trees, read files, search text, find symbols, query semantic indexes, and prepare compact context for the chosen model.

### Tool-Powered Development

Agent tools can create and update files, apply patches, move or remove files, run shell commands, execute configured command-line tools, manage long-running services, fetch web pages, search the web, and delegate focused work to subagents.

### Browser And UI Investigation

The built-in Chrome runtime can open pages, click and fill controls, wait for page changes, capture screenshots, inspect DOM/accessibility state, run JavaScript, and read console logs. This makes UI debugging and browser-based validation part of the same agent workflow.

### Integrations And MCP

Refact connects to GitHub, GitLab, Bitbucket, PostgreSQL, MySQL, PDB, one-off command tools, services, and MCP servers. MCP lazy discovery keeps large external tool catalogs available without flooding every model request with every schema.

### Checkpoints, Git, And Review Loops

The engine can preview and restore workspace checkpoints, inspect git changes, generate commit messages from diffs, run code review flows, and keep edits visible as patches before they are accepted.

### Knowledge, Tasks, Skills, And Trajectories

Save reusable project knowledge, search previous trajectories, manage task boards, spawn task agents, activate skills, install commands/subagents, and resume previous work. These features make longer agent workflows repeatable instead of one-off chat sessions.

## What You Can Ask Refact To Do

- Generate new code from a feature request or implementation plan.
- Refactor code for readability, architecture, or maintainability.
- Explain unfamiliar modules, functions, errors, and stack traces.
- Debug failing tests, runtime errors, browser issues, or integration behavior.
- Write or update tests, fixtures, documentation, and docstrings.
- Review code changes and call out correctness, style, and integration risks.
- Run project checks, linters, builds, and custom command-line tools.
- Investigate web pages or app flows with browser automation.

## Providers And Local Runtimes

Refact discovers and enables models from configured providers and local runtimes. Current provider families include Anthropic, OpenAI, OpenAI Responses, OpenAI Codex, OpenRouter, Groq, DeepSeek, Doubao, xAI, Gemini, Qwen, Kimi, Zhipu, MiniMax, GitHub Copilot, Claude Code, custom OpenAI-compatible endpoints, Ollama, LM Studio, and vLLM.

Model availability, pricing, quotas, and data policies are controlled by the provider or runtime you choose. Refact adds local capability metadata so the UI and engine can select appropriate models for chat, reasoning, agent work, and embeddings.

📜 [Read more about supported models](https://docs.refact.ai/supported-models/)

## Getting Started

1. **Install an IDE plugin**
   - VS Code: follow the [VS Code installation guide](https://docs.refact.ai/installation/vs-code/).
   - JetBrains IDEs: follow the [JetBrains installation guide](https://docs.refact.ai/installation/jetbrains/).
2. **Open a workspace** and launch the Refact sidebar or tool window. The plugin starts the local `refact-lsp` engine.
3. **Configure a provider or runtime** in **Provider Setup**.
4. **Pick defaults** in **Default Models** for chat, agent work, reasoning, and embeddings where applicable.
5. **Start working**: ask a question, use a toolbox command, request a code change, or run an agent workflow.

## Repository Map

| Area | Path | Purpose |
| --- | --- | --- |
| Agent Engine | `refact-agent/engine/` | Rust `refact-lsp` HTTP/LSP engine, providers, tools, indexes, integrations |
| Agent GUI | `refact-agent/gui/` | React/Vite chat UI package used by IDE webviews and standalone development |
| VS Code extension | `plugins/vscode/` | VS Code host integration |
| JetBrains plugin | `plugins/intellij/` | JetBrains host integration |
| Docs site | `docs/` | Astro/Starlight documentation site |

## Developer Commands

```bash
# Engine
(cd refact-agent/engine && cargo check && cargo test --lib)

# GUI
(cd refact-agent/gui && npm ci && npm run types && npm run lint && npm run test)

# VS Code plugin (after building/packing refact-agent/gui)
(cd plugins/vscode && npm ci && npm install ../../refact-agent/gui/refact-chat-js-*.tgz --no-save && npm run compile && npm run lint)

# JetBrains plugin
(cd plugins/intellij && ./gradlew check)

# Docs
(cd docs && npm ci && npm run build)
```

See the dedicated READMEs in each subproject for full development workflows.

## Documentation And Support

- [Documentation](https://docs.refact.ai/)
- [Quickstart](https://docs.refact.ai/introduction/quickstart/)
- [Provider setup](https://docs.refact.ai/byok/)
- [Agent tools](https://docs.refact.ai/features/autonomous-agent/tools/)
- [GitHub issues](https://github.com/smallcloudai/refact/issues)
- [GitHub discussions](https://github.com/smallcloudai/refact/discussions)

## Contributing

Contributions are welcome. Please open an issue or discussion for larger changes, and run the relevant engine, GUI, or docs checks before submitting a pull request.

### Star History

[![Star History Chart](https://api.star-history.com/svg?repos=smallcloudai/refact&type=Date)](https://www.star-history.com/#smallcloudai/refact&Date)

## License

Refact is distributed under the BSD-3-Clause license. See the repository license for details.
