# Refact Agent Engine

Binary: `refact-lsp` — AI coding agent, HTTP + LSP server. Rust 2021 edition, async/tokio.

## Stack

Axum (HTTP), tower-lsp (LSP), tree-sitter (AST), SQLite + vec0 (VecDB), LMDB/Heed (AST store), git2, headless_chrome, whisper-rs (optional, feature-gated), rmcp (MCP).

## Build

```bash
cargo build --release                    # binary at target/release/refact-lsp
cargo build --release --features voice   # with Whisper transcription
cargo test --lib && cargo test --doc
bash tools/compile_bench.sh               # compile-time before/after benchmark
```

Release profile: `opt-level = "z"`, `lto = true`, `strip = true`, `codegen-units = 1`.

## Architecture

`GlobalContext` (`Arc<ARwLock<GlobalContext>>`) is the central shared state. HTTP server (Axum) and LSP server (tower-lsp) both hold a reference. Background tasks (AST indexer, VecDB, git shadow cleanup, knowledge graph, trajectory memos, agent monitor, OAuth refresh) are spawned via `start_background_tasks()` (~12 tokio tasks).

### Source Layout

```
src/
  main.rs              — entry point, CLI (--http-port, --lsp-stdin-stdout, --ast, --vecdb, etc.)
  global_context.rs    — SharedGlobalContext
  lsp.rs               — tower-lsp LanguageServer impl
  http/routers/v1/     — 27+ endpoint modules
  chat/                — 22+ files, ~15K LOC (session, queue, generation, tools, trajectories, linearize, stream_core, etc.)
  llm/                 — LLM adapters (OpenAI, Anthropic wire formats), streaming
  tools/               — 50+ tools (file_edit/, search, web, shell, subagent, knowledge, tasks)
  ast/                 — tree-sitter indexing, 7 parsers (C/C++, Python, Java, Kotlin, JS, Rust, TS)
  vecdb/               — SQLite vec0 semantic search
  providers/           — 15+ LLM providers (Anthropic, OpenAI, Codex, DeepSeek, Gemini, Groq, LM Studio, Ollama, OpenRouter, vLLM, xAI, Claude Code, custom)
  integrations/        — GitHub, GitLab, Bitbucket, Chrome, PostgreSQL, MySQL, Docker, PDB, cmdline, services, MCP (stdio+SSE)
  knowledge_graph/     — petgraph DiGraph, builder/cleanup/staleness/query
  scratchpads/         — FIM code completion (PSM/SPM), RAG, multimodality
  tasks/               — Kanban task board (planning/active/paused/completed/abandoned)
  caps/                — model capabilities resolution
  git/                 — shadow repos, checkpoints
  voice/               — Whisper transcription, streaming sessions
  yaml_configs/        — defaults for modes, providers, toolbox commands, prompts
  postprocessing/      — token-aware truncation, AST prioritization
  agentic/             — commit messages, agentic edit flows
  buddy/               — Buddy agent runtime (actor, jobs, observers, chat_reactions, diagnostics)
  daemon/              — headless daemon (CLI, client, auth, config, chat client)
  exec/                — unified exec runtime (PTY, spawn, registry, spill)
  scheduler/           — cron expression, delivery, exec actions, jitter
  ext/                 — extensions marketplace, hooks runner, competitor import
  at_commands/         — `@`-prefixed IDE commands (file, search, ast_definition, knowledge)
  bin/refact.rs        — alternate entry point
```

## Chat System

### Session State Machine

`SessionState` enum: `Idle`, `Generating`, `ExecutingTools`, `Paused`, `WaitingIde`, `WaitingUserInput`, `Completed`, `Error`.

### Modes

| Mode | Purpose |
|------|---------|
| `NO_TOOLS` | Plain chat |
| `EXPLORE` | Context gathering with quick tools |
| `AGENT` | Autonomous task execution, full toolset |
| `TASK_PLANNER` | Kanban board management |
| `TASK_AGENT` | Execute task cards |

### SSE Events

Subscribe: `GET /p/{project_id}/v1/chats/subscribe?chat_id={id}` (project-scoped proxy path used by daemon frontends via `daemon::chat_client::ProxyChatClient`; the worker itself serves `/v1/chats/subscribe`). Events have monotonic `seq: u64`.

Key types: `Snapshot`, `StreamStarted`, `StreamDelta`, `StreamFinished`, `MessageAdded`, `MessageUpdated`, `MessageRemoved`, `MessagesTruncated`, `ThreadUpdated`, `QueueUpdated`, `RuntimeUpdated`, `PauseRequired`.

Background process completion is represented by a hidden `event(process_completed)` message delivered through `MessageAdded`. If a future dedicated `ProcessCompleted` envelope is reintroduced, keep it additive to `MessageAdded` and document it in both engine and GUI AGENTS before clients depend on it.

### Commands

`POST /v1/chats/{chat_id}/commands` — queued processing.

Variants (Rust enum names; on the wire they are flattened JSON objects with `type` in snake_case): `UserMessage` (`user_message`), `SetParams` (`set_params`, payload under `patch`), `SetGoal` (`set_goal`, `content`), `UpdateGoal` (`update_goal`, `note`), `GoalControl` (`goal_control`, `action: pause|resume|stop`), `UpdateMessage`, `RemoveMessage`, `TruncateMessages`, `RetryFromIndex`, `Abort` (`abort`), `ApproveTools` / `RejectTools` (combined as `tool_decisions` with `decisions: [{tool_call_id, accepted}]`), `RestoreMessages`, `BranchFromChat`, `RestoreFromTrajectory`, `ClearDraft`, `SetDraft`, `Regenerate`. All carry `client_request_id`; optional `priority` is accepted. Goal command types are shared in `refact-chat-api`; do not import similarly named Buddy conductor goal types.

### Delta Operations

`AppendContent`, `AppendReasoning`, `SetToolCalls`, `SetThinkingBlocks`, `AddCitation`, `AddServerContentBlock`, `SetUsage`, `MergeExtra`.

### Message Flow

```
UserMessage → queue → prepare (system prompt, knowledge RAG, history limit) → linearize → LLM stream → StreamCollector → tool calls → loop
```

- **`linearize.rs`**: merges consecutive user messages, strips thinking blocks for LLM cache compatibility.
- **`stream_core.rs`**: `merge_thinking_blocks()` — deduplicates by (type,index) → (type,id) → (type,signature); signatures are opaque, latest-wins replacement.
- **`history_limit.rs`**: tool-call repair + history validation (`fix_and_limit_messages_history`) plus shared helpers: `compute_context_budget` (counts every message including plan roles, tool-call arguments, thinking blocks, ~1K tokens per image), `pressure_for_used_tokens` (70/85/95% → Low/Medium/High/Critical), `compress_duplicate_context_files` (keeps the `context_file` role on tool-answering messages so pairs stay valid). `CompressionStrength`: Absent/Low/Medium/High.

### Compression contract

Chat compression produces first-class chat messages and runtime status, not trailing instructions.

