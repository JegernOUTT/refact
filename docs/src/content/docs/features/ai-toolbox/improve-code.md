---
title: Improve Code
description: Use toolbox commands to rewrite selected code for clarity.
---

Improvement workflows are best handled as custom toolbox commands or by the default `/shorter` command when the goal is concision. They operate on the selected code and return a focused rewrite.

Use this style of command for:

- Reducing unnecessary nesting.
- Simplifying conditionals.
- Removing repeated expressions.
- Making a small block easier to read.

Use Agent mode for larger refactors that must update call sites, tests, or multiple files.
