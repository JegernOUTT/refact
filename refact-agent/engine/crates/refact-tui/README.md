# refact-tui

`refact-tui` is a protocol-only ratatui client for the Refact daemon. It talks to the daemon through `/daemon/v1/*` and project-scoped `/p/{project_id}/v1/*` endpoints.

## Codex TUI attribution

This crate uses `competitors/codex/codex-rs/tui` as the reference implementation. The following files contain code adapted from `openai/codex` `codex-rs/tui`, licensed under Apache-2.0:

- `src/vendored/markdown_stream.rs`
- `src/vendored/line_truncation.rs`
- `src/vendored/decoded_text_merge.rs`
- `src/vendored/terminal_hyperlinks.rs`
- `src/streaming/chunking.rs`
- `src/streaming/commit_tick.rs`
- `src/streaming/controller.rs`
- `src/streaming/table_holdback.rs`
- `src/table_detect.rs`

The first TUI slice ports only the protocol-agnostic streaming and rendering helpers. The app loop, daemon client, project picker, and terminal guard are local implementations following the Codex event-loop and terminal-restore patterns without importing Codex protocol, auth, app-server, cloud, or onboarding code.

## Parity matrix closure

Every Wave C row is now dispositioned. ✅ rows have native Refact TUI implementation in the linked module(s). ❌ rows are explicitly deferred with the blocking reason; no command or matrix row is left as a silent no-op.