- `compression_report` is a visible message role for deterministic/reactive compression reports. It stores Markdown content plus `extra.compression_report = { kind: "chat_compression_report", context_files_removed, context_messages_dropped, tool_results_truncated, tokens_before, tokens_after, estimated_tokens_saved, reduction_percent }`, uses `summarization_tier: "tier2_reactive"`, and is preserved by model-switch/new-thread sanitization.
- Trajectory compression and manual `compress_chat_apply` must share `build_compression_report_message()` (or the fingerprinted variant) and `insert_compression_report_at_boundary()`. The helper inserts the report at the earliest affected boundary while staying after the entire leading control prefix (`system`/`event`/`goal`/`plan`) and after the first user when present, and never between an assistant's tool calls and their results. Report equivalence for idempotent removal is decided by the stable `op_fingerprint` when both reports carry one (deterministic compaction hashes changed message ids, `compress_chat_apply` hashes its op lists, `compress_in_place` hashes options + affected ids); fingerprint-less legacy pairs still compare by metrics, and mixed pairs are never equivalent.
- Runtime compression status is carried on both `ChatSession` and `RuntimeState` as `is_compressing`, `compression_phase`, and `compression_reason`, and is emitted through `RuntimeUpdated` and snapshots.
- Active compression phases are `checking` and `running`; terminal phases are `applied`, `skipped`, and `failed`. Active phases must set/keep `is_compressing` consistently, while terminal runtime transitions clear stale active phases and preserve already-terminal phase/reason metadata.
- Segment summarization writes compressed assistant messages with `extra.compression.kind === "llm_segment_summary"`; these summaries remain visible UI artifacts and are excluded from repeated summarization by source metadata.
- The proactive per-iteration gate (`proactive_gate_quiet`) is event-silent: routine skips (low pressure, no eligible segment, attempt limits) record `compression_phase`/`compression_reason` on the session without emitting `RuntimeUpdated`, so the GUI indicator does not blink every generation round. Forced (reactive) compression keeps full Checking/Running/terminal event emission.
- The proactive pressure gate uses the provider-visible estimate (`estimated_provider_context_pressure_with_usage`, post-linearize) rather than the raw transcript, because source-preserving summaries keep their sources in the session and the raw estimate only grows. The provider-usage side of the pressure is also computed from the linearized view, and `ChatSession.provider_usage_stale` (set by every compression apply path and `reset_compaction_runtime_state`, cleared on the next successful generation) suppresses stale pre-compression usage records so compression does not retrigger on its own output.
- Quiet compression status writes (`set_compression_status_quiet`) never mutate state while another reserved attempt is active, and `compression_attempt_active` treats attempts older than 15 minutes (`compression_attempt_started_at_ms`) as stale so an aborted/cancelled summarizer cannot wedge `is_compressing` forever.
- Source-preserving summary suppression is hash-validated at linearize time: when all summarized sources are present but the recomputed `source_hash` mismatches, the summary is stale — it is dropped from the provider wire and the sources stay visible. When sources are not fully present (handoff/branch flows carry summaries without their sources on purpose), the summary stays visible as carried context and suppresses nothing. Hash-less legacy summaries keep the trust-the-ids path. `remove_message`/`truncate_messages` invalidate referencing summaries like `update_message` does (in-thread orphans are removed at mutation time), and new-thread/model-switch sanitization recomputes summary + report `source_hash` over the fully-present sanitized sources.
- Candidates below `MIN_SOURCE_TOKENS_FOR_COMPRESSION` are skipped before any summarizer LLM call, and source hashes whose summaries produced insufficient savings are remembered per session (`compression_insufficient_hashes`, cleared on edit/remove/truncate/replace) so the same useless LLM call is never repeated.
- Applied report + summary pairs are emitted immediately as `MessageAdded` events (plus the final snapshot), so reports stay visible even if a later pass aborts.
- The reactive context-limit breaker (`thread.reactive_compact_attempts`, max 2) is per-episode: it resets on any successful generation, on `reset_compaction_runtime_state` (manual `ctx_apply`, restores, external reloads), and trajectory load clamps persisted values to 1 so a reloaded thread always has one attempt available.
- When LLM segment summarization cannot deliver on a context-limit error (failed, skipped, or breaker exhausted), `apply_deterministic_compaction_for_recovery` runs: it truncates old non-preserved tool outputs (keeps the most recent `4` *eligible* outputs — preserved/`srvtoolu_`/`TOOLS_TO_PRESERVE`/ui-only results neither consume the recent window nor get truncated; redacts before truncating), dedups context files, inserts a `compression_report`, resets `previous_response_id` and forces the cache guard, then generation retries. Idempotency is size-based (already-truncated outputs fall under the size threshold), so raw output that merely starts with the truncation marker is still compacted.
- `compress_duplicate_context_files` treats a newer attachment of the same path with the same line range as superseding older copies (the file may have changed); across different ranges the largest copy still wins, with newer preferred on size ties. Wire preparation (`fix_and_limit_messages_history`) relocates stray tool results to sit contiguously after their assistant call, and history validation rejects results separated from their call by a `user`/`assistant`/`system` barrier.
- Forced context-limit compression that still cannot apply appends a visible `event(system_notice, "chat.summarizer")` whose content starts with `Context compression failed:`; context-overflow threads must never fail without a visible explanation. The final error message points at `ctx_probe()`/`ctx_apply()`.
- Provider length stops (`finish_reason: length`-class) are auto-recovered in `maybe_recover_after_length_stop`: empty/near-empty outputs (e.g. reasoning ate the budget) drop the dead-end assistant message, boost `max_new_tokens` once via `pending_max_new_tokens_boost` (16K, only when the user has not set `thread.max_tokens`), append a `cd_instruction` marker (`length_stop_continue`), and retry; partial cuts append a continue instruction and retry. The marker bound is checked **before** any compaction and is authoritative: recovery is bounded by marker count per user turn (max 2), every retry — including compaction-driven retries on the user-`max_tokens` partial path — appends a marker, and exhaustion appends a visible ui-only error without running compaction. High-pressure empty stops still trigger forced compaction first when the budget allows.
- Editing a message via `update_message` removes any segment summaries (and their reports, matched by `source_hash`) whose `summarized_source_message_ids` reference the edited message, so stale summaries never keep suppressing changed sources.
- `compression_report` never reaches the provider wire: prepare filters it, linearize drops it, and `render_extra::is_context_role` excludes it; the role is GUI-only.
- Compressed/truncated tool previews and public summarization failure text must redact sensitive values before truncating or persisting preview text, so split secrets cannot leak through length caps.
- Manual `compress_chat_apply` must follow trajectory compression placement, report-deduplication, memory-path detection, redaction, path-preservation, and provider-order tool-call cleanup rules. It only mutates the selected modifiable prefix; preserved and active tails remain byte-identical except for insertion of the current compression report at the boundary.

### Hidden message roles

The chat thread can contain hidden internal roles that are stored in trajectories and SSE snapshots but are not rendered as normal chat turns:

| Role | Stored shape | Purpose | GUI default |
|---|---|---|---|
| `event` | `extra.event = { subkind, source, payload }` plus human-readable `content` | Internal facts such as mode switches, tool decisions, plan deltas, goal deltas, goal pursuit, cron fires, process exits, ticks, verifier reports, and notices | Hidden from normal transcript; shown in EventLog except `plan_delta`, `goal_delta`, and `goal_pursuit` |
| `goal` | `extra.goal = { mode, version, created_at_ms, supersedes, active, status?, budget, progress?, attempts?, events?, transferred_from?, transferred_to?, truncated?, original_chars? }` plus Markdown `content` | Single install-once base goal; body is capped at 96KB (`MAX_GOAL_BODY_CHARS`) and owns the active goal projection | Hidden from normal transcript; projected into `Snapshot.goal`, runtime goal fields, TUI `/goal`, and GUI TaskProgressWidget |
| `event(goal_delta)` | `extra.event = { subkind: "goal_delta", source, payload: { seq, truncated?, original_chars?, kept_chars?, at_ms? } }` plus Markdown `content` | Append-only goal updates; note content is capped at 16KB (`MAX_GOAL_DELTA_CHARS`) | Hidden from normal transcript and EventLog; merged into the synthesized goal/get_goal |
| `event(goal_pursuit)` | `extra.event = { subkind: "goal_pursuit", source, payload: { kind, at_ms?, gaps?, account_progress? } }` plus human-readable `content` | Internal pursuit facts such as verifier verdicts, re-arm gaps, monitor nudges, budget stops, and transfer notices | Hidden from normal transcript and EventLog; contributes to `GoalSnapshot.events` and TaskProgressWidget history |
| `plan` | `extra.plan = { mode, version, created_at_ms, supersedes, truncated?, original_chars? }` plus Markdown `content` | Single install-once base plan; body is capped at 96KB (`MAX_PLAN_BODY_CHARS`) | Hidden from normal transcript; latest shown in PlanBanner |
| `event(plan_delta)` | `extra.event = { subkind: "plan_delta", source, payload: { seq, summary?, truncated?, original_chars?, kept_chars? } }` plus Markdown `content` | Append-only plan updates; note content is capped at 16KB (`MAX_PLAN_DELTA_CHARS`) | Hidden from normal transcript and general EventLog; merged into PlanBanner/get_plan |

`EventSubkind` serializes in snake_case. Current subkinds:

| Subkind | Typical source | Compression rule |
|---|---|---|
| `mode_switch` | `chat.session` | DropOnAge |
| `tool_decision` | `chat.session` | PreserveWindow |
| `ide_callback` | `ide.bridge` | PreserveWindow |
| `process_completed` | `exec.registry` | KeepRecentN |
| `cron_fire` | `scheduler.cron` | KeepRecentN |
| `tick` | `tool.sleep` | DropOnAge |
| `summarization_marker` | `chat.summarizer` | PreserveAnchor |
| `verifier_report` | `chat.verifier` | PreserveWindow |
| `cancellation_note` | cancellation paths | PreserveAnchor |
| `system_notice` | assorted internal emitters | PreserveAnchor |
| `plan_delta` | `tool.update_plan` | Never |
| `goal_delta` | `tool.update_goal` / `chat.command.update_goal` | Never |
| `goal_pursuit` | `chat.goal_verifier` / `chat.goal_monitor` / transition helpers | PreserveAnchor |

Compression rules live in `crates/refact-chat-history/src/compression_exemption.rs`: `plan`, `goal`, `event(plan_delta)`, and `event(goal_delta)` are `Never` and must never be compressed, truncated by compression, or dropped; `event(goal_pursuit)` is `PreserveAnchor`; non-event/non-plan/non-goal roles are `PreserveAnchor`; unknown event subkinds default to `PreserveAnchor`. Keep the table above in sync when adding a subkind.

Wire mapping rules: provider adapters must never send literal `event`, `goal`, or `plan` roles. Normal `event` lowers to provider-visible user context with structured `<event subkind="..." source="...">` framing. Base `goal` lowers as `<goal mode="..." version="...">...` and must appear before `<plan>` whenever both are present; `event(goal_delta)` lowers as append-only `<goal-update seq="...">...` blocks. Base `plan` lowers as `<plan mode="..." version="...">...`; `event(plan_delta)` lowers as append-only `<plan-update seq="...">...` blocks. This keeps cached base goal/plan bytes stable while still exposing synthesized current goal and plan text to the model. `event(goal_pursuit)` remains a generic event block. Preserve Anthropic thinking/signature block order across hidden-role lowering.

### Plan tools

#### `set_plan`

Model-facing prompt: "Install the chat's single detailed implementation plan (Markdown). Provide exactly one of `content` (full plan body) or `path` (absolute path to a `.md` report). Fails if a plan already exists — use `update_plan` to evolve it."

Schema:

```json
{
  "type": "object",
  "properties": {
    "content": { "type": "string", "description": "Full Markdown plan body. Optional; provide exactly one of content or path." },
    "path": { "type": "string", "description": "Absolute path to a .md report to install as the plan" },
    "summary": { "type": "string", "description": "Short description of what changed, ≤120 chars. Optional." }
  },
  "required": []
}
```

Returns `{ "version": 1, "supersedes": null }`, queues one hidden `plan`, and appends `event(system_notice, "tool.set_plan", {version, summary}, "Plan updated to v1")`. It rejects missing/non-string arguments, rejects calls that provide both or neither of `content` and `path`, rejects empty content, rejects `summary` longer than 120 chars, and rejects any second install before queuing with `a plan already exists; use update_plan to change it`. The stored base plan is capped at 96KB chars and records truncation metadata when capped. Available by default in `agent`, `task_planner`, and `task_agent` modes.

Example:

```json
{"content":"## Plan\n- Inspect scheduler docs\n- Update runbooks","summary":"Document scheduler surface"}
```

#### `update_plan`

Model-facing prompt: "Append an incremental update to the current plan (cache-safe delta merged into the current plan). Use when the plan evolves; it does not rewrite the original plan."

Schema:

```json
{
  "type": "object",
  "properties": {
    "note": { "type": "string", "description": "Plan update note. Required." },
    "summary": { "type": "string", "description": "Short description of what changed, ≤120 chars. Optional." }
  },
  "required": ["note"]
}
```

Returns `{ "seq": number, "truncated": false }` for normal notes. Notes are capped at 16KB chars (`MAX_PLAN_DELTA_CHARS`); when capped, the result is `{ "seq": number, "truncated": true, "original_chars": number, "kept_chars": number }`. It queues one hidden `event(plan_delta, "tool.update_plan", {seq, summary, truncated?, original_chars?, kept_chars?}, note)`, and appends `event(system_notice, "tool.update_plan", {seq, summary}, "Plan updated (delta N)")`. It requires an existing or queued base plan, rejects empty `note`, and rejects `summary` longer than 120 chars. `plan_delta` is append-only, snake_case on the wire, `Never` compressed, hidden from the normal transcript and general EventLog, and merged with the base plan for current-plan consumers.

#### `get_plan`

Model-facing prompt: "Read the current plan installed on this chat. Returns the merged current content, mode, base version, creation timestamp, and delta count."

Schema:

```json
{ "type": "object", "properties": {}, "required": [] }
```

Returns `{ "plan": null }` when no plan is installed or `{ "plan": { "content", "mode", "version", "created_at_ms", "delta_count" } }`. `content` is synthesized from the base `plan` plus append-only `plan_delta` notes; the base plan bytes are not rewritten.

### Goal tools

Goal state has two synchronized representations: hidden `goal` + `event(goal_delta|goal_pursuit)` messages in chat history, and the `GoalSnapshot` projection exposed on snapshots, runtime updates, trajectories, and GUI state. All shared goal API types live in `crates/refact-chat-api/src/lib.rs`.

