---
title: Add Console Logs
description: Use toolbox-style prompts to instrument selected code for debugging.
---

A toolbox command can be configured to add temporary logging or tracing statements to selected code. This is useful when you know exactly which block needs instrumentation.

Use a debugging toolbox workflow for small edits such as:

- Adding console logs around a suspicious branch.
- Printing function inputs or return values.
- Marking execution paths before running a local check.

For multi-file diagnosis, stack traces, tests, or debugger sessions, use Debug or Agent mode so Refact can gather context and run tools.