| Card | Matrix area | Codex reference | Final Refact TUI disposition |
|---|---|---|---|
| C-1 | Full protocol model | `chatwidget/protocol.rs`, `chatwidget/protocol_requests.rs`, `thread_transcript.rs` | ✅ Done: typed Refact SSE handling and `DeltaOp` coverage live in `src/protocol/mod.rs`, `src/client.rs`, `tests/tui_protocol.rs`. |
| C-2 | Streaming pipeline | `streaming/controller.rs`, `streaming/commit_tick.rs`, `streaming/chunking.rs`, `streaming/table_holdback.rs`, `markdown_stream.rs` | ✅ Done: stable incremental stream commit behavior lives in `src/streaming/*` with fixture coverage in `tests/tui_protocol.rs`. |
| C-3 | Scrollback model | `thread_transcript.rs`, `pager_overlay.rs`, `resize_reflow_cap.rs`, `wrapping.rs`, `chatwidget/transcript.rs` | ✅ Done: native scrollback insertion and resize policy live in `src/history/mod.rs`, `src/app.rs`, and `src/ui/mod.rs`; fallback is gated by `REFACT_TUI_ALT_SCREEN=1`. |
| C-4 | Markdown and diff rendering | `markdown.rs`, `markdown_render.rs`, `markdown_render/table_key_value.rs`, `diff_render.rs`, `diff_model.rs`, `line_truncation.rs` | ✅ Done: Markdown, wrapping, highlight, and unified diff rendering live in `src/render/*` and `src/history/cells.rs`. |
| C-5 | Rich transcript cells | `history_cell/*`, `chatwidget/transcript.rs`, `approval_events.rs` | ✅ Done: assistant, reasoning, notices, info, plan, diff, approval, tool, and session cells live in `src/history/cells.rs`. |
| C-6 | Tool lifecycle cells | `chatwidget/tool_lifecycle.rs`, `chatwidget/tool_requests.rs`, `chatwidget/exec_state.rs`, `status_indicator_widget.rs` | ✅ Done: stable tool card lifecycle, status labels, collapse state, and output truncation live in `src/tools.rs` and `src/history/cells.rs`. |
| C-7 | Composer | `public_widgets/composer_input.rs`, `chatwidget/input_submission.rs`, `chatwidget/input_restore.rs`, `clipboard_paste.rs`, `file_search.rs` | ✅ Done: multiline composer, cursor movement, history restore, paste handling, queue awareness, file mentions, and slash popup entry live in `src/composer/*` and `src/app.rs`. |
| C-8 | Popups and pickers | `chatwidget/model_popups.rs`, `chatwidget/permission_popups.rs`, `chatwidget/settings_popups.rs`, `chatwidget/keymap_picker.rs`, `theme_picker.rs`, `oss_selection.rs` | ✅ Done: reusable modal picker surfaces for projects, models, modes, permissions, sessions, slash commands, file mentions, and themes live in `src/pickers.rs` and `src/app.rs`; the capability-aware per-chat settings surface is in `src/app/surfaces/settings.rs`. Settings, modes, and permissions require `REFACT_TUI_SURFACES=1`. |
| C-9 | Input queue and turn runtime | `chatwidget/input_queue.rs`, `chatwidget/turn_runtime.rs`, `chatwidget/interrupts.rs`, `chatwidget/command_lifecycle.rs` | ✅ Done: local editable queue, cancellation, dispatch-after-finish, and passive daemon queue state live in `src/composer/queue.rs` and `src/app.rs`. Abort keeps queued inputs and dispatches the next queued prompt once the abort command succeeds while idle; deleting a selected queue item remains the explicit queue-removal path. |
| C-10 | Approvals UX | `approval_events.rs`, `chatwidget/permission_popups.rs`, `chatwidget/tool_requests.rs` | ✅ Done: FIFO approval modal, details toggle, allow once/chat, deny, and patch auto-approval live in `src/approvals.rs` and `src/app.rs`. The `ask_questions` answer form opens when a successful pending ask_questions tool result puts the chat in `waiting_user_input`; Esc closes the form and leaves the composer available for manual fallback. |
| C-11 | Session ops | `session_archive_commands.rs`, `thread_transcript.rs`, `chatwidget/session_flow.rs`, `chatwidget/replay.rs`, `app_backtrack.rs` | ✅ Done: `/new`, `/resume`, `/fork`, `/rename`, `/archive`, snapshot resume, and backtrack/retry live in `src/commands/session.rs`, `src/sessions.rs`, and `src/app.rs`. |
| C-12 | Backtrack, overlays, external editor | `app_backtrack.rs`, `pager_overlay.rs`, `external_editor.rs`, `clipboard_copy.rs`, `get_git_diff.rs` | ✅ Done: Esc-Esc backtrack, transcript pager/raw overlay, OSC52 terminal clipboard, external editor, and git diff loading live in `src/overlay.rs`, `src/clipboard.rs`, `src/terminal.rs`, `src/app.rs`, and `src/commands/workflow.rs`. |
| C-13 | Usage and status footer | `status_indicator_widget.rs`, `chatwidget/status_surfaces.rs`, `chatwidget/status_state.rs`, `chatwidget/status_controls.rs`, `service_tier_resolution.rs`, `chatwidget/rate_limits.rs` | ✅ Done: model/mode, worker/daemon state, queue/busy state, usage totals, and `/status` info card live in `src/ui/footer.rs`, `src/app.rs`, and `src/commands/session.rs`; ❌ service-tier/rate-limit controls are deferred because the Refact daemon does not expose that surface to the TUI. |
| C-14 | Keymap, Vim, theme | `key_hint.rs`, `keymap_setup/*`, `tui/keyboard_modes.rs`, `theme_picker.rs`, `style.rs`, `terminal_title.rs` | ✅ Done: configurable keymap, generated help, Vim composer mode, built-in themes, live `/theme`, themed UI styles, and terminal-title control live in `src/keymap.rs`, `src/theme.rs`, `src/ui/*`, `src/commands/misc.rs`, and `src/terminal.rs`. Terminal title defaults on for a TTY and is configured with top-level `terminal_title` or `[terminal].title`; `REFACT_TUI_TERMINAL_TITLE` overrides either. ❌ terminal statusline preferences remain deferred. |
| C-15 | Slash commands: core/session | `slash_command.rs`, `chatwidget/slash_dispatch.rs` | ✅ Done: `/new`, `/clear`, `/quit`, `/exit`, `/model`, `/permissions`, `/keymap`, `/vim`, `/status`, `/debug-config`, `/copy`, `/raw`, `/diff`, and `/mention` are in the registry. Implemented commands link to `src/commands/session.rs`, `src/commands/workflow.rs`, `src/commands/misc.rs`, and `/copy` writes the last assistant response via OSC52. |
| C-16 | Slash commands: workflow/integrations | `slash_command.rs`, `chatwidget/slash_dispatch.rs`, `chatwidget/skills.rs`, `chatwidget/hooks.rs`, `chatwidget/mcp_startup.rs` | ✅ Done: `/review`, `/plan`, `/goal`, `/agent`, `/diff`, `/compact`, `/mention`, and `/stop` live in `src/commands/workflow.rs` and `src/app.rs`; `/subagents` has a local card by default and opens its Activity surface behind `REFACT_TUI_SURFACES=1`; `/mcp`, `/skills`, `/hooks`, and `/memories` open read-only overlays; `/ps` aliases `/events`. ❌ `/side`, `/btw`, `/apps`, and `/plugins` remain explicit unavailable commands. |
| C-17 | Slash commands: settings/debug/edge | `slash_command.rs`, `debug_config.rs`, `local_chatgpt_auth.rs`, `config_update.rs`, `terminal_title.rs` | ✅ Done: `/rename`, `/title`, `/resume`, `/fork`, `/archive`, `/init`, `/compact`, `/theme`, `/events`, `/help`, `/keymap`, `/vim`, `/debug-config`, `/raw`, `/quit`, `/settings`, and `/logout` have deterministic handlers. ❌ Codex-only `/ide`, `/statusline`, `/pets`, `/personality`, `/realtime`, `/feedback`, `/rollout`, `/approve`, `/test-approval`, `/app`, `/experimental`, `/setup-default-sandbox`, `/sandbox-add-read-dir`, `/debug-m-drop`, and `/debug-m-update` are explicit unavailable commands with one-line reasons. |

