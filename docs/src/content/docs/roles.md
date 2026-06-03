---
title: Hidden Roles and Plans
description: How Refact stores internal events and durable plans without showing them as normal chat turns.
---

Refact chat history contains more than the visible user and assistant messages. The engine also stores hidden message roles so the agent can remember internal facts without pretending that the user typed them.

The two hidden roles are:

| Role | What it stores | Where you see it |
| --- | --- | --- |
| `event` | Internal facts such as tool decisions, mode switches, cron fires, sleep ticks, process completions, verifier reports, and system notices. | The collapsible Event log in chat. |
| `plan` | The agent's current and previous Markdown plans, including version metadata. | The Plan banner pinned above the chat transcript. |

Hidden roles are still part of the thread. They are saved in trajectories, included in snapshots, and available to the model through provider-safe formatting. When Refact starts a new thread from existing history, it preserves these hidden roles while sanitizing UI-only fields so internal bookkeeping stays separate from human conversation.

## Why hidden roles exist

Older flows sometimes inserted internal status as synthetic user messages. That could confuse agents, users, and compression because an engine-generated event looked the same as a real instruction from you. Hidden roles make the distinction explicit:

- User messages are only things the user or scheduler intentionally asks the agent to do.
- Events are internal context and audit trail entries.
- Plans are long-lived task state.

## Event messages

An event message has this shape in the engine:

```json
{
  "role": "event",
  "content": "Process tests exited with code 0",
  "extra": {
    "event": {
      "subkind": "process_completed",
      "source": "exec.registry",
      "payload": {
        "process_id": "exec_123",
        "status": "exited",
        "exit_code": 0
      }
    }
  }
}
```

`content` is a short human-readable summary. `extra.event.payload` is structured data for the GUI and future tools.

Current event subkinds are:

| Subkind | Meaning |
| --- | --- |
| `mode_switch` | The chat mode changed. |
| `tool_decision` | The user approved or rejected tool calls. |
| `ide_callback` | The IDE returned an IDE-side tool result. |
| `process_completed` | A background or service process reached a terminal state. |
| `cron_fire` | A scheduled prompt fired. |
| `tick` | A sleep/wait operation emitted a progress tick. |
| `summarization_marker` | Chat compaction inserted an anchor. |
| `verifier_report` | A verifier generated review or validation output. |
| `cancellation_note` | A cancellation was recorded. |
| `system_notice` | General internal notice. |

## Plan messages

A plan message stores Markdown plus metadata:

```json
{
  "role": "plan",
  "content": "## Plan\n- Inspect the scheduler\n- Update docs",
  "extra": {
    "plan": {
      "mode": "agent",
      "version": 3,
      "created_at_ms": 1780000000000,
      "supersedes": "previous-message-id"
    }
  }
}
```

The latest version appears in the Plan banner. Older versions are kept for history and recovery.

## Tools for plans

### `set_plan`

The model uses `set_plan` when its understanding changes.

Schema:

```json
{
  "type": "object",
  "properties": {
    "content": { "type": "string", "description": "Markdown plan body. Required." },
    "summary": { "type": "string", "description": "Short description of what changed, ≤120 chars. Optional." }
  },
  "required": ["content"]
}
```

Example:

```json
{
  "content": "## Plan\n- Confirm the failing test\n- Patch the reducer\n- Re-run vitest",
  "summary": "Focus plan on reducer fix"
}
```

Edge cases:

- `content` must be a non-empty string.
- `summary` is optional but must be short when present.
- A new version is appended; older plan messages are not deleted.
- Plan messages are compression-exempt.

### `get_plan`

The model uses `get_plan` to read the latest installed plan.

Schema:

```json
{ "type": "object", "properties": {}, "required": [] }
```

It returns:

```json
{
  "plan": {
    "content": "## Plan\n- ...",
    "mode": "agent",
    "version": 3,
    "created_at_ms": 1780000000000
  }
}
```

If no plan exists, it returns `{ "plan": null }`.

## Compression rules

Plans are never compressed, truncated, or dropped. This is intentional: the current plan is the agent's durable working state.

Events use subkind-specific retention:

- `tick` and old `mode_switch` events can be dropped with age.
- `process_completed` and `cron_fire` keep recent entries and summarize older history.
- `tool_decision`, `ide_callback`, and `verifier_report` are preserved near recent user turns.
- `summarization_marker`, `system_notice`, and `cancellation_note` are preserved as anchors.

## Provider wire behavior

Hidden roles are not sent to model providers as literal `event` or `plan` roles. Refact lowers them into provider-supported user/system context with structured framing. This keeps provider APIs valid while preserving the distinction inside Refact.
