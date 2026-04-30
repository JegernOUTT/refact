---
title: Agent Tools
description: Built-in and configured tools available to Refact Agent.
---

Agent tools let Refact inspect your project, edit files, run commands, browse the web, and connect to configured services. The exact tool list depends on the selected mode, model capabilities, privacy settings, and integration configuration.

## Codebase search

The agent can gather local project context with tools for:

- Project tree and file listings.
- Reading files and images.
- Regex/text search.
- Semantic vector search when the vector database is enabled.
- AST symbol definitions when syntax parsing is enabled.

These tools are used heavily by Explore, Review, Debug, and Agent workflows.

## Codebase changes

Editing tools can:

- Create new text files.
- Update files by exact text, line range, regex, or anchors.
- Apply patches.
- Move or remove files.
- Undo recent text edits.

Patch-like edits are shown in chat and can require confirmation depending on the mode and settings.

## Web and browser tools

Refact includes tools to fetch web pages, search the web when the selected provider does not handle web search itself, and automate Chrome. Browser automation can navigate pages, click and fill elements, wait for page changes, capture screenshots, extract text/HTML/tables/links, inspect accessibility snapshots, run JavaScript, and read console logs.

## System tools

System tools can run shell commands, manage configured long-running command-line services, and add workspace folders. Use them for tests, builds, local scripts, dev servers, and diagnostics. Destructive or sensitive commands should be controlled with confirmation rules.

## Planning, review, research, and subagents

Higher-level tools help the agent split work into steps, perform code review, research unfamiliar systems, and delegate focused sub-tasks to subagents. Project-defined subagents can also be exposed as tools.

## Knowledge and tasks

The agent can activate skills, search and save project knowledge, use previous trajectories as context, manage task boards, spawn task agents, and record task memories.

## Integrations and MCP

Configured integrations add tools for GitHub, GitLab, Bitbucket, PostgreSQL, MySQL, PDB, custom command-line tools, command-line services, and MCP servers. MCP servers with many tools may be exposed through lazy discovery tools so the model can search for a tool schema before calling it.

## Best practices

- Let the agent inspect relevant files before editing.
- Keep confirmation rules strict for shell, database, and external service tools.
- Ask for verification commands to run after code changes.
- Use checkpoints when working on large or risky changes.