### Slash command inventory closure

Commands are searchable in the slash popup via `src/commands/mod.rs`. The behavior contract is registry-wide:

- `BackendCommand`: sent only for supported backend pass-through commands such as `/stop`.
- `OpenPicker`: opens a deterministic modal picker.
- `LocalToggle` or `ShowInfo`: changes local TUI state or displays generated information.
- `Session`, `Workflow`, or `Misc`: invokes a concrete app handler.
- `Unavailable`: reports the reason as a visible notice.

Current adopted commands and aliases:

| Command | Final disposition |
|---|---|
| `/new` | ✅ session handler in `src/commands/session.rs` |
| `/clear` | ✅ local transcript clear in `src/commands/misc.rs` |
| `/quit`, `/exit` | ✅ local quit action in `src/commands/misc.rs` |
| `/model` | ✅ model picker via daemon caps |
| `/mode`, `/tool-use` | ✅ mode picker via daemon modes; requires `REFACT_TUI_SURFACES=1` |
| `/permissions`, `/approval` | ✅ permissions picker using supported per-chat flags; requires `REFACT_TUI_SURFACES=1` |
| `/keymap` | ✅ generated help from the active keymap registry |
| `/vim` | ✅ live composer Vim-mode toggle |
| `/status` | ✅ daemon, worker, session, model, and usage info card |
| `/debug-config`, `/debug` | ✅ TUI config path/theme/vim/registry info card |
| `/copy` | ✅ copies the last assistant response via OSC52 terminal clipboard, with tmux passthrough when `$TMUX` is set |
| `/raw` | ✅ raw transcript pager/copy mode overlay |
| `/diff` | ✅ local `git diff --no-ext-diff --` rendered as rich diff cell |
| `/mention`, `/file`, `/files` | ✅ file mention picker reuse |
| `/review` | ✅ structured review prompt |
| `/plan` | ✅ generated local plan cell from hidden plan messages |
| `/goal` | ✅ shows the current goal; set, update, budget, pause, resume, and stop controls require `REFACT_TUI_SURFACES=1` |
| `/agent` | ✅ backend `set_params` patch for Agent mode |
| `/compact` | ✅ structured compaction prompt fallback |
| `/stop`, `/cancel`, `/clean` | ✅ abort active generation |
| `/theme` | ✅ theme picker or direct built-in theme apply |
| `/events`, `/ps` | ✅ local daemon events/workers pane toggle |
| `/resume`, `/sessions`, `/history` | ✅ recent session picker |
| `/fork`, `/branch` | ✅ branch current chat |
| `/rename`, `/title` | ✅ set chat title |
| `/archive`, `/remove` | ✅ delete current chat from recent sessions |
| `/init` | ✅ structured bootstrap prompt |
| `/subagents`, `/multi-agents` | ✅ local active/recent subagent card; opens Activity when `REFACT_TUI_SURFACES=1` |
| `/side` | ❌ deferred: no Refact daemon side-conversation command |
| `/btw` | ❌ deferred: no background side-note routing command |
| `/skills` | ✅ read-only overlay listing available skills and slash commands |
| `/hooks` | ✅ read-only overlay listing configured hooks |
| `/memories` | ✅ read-only overlay showing knowledge-graph memory entries and summary |
| `/mcp` | ✅ read-only overlay showing configured MCP servers, status, and tools |
| `/apps` | ❌ deferred: no Refact daemon apps surface |
| `/plugins` | ❌ deferred: plugin marketplace not exposed in TUI |
| `/ide` | ❌ deferred: IDE attach state not exposed in TUI |
| `/statusline` | ❌ deferred: terminal statusline preferences not exposed in TUI |
| `/pets`, `/pet` | ❌ deferred: Buddy pets are GUI-only |
| `/personality` | ❌ deferred: Buddy personality settings are GUI-only |
| `/realtime` | ❌ deferred: realtime voice controls are GUI-only |
| `/settings` | ✅ capability-aware per-chat settings surface; requires `REFACT_TUI_SURFACES=1` |
| `/feedback` | ❌ deferred: no TUI feedback endpoint |
| `/logout` | ✅ backend action that clears provider OAuth credentials |
| `/browser` | ✅ browser status, frame, timeline, and context-selection surface; requires `REFACT_TUI_SURFACES=1` |
| `/board`, `/tasks` | ✅ read-only task-board surface; requires `REFACT_TUI_SURFACES=1` |
| `/worktrees`, `/worktree` | ✅ inspect, diff, merge, or clean worktrees; requires `REFACT_TUI_SURFACES=1` |
| `/rollout` | ❌ deferred: no rollout-control endpoint |
| `/approve` | ❌ deferred: approval decisions happen in the approval modal or `/permissions` picker |
| `/test-approval` | ❌ deferred: synthetic approval injection is not release TUI behavior |
| `/app` | ❌ deferred: no app chooser command surface |
| `/experimental` | ❌ deferred: Codex experimental flags have no Refact equivalent |
| `/setup-default-sandbox` | ❌ deferred: Codex sandbox defaults do not apply to Refact daemon chats |
| `/sandbox-add-read-dir` | ❌ deferred: Codex sandbox read dirs do not apply to Refact daemon chats |
| `/debug-m-drop` | ❌ deferred: Codex model-debug mutation has no Refact backend surface |
| `/debug-m-update` | ❌ deferred: Codex model-debug mutation has no Refact backend surface |

