---
title: Agent Integrations
description: Configure external services and built-in runtime tools for Refact Agent.
---

Integrations extend Refact Agent beyond the local codebase. Some tools are built in, while others become available after you configure an integration in settings.

## Built-in runtime tools

- [Chrome and browser automation](./chrome/) — browser tabs, screenshots, DOM inspection, element interaction, console logs, and page extraction.
- [Shell commands](./shell-commands/) — one-off local commands with timeout, output filtering, and confirmation rules.

## Version control and code hosting

- [GitHub](./github/) — GitHub CLI operations such as issues and pull requests.
- [GitLab](./gitlab/) — GitLab CLI operations such as issues and merge requests.
- [Bitbucket](./bitbucket/) — Bitbucket Cloud API operations for repositories and pull requests.

## Databases and debugging

- [PostgreSQL](./postgresql/) — execute a single `psql` query per tool call.
- [MySQL](./mysql/) — execute a single `mysql` query per tool call.
- [PDB](./pdb/) — control an interactive Python debugger session.

## Custom tools and protocols

- [Command-line Tool](./command-line-tool/) — expose one blocking command with model-filled parameters.
- [Command-line Service](./command-line-service/) — start, stop, restart, and check long-running processes.
- [MCP Server](./mcp/) — connect local stdio or remote HTTP/SSE MCP servers.

## How to configure

Open integrations from Refact settings or the integrations control in chat. Configure credentials, command paths, working directories, parameters, output filters, and confirmation rules as needed. After saving, switch to a mode that allows integrations and ask the agent to test the tool.

## Safety

Use confirmation rules for commands, queries, and external service operations that can mutate data. Prefer secrets or variables for tokens and passwords. Keep destructive actions denied unless you intentionally need them.
