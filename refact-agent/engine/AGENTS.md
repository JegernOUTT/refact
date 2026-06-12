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

Subscribe: `GET /v1/chats/subscribe?chat_id={id}`. Events have monotonic `seq: u64`.

Key types: `Snapshot`, `StreamStarted`, `StreamDelta`, `StreamFinished`, `MessageAdded`, `MessageUpdated`, `MessageRemoved`, `MessagesTruncated`, `ThreadUpdated`, `QueueUpdated`, `RuntimeUpdated`, `PauseRequired`.

Background process completion is represented by a hidden `event(process_completed)` message delivered through `MessageAdded`. If a future dedicated `ProcessCompleted` envelope is reintroduced, keep it additive to `MessageAdded` and document it in both engine and GUI AGENTS before clients depend on it.

### Commands

`POST /v1/chats/{chat_id}/commands` — queued processing.

Variants: `UserMessage`, `SetParams`, `UpdateMessage`, `RemoveMessage`, `TruncateMessages`, `RetryFromIndex`, `Abort`, `ApproveTools`, `RejectTools`, `BranchFromChat`, `RestoreFromTrajectory`, `ClearDraft`, `SetDraft`, `Regenerate`.

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
- Trajectory compression and manual `compress_chat_apply` must share `build_compression_report_message()` (or the fingerprinted variant) and `insert_compression_report_at_boundary()`. The helper inserts the report at the earliest affected boundary while staying after the entire leading control prefix (`system`/`event`/`plan`) and after the first user when present, and never between an assistant's tool calls and their results. Report equivalence for idempotent removal is decided by the stable `op_fingerprint` when both reports carry one (deterministic compaction hashes changed message ids, `compress_chat_apply` hashes its op lists, `compress_in_place` hashes options + affected ids); fingerprint-less legacy pairs still compare by metrics, and mixed pairs are never equivalent.
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
| `event` | `extra.event = { subkind, source, payload }` plus human-readable `content` | Internal facts such as mode switches, tool decisions, plan deltas, cron fires, process exits, ticks, verifier reports, and notices | Hidden from normal transcript; shown in EventLog except `plan_delta` |
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

Compression rules live in `crates/refact-chat-history/src/compression_exemption.rs`: `plan` and `event(plan_delta)` are `Never` and must never be compressed, truncated by compression, or dropped; non-event/non-plan roles are `PreserveAnchor`; unknown event subkinds default to `PreserveAnchor`. Keep the table above in sync when adding a subkind.

Wire mapping rules: provider adapters must never send literal `event` or `plan` roles. Normal `event` lowers to provider-visible user context with structured `<event subkind="..." source="...">` framing. Base `plan` lowers as `<plan mode="..." version="...">...`; `event(plan_delta)` lowers as append-only `<plan-update seq="...">...` blocks. This keeps the cached base plan bytes stable while still exposing the synthesized current plan to the model. Preserve Anthropic thinking/signature block order across hidden-role lowering.

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

### Plan transitions

`handoff_to_mode` and mode-transition endpoints create a pinned `initial-plan` task document when transitioning into Task Planner with an `initial_plan`. The document is created with kind `plan`, role `planner`, and `pinned=true`; failures are non-blocking and reported/logged without mutating the source chat's cached provider state.

### Anthropic Thinking/Signatures

Thinking blocks with cryptographic signatures must be preserved verbatim — no JSON rebuilding, no field reordering. Signatures validate exact prior content-block sequence. During streaming, accumulate deltas preserving metadata (block_index, signature) separately from text. For multi-provider chats, strip provider-specific blocks (thinking/signatures) on model switch. `strip_thinking_blocks_if_disabled()` in prepare.rs removes them when model lacks reasoning support.

### Trajectories

Stored: `.refact/trajectories/{chat_id}.json`. Atomic writes (`.tmp` → rename). Rich JSON: id, title, model, mode, tool_use, messages, task_meta, version, created_at, reasoning_effort, checkpoints_enabled, parent_id, root_chat_id, etc.

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

## Scheduler

The scheduler owns cron-style scheduled tasks and is spawned from background tasks only when enabled. Durable jobs are stored per project at `<project>/.refact/scheduled_tasks.json`; session jobs live in the in-memory scheduler store and disappear on engine restart.

### Runtime behavior

