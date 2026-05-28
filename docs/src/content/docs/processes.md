---
title: Processes, Background Jobs, and PTY
description: Manage long-running commands, interactive terminals, stdin, and process completion notifications.
---

Refact's execution runtime can run commands in the foreground, keep long-running processes in the background, manage named services, and attach interactive commands to a pseudo-terminal (PTY).

Use process tools when a command is long-running, interactive, or should be inspected after it starts.

## Foreground vs background vs service

| Mode | Best for | Lifetime |
| --- | --- | --- |
| Foreground shell | Tests, builds, one-shot diagnostics. | Tool call waits until completion or timeout. |
| Background process | Long-running one-off commands. | Continues after the tool returns; read/wait/kill by process ID. |
| Service | Named dev servers or watchers. | One running service per owner/workspace/name. |

The shell tool can also start a background process with `run_in_background: true`.

## Standard exec environment

Refact injects stable defaults into exec spawns unless a tool call explicitly overrides them:

```text
NO_COLOR=1
TERM=dumb
LANG=C.UTF-8
LC_CTYPE=C.UTF-8
LC_ALL=C.UTF-8
COLORTERM=
PAGER=cat
GIT_PAGER=cat
GH_PAGER=cat
REFACT_EXEC=1
```

These defaults avoid ANSI noise, locale drift, and commands blocking in pagers.

## PTY mode

Both `shell` and `process_start` support `tty: true`.

Use `tty: true` for:

- REPLs such as Python, Node, Rails console, or shells;
- CLIs that prompt for input;
- programs that buffer differently unless attached to a terminal;
- debugging interactive flows.

Keep `tty: false` for normal tests, linters, and build commands.

PTY trade-offs:

- stdout and stderr are merged into the `combined` stream;
- output may include prompts and terminal control characters;
- some commands behave differently when they detect a terminal;
- on Windows the backend uses the portable PTY/ConPTY path when available.

## Tools

### `shell`

Runs a command in the current workspace. Use it for one-shot commands, or set `run_in_background: true` for a background process.

Schema highlights:

```json
{
  "type": "object",
  "properties": {
    "command": { "type": "string" },
    "description": { "type": "string" },
    "workdir": { "type": "string" },
    "timeout": { "type": "integer" },
    "tty": { "type": "boolean", "default": false },
    "run_in_background": { "type": "boolean", "default": false }
  },
  "required": ["command", "description"]
}
```

Examples:

```json
{ "command": "cargo test --lib", "description": "Run Rust unit tests" }
```

```json
{
  "command": "npm run dev",
  "description": "Start frontend dev server",
  "run_in_background": true
}
```

Edge cases:

- `description` is required and must be concise.
- Numeric `timeout` values must be positive integers.
- `run_in_background` returns immediately with a process ID; do not add `&` to the command.
- Background shell commands emit a completion event when they exit.

### `process_start`

Starts a runtime-owned background or service process.

Schema highlights:

```json
{
  "type": "object",
  "properties": {
    "command": { "type": "string" },
    "description": { "type": "string" },
    "mode": { "type": "string", "enum": ["background", "service"] },
    "service_name": { "type": "string" },
    "workdir": { "type": "string" },
    "startup_wait_ms": { "type": "integer" },
    "startup_wait_port": { "type": "integer" },
    "startup_wait_keyword": { "type": "string" },
    "tty": { "type": "boolean", "default": false }
  },
  "required": ["command", "description"]
}
```

Examples:

```json
{
  "command": "npm run dev",
  "description": "Start web dev server",
  "mode": "service",
  "service_name": "web",
  "startup_wait_port": 5173
}
```

```json
{
  "command": "python3 -i",
  "description": "Open Python REPL",
  "tty": true
}
```

Edge cases:

- `service` mode requires `service_name`.
- Starting the same service twice in the same owner/workspace is rejected until you kill the old one.
- Workdirs are resolved through workspace and privacy rules.

### `process_list`

Lists runtime-owned processes.

Schema:

```json
{
  "type": "object",
  "properties": {
    "status": { "type": "string", "description": "running, completed, or all" },
    "scope": { "type": "string", "description": "chat, workspace, or all" }
  },
  "required": []
}
```

Use it to rediscover process IDs or audit completed jobs.

### `process_read`

Reads buffered output from a process.

Schema highlights:

```json
{
  "type": "object",
  "properties": {
    "process_id": { "type": "string" },
    "since_seq": { "type": "integer" },
    "stream": { "type": "string", "description": "stdout, stderr, combined, or all" },
    "output_filter": { "type": "string" },
    "output_limit": { "type": "string" }
  },
  "required": ["process_id"]
}
```

`process_read` returns cursor fields such as `next_seq` and `latest_seq`. Pass `next_seq` as the next `since_seq` to poll for only new output.

### `process_wait`

Waits for a process to exit, then returns output and metadata. Use it when you expect a background command to complete soon but do not want to poll manually.

### `process_kill`

Kills a runtime-owned process by ID.

Schema:

```json
{
  "type": "object",
  "properties": {
    "process_id": { "type": "string" }
  },
  "required": ["process_id"]
}
```

### `process_write_stdin`

`process_write_stdin` is the stdin contract for PTY processes.

Schema:

```json
{
  "type": "object",
  "properties": {
    "process_id": { "type": "string" },
    "chars": { "type": "string", "default": "" },
    "yield_time_ms": { "type": "integer", "default": 250, "maximum": 10000 }
  },
  "required": ["process_id"]
}
```

Behavior:

- Requires the process to have started with `tty: true`.
- Writes `chars` exactly as provided.
- Waits up to `yield_time_ms` for new output or exit.
- `chars: ""` is a poll: it writes nothing and only waits for output.
- Output metadata should include `bytes_written` and `chunks_returned`.

Examples:

```json
{ "process_id": "exec_123", "chars": "print('hi')\n", "yield_time_ms": 500 }
```

```json
{ "process_id": "exec_123", "chars": "", "yield_time_ms": 1000 }
```

Edge cases:

- Non-PTY process IDs are rejected.
- Include newlines when the process expects Enter.
- Control characters are literal; be deliberate.

## Completion notifications

When a background or service process exits, Refact injects `event(process_completed)` into the owning chat. The event payload includes:

```json
{
  "process_id": "exec_123",
  "status": "exited",
  "exit_code": 0,
  "duration_ms": 1000,
  "short_description": "Run backend server"
}
```

The GUI shows this in the Event log and can scroll to matching process cards when available. Foreground shell commands do not generate background completion events.

## Recommended lifecycle

1. Start long-running work with `process_start` or `shell` + `run_in_background`.
2. Keep the `process_id` from the result.
3. Use `process_read` with cursors to inspect output.
4. Use `process_write_stdin` only for `tty: true` sessions.
5. Use `process_wait` if you expect the process to finish soon.
6. Use `process_kill` before restarting services or cleaning up work.