## Golden protocol fixture harness

Offline protocol fixtures live in `tests/fixtures/*.jsonl`. Each line is one Refact chat SSE JSON payload. `tests/tui_protocol.rs` loads a fixture, converts lines to `ChatEvent`, runs `ChatSeqTracker`, drives `App::apply_chat_event`, and asserts transcript/app state without a daemon, terminal, or network. `fixture_directory_covers_required_protocol_cases` is the exact manifest: adding, renaming, or removing a fixture requires updating both that test and this register.

### Disposition rules

- Transcript roles and content blocks produce a visible transcript/history cell, plan/goal cell, tool card, notice, or event-pane record.
- Runtime, queue, background-agent, process, IDE, and browser envelopes are retained in typed app state. Their interactive Activity and Browser presentation belongs to the later surface cards, so retention without a normal transcript cell is intentional.
- Malformed authoritative envelopes request resubscription before state changes. Unknown SSE envelopes and delta operations are retained and visibly noticed; unknown content parts and roles retain sanitized payload text in a visible fallback.

### Exact fixture register

| Fixture | Coverage and disposition |
|---|---|
| `all_roles.jsonl` | All 15 wire roles: transcript-visible user/assistant/tool/notice/system/context/diff/plain-text/instruction/compression/error/unknown, plan and goal cells, and retained `event` in the event pane. |
| `approvals.jsonl`, `sse_pause_cleared.jsonl` | `pause_required` creates approval state; `pause_cleared` clears its scoped approval while the runtime state remains authoritative. |
| `assistant_streaming.jsonl`, `assistant_message_added_dedup.jsonl` | Snapshot, stream lifecycle, split Markdown table/code fence, final usage, and persisted assistant deduplication. |
| `reasoning.jsonl` | `append_reasoning` plus `set_reasoning` replacement during an assistant stream. |
| `tool_calls.jsonl`, `subchat_turn_cleanup.jsonl` | `set_tool_calls`, tool-result `message_added`, `subchat_update`, turn-end status cleanup, and attached files. |
| `citations.jsonl`, `thinking_blocks.jsonl`, `server_content_blocks.jsonl` | Citation, signed thinking, and server-block delta retention with visible content-block cells. |
| `redacted_thinking.jsonl`, `refusal.jsonl`, `image_part.jsonl`, `file_part.jsonl`, `audio_part.jsonl`, `unknown_content_part.jsonl` | Every retained content-block family: redaction marker, refusal text, multimodal placeholders, and sanitized unknown-content fallback. |
| `extra_updates.jsonl`, `usage_updates.jsonl` | `merge_extra`, `set_usage`, runtime usage, and final usage replacement. |
| `protocol_mutations.jsonl` | `thread_updated`, `message_updated`, `message_removed`, and `messages_truncated`. |
| `snapshot_resume.jsonl`, `snapshot_recovery_content.jsonl` | Persisted snapshot reconstruction, sequence-gap recovery, and stable-ID native-scrollback replacement without duplicate cells. |
| `seq_gap.jsonl`, `malformed_stream_delta.jsonl`, `malformed_authoritative.jsonl` | Sequence-gap recovery and fail-closed malformed stream/snapshot/ack/truncation contracts without transcript mutation. |
| `sse_snapshot_auxiliary.jsonl`, `sse_background_agent_updated.jsonl` | Snapshot and incremental background-agent state retention. |
| `sse_ack.jsonl` | Accepted command correlation acknowledgement. |
| `sse_process_completed.jsonl`, `sse_ide_tool_required.jsonl` | Typed process completion and IDE-tool-request state retention. |
| `sse_queue_updated.jsonl` | Passive daemon queue size and preview retention; never mutates the local editable queue. |
| `sse_runtime_updated.jsonl` | Runtime state plus goal budget/progress, compression, and usage retention. |
| `sse_browser_frame.jsonl`, `sse_browser_status.jsonl`, `sse_browser_closed.jsonl`, `sse_browser_timeline.jsonl`, `sse_browser_context_oversize.jsonl`, `sse_browser_toolbar_action.jsonl` | Typed browser state/event retention; Browser surface rendering is intentionally deferred. |
| `unknown_delta_ops.jsonl`, `unknown_sse_event.jsonl` | Unknown operations and envelopes remain forward-compatible: raw state is retained and a visible notice is emitted while following sequence events still apply. |