#### `set_goal`

Model-facing prompt: "Install the chat's single active goal. Fails if a goal already exists — use `update_goal` to evolve it."

Schema:

```json
{
  "type": "object",
  "properties": {
    "content": { "type": "string", "description": "Full goal body. Required." }
  },
  "required": ["content"]
}
```

Returns `{ "version": 1, "supersedes": null }`, queues one hidden `goal`, and appends `event(system_notice, "tool.set_goal", {version}, "Goal updated to v1")`. It rejects missing/non-string/empty `content` and rejects any second install with `goal already exists; use update_goal`. The stored base goal is capped at 96KB chars and records truncation metadata when capped. Available by default in `agent`, `task_planner`, and `task_agent` modes.

#### `update_goal`

Model-facing prompt: "Append an incremental update to the current goal. Use when the goal evolves; it does not rewrite the original goal."

Schema:

```json
{
  "type": "object",
  "properties": {
    "note": { "type": "string", "description": "Goal update note. Required." }
  },
  "required": ["note"]
}
```

Returns `{ "seq": number, "truncated": false }` for normal notes. Notes are capped at 16KB chars (`MAX_GOAL_DELTA_CHARS`); when capped, the result is `{ "seq": number, "truncated": true, "original_chars": number, "kept_chars": number }`. It queues one hidden `event(goal_delta, "tool.update_goal", {seq, truncated?, original_chars?, kept_chars?}, note)`. It requires an existing or queued base goal and rejects empty `note`.

#### `get_goal`

Model-facing prompt: "Read the current goal installed on this chat. Returns merged goal content, status, version, delta count, budget counters, latest verifier verdict, and gaps."

Schema:

```json
{ "type": "object", "properties": {}, "required": [] }
```

Returns `{ "goal": null }` when no goal is installed or `{ "goal": { "content", "status", "version", "delta_count", "turns_used", "tokens_used", "latest_verdict", "gaps" } }`. `content` is synthesized from the base `goal` plus append-only `goal_delta` notes; the base goal bytes are not rewritten.

### Goal pursuit contract

`GoalSnapshot` fields are `content`, `version`, `active`, `status`, `budget`, `progress`, `attempts`, `events`, `transferred_from`, and `transferred_to`. `active=true` means ownership, not current execution; pursuit is allowed only when `active && status == Active`. `GoalStatus` serializes as `active`, `verifying`, `paused`, `completed`, `stopped`, `budget_exhausted`, `no_progress`, or `transferred`.

`RuntimeState` and `RuntimeUpdated` mirror `goal_active`, `goal_status`, `goal_turns_used`, `goal_tokens_used`, and `goal_no_progress_turns`. `Snapshot` carries `goal: Option<GoalSnapshot>`. `ChatCommand::{SetGoal, UpdateGoal, GoalControl}` mutates the hidden messages/projection through the queue and persists trajectories. GUI command dispatchers send `set_goal`, `update_goal`, and `goal_control`.

Verifier-on-done gates finish-like tools (`task_done`, `finish`, `agent_finish`) before the session reaches `Completed`. Active goals move to `verifying`; the verifier runs in a cache-reusing fork of the parent chat with one hidden verification prompt. `GOAL: MET` records an attempt, emits `event(goal_pursuit, kind=verified)`, marks the goal `completed`, and then completes the session. `GOAL: UNMET` or an inconclusive verifier records gaps, emits `event(goal_pursuit, kind=verification_gaps)`, re-arms the goal as `active`, and enqueues priority `Regenerate` without emitting a completed runtime or Buddy event.

The goal monitor nudges stalled active goal owners with hidden `event(goal_pursuit, kind=nudge, account_progress=true)` events and priority regeneration. Budget windows track max turns, minutes, tokens, cooldown, low-output no-progress turns, and no-progress token thresholds. Turn-end accounting occurs after assistant output and can mark terminal `budget_exhausted` or `no_progress` status instead of completing.

Ownership transfer on `handoff_to_mode` and mode-transition routes carries the current goal into the target chat before any plan messages, resets the target progress window, sets `transferred_from`, and marks the source `active=false, status=transferred, transferred_to=<target_chat_id>`. Restart/reload rehydrates the same owner from `TrajectorySnapshot.goal` and hidden goal messages; it must not synthesize a transfer event.

### Plan transitions

`handoff_to_mode` and mode-transition endpoints create a pinned `initial-plan` task document when transitioning into Task Planner with an `initial_plan`. The document is created with kind `plan`, role `planner`, and `pinned=true`; failures are non-blocking and reported/logged without mutating the source chat's cached provider state.

### Anthropic Thinking/Signatures

Thinking blocks with cryptographic signatures must be preserved verbatim — no JSON rebuilding, no field reordering. Signatures validate exact prior content-block sequence. During streaming, accumulate deltas preserving metadata (block_index, signature) separately from text. For multi-provider chats, strip provider-specific blocks (thinking/signatures) on model switch. `strip_thinking_blocks_if_disabled()` in prepare.rs removes them when model lacks reasoning support.

### Trajectories

Stored: `.refact/trajectories/{chat_id}.json`. Atomic writes (`.tmp` → rename). Rich JSON: id, title, model, mode, tool_use, messages, `goal`, task_meta, version, created_at, reasoning_effort, checkpoints_enabled, parent_id, root_chat_id, etc. `goal` is a serialized `GoalSnapshot` projection and is rebuilt from hidden goal messages on restore/restart when possible.

OpenAI conversion lives in `src/llm/adapters/openai_chat.rs` (`convert_messages_to_openai()`).

## Tools

~50+ tools, filtered by mode/capabilities/config. Registered in `tools_list.rs`.

**Categories**: Codebase search (AST defs, tree, cat, regex, semantic) · Codebase change (create/update/rm/mv/undo/apply_patch — confirmation required) · Web (fetch, search, Chrome automation) · Code execution (shell, process_*, sleep, cron_*) · System integrations (cmdline_*, service_*) · Knowledge (search, create, trajectories) · Agent (subagent, strategic_planning, deep_research, code_review) · Task management (~18 tools) · IDE (open_file, paste_text) · Integration-defined + MCP tools.

Tool trait: `tool_execute(&mut self, ccx, tool_call_id, args) -> Result<(bool, Vec<ContextEnum>)>`.

`AtCommandsContext` provides: global_context, chat_id, n_ctx, abort_flag, messages, current_model, task_meta, subchat depth/channels, postprocess params.

### Exec runtime — PTY and process tools

The unified exec runtime owns foreground commands, background processes, and services. `shell` and `process_start` both accept `tty: bool` (default `false`):

- `tty: false` uses normal stdout/stderr pipes. Streams remain separate and output buffering follows pipe behavior.
- `tty: true` runs through the PTY path. It exposes an interactive stdin writer and combines stdout/stderr into the `combined` stream. Use it for REPLs, prompts, interactive CLIs, and programs that only flush when connected to a terminal.
- PTY output is transcripted through the same bounded runtime buffers as pipe output. PTY can change command behavior; do not turn it on for plain builds/tests unless needed.
- Windows uses the portable PTY backend (ConPTY where available). If the host cannot allocate a PTY, the tool must fail clearly rather than silently falling back to pipes.

