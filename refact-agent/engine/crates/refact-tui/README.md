# refact-tui

`refact-tui` is a protocol-only ratatui client for the Refact daemon. It talks to the daemon through `/daemon/v1/*` and project-scoped `/p/{project_id}/v1/*` endpoints.

## Codex TUI attribution

This crate uses `competitors/codex/codex-rs/tui` as the reference implementation. The following files contain code adapted from `openai/codex` `codex-rs/tui`, licensed under Apache-2.0:

- `src/vendored/markdown_stream.rs`
- `src/vendored/line_truncation.rs`
- `src/vendored/decoded_text_merge.rs`
- `src/streaming/chunking.rs`
- `src/streaming/commit_tick.rs`
- `src/streaming/controller.rs`
- `src/streaming/table_holdback.rs`

The first TUI slice ports only the protocol-agnostic streaming and rendering helpers.
The first TUI slice ports only the protocol-agnostic streaming and rendering helpers. The app loop, daemon client, project picker, and terminal guard are local implementations following the codex event-loop and terminal-restore patterns without importing codex protocol, auth, app-server, cloud, or onboarding code.

The first TUI slice ports only the protocol-agnostic streaming and rendering helpers. The app loop, daemon client, project picker, and terminal guard are local implementations following the codex event-loop and terminal-restore patterns without importing codex protocol, auth, app-server, cloud, or onboarding code.
| Card | Matrix area | Codex reference | Refact TUI target and status |
|---|---|---|---|
| C-1 | Full protocol model | `chatwidget/protocol.rs`, `chatwidget/protocol_requests.rs`, `thread_transcript.rs` | Replace ad hoc JSON matching with typed Refact SSE events and delta ops: `snapshot`, `stream_started`, `stream_delta`, `stream_finished`, `message_*`, `runtime_updated`, `pause_*`, `queue_updated`, `ack`, and `DeltaOp` variants. C-0 fixtures lock the current contract. |
| C-2 | Streaming pipeline | `streaming/controller.rs`, `streaming/commit_tick.rs`, `streaming/chunking.rs`, `streaming/table_holdback.rs`, `markdown_stream.rs` | Match Codex's stable incremental commit behavior for text, split code fences, and Markdown tables while preserving Refact reasoning, citations, usage, thinking blocks, and server content blocks. Current status: basic MarkdownStreamCollector wiring exists; parity work continues here. |
| C-3 | Scrollback model | `thread_transcript.rs`, `pager_overlay.rs`, `resize_reflow_cap.rs`, `wrapping.rs`, `chatwidget/transcript.rs` | Move from simple `Vec<TranscriptItem>` + `scroll_offset` to native scrollback with resize-safe reflow, raw/copy mode, and bounded history. This is the riskiest rendering state card. |
| C-4 | Markdown and diff rendering | `markdown.rs`, `markdown_render.rs`, `markdown_render/table_key_value.rs`, `diff_render.rs`, `diff_model.rs`, `line_truncation.rs` | Bring Codex-grade Markdown tables, code fences, wrapping, syntax-neutral styling, and unified diff cells into Refact's renderer. Current status: minimal Markdown renderer and diff-colored tool output. |
| C-5 | Rich transcript cells | `history_cell/*`, `chatwidget/transcript.rs`, `approval_events.rs` | Introduce structured cells for assistant turns, reasoning, compression/system notices, approval events, hidden-event summaries, and message grouping. Current status: flat enum cells. |
| C-6 | Tool lifecycle cells | `chatwidget/tool_lifecycle.rs`, `chatwidget/tool_requests.rs`, `chatwidget/exec_state.rs`, `status_indicator_widget.rs` | Expand `ToolCard` into Codex-like running/success/error/server-hosted tool lifecycle cells with stable ids, collapse state, output truncation, and status labels. Current status: stable id replacement and basic result cards. |
| C-7 | Composer | `public_widgets/composer_input.rs`, `chatwidget/input_submission.rs`, `chatwidget/input_restore.rs`, `clipboard_paste.rs`, `file_search.rs` | Add multiline editor parity: cursor movement, history restore, paste handling, file mention completion, slash completion entry point, and submit queue awareness. Current status: basic text buffer with Shift-Enter newline. |
| C-8 | Popups and pickers | `chatwidget/model_popups.rs`, `chatwidget/permission_popups.rs`, `chatwidget/settings_popups.rs`, `chatwidget/keymap_picker.rs`, `theme_picker.rs`, `oss_selection.rs` | Replace minimal model/mode pickers with reusable modal/popup surfaces for models, permissions, settings, keymaps, themes, projects, and command palettes. |
| C-9 | Input queue and turn runtime | `chatwidget/input_queue.rs`, `chatwidget/turn_runtime.rs`, `chatwidget/interrupts.rs`, `chatwidget/command_lifecycle.rs` | Match Codex behavior when prompts arrive during active turns: queue visibility, cancellation, retry/interrupt paths, and command completion lifecycle. Current status: local editable queue owns queued prompt order; daemon `queue_updated`/`queued_items` is rendered passively because server-side queued commands cannot be edited before dispatch. |
| C-10 | Approvals UX | `approval_events.rs`, `chatwidget/permission_popups.rs`, `chatwidget/tool_requests.rs` | Codex-style approval overlay with FIFO requests, full-args toggle, allow once/chat, deny, and patch-tool auto-approval. Current status: FIFO approval queue and modal keymap exist. |
| C-11 | Session ops | `session_archive_commands.rs`, `thread_transcript.rs`, `chatwidget/session_flow.rs`, `chatwidget/replay.rs`, `app_backtrack.rs` | Add session list/resume/fork/archive/new-chat/backtrack parity over Refact daemon/chat APIs. Current status: Ctrl-N new local chat and project open only. |
| C-12 | Backtrack, overlays, external editor | `app_backtrack.rs`, `pager_overlay.rs`, `external_editor.rs`, `clipboard_copy.rs`, `get_git_diff.rs` | Add transcript backtrack, pager overlays, open-in-editor flows, copy-last-response, raw scrollback, and diff overlays. |
| C-13 | Usage and status footer | `status_indicator_widget.rs`, `chatwidget/status_surfaces.rs`, `chatwidget/status_state.rs`, `chatwidget/status_controls.rs`, `service_tier_resolution.rs`, `chatwidget/rate_limits.rs` | Match Codex status surfaces for model/mode, worker/daemon state, queue/busy state, usage totals, service tier, and rate-limit warnings. Current status: footer shows project/model/mode/state/daemon/worker and fixture-proven usage totals. |
| C-14 | Keymap, Vim, theme | `key_hint.rs`, `keymap_setup/*`, `tui/keyboard_modes.rs`, `theme_picker.rs`, `style.rs`, `terminal_title.rs` | Add discoverable key hints, configurable keymap, Vim composer mode, theme picker, title/statusline settings, and host-friendly colors. |
| C-15 | Slash commands: core/session | `slash_command.rs`, `chatwidget/slash_dispatch.rs` | Adopt Refact-native `/new`, `/clear`, `/quit`, `/exit`, `/model`, `/permissions`, `/keymap`, `/vim`, `/status`, `/debug-config`, `/copy`, `/raw`, `/diff`, and `/mention`. |
| C-16 | Slash commands: workflow/integrations | `slash_command.rs`, `chatwidget/slash_dispatch.rs`, `chatwidget/skills.rs`, `chatwidget/hooks.rs`, `chatwidget/mcp_startup.rs` | Adopt or translate `/review`, `/plan`, `/goal`, `/agent`, `/subagents`, `/side`, `/btw`, `/skills`, `/hooks`, `/memories`, `/mcp`, `/apps`, `/plugins`, `/ps`, and `/stop`. |
| C-17 | Slash commands: settings/debug/edge | `slash_command.rs`, `debug_config.rs`, `local_chatgpt_auth.rs`, `config_update.rs`, `terminal_title.rs` | Adopt, translate, or explicitly defer the remaining Codex commands: `/rename`, `/resume`, `/fork`, `/archive`, `/init`, `/compact`, `/ide`, `/theme`, `/title`, `/statusline`, `/pets`, `/personality`, `/realtime`, `/settings`, `/feedback`, `/logout`, `/rollout`, `/approve`, `/test-approval`, `/app`, `/experimental`, `/setup-default-sandbox`, `/sandbox-add-read-dir`, `/debug-m-drop`, and `/debug-m-update`. |