### Outbound command register

`client::tests` captures the JSON body sent through the project-scoped command endpoint. Every TUI chat command wrapper has an exact wire-shape assertion:

| Command family | Wire types |
|---|---|
| Conversation control | `branch_from_chat`, `user_message`, `retry_from_index`, `abort`, `regenerate` |
| Thread mutations | `set_params`, `update_message`, `remove_message`, `restore_messages` |
| Goal control | `set_goal`, `set_goal_budget`, `update_goal`, `goal_control` |
| Tool and runtime responses | `ide_tool_result`, `tool_decisions`, `clean_background_processes`, `browser_context_decision` |

The singular tool-decision wrapper deliberately serializes the shared plural `tool_decisions` envelope. `set_goal` omits `budget` for unlimited goals, and only `pause`, `resume`, and `stop` are valid goal-control actions.

## Render snapshot strategy

Rendered output snapshots use `ratatui::backend::TestBackend` in plain `cargo test`. The test harness renders the real `ui::render` into an in-memory buffer, normalizes rows with trailing spaces trimmed, and compares a deterministic string snapshot. This starts with one assistant-streaming snapshot and can later move to `insta` if the crate adopts snapshot files.

### Degradation rules

The TUI has one text-first degradation contract, exercised by the render-parity matrix:

1. **Colour has a textual counterpart.** Colour may emphasize state, but every state remains identifiable through a glyph or text label in `NO_COLOR` output.
2. **Modals lose their frame below 40 columns.** Their title, content, and controls remain; only the border is removed.
3. **Two-column surfaces stack below 60 columns.** The daemon-events and workers dock becomes vertical instead of squeezing either field away.
4. **Docks keep their highest-priority leading field.** At `30×10` or smaller, header, transcript, status, composer, and footer shorten their suffixes before their leading field; an open secondary dock yields to the transcript.
5. **Box drawing becomes ASCII below 60 columns.** `┌─┐│└┘`-style frame characters become `+-|`, so narrow terminals that cannot display box drawing keep structural meaning.
6. **Nothing vanishes silently.** At `30×10` or smaller, the layout reserves two transcript rows and renders `… content truncated` whenever compact layout omits overflow.

