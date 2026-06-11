# refact-tui

`refact-tui` is a protocol-only ratatui client for the Refact daemon. It talks to the daemon through `/daemon/v1/*` and project-scoped `/p/{project_id}/v1/*` endpoints.

## Codex TUI attribution

This crate uses `competitors/codex/codex-rs/tui` as the reference implementation. The following files contain code adapted from `openai/codex` `codex-rs/tui`, licensed under Apache-2.0:

- `src/vendored/markdown_stream.rs`
- `src/vendored/line_truncation.rs`
- `src/vendored/decoded_text_merge.rs`

The first TUI slice ports only the protocol-agnostic streaming and rendering helpers. The app loop, daemon client, project picker, and terminal guard are local implementations following the codex event-loop and terminal-restore patterns without importing codex protocol, auth, app-server, cloud, or onboarding code.

## Manual smoke

```bash
cd refact-agent/engine
REFACT_SKIP_GUI_BUILD=1 REFACT_DAEMON_WORKER_CMD="python3 tests/fake_worker.py" cargo run --bin refact -- tui --project .
```

Type a prompt and press Enter to stream the fake worker response, press Esc during a turn to send abort, and press Ctrl-Q to restore the terminal and exit cleanly.