- Kill switches: `REFACT_DISABLE_SCHEDULER=1`, the `--no-scheduler` CLI flag, or `scheduler.enabled: false` in global config skip runner startup and cron tool effects.
- Durable restriction: `scheduler.disable_durable: true` makes `cron_create` fall back to session-only and return the note `durable schedules disabled by config`.
- Job cap: `scheduler.max_jobs` defaults to 50.
- Jitter: recurring jobs use deterministic per-task jitter capped by scheduler defaults so many jobs do not fire at the same instant. One-shot jobs use the one-shot jitter path near matching minutes. Jitter only shifts fire checks; it does not change the stored cron expression.
- REPL-idle gate: due jobs do not inject while the owning chat is `Generating`, `ExecutingTools`, or `Paused`. They defer and re-check after the runner delay.
- Missed-task catch-up: recurring durable jobs compute their next future fire from now and do not replay a burst. Past one-shot durable jobs fire ASAP and mark the `cron_fire` payload as missed when that path is available.
- Auto-expire: recurring jobs default to a 30-day auto-expiration window and final fires carry `final=true` before deletion.

### Cron tools

#### `cron_create`

Model-facing prompt: "Schedule a prompt to be enqueued later. Use a standard 5-field cron expression (`minute hour day-of-month month day-of-week`) evaluated in the local timezone. Set `recurring` to true for repeated prompts or false for a one-shot prompt that is removed after it fires. Set `durable` to true when the job should survive engine restarts in the current project; leave it false for a session-only in-memory schedule. Scheduler jitter is applied automatically so jobs may run shortly after the exact cron instant. Recurring jobs auto-expire after 30 days unless canceled earlier."

Schema:

```json
{
  "type": "object",
  "properties": {
    "cron": { "type": "string", "description": "Standard 5-field cron expression in local time." },
    "prompt": { "type": "string", "description": "Prompt enqueued at each fire time." },
    "recurring": { "type": "boolean", "default": true },
    "durable": { "type": "boolean", "default": false },
    "description": { "type": "string", "description": "Short description (≤80 chars) shown in cron_list UI." }
  },
  "required": ["cron", "prompt", "description"]
}
```

Returns `{ id, human_schedule, recurring, durable }`. Emits `event(system_notice, "scheduler.cron", {id, cron, recurring, durable}, summary)`. Edge cases: rejects invalid cron expressions, expressions with no match within a year, descriptions over 80 chars, no project root for durable jobs, and jobs beyond the configured cap.

Examples:

```json
{"cron":"0 9 * * 1-5","prompt":"Prepare the daily standup summary","recurring":true,"durable":true,"description":"Daily standup prep"}
{"cron":"30 14 28 2 *","prompt":"Check leap-day task state","recurring":false,"description":"Leap check"}
```

#### `cron_list`

Model-facing prompt: list scheduled tasks, optionally filtering by session-only or durable scope.

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

Returns an array of `{ id, cron, human_schedule, description, prompt, recurring, durable, next_fire_at_ms, fire_count, created_at_ms }`. The prompt is truncated to the first 200 characters. Edge cases: invalid scopes are rejected; `next_fire_at_ms` can be 0 if no next run can be calculated.

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

Returns `{ "removed": boolean }` and notifies the runner to recompute wakeups. Missing IDs return `removed: false`; non-string or missing `id` is rejected.

### Cron fire injection

On fire, the runner appends `event(cron_fire, "scheduler.cron", {task_id, cron, recurring, fire_count, final?, missed?}, prompt)` and enqueues a `ChatCommand::UserMessage` with the configured prompt so the agent actually wakes up. Keep both pieces unless the product intentionally changes autonomous scheduling semantics.

## HTTP API

Base: `http://127.0.0.1:{port}/v1/`. Middleware: permissive CORS, 15MB body limit.

Key endpoints: `/ping`, `/caps`, `/graceful-shutdown`, `/chats/{id}/commands`, `/chats/subscribe`, `/chat` (legacy), `/code-completion`, `/code-lens`, `/tools`, `/tools-check-if-confirmation-needed`, `/ast-file-symbols`, `/ast-status`, `/rag-status`, `/vecdb-search`, `/git-commit`, `/checkpoints-preview`, `/checkpoints-restore`, `/integrations`, `/integration-get`, `/integration-save`, `/knowledge/update-memory`, `/knowledge/delete-memory`, `/knowledge-graph`, `/voice/transcribe`, `/voice/stream/{id}`, `/voice/stream/{id}/chunk`.

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

- **Python integration tests** (~38 files in `tests/`): live HTTP+SSE against running server. 7 `test_chat_session_*.py` files.
- **Rust unit tests**: `src/chat/tests.rs`, AST parser tests, 50+ modules. `cargo test --lib`.
- **Test data**: `tests/emergency_frog_situation/` — themed frog simulations for parsing edge cases.

## Config

- **User**: `~/.config/refact/` (default_privacy.yaml, providers.d/*.yaml)
- **Cache**: `~/.cache/refact/` (shadow repos, logs, integrations)
- **Project**: `.refact/` (trajectories/, knowledge/, tasks/, integrations/)
- **System prompts**: `yaml_configs/defaults/` — modes, subagents, toolbox commands. Magic vars: `%ARGS%`, `%CODE_SELECTION%`, `%WORKSPACE_INFO%`, `%PROJECT_TREE%`.
