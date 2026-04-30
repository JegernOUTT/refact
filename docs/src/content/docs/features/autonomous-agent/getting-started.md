---
title: How to Start Using Refact Agent
description: Start an agent workflow in your IDE.
---

Refact Agent is the chat workflow for tasks that require multiple steps, tools, and code changes.

## Start a workflow

1. Open Refact chat in your IDE.
2. Choose an agent-capable mode, such as Agent, Quick Agent, Review, Debug, Plan, or a project-specific mode.
3. Describe the goal and include constraints, files, commands, or acceptance criteria.
4. Let the agent gather context. It may read files, search symbols, inspect history, or ask a clarifying question.
5. Review tool confirmations and patch previews before applying sensitive changes.
6. Ask the agent to run the project's normal verification command before you finish.

## Good prompts

Be specific about the outcome you want:

- "Find why this test fails, fix it, and run the test again."
- "Refactor this component without changing behavior; update tests if needed."
- "Review the current diff for correctness and security issues."
- "Plan the migration first, then wait before editing files."

## Tips

- Start with Explore or Plan if the task is broad or risky.
- Attach the current file or selected snippet when the problem is local.
- Keep generated files, migrations, and credentials out of scope unless the agent specifically needs them.
- Use checkpoints or rollback when experimenting with large edits.
