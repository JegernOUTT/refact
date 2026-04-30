---
title: AI Chat
description: Ask questions, explore code, and run agent workflows with Refact chat.
---

Refact chat is the main interface for working with your project. It can answer questions, explain code, gather local context, make edits, run tools, and coordinate longer agent workflows.

## What chat can do

- Explain files, symbols, errors, and selected snippets.
- Gather context from the project tree, file contents, AST symbols, semantic search, previous trajectories, tasks, and saved knowledge.
- Fetch web pages or search the web when the selected mode and model configuration allow it.
- Propose and apply file edits, patches, moves, removals, and undo operations in agent workflows.
- Run shell commands, command-line services, browser automation, and configured integrations with the appropriate confirmations.
- Continue a conversation while work is running by queueing additional messages for the same thread.

## Modes and workflows

Mode names can be customized, but Refact commonly separates chat into these product-level workflows:

### Ask and quick Q&A

Use a direct chat mode for explanations, small questions, and lightweight help. The model answers from the conversation, IDE context, attached files, and any explicitly enabled tools. This is the best choice when you want a fast answer and do not need the agent to edit files.

### Explore, learn, debug, review, and plan

Exploration-style modes focus on gathering context before answering. They can inspect the project tree, read files, search text or vectors, inspect symbol definitions, and use specialized analysis tools. Use these modes when you want the assistant to understand the codebase, investigate a bug, review a change, or draft an implementation plan before editing.

### Agent and task workflows

Agent modes can take multi-step actions. They may read and edit files, apply patches, run checks, call browser or web tools, use integrations, spawn subagents, and update task boards. The agent reports tool calls and patch previews in chat so you can follow and approve sensitive steps.

## Context sources

Refact builds context locally before sending a request to the configured provider or local runtime. Depending on settings and mode, context can include:

- Open files, the current cursor location, and selected snippets from the IDE.
- Files and directories allowed by privacy settings.
- Project tree and file summaries.
- AST definitions and references when syntax parsing is enabled.
- Semantic search results when the vector database is enabled.
- Chat history, saved knowledge, trajectories, and task metadata.
- Tool results such as shell output, browser screenshots, web pages, and integration responses.

## Context windows

The amount of context available depends on the selected model and provider. Refact prepares and compresses local context to fit the model's context window, but larger models and local runtimes can behave differently. If a thread becomes large, Refact may compress older messages or ask for confirmation before including very large tool results.

## Tool confirmations

Potentially sensitive actions are shown as tool calls. File patches, shell commands, integration calls, database queries, and browser actions can require confirmation depending on the selected mode and configured rules. You can allow a single action, allow similar actions for the chat, or stop the workflow.

## Tips

- Attach the exact file or snippet when you know where the issue is.
- Use Explore or Plan before asking for a broad refactor.
- Ask the agent to run the project's normal checks after it edits code.
- Review patches before applying them, especially when the task touches generated files, migrations, or production configuration.
