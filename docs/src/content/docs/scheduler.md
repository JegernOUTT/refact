---
title: Scheduler
description: Schedule future Refact agent prompts with cron expressions.
---

The Refact scheduler lets the agent schedule prompts for later. It is useful for recurring check-ins, reminders, delayed verification, and long-running workflows that should resume at a predictable time.

A scheduled task stores:

- a five-field cron expression;
- the prompt to enqueue when it fires;
- whether it is recurring or one-shot;
- whether it is session-only or durable;
- a short description;
- fire counters and timestamps.

When a task fires, Refact records `event(cron_fire)` in the thread and enqueues the configured prompt as a user message so the agent wakes up and acts.

## Session vs durable schedules

| Scope | Storage | Lifetime |
| --- | --- | --- |
| Session | In-memory scheduler store | Ends when the engine stops. |
| Durable | `<project>/.refact/scheduled_tasks.json` | Survives engine restarts for that project. |

Durable scheduling can be disabled by configuration. If `scheduler.disable_durable: true`, a request for a durable schedule falls back to session-only and returns the note `durable schedules disabled by config`.

## Scheduler controls

The runner can be disabled in three ways:

- set `REFACT_DISABLE_SCHEDULER=1`;
- start the engine with `--no-scheduler`;
- set `scheduler.enabled: false` in the engine config.

The default job cap is `scheduler.max_jobs: 50`.

## Cron syntax

Refact uses standard five-field cron syntax:

```text
minute hour day-of-month month day-of-week
```

Examples:

| Expression | Meaning |
| --- | --- |
| `*/15 * * * *` | Every 15 minutes. |
| `0 9 * * 1-5` | 09:00 every weekday. |
| `30 14 * * *` | 14:30 every day. |
| `0 10 1 * *` | 10:00 on the first day of each month. |

Cron expressions are evaluated in the local timezone.

## Tools

### `cron_create`

Model-facing prompt:

> Schedule a prompt to be enqueued later. Use a standard 5-field cron expression (`minute hour day-of-month month day-of-week`) evaluated in the local timezone. Set `recurring` to true for repeated prompts or false for a one-shot prompt that is removed after it fires. Set `durable` to true when the job should survive engine restarts in the current project; leave it false for a session-only in-memory schedule. Scheduler jitter is applied automatically so jobs may run shortly after the exact cron instant. Recurring jobs auto-expire after 30 days unless canceled earlier.

Schema:

```json
{
  "type": "object",
  "properties": {
    "cron": { "type": "string", "description": "Standard 5-field cron expression in local time." },
    "prompt": { "type": "string", "description": "Prompt enqueued at each fire time." },
    "recurring": { "type": "boolean", "default": true },
    "durable": { "type": "boolean", "default": false },
    "description": { "type": "string", "description": "Short description (≤80 chars) shown in cron_list UI." }
  },
  "required": ["cron", "prompt", "description"]
}
```

Example:

```json
{
  "cron": "0 9 * * 1-5",
  "prompt": "Review open task cards and summarize blockers.",
  "recurring": true,
  "durable": true,
  "description": "Weekday blocker review"
}
```

Edge cases:

- Invalid cron expressions are rejected.
- Expressions with no next fire within the next year are rejected.
- Descriptions longer than 80 characters are rejected.
- Durable schedules require a project root unless durable scheduling is disabled and the request falls back to session-only.
- The total job cap is enforced before creation.

### `cron_list`

Model-facing prompt: list scheduled tasks, optionally filtering by session-only or durable scope.

Schema:

```json
{
  "type": "object",
  "properties": {
    "scope": {
      "type": "string",
      "enum": ["session", "durable", "all"],
      "default": "all"
    }
  },
  "required": []
}
```

Example:

```json
{ "scope": "all" }
```

Returns an array of tasks with `id`, `cron`, `human_schedule`, `description`, first 200 characters of `prompt`, `recurring`, `durable`, `next_fire_at_ms`, `fire_count`, and `created_at_ms`.

Edge cases:

- Unknown scopes are rejected.
- `next_fire_at_ms` may be `0` if a future run cannot be calculated.

### `cron_delete`

Model-facing prompt: cancel a scheduled task by ID.

Schema:

```json
{
  "type": "object",
  "properties": {
    "id": { "type": "string" }
  },
  "required": ["id"]
}
```

Example:

```json
{ "id": "cron_123" }
```

Returns `{ "removed": true }` when a task was removed and `{ "removed": false }` when the ID did not exist.

## Jitter and load spreading

Scheduler jitter is deterministic per task ID. This means the same task keeps the same offset, but many tasks scheduled for the same cron instant spread out instead of stampeding the engine.

Recurring jobs are delayed by a bounded fraction of the interval. One-shot jobs use a separate one-shot jitter path near matching minutes. Jitter is runtime scheduling behavior; the stored cron expression is unchanged.

## Idle gating

The scheduler avoids interrupting active work. If the chat is generating, executing tools, or paused for confirmation, the runner defers the fire and checks again later. Once the chat is idle, the task fires and the prompt is queued.

## Missed tasks and expiration

On startup, recurring durable jobs do not replay every missed fire. They calculate the next run from now to avoid a burst. Past one-shot durable jobs fire as soon as possible and can be marked as missed in the `cron_fire` payload.

Recurring jobs auto-expire after the configured horizon, which defaults to 30 days. A final fire can include `final: true` before the task deletes itself.
