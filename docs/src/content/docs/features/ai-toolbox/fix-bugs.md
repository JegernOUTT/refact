---
title: Fix Bugs
description: Find likely issues in a selected code block.
---

The default `/bugs` toolbox command asks the model to inspect the selected code for likely defects and suggest a fix. It works best on short, self-contained selections.

Good candidates include:

- Incorrect identifiers or expressions.
- Edge cases in conditionals or loops.
- Simple API misuse.
- Obvious missing validation or error handling.

For bugs that require reproducing behavior, reading multiple files, or running tests, use Debug or Agent mode instead.
