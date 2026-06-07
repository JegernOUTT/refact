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

**10+ agent modes · 50+ tools · 20+ provider families · worktree-isolated agent fleets · MCP · skills · subagents · BYOK · 100% local — zero cloud**

Refact is a local-first agentic coding engine built around `refact-lsp`: a Rust core that runs beside your workspace, streams the chat UI from your IDE, and gives autonomous tool-using agents the same concrete surfaces you use every day — files, shells, browser automation, AST search, patches, checkpoints, integrations, and verification loops. It is not just a sidebar chat: the task planner can break big work into cards, launch fleets of task agents in isolated git worktrees, and let Buddy keep a nosy little watch over project memory, diagnostics, docs, dependencies, and opportunities without handing your repo to a hosted control plane.

What makes Refact different is ownership. You bring the models and keys, choose hosted or local providers, wire MCP, skills, hooks, and subagents into your own workflows, and keep every byte of project state under `<project>/.refact/` where it can be inspected, backed up, deleted, or versioned on your terms. The whole stack is steerable and hackable: modes are YAML, plans are durable, memory is project-scoped, and the Rust engine serves a browser-reachable UI from your machine — powerful enough for multi-agent coding runs, transparent enough to debug when the gremlin gets spicy.