#### `shell`

Model-facing prompt includes: run a command, `description` is required, `tty` enables PTY behavior, `run_in_background` returns immediately and points to process tools.

Schema highlights: `command: string` required, `description: string` required, optional `workdir`, optional `timeout`, optional output filters, optional `tty: boolean = false`, optional `run_in_background: boolean = false`.

Examples:

```json
{"command":"npm test","description":"Run frontend tests"}
{"command":"python3 -i","description":"Start Python REPL","tty":true,"run_in_background":true}
```

Edge cases: `description` must be non-empty; numeric `timeout` must be a positive integer; `run_in_background` skips the foreground timeout path and returns a process id; do not append `&` when using `run_in_background`.

#### `process_start`

Model-facing prompt: start a runtime-owned background or service process and return its process ID, initial status, output cursor, and metadata.

Schema highlights: `command: string`, `description: string`, optional `mode: "background" | "service"` (default `background`), optional `service_name` for services, optional `workdir`, optional `startup_wait_ms`, `startup_wait_port`, `startup_wait_keyword`, optional `tty: boolean = false`.

Examples:

```json
{"command":"npm run dev","description":"Start dev server","mode":"service","service_name":"web","startup_wait_port":5173}
{"command":"bash","description":"Open interactive shell","tty":true}
```

Edge cases: service mode requires `service_name`; duplicate running services in the same owner/workspace are rejected; workdir is resolved through active worktree privacy rules.

#### `process_list`

Schema: optional `status: "running" | "completed" | "all"` (default `running`), optional `scope: "chat" | "workspace" | "all"` (default `chat`). Returns process summaries under `extra.exec.processes`.

#### `process_read`

Schema highlights: `process_id: string` required, optional `since_seq`, optional `stream: "stdout" | "stderr" | "combined" | "all"`, optional output filters. It returns transcript chunks and cursor metadata (`since_seq`, `next_seq`, `latest_seq`) under `extra.exec.transcript`.

Empty-output reads are normal for long-running processes that have not emitted new chunks yet. Use the returned cursor for the next poll.

#### `process_wait`

Schema highlights: `process_id: string` required, optional `timeout_ms`, optional output filters. Waits until terminal status or timeout, then returns final/partial transcript metadata.

#### `process_kill`

Schema: `{ "process_id": "exec_..." }`. Kills a runtime-owned process and returns its terminal metadata. Use before restarting a service with the same name.

#### `process_write_stdin`

Planned/contracted tool for PTY processes. Schema:

```json
{
  "type": "object",
  "properties": {
    "process_id": { "type": "string" },
    "chars": { "type": "string", "default": "" },
    "yield_time_ms": { "type": "integer", "default": 250, "maximum": 10000 }
  },
  "required": ["process_id"]
}
```

Behavior contract: require a `tty=true` process, write `chars` bytes to stdin, then wait up to `yield_time_ms` for new output or exit. `chars: ""` means poll only: do not write, just wait and return new chunks. Output metadata should include standard `extra.exec` fields plus `bytes_written` and `chunks_returned`.

Example:

```json
{"process_id":"exec_123","chars":"echo hi\n","yield_time_ms":500}
```

Edge cases: reject non-PTY processes with a clear error; cap `yield_time_ms`; preserve exact bytes, including newlines/control characters.

### Background process notifications

`ExecRegistry` emits a completion event on the first terminal transition for background/service processes with an owning `chat_id`. `chat::notifications` subscribes from background tasks, waits until the chat is idle if generation/tool execution is active, then appends:

```json
{
  "role": "event",
  "content": "Process <description> exited with code 0",
  "extra": {
    "event": {
      "subkind": "process_completed",
      "source": "exec.registry",
      "payload": {
        "process_id": "exec_...",
        "status": "exited",
        "exit_code": 0,
        "duration_ms": 1234,
        "short_description": "Run dev server"
      }
    }
  }
}
```

Foreground processes and records without `chat_id` do not inject notifications. Closed/missing chats are dropped cleanly. Current SSE delivery is the ordinary `MessageAdded` envelope carrying the hidden event; keep any future dedicated `ProcessCompleted` envelope additive and update GUI docs/tests together.

### `sleep`

Model-facing prompt: "Wait for the specified duration. User-interruptible at any time. Use when you have nothing to do, when waiting for something, or when the user asks you to pause. Prefer this over Bash(sleep ...) — it doesn't hold a shell process. You can call this concurrently with other tools."

Schema:

```json
{
  "type": "object",
  "properties": {
    "duration_ms": { "type": "integer", "minimum": 100, "maximum": 3600000 },
    "tick_interval_ms": { "type": "integer", "minimum": 5000 },
    "description": { "type": "string", "description": "Short description (≤80 chars)." }
  },
  "required": ["duration_ms", "description"]
}
```

Returns `{ "slept_ms": number, "interrupted": boolean }`. If `tick_interval_ms` is set, it injects `event(tick, "tool.sleep", {elapsed_ms, remaining_ms}, "tick")` at each interval. Edge cases: duration max is 1 hour; abort returns early; description must be ≤80 chars.

## Scheduler / Automation Platform

The scheduler is now a small automation platform built around `Job { trigger, action, delivery }`.
Session jobs live in the in-memory `session_cron_store()` and disappear on engine restart. Durable
jobs are project-scoped and stored at `<project>/.refact/scheduled_tasks.json`.

### Job model

| Field | Purpose |
|---|---|
| `trigger` | When work becomes due: `cron`, `interval`, `once`, `manual`, `webhook`, or reserved `on_process_exit`. |
| `action` | What runs: an agent turn or a foreground command. |
| `delivery` | Where command output goes: chat, outgoing webhook, notifier integration, or nowhere. |
| `recurring` | Explicit compatibility flag. `once` forces `false`; legacy one-shot cron jobs stay one-shot. |
| `durable` | `true` stores the job in `.refact/scheduled_tasks.json`; `false` stores it in memory. |
| `enabled` / `paused_at_ms` | Pause/resume state. Paused jobs skip ordinary schedule fires; a queued `trigger_at_ms` still makes the job due. |
| `trigger_at_ms` | Manual run marker set by `cron_update(run_now=true)`, HTTP run, or webhook dispatch. |
| `last_fired_at_ms`, `fire_count` | Last successful/error firing timestamp and counter. |
| `last_status`, `last_error`, `recent_runs` | Run history. `recent_runs` is capped by `scheduler.recent_runs_cap` (default 20). |
| `auto_expire_after_ms` | Recurring jobs default to 30 days and emit an auto-expire notice after the final fire. |
| `retry_attempts` | Transient retry counter for classifiable rate-limit/overload/network/timeout/5xx failures. |

Serialized shapes:

```json
{
  "id": "cron_...",
  "description": "Nightly build",
  "enabled": true,
  "durable": true,
  "created_at_ms": 1770000000000,
  "recurring": true,
  "trigger": { "kind": "cron", "expr": "0 2 * * *", "tz": "UTC" },
  "action": {
    "kind": "command",
    "argv": ["cargo", "check"],
    "target": { "kind": "isolated" },
    "cwd": ".",
    "env": null,
    "timeout_secs": 600
  },
  "delivery": { "kind": "webhook", "url": "https://example.test/hook", "token": "secret" },
  "last_fired_at_ms": null,
  "fire_count": 0,
  "last_status": null,
  "last_error": null,
  "recent_runs": [],
  "paused_at_ms": null,
  "trigger_at_ms": null,
  "auto_expire_after_ms": 2592000000,
  "retry_attempts": 0
}
```