`tests/tui_render_parity.rs` runs all 19 registered scenarios at `120×40`, `96×30`, `60×20`, and `40×15` with truecolor, ANSI-16, and `NO_COLOR` environments: idle, streaming, running and failed tools, approval, ask form, error turn, goal dock, mode transition, history surface, history events, Activity, a 500-turn trajectory, mid-resize, post-reconnect, image fallback, task board, Browser, and worktree identity. A registered scenario must supply an expected visible marker; the harness fails immediately if it does not, preventing silent snapshot gaps. Image content uses its textual fallback in this matrix.

Run the matrix with `RUST_TEST_THREADS=1 cargo test -p refact-tui --test tui_render_parity`. The harness changes process-global terminal environment variables for each color mode and enables the surface rollout flag for its Activity and Browser scenarios, so serial execution keeps the matrix isolated from other environment-mutating tests.

## Runtime configuration

- `REFACT_DAEMON_URL` takes precedence over the launcher-provided daemon URL. `REFACT_DAEMON_TOKEN`, when set, is supplied as the explicit daemon token.
- `REFACT_TUI_SURFACES=1` enables the rollout surfaces and controls: settings, mode and permissions pickers, goal mutations, Activity, Browser, task board, and worktree operations. The accepted enable values are `1`, `true`, `yes`, and `on`, case-insensitively; the default is disabled. `/subagents` still shows its local activity card without the flag.
- `REFACT_TUI_ALT_SCREEN=1` uses the alternate-screen transcript fallback instead of the default inline viewport. `REFACT_TUI_HYPERLINKS=1` or `0` forces OSC8 hyperlink probing on or off.
- `REFACT_TUI_SIXEL` opts into sixel image output when color is enabled. Kitty and iTerm2 image protocols are detected from terminal environment data; unsupported terminals retain the textual image fallback.
- `REFACT_TUI_TERMINAL_TITLE=1` or `0` overrides the terminal-title setting. `REFACT_TUI_REDUCED_MOTION` enables reduced motion, and `REFACT_TUI_NOTIFY` can disable notifications with `quiet`, `silent`, `none`, `disabled`, or a false value.

## Native scrollback

C-3 uses `ratatui::Viewport::Inline` by default. Finalized transcript cells are rendered once, queued through `history::HistoryBuffer`, and inserted with `Terminal::insert_before`, so the transcript lives in the terminal's native scrollback and mouse copy works on real terminal text. The frame render path only redraws the inline live region: active stream tail, running tools, approvals, composer, and footer.

Set `REFACT_TUI_ALT_SCREEN=1` to use the previous alternate-screen/full-transcript fallback for terminals or CI environments where inline viewport behavior is not usable. In fallback mode, the flat transcript remains in the frame buffer and PageUp/PageDown keep the legacy local scroll behavior.

Resize policy matches Codex: pending finalized cells re-render at the current width before insertion, while content already inserted into native scrollback keeps the width it had when inserted.
Resize reflow is capped to 1,000 pending finalized cells per frame. Extra pending cells remain queued and render on later frames, so resize cannot force unbounded transcript rewrapping.

Markdown links carry hyperlink metadata beside visible ratatui lines. OSC8 bytes are emitted only at the terminal output boundary, so wrapping and width calculations see plain visible text. `NO_COLOR`, `TERM=dumb`, and unsupported terminals keep the same styled visible text without OSC8; `REFACT_TUI_HYPERLINKS=1` or `0` overrides probing.

Recovery snapshots replace the inline live region and pending finalized cells using revision-aware transcript keys, so changed content with stable message ids is rendered while identical snapshots do not enqueue duplicate cells. Finalized cells already inserted into native terminal scrollback are intentionally left as-is; the live transcript and future pending insertions follow the latest snapshot.

## External editor

External editor support reads `$EDITOR` first, then `$VISUAL`, parses the value as a shell-style command line for quoted arguments, and executes the program directly with the temporary draft path appended. If neither variable is set, the TUI falls back to `vi` when it is available on `PATH`.

## Manual smoke

```bash
cd refact-agent/engine
REFACT_SKIP_GUI_BUILD=1 REFACT_DAEMON_WORKER_CMD="python3 tests/fake_worker.py" cargo run --bin refact -- tui --project .
```

This smoke command intentionally skips refreshing embedded GUI assets; use the normal engine build when those assets must be bundled. Type a prompt and press Enter to stream the fake worker response, press Esc during a turn to send abort, press F2 or `/events` to toggle daemon events/workers, use `/theme` to apply a built-in theme, and press Ctrl-Q or `/quit` to restore the terminal and exit cleanly.
