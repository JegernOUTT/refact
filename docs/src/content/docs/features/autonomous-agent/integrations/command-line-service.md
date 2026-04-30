---
title: Command-line Service
description: Manage long-running local processes from Refact Agent.
---

Command-line service integrations wrap long-running processes such as dev servers, file watchers, tunnels, or log streams. The agent can start, restart, stop, or check the status of the service.

## Configuration

A `service_*` integration defines:

- **Command**: the process to run. Use `%param_name%` placeholders for model-filled values.
- **Working directory**: optional service workdir. If empty, Refact uses the workspace directory.
- **Description**: what the model sees when deciding whether to call the service.
- **Parameters**: values the model can provide when starting the service.
- **Startup wait port**: a TCP port that indicates the service is ready.
- **Startup wait keyword**: text in stdout or stderr that indicates readiness.
- **Startup wait timeout**: maximum time to wait during startup.
- **Output filter**: limits and extracts useful logs from long service output.

## Actions

The tool accepts `start`, `restart`, `stop`, and `status`. Status also returns accumulated stdout and stderr since the last check.

## Safety

Use confirmation rules for services that modify data, open network listeners, or run expensive jobs. Prefer one-shot command-line tools for commands that finish immediately.