Back-compat: `JsonFileCronStore` deserializes the old flat
`{ cron, prompt, chat_id, mode, ... }` shape into a nested `cron` trigger,
`agent_turn` action, and `chat` delivery. The next write persists only the nested
shape and drops the legacy `cron` / `prompt` top-level fields.

### Triggers

| Trigger | Public creation path | Runtime behavior |
|---|---|---|
| `Cron { expr, tz }` | `cron` plus optional `tz` | 5-field cron, evaluated in `tz` or `scheduler_timezone()`; cron fires receive deterministic jitter. |
| `Interval { every_ms }` | `every: "30m"`, `"2h"`, `"1d"`, etc. | Repeats from `created_at_ms` / `last_fired_at_ms`; no cron jitter. |
| `Once { at_ms }` | `at` as RFC3339 or `in 30m` | One-shot; `recurring` is forced to `false` and the job is removed after fire/error/skip unless retry is scheduled. |
| `Manual` | Inline daemon hook agent jobs | No time-based next run; fired immediately through the manual runner. |
| `Webhook { hook_id }` | `trigger: {kind:"webhook", hook_id}` or top-level `hook_id` | No time-based next run; fired by daemon/worker hook dispatch. |
| `OnProcessExit { match_kind }` | Storage enum only | Reserved optional trigger; not created by public cron tools and has no time-based next run. |

`parse_schedule()` requires exactly one of `cron`, `every`, or `at`. A webhook trigger is
mutually exclusive with `cron`, `every`, `at`, and `tz`.

### Actions and targets

| Action | Shape | Notes |
|---|---|---|
| `agent_turn` | `{ kind, prompt, target, mode?, model?, tools? }` | Enqueues a `ChatCommand::UserMessage`. Existing-chat targets require an open/restorable chat id. Isolated targets create a fresh `cron_<job_id>_<fire_ms>` chat per fire. |
| `command` | `{ kind, argv, target, cwd?, env?, timeout_secs? }` | No-agent path. Runs via the exec registry as a foreground non-PTY command. `command` input is shell-split into `argv`; `command_argv` is used verbatim. `cwd` must stay inside the active project. Default timeout is 300s, capped at 3600s. |

Public `cron_create` does not expose `env`; it stores `env: null`. Command output is captured
from stdout/stderr with exec transcript limits. Command jobs can target chat for output delivery,
but `webhook`, `notifier`, and `none` deliveries do not require a chat target.

### Delivery

| Delivery | Behavior |
|---|---|
| `chat` | Agent turns enqueue into chat. Command jobs append `event(cron_fire)` and a `plain_text` output message; error output becomes a `system_notice`. Empty output is silent. |
| `webhook` | Command jobs POST `{job_id, description, status, output, ts}` to `url` with optional `Authorization: Bearer <token>`. Tool/HTTP responses expose `has_token`, never the token. Timeout is 10s. |
| `notifier` | Command jobs resolve `integration_id` through the notifier framework and call `NotifierBackend::send(target, output)`. Current built-in backend: `notifier_telegram`. |
| `none` | Command output is discarded after run history is recorded. |

Non-chat delivery is supported for command jobs only. Agent-turn jobs and inline daemon agent hooks
must use `chat` delivery.

### Runtime behavior

- Background startup spawns the session runner and, when an active project exists, a durable runner
  over `<project>/.refact/scheduled_tasks.json`. `REFACT_DISABLE_SCHEDULER=1` suppresses runner spawn.
  `--no-scheduler`, engine `scheduler.enabled: false`, and daemon `scheduler.enabled: false` are
  resolved through `SchedulerConfig`; disabled daemon schedulers still scan durable schedules for
  status/idle-stop visibility but do not wake workers.
- `schedule::next_run_ms` is the shared dispatcher used by the runner, cron-clock wakeups, list output,
  and create/update validation.
- Existing-chat agent turns and chat-delivered command jobs defer while the chat is busy, paused, missing,
  or closed. Busy defers retry after 30s; invalid targets defer after 60s for durable/recurring jobs.
- Command jobs and isolated agent-turn jobs use the cron lane and are bounded by
  `scheduler.max_concurrent_runs` (default 8).
- Durable catch-up runs on runner start. Legacy one-shot cron jobs whose first scheduled time passed
  fire ASAP when their target can be restored; recurring jobs use `recurring_missed_grace_state()`.
- Missed recurring grace is half the schedule period clamped by `scheduler.missed_grace_min_ms` (default
  120s) and `scheduler.missed_grace_max_ms` (default 2h). Runs outside the grace window are advanced
  without replaying a burst.
- Classifiable transient failures (`429`/rate-limit, overload/529, network, timeout, HTTP 5xx) schedule
  retries using `scheduler.retry` (default delays: 60s, 120s, 300s; default max attempts: 3).
- The runner is best-effort at-most-once per due marker: successful/error fires update
  `last_fired_at_ms` / `fire_count`, clear due `trigger_at_ms`, and one-shot jobs are removed unless a
  retry was scheduled. Chat and isolated jobs are counted only after their scheduled prompt is accepted
  by the queue; command jobs are counted after the command run completes.
- Run history status values include `fired`, `error`, `deferred`, `skipped`, and `advanced`.
- Recurring jobs auto-expire after `auto_expire_after_ms` when set. The default for recurring jobs is
  30 days; one-shot jobs use `0`.

### Cron tools

#### `cron_create`

Model-facing prompt: schedule an agent prompt or command for cron, interval, one-shot, or webhook
triggering. Cron expressions are standard 5-field expressions evaluated in the local timezone unless
`tz` is supplied. Webhook jobs never time-fire.

Schema:

```json
{
  "type": "object",
  "properties": {
    "cron": { "type": "string", "description": "Standard 5-field cron expression in local time. Required unless every or at is set." },
    "every": { "type": "string", "description": "Interval such as 30m, 2h, or 1d. Mutually exclusive with cron and at." },
    "at": { "type": "string", "description": "One-shot time as RFC3339 or relative duration such as in 30m. Mutually exclusive with cron and every." },
    "trigger": {
      "type": "object",
      "properties": {
        "kind": { "type": "string", "enum": ["webhook"] },
        "hook_id": { "type": "string", "description": "Inbound daemon hook id that fires this job." }
      },
      "required": ["kind", "hook_id"],
      "description": "Webhook trigger. Mutually exclusive with cron, every, and at. Webhook jobs never time-fire."
    },
    "hook_id": { "type": "string", "description": "Shortcut for trigger {kind:'webhook', hook_id}." },
    "tz": { "type": "string", "description": "IANA timezone for cron schedules, such as UTC or Asia/Kolkata." },
    "prompt": { "type": "string", "description": "Prompt enqueued at each fire time. Mutually exclusive with command and command_argv." },
    "command": { "type": "string", "description": "Command line to shell-split and run without an agent turn. Mutually exclusive with prompt and command_argv." },
    "command_argv": { "type": "array", "items": { "type": "string" }, "description": "Command argv to run without an agent turn. Mutually exclusive with prompt and command." },
    "cwd": { "type": "string", "description": "Optional command working directory, resolved under the active project." },
    "timeout_secs": { "type": "integer", "description": "Optional command timeout in seconds." },
    "delivery": {
      "oneOf": [
        { "type": "string", "enum": ["chat", "none"] },
        {
          "type": "object",
          "properties": {
            "kind": { "type": "string", "enum": ["webhook"] },
            "url": { "type": "string" },
            "token": { "type": "string" }
          },
          "required": ["url"]
        },
        {
          "type": "object",
          "properties": {
            "kind": { "type": "string", "enum": ["notifier"] },
            "integration_id": { "type": "string" },
            "target": { "type": "string" }
          },
          "required": ["integration_id"]
        }
      ],
      "description": "Delivery target: chat (default), none, webhook {url, token?}, or notifier {integration_id, target?}."
    },
    "recurring": { "type": "boolean", "default": true },
    "durable": { "type": "boolean", "description": "Persist in the current project when true; stay session-only when false. Omitted defaults to durable when a project store exists." },
    "isolated": { "type": "boolean", "default": false, "description": "Create a fresh isolated chat session for each fire instead of enqueueing into the current chat." },
    "description": { "type": "string", "description": "Short description (≤80 chars) shown in cron_list UI." }
  },
  "required": ["description"]
}
```

Validation: one schedule source (`cron` / `every` / `at` / webhook) and one action
(`prompt` / `command` / `command_argv`) are required. Descriptions over 80 chars,
invalid timezones, schedules with no match in the next year, and more than 50 jobs are rejected.
`prompt` with non-chat delivery is rejected.

Returns `{ id, human_schedule, recurring, durable, action_kind, delivery, isolated }` and appends a
`system_notice` event summarizing the created job. Webhook tokens are stored for delivery but returned
only as `has_token`.

Examples:

```json
{"cron":"0 9 * * 1-5","prompt":"Prepare the daily standup summary","recurring":true,"durable":true,"description":"Daily standup prep"}
{"every":"30m","command":"cargo check","delivery":"none","description":"Build check"}
{"at":"in 30m","command_argv":["python3","scripts/check.py"],"delivery":{"kind":"webhook","url":"https://example.test/hook","token":"secret"},"description":"One-shot check"}
{"hook_id":"deploy","command":"./deploy.sh","delivery":{"kind":"notifier","integration_id":"notifier_telegram","target":"12345"},"description":"Deploy hook"}
```

#### `cron_list`

Model-facing prompt: list scheduled tasks with target chat and mode, optionally filtering by
session-only or durable scope.

Schema:

```json
{
  "type": "object",
  "properties": {
    "scope": { "type": "string", "enum": ["session", "durable", "all"], "default": "all" }
  },
  "required": []
}
```

Returns an array sorted by `next_fire_at_ms` then `id`. Tool rows contain
`{ id, cron, human_schedule, description, prompt, action_kind, delivery, chat_id, target,
isolated, mode, recurring, durable, next_fire_at_ms, fire_count, created_at_ms }`. `prompt` is
truncated to 200 chars; non-time triggers have `next_fire_at_ms: 0`.

#### `cron_update`

Model-facing prompt: update, pause, resume, or run a scheduled task by ID.

Schema:

```json
{
  "type": "object",
  "properties": {
    "id": { "type": "string" },
    "cron": { "type": "string" },
    "every": { "type": "string" },
    "at": { "type": "string" },
    "tz": { "type": "string" },
    "prompt": { "type": "string" },
    "description": { "type": "string" },
    "enabled": { "type": "boolean" },
    "run_now": { "type": "boolean" }
  },
  "required": ["id"]
}
```

Schedule updates use the same `parse_schedule()` rules as create; `tz` must accompany a cron
schedule. Updating to `at` forces `recurring=false`. `prompt` can only change `agent_turn` jobs;
command jobs reject prompt updates. `enabled=false` pauses and records `paused_at_ms`; `enabled=true`
resumes. `run_now=true` sets `trigger_at_ms` and wakes the runner. Returns
`{ id, updated: true, human_schedule }`.

#### `cron_delete`

Model-facing prompt: cancel a scheduled task by ID.

Schema:

```json
{
  "type": "object",
  "properties": { "id": { "type": "string" } },
  "required": ["id"]
}
```

Removes from the session store first, then the active durable store. Returns `{ "removed": boolean }`
and notifies the runner only when a job was removed.

### HTTP scheduler surface

Routes live under `/v1/scheduler/cron`:

| Route | Request | Response |
|---|---|---|
| `GET /v1/scheduler/cron` | none | `CronTaskResponse[]` with `enabled`, `paused`, trigger fields (`trigger_kind`, `hook_id`, `tz`, `every_ms`, `at_ms`), `last_status`, `last_error`, and `recent_runs`. |
| `POST /v1/scheduler/cron` | `CronCreateRequest` | `{ id, human_schedule, recurring, durable, action_kind, delivery }` |
| `PATCH /v1/scheduler/cron/:id` | `CronUpdateRequest` | `{ id, updated, human_schedule }` |
| `POST /v1/scheduler/cron/:id/run` | none | Sets `trigger_at_ms`; returns `{ id, triggered: true }`. |
| `DELETE /v1/scheduler/cron/:id` | none | `{ removed }` |

HTTP creation mirrors `cron_create` plus `chat_id` and `mode`. If `durable` is omitted, HTTP and
`cron_create` persist into the active project store when one exists; explicit `durable: false`
keeps the job session-only. If the request creates an agent turn or uses `chat` delivery, `chat_id`
must name an existing open chat. HTTP list responses flatten trigger details for the GUI and still
redact webhook tokens as `has_token`.

### Daemon cron clock

The headless daemon does not run jobs itself. Its `cron_clock` scans each open project's
`.refact/scheduled_tasks.json`, records the nearest pending durable job, and wakes the project worker
about 90s before fire time. The scan intentionally supports only durable cron-triggered
`agent_turn` jobs targeting an existing chat with `chat` delivery; unsupported jobs are skipped by the
clock, but can still run once a worker is already alive.

`GET /cron/status` returns:

```json
{ "enabled": true, "jobs": 1, "next_wake_ms": 1770000000000 }
```

`jobs` is the count of projects with pending cron-clock entries, not a full job count.

### Daemon inbound webhooks

Daemon HTTP routes:

| Route | Body | Behavior |
|---|---|---|
| `POST /hooks/wake` | `{project, text}` | Wakes the project worker and injects `text` as a `system_notice`. |
| `POST /hooks/agent` | `{project, message, mode?, model?, deliver?}` | Wakes the worker and fires an isolated inline agent job. Delivery must resolve to `chat`. |
| `POST /hooks/:name` | Mapping-specific body | Resolves `hooks.mappings[name]`, applies mapping defaults, wakes the worker, then forwards to `/v1/hooks/fire`. |
| `POST /hooks` | none | Authenticates, then returns `400 missing hook name` when hooks are enabled. |

Daemon config (`daemon.yaml`):

