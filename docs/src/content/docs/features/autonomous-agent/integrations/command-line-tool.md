---
title: Command-line Tool
description: Expose a one-shot command as a Refact integration tool.
---

Command-line tool integrations let you adapt an existing local command into a model-callable tool. They are for commands that start, finish, and return output in a single call.

## Configuration

A `cmdline_*` integration defines:

- **Command**: the shell command to run. Use `%param_name%` placeholders for model-filled values.
- **Working directory**: optional command workdir. If empty, Refact uses the workspace directory.
- **Description**: what the model sees when deciding whether to call the tool.
- **Parameters**: names, types, and descriptions for values the model must provide.
- **Timeout**: how long the command may run before Refact terminates it.
- **Output filter**: limits, top/bottom prioritization, regex filtering, and cleanup for large output.

## Example uses

- Project-specific build or test wrappers.
- Log parsing commands.
- Internal CLIs with narrow, safe parameters.
- Data extraction scripts that print results and exit.

## Safety

Configure confirmation rules for commands that can mutate files, call external services, or expose secrets. Use command-line services instead when the process is meant to keep running, such as a dev server or `tail -f`.
