---
title: AI Toolbox
description: Run reusable prompt workflows on selected code or text.
---

The AI Toolbox is a set of reusable prompt workflows for selected code or text. It is useful for small, focused actions that do not need a full autonomous agent workflow.

## Built-in commands

Refact ships with default toolbox commands such as:

- `/bugs` — find likely bugs in the selected code.
- `/comment` — add explanatory comments.
- `/explain` — explain what the selected code does.
- `/shorter` — rewrite the selection more concisely.
- `/summarize` — summarize the selection in one paragraph.
- `/typehints` — add type hints where appropriate.
- `/typos` — fix typos, especially in strings and comments.

The exact list can vary because projects can add, remove, or override toolbox commands.

## When to use it

Use the Toolbox when you already know the relevant selection and want a fast, repeatable transformation or explanation. Use chat or Agent mode when the task needs broader context, multiple files, shell commands, tests, or patch review.

## Custom toolbox commands

Project toolbox commands live in `.refact/toolbox_commands/*.yaml`. A command defines its id, description, optional selection size limits, and the messages sent to the model.

````yaml
schema_version: 1
id: simplify_api

description: Simplify selected API code
selection_needed: [1, 80]

messages:
  - role: user
    content: |
      @file %CURRENT_FILE%:%CURSOR_LINE%
      Simplify the selected code while preserving behavior:
      ```
      %CODE_SELECTION%
      ```
````

Useful variables include `%CURRENT_FILE%`, `%CURSOR_LINE%`, and `%CODE_SELECTION%`. Keep toolbox commands prompt-oriented. Use command-line integrations for external programs and service integrations for long-running processes.

## Related pages

- [AI Chat](/features/ai-chat/) for broader Q&A and agent workflows.
- [Context](/features/context/) for local context and indexing behavior.
- [Agent Tools](/features/autonomous-agent/tools/) for multi-step tool use.