### Slash command inventory

Current Codex `SlashCommand` inventory has 54 enum entries plus aliases such as `clean` for `/stop`, `pet` for `/pets`, `subagents` for `/multi-agents`, `setup-default-sandbox`, and `sandbox-add-read-dir`. Refact TUI should not blindly clone Codex-only behavior; each command is either adopted as native Refact behavior, translated to an existing daemon/chat command, or documented as deferred.

- C-15 adopts core/session and inspection commands: `/new`, `/clear`, `/quit`, `/exit`, `/model`, `/permissions`, `/keymap`, `/vim`, `/status`, `/debug-config`, `/copy`, `/raw`, `/diff`, `/mention`.
- C-16 adopts workflow/integration commands where Refact has matching backend features: `/review`, `/plan`, `/goal`, `/agent`, `/subagents`, `/side`, `/btw`, `/skills`, `/hooks`, `/memories`, `/mcp`, `/apps`, `/plugins`, `/ps`, `/stop`.
- C-17 handles settings and Codex-specific leftovers: `/rename`, `/resume`, `/fork`, `/archive`, `/init`, `/compact`, `/ide`, `/theme`, `/title`, `/statusline`, `/pets`, `/personality`, `/realtime`, `/settings`, `/feedback`, `/logout`, `/rollout`, `/approve`, `/test-approval`, `/app`, `/experimental`, `/setup-default-sandbox`, `/sandbox-add-read-dir`, `/debug-m-drop`, `/debug-m-update`.