```yaml
bind: 127.0.0.1
mdns: {}
scheduler:
  enabled: true
  disable_durable: false
hooks:
  enabled: true
  token: hook-secret
  default_project: demo
  allowed_projects: [demo, /abs/path/to/project]
  mappings:
    deploy:
      project: demo
      kind: agent        # wake | agent
      mode: agent
      model: test-model
      deliver:
        type: chat
```

Project resolution order is request body `project` → mapping `project` → `default_project`.
Allowed projects can match project id, slug, or root path. Hook auth accepts `Authorization: Bearer`
or `x-refact-token`; query-string daemon tokens are rejected for hook routes. Hooks may be open only
when the daemon is bound to loopback (`127.0.0.1` or `::1`); non-loopback binds require `hooks.token`
or daemon auth and refuse to start otherwise. The worker forward token is the daemon auth token when
present, otherwise the hook token.
Daemon bind defaults to `127.0.0.1`; explicit `bind: 0.0.0.0` keeps LAN exposure opt-in. Daemon mDNS uses `mdns.enabled: true|false`; omitted means auto-advertise only for non-loopback binds. Advertisements use the generic `Refact Daemon` instance name and include TXT `auth=required|none`.

Worker endpoint `POST /v1/hooks/fire` accepts:

```json
{
  "kind": "wake",
  "text": "optional wake text",
  "message": "optional agent message",
  "mode": "agent",
  "model": "model-id",
  "hook_id": "deploy",
  "deliver": { "kind": "chat" }
}
```

`kind="wake"` with text injects a notice. `kind="agent"` with message creates and fires an isolated
manual `agent_turn` job. Any `hook_id` also fires matching stored `Trigger::Webhook` jobs from both
session and durable stores, so a single hook call may perform the inline action and stored webhook jobs.

### Non-goals / boundaries

- No chat-platform messaging gateway is implemented here. Deliveries are Refact chat session,
  outgoing HTTP webhook, configured notifier backend, or no-op.
- Slack/email notifiers are framework-ready future plugins, not shipped backends. The in-tree notifier
  backend is Telegram (`notifier_telegram`).
- `OnProcessExit` is an optional/reserved trigger shape and is not wired through public cron creation or
  process-exit event handling.

## HTTP API

Base: `http://127.0.0.1:{port}/v1/`. Middleware: permissive CORS, 15MB body limit.

Key endpoints: `/ping`, `/caps`, `/graceful-shutdown`, `/p/{project_id}/v1/chats/{id}/commands`, `/p/{project_id}/v1/chats/subscribe` (project-scoped chat protocol; `daemon::chat_client::ProxyChatClient` is the in-tree client), `/chat` (legacy), `/code-completion`, `/code-lens`, `/tools`, `/tools-check-if-confirmation-needed`, `/ast-file-symbols`, `/ast-status`, `/rag-status`, `/vecdb-search`, `/git-commit`, `/checkpoints-preview`, `/checkpoints-restore`, `/integrations`, `/integration-get`, `/integration-save`, `/knowledge/update-memory`, `/knowledge/delete-memory`, `/knowledge-graph`, `/voice/transcribe`, `/voice/stream/{id}`, `/voice/stream/{id}/chunk`.

## AST

8 languages: C, C++, Python, Java, Kotlin, JavaScript, Rust, TypeScript (7 tree-sitter parsers; C/C++ share parser). Two-phase indexing: parse+store → link cross-references. Storage in LMDB with key prefixes (`d|` defs, `c|` fuzzy lookup, `u|` back-links, `classes|` inheritance). Background thread with batch processing. Skeletonizer generates abbreviated code for embeddings.

## VecDB

SQLite + vec0 extension. File splitters: trajectory JSON (4 msgs/chunk), Markdown (heading-aware), code (AST-aware token windows). Embedding via external HTTP API with batching/retry. Search: cosine KNN → reject threshold → normalize usefulness score. Background thread: enqueue → split → cache check → embed → store. Cleanup: keep 10 newest tables, drop >7 days.

## Providers

15+ providers in `src/providers/`: Anthropic, Claude Code, OpenAI, Codex, DeepSeek, Google Gemini, Groq, LM Studio, Ollama, OpenRouter, vLLM, xAI, custom. Each defines ProviderDefaults (chat/completion/embedding models). OAuth support for Codex/Claude Code. YAML configs in `yaml_configs/default_providers/`.

## Integrations

GitHub, GitLab, Bitbucket, Chrome (headless), PostgreSQL, MySQL, Docker, PDB, shell, cmdline_* (one-off), service_* (long-running), MCP (stdio + SSE). Config: `.refact/integrations/*.yaml`. Trait: `integr_tools()`, `integr_schema()`, `integr_settings_apply()`.

### Standardized exec env

All foreground, background, service, and PTY exec spawns apply `EXEC_ENV_DEFAULTS` before request env overrides. Request-provided env values win. Defaults:

| Key | Value | Why |
|---|---|---|
| `NO_COLOR` | `1` | Keep transcripts stable and readable without ANSI color noise. |
| `TERM` | `dumb` | Discourage interactive/full-screen terminal behavior unless `tty=true`. |
| `LANG` | `C.UTF-8` | Provide deterministic UTF-8 locale. |
| `LC_CTYPE` | `C.UTF-8` | Preserve UTF-8 character classification. |
| `LC_ALL` | `C.UTF-8` | Avoid locale-specific output drift. |
| `COLORTERM` | empty | Disable color auto-detection. |
| `PAGER` | `cat` | Prevent commands from blocking in pagers. |
| `GIT_PAGER` | `cat` | Prevent git from blocking in pagers. |
| `GH_PAGER` | `cat` | Prevent GitHub CLI from blocking in pagers. |
| `REFACT_EXEC` | `1` | Marker that a process is running under Refact exec. |

## Testing

- **Integration tests in `tests/`**: live HTTP+SSE Python suites plus Rust e2e suites (`daemon_e2e.rs`, `daemon_proxy.rs`, `daemon_supervisor.rs`) that share the `tests/e2e_helpers/` module. Python helpers include `fake_worker.py` and `lsp_connect.py`; 7 `test_chat_session_*.py` files cover the chat session flow.
- **Rust unit tests**: `src/chat/tests.rs`, AST parser tests, 50+ modules. `cargo test --lib`.
- **Test data**: `tests/emergency_frog_situation/` — themed frog simulations for parsing edge cases.

## Config

- **User**: `~/.config/refact/` (default_privacy.yaml, providers.d/*.yaml)
- **Cache**: `~/.cache/refact/` (shadow repos, logs, integrations)
- **Project**: `.refact/` (trajectories/, knowledge/, tasks/, integrations/, `project_information.yaml` — schema_version 1, toggles + size caps for the `system_info` / `environment_instructions` / `detected_environments` / `git_info` / `project_tree` / `instruction_files` / `project_configs` / `memories` sections surfaced to the model)
- **System prompts**: `yaml_configs/defaults/` — modes (built-in modes in `modes/`, plus project-setup wizard modes like `setup`, `setup_skills`, `setup_agents_md`, `setup_mcp`, `setup_commands`, `setup_subagents`, `setup_modes`, `setup_hooks`, `setup_knowledge`), subagents, toolbox commands. Magic vars: `%ARGS%`, `%CODE_SELECTION%`, `%WORKSPACE_INFO%`, `%PROJECT_TREE%`.
