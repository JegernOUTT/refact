## Autonomous agents & tools
Refact is not a one-shot autocomplete box; it is a live agent loop that can inspect a repo, change it, run the result, read the fallout, and try again. The fun part: the same brain that streams an answer can pause for permission, fan out tool calls, preserve signed thinking blocks, and keep working like a tiny dev team in a trench coat.

- Real session runtime: chats move through `Idle → Generating → ExecutingTools → WaitingUserInput/Paused → Completed/Error`, so a request can become an iterative edit-test-debug cycle instead of a single reply.
- Search stack with teeth: `tree`, `cat`, regex search, AST `symbol_def`, semantic search, trajectory/knowledge lookup, and project memory let the agent triangulate code by path, structure, and meaning.
- Editing is first-class and guarded: `apply_patch`, `create_textdoc`, `update_textdoc*`, `mv`, `rm`, and `undo_textdoc` produce concrete workspace mutations, with confirmation gates and limited auto-approval for patch-like edits.
- Runtime tools close the loop: `shell`, `process_start`, `process_read`, `process_wait`, `process_kill`, PTY `process_write_stdin`, `sleep`, and `cron_*` let agents run commands, monitor services, drive interactive processes, and schedule follow-ups.
- Delegation is built in: `subagent`, `delegate`, `strategic_planning`, `deep_research`, and `code_review` can split exploration, review, and implementation work across specialized agent passes.
- Web and browser reach: web fetch/search plus Chrome automation tools can inspect external docs, reproduce UI flows, and bring live evidence back into the chat loop.
- Streaming stays structured: content, reasoning, thinking blocks, citations, server content blocks, and tool calls are merged as typed deltas; compatible tools can run in parallel and report back without flattening the transcript into mush.

| Need | Tool families |
| --- | --- |
| Understand | `tree`, `cat`, regex, AST, semantic search, knowledge |
| Change | `apply_patch`, `create/update_textdoc`, `mv`, `rm`, `undo` |
| Prove | `shell`, `process_*`, PTY stdin, browser, cron |

→ Deep dive: [Agent Modes](https://github.com/JegernOUTT/refact/wiki/Agent-Modes), [Agent Tools](https://github.com/JegernOUTT/refact/wiki/Agent-Tools)
