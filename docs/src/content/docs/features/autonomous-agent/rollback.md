---
title: Agent Rollback
description: Preview and restore workspace checkpoints created during agent workflows.
---

Refact can create checkpoints around agent edits so you can review or restore workspace changes later. This is useful when experimenting with a fix, testing a refactor, or comparing alternative approaches.

## What rollback does

Rollback restores files to an earlier checkpoint. It can remove changes made by the agent and any manual changes made after that checkpoint, so always review the preview before confirming.

## Using checkpoints

1. Enable checkpoints for the chat or mode when available.
2. Let the agent make changes.
3. Open the checkpoint or rollback action from the relevant chat message.
4. Review the preview of files that would change.
5. Confirm only if the preview matches what you want to restore.

## Good practices

- Commit or stash important manual work before restoring an old checkpoint.
- Use rollback for experiments, not as a replacement for version control.
- Ask the agent to summarize what changed before deciding whether to restore.
- Re-run verification after restoring a checkpoint.

## Related pages

- [Agent Overview](/features/autonomous-agent/overview/)
- [Agent Tools](/features/autonomous-agent/tools/)
