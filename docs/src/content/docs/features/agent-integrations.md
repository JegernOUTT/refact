---
title: Agent Integrations
description: Built-in and configured integrations available to Refact Agent.
---

Refact Agent combines built-in tools with optional configured integrations. Built-in tools such as shell and Chrome are available through modes that enable them. Configured integrations add service-specific tools after you provide credentials or command settings.

## Built-in tools

- [Chrome and browser automation](/features/autonomous-agent/integrations/chrome/) — launch or connect to Chrome, navigate pages, capture screenshots, and inspect DOM or console output.
- [Shell commands](/features/autonomous-agent/integrations/shell-commands/) — run one-off local commands with timeout, output filtering, and confirmation rules.

## Configured integrations

- [GitHub](/features/autonomous-agent/integrations/github/) — run GitHub CLI workflows with a configured token.
- [GitLab](/features/autonomous-agent/integrations/gitlab/) — run GitLab CLI workflows with a configured token.
- [Bitbucket](/features/autonomous-agent/integrations/bitbucket/) — use the Bitbucket Cloud API for repositories and pull requests.
- [PostgreSQL](/features/autonomous-agent/integrations/postgresql/) — run one SQL query per tool call through `psql`.
- [MySQL](/features/autonomous-agent/integrations/mysql/) — run one SQL query per tool call through `mysql`.
- [PDB](/features/autonomous-agent/integrations/pdb/) — start and control Python debugger sessions.
- [Command-line Tool](/features/autonomous-agent/integrations/command-line-tool/) — expose a one-shot command as a model-callable tool.
- [Command-line Service](/features/autonomous-agent/integrations/command-line-service/) — manage long-running local processes such as dev servers.
- [MCP Server](/features/autonomous-agent/integrations/mcp/) — connect local stdio or remote HTTP/SSE Model Context Protocol servers.

## Configuration and confirmations

Use the integrations settings to enable or edit integrations. Many integrations support secrets or variables so credentials do not need to be written directly in prompts. Confirmation rules decide which commands or queries require approval and which are denied automatically.
