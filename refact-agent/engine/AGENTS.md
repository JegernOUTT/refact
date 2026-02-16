# Refact Agent Engine

Binary: `refact-lsp` — AI coding agent, HTTP + LSP server. Rust 2021 edition, async/tokio.

## Stack

Axum (HTTP), tower-lsp (LSP), tree-sitter (AST), SQLite + vec0 (VecDB), LMDB/Heed (AST store), git2, headless_chrome, whisper-rs (optional, feature-gated), rmcp (MCP).

## Build

```bash
cargo build --release                    # binary at target/release/refact-lsp
cargo build --release --features voice   # with Whisper transcription
cargo test --lib && cargo test --doc
```

Release profile: `opt-level = "z"`, `lto = true`, `strip = true`, `codegen-units = 1`.

## Architecture

`GlobalContext` (`Arc<ARwLock<GlobalContext>>`) is the central shared state. HTTP server (Axum) and LSP server (tower-lsp) both hold a reference. Background tasks (AST indexer, VecDB, git shadow cleanup, telemetry, knowledge graph, trajectory memos, agent monitor, OAuth refresh) are spawned via `start_background_tasks()` (~12 tokio tasks).

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
  providers/           — 15+ LLM providers (Anthropic, OpenAI, Codex, Refact, DeepSeek, Gemini, Groq, LM Studio, Ollama, OpenRouter, vLLM, xAI, Claude Code, custom)
  integrations/        — GitHub, GitLab, Bitbucket, Chrome, PostgreSQL, MySQL, Docker, PDB, cmdline, services, MCP (stdio+SSE)
  knowledge_graph/     — petgraph DiGraph, builder/cleanup/staleness/query
  scratchpads/         — FIM code completion (PSM/SPM), RAG, multimodality
  tasks/               — Kanban task board (planning/active/paused/completed/abandoned)
  caps/                — model capabilities resolution, cloud/local detection
  git/                 — shadow repos, checkpoints
  voice/               — Whisper transcription, streaming sessions
  telemetry/           — usage tracking
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
- **`history_limit.rs`**: 4-stage compression (dedup context files → compress tool results → fix tool calls → limit history). `CompressionStrength`: Absent/Low/Medium/High.

### Anthropic Thinking/Signatures

Thinking blocks with cryptographic signatures must be preserved verbatim — no JSON rebuilding, no field reordering. Signatures validate exact prior content-block sequence. During streaming, accumulate deltas preserving metadata (block_index, signature) separately from text. For multi-provider chats, strip provider-specific blocks (thinking/signatures) on model switch. `strip_thinking_blocks_if_disabled()` in prepare.rs removes them when model lacks reasoning support.

### Trajectories

Stored: `.refact/trajectories/{chat_id}.json`. Atomic writes (`.tmp` → rename). Rich JSON: id, title, model, mode, tool_use, messages, task_meta, version, created_at, reasoning_effort, checkpoints_enabled, parent_id, root_chat_id, etc.

OpenAI conversion lives in `src/llm/adapters/openai_chat.rs` (`convert_messages_to_openai()`).

## Tools

~50+ tools, filtered by mode/capabilities/config. Registered in `tools_list.rs`.

**Categories**: Codebase search (AST defs, tree, cat, regex, semantic) · Codebase change (create/update/rm/mv/undo/apply_patch — confirmation required) · Web (fetch, search, Chrome automation) · System (shell, cmdline_*, service_*) · Knowledge (search, create, trajectories) · Agent (subagent, strategic_planning, deep_research, code_review) · Task management (~18 tools) · IDE (open_file, paste_text) · Integration-defined + MCP tools.

Tool trait: `tool_execute(&mut self, ccx, tool_call_id, args) -> Result<(bool, Vec<ContextEnum>)>`.

`AtCommandsContext` provides: global_context, chat_id, n_ctx, abort_flag, messages, current_model, task_meta, subchat depth/channels, postprocess params.

## HTTP API

Base: `http://127.0.0.1:{port}/v1/`. Middleware: permissive CORS, 15MB body limit, telemetry logging.

Key endpoints: `/ping`, `/caps`, `/graceful-shutdown`, `/chats/{id}/commands`, `/chats/subscribe`, `/chat` (legacy), `/code-completion`, `/code-lens`, `/tools`, `/tools-check-if-confirmation-needed`, `/ast-file-symbols`, `/ast-status`, `/rag-status`, `/vecdb-search`, `/git-commit`, `/checkpoints-preview`, `/checkpoints-restore`, `/integrations`, `/integration-get`, `/integration-save`, `/knowledge/update-memory`, `/knowledge/delete-memory`, `/knowledge-graph`, `/voice/transcribe`, `/voice/stream/{id}`, `/voice/stream/{id}/chunk`, `/telemetry-network`, `/snippet-accepted`.

## AST

8 languages: C, C++, Python, Java, Kotlin, JavaScript, Rust, TypeScript (7 tree-sitter parsers; C/C++ share parser). Two-phase indexing: parse+store → link cross-references. Storage in LMDB with key prefixes (`d|` defs, `c|` fuzzy lookup, `u|` back-links, `classes|` inheritance). Background thread with batch processing. Skeletonizer generates abbreviated code for embeddings.

## VecDB

SQLite + vec0 extension. File splitters: trajectory JSON (4 msgs/chunk), Markdown (heading-aware), code (AST-aware token windows). Embedding via external HTTP API with batching/retry. Search: cosine KNN → reject threshold → normalize usefulness score. Background thread: enqueue → split → cache check → embed → store. Cleanup: keep 10 newest tables, drop >7 days.

## Providers

15+ providers in `src/providers/`: Anthropic, Claude Code, OpenAI, Codex, Refact, DeepSeek, Google Gemini, Groq, LM Studio, Ollama, OpenRouter, vLLM, xAI, custom. Each defines ProviderDefaults (chat/completion/embedding models). OAuth support for Codex/Claude Code. YAML configs in `yaml_configs/default_providers/`.

## Integrations

GitHub, GitLab, Bitbucket, Chrome (headless), PostgreSQL, MySQL, Docker, PDB, shell, cmdline_* (one-off), service_* (long-running), MCP (stdio + SSE). Config: `.refact/integrations/*.yaml`. Trait: `integr_tools()`, `integr_schema()`, `integr_settings_apply()`.

## Testing

- **Python integration tests** (~38 files in `tests/`): live HTTP+SSE against running server. 7 `test_chat_session_*.py` files.
- **Rust unit tests**: `src/chat/tests.rs`, AST parser tests, 50+ modules. `cargo test --lib`.
- **Test data**: `tests/emergency_frog_situation/` — themed frog simulations for parsing edge cases.

## Config

- **User**: `~/.config/refact/` (default_privacy.yaml, providers.d/*.yaml)
- **Cache**: `~/.cache/refact/` (shadow repos, logs, telemetry, integrations)
- **Project**: `.refact/` (trajectories/, knowledge/, tasks/, integrations/)
- **System prompts**: `yaml_configs/defaults/` — modes, subagents, toolbox commands. Magic vars: `%ARGS%`, `%CODE_SELECTION%`, `%WORKSPACE_INFO%`, `%PROJECT_TREE%`.
