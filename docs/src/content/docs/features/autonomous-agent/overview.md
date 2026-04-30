---
title: Agent Overview
description: What Refact Agent can do in autonomous development workflows.
---

Refact Agent turns chat into a multi-step development workflow. It can gather local context, reason about a task, edit files, run checks, use browser and web tools, call configured integrations, and report progress in the thread.

## Core capabilities

- **Context gathering**: inspect project trees, read files, search text and vectors, and look up AST symbols.
- **Code changes**: create files, update text, apply patches, move or remove files, and undo recent edits.
- **Tool confirmations**: show sensitive actions before they run, with configurable allow and deny rules.
- **Shell and services**: run one-off commands, tests, linters, builds, or manage configured background services.
- **Web and browser work**: fetch pages, search the web when needed, and automate Chrome for screenshots, forms, DOM inspection, and console logs.
- **Review and research**: use planning, code review, deep research, and subagents for larger tasks.
- **Tasks and knowledge**: update task boards, save reusable knowledge, and search previous trajectories.
- **Rollback**: preview and restore workspace checkpoints when checkpointing is enabled.

## How agent workflows run

The agent alternates between reasoning, tool calls, and responses. Tool calls produce visible results in chat. File edits are shown as patches or diffs so you can inspect what changed. Commands and integrations follow confirmation rules, so you remain in control of actions that affect your machine or external services.

## When to use Agent mode

Use Agent mode for tasks that require more than a single answer:

- Fixing bugs across multiple files.
- Implementing a feature with tests.
- Running verification commands.
- Reviewing a diff or pull request.
- Investigating browser, API, database, or integration behavior.

For quick explanations or small selected-code transformations, use [AI Chat](/features/ai-chat/) or [AI Toolbox](/features/ai-toolbox/).

## Related pages

- [Agent Tools](/features/autonomous-agent/tools/)
- [Agent Integrations](/features/autonomous-agent/integrations/)
- [Agent Rollback](/features/autonomous-agent/rollback/)