## Golden protocol fixture harness

Offline protocol fixtures live in `tests/fixtures/*.jsonl`. Each line is one Refact chat SSE JSON payload. `tests/tui_protocol.rs` loads a fixture, converts lines to `ChatEvent`, runs `ChatSeqTracker`, drives `App::apply_chat_event`, and asserts transcript/app state without a daemon, terminal, or network.

Fixture coverage:

| Fixture | Coverage |
|---|---|
| `assistant_streaming.jsonl` | `snapshot`, assistant `stream_started`, split Markdown table/code-fence `append_content`, `stream_finished`, usage totals |
| `reasoning.jsonl` | `append_reasoning` interleaved with assistant content |
| `tool_calls.jsonl` | `set_tool_calls`, stable tool id, `message_added` tool result |
| `approvals.jsonl` | `pause_required` and approval modal state |
| `usage_updates.jsonl` | `runtime.usage`, `set_usage`, and final usage update |
| `citations.jsonl` | `add_citation` delta visibility |
| `extra_updates.jsonl` | `merge_extra` updates preserved on the transcript model |
| `thinking_blocks.jsonl` | `set_thinking_blocks` delta visibility |
| `server_content_blocks.jsonl` | `add_server_content_block` delta visibility |
| `snapshot_resume.jsonl` | snapshot/resume rebuild from persisted user/assistant/tool messages |
| `seq_gap.jsonl` | sequence gap recovery without applying the gap delta |
| `unknown_delta_ops.jsonl` | unknown delta ops preserved and tolerated without stopping later deltas |

## Render snapshot strategy

Rendered output snapshots use `ratatui::backend::TestBackend` in plain `cargo test`. The test harness renders the real `ui::render` into an in-memory buffer, normalizes rows with trailing spaces trimmed, and compares a deterministic string snapshot. This starts with one assistant-streaming snapshot and can later move to `insta` if the crate adopts snapshot files.

## Native scrollback

C-3 uses `ratatui::Viewport::Inline` by default. Finalized transcript cells are rendered once, queued through `history::HistoryBuffer`, and inserted with `Terminal::insert_before`, so the transcript lives in the terminal's native scrollback and mouse copy works on real terminal text. The frame render path only redraws the inline live region: active stream tail, running tools, approvals, composer, and footer.

Set `REFACT_TUI_ALT_SCREEN=1` to use the previous alternate-screen/full-transcript fallback for terminals or CI environments where inline viewport behavior is not usable. In fallback mode, the flat transcript remains in the frame buffer and PageUp/PageDown keep the legacy local scroll behavior.

Resize policy matches Codex: pending finalized cells re-render at the current width before insertion, while content already inserted into native scrollback keeps the width it had when inserted.

## Manual smoke

```bash
cd refact-agent/engine
REFACT_SKIP_GUI_BUILD=1 REFACT_DAEMON_WORKER_CMD="python3 tests/fake_worker.py" cargo run --bin refact -- tui --project .
```

Type a prompt and press Enter to stream the fake worker response, press Esc during a turn to send abort, and press Ctrl-Q to restore the terminal and exit cleanly.
