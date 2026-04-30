---
title: Shell Tool
description: Run one-off shell commands with confirmations and output filtering.
---

The built-in shell tool lets agent modes run local commands such as tests, builds, linters, formatters, and diagnostics. It is intended for one-off commands that finish and return stdout and stderr.

## Behavior

- Commands run in a shell on the user's machine.
- The default working directory is the workspace unless the tool call specifies another allowed directory.
- Commands have a timeout.
- Output can be limited, filtered by regex, and focused on the beginning or end of large output.
- Results are shown in chat for the agent and user to inspect.

## Confirmation rules

Shell commands can be controlled with allow, ask, and deny rules. Keep confirmation enabled for destructive operations such as deleting files, changing remotes, installing packages globally, or touching credentials.

## When to use shell vs integrations

Use shell for project-local commands that are not worth turning into a reusable tool. Use command-line tool integrations for repeated commands with structured parameters. Use command-line service integrations for long-running processes.
