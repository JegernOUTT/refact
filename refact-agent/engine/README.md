# Refact Agent Engine (`refact-lsp`)

`refact-lsp` is the local Rust engine behind Refact. It runs on the user's machine, exposes HTTP and LSP entry points for IDE clients, maintains workspace indexes, talks to configured model providers, and executes the tools used by chat and autonomous agent workflows.

The engine is designed for local-first/BYOK usage: provider credentials and project state live in local configuration directories, while model calls go only to the providers or local runtimes the user enables.

## What the engine provides

- **HTTP API** on localhost for chat commands, SSE subscriptions, code completion, caps, tools, integrations, knowledge, tasks, trajectories, checkpoints, and voice endpoints.
- **LSP transport** over stdio or TCP for IDE integrations.
- **Chat and agent runtime** with streaming deltas, tool calls, pause/confirmation handling, subagents, and trajectory persistence.
- **Code intelligence** with workspace file tracking, AST indexes, semantic search, code lens, and completion context.
- **Provider registry** that loads BYOK/local provider configs and dynamically refreshes available models.
- **Tooling layer** for filesystem edits, search, shell/cmdline execution, browser automation, MCP, knowledge, tasks, and VCS workflows.
- **Integrations** for GitHub, GitLab, Bitbucket, PDB, PostgreSQL, MySQL, command-line tools, long-running services, and MCP transports.

## Build and run

```bash
cd refact-agent/engine

# Fast type/borrow check
cargo check

# Debug build
cargo build

# Release build with default features
cargo build --release

# Release build without optional voice dependencies
cargo build --release --no-default-features
```

Run a local HTTP endpoint for development:

```bash
cargo run -- --http-port 8001 --logs-stderr --workspace-folder /path/to/project --ast --vecdb
```

Useful flags:

- `--http-port <port>` binds the HTTP API and embedded GUI to `127.0.0.1:<port>` by default.
- `--http-host <ip>` changes the HTTP bind address. Use `0.0.0.0` only on trusted networks because chat/tool APIs are reachable from the LAN when firewall rules allow it. When HTTP starts, the engine advertises the GUI over mDNS as `http://<hostname>.local:<port>/` where the local network supports mDNS.
- `--lsp-stdin-stdout 1` runs the LSP transport over stdio.
- `--lsp-port <port>` runs LSP over TCP.
- `--workspace-folder <path>` seeds workspace indexing before an IDE connects.
- `--ast` enables AST indexing.
- `--vecdb` enables vector search indexing when an embedding provider is configured.
- `--logs-stderr` sends logs to stderr; otherwise logs are stored under `~/.cache/refact/logs/`.
- `--only-create-yaml-configs` creates default YAML configuration files and exits.

Run `cargo run -- --help` for the full option list.

## Embedded standalone GUI

The engine serves the standalone chat UI from the same HTTP origin:

```text
http://127.0.0.1:<http-port>/
```

`cargo build`, `cargo run`, and release builds automatically run the GUI build from `refact-agent/gui`, copy `dist/chat` into `refact-agent/engine/assets/chat/dist/chat`, and embed those assets into `refact-lsp`. Node.js and npm must be available for normal engine builds. Set `REFACT_SKIP_GUI_BUILD=1` only for API-only developer builds that intentionally do not refresh the embedded GUI assets.

The embedded page uses `window.location.origin` for `/v1` API and SSE calls, so browser clients and LAN clients use the same engine origin that served the page. The engine also advertises a DNS-SD service named `_refact-lsp._tcp.local.` and logs the `http://<hostname>.local:<port>/` URL when mDNS starts successfully.

## Refact daemon

The `refact daemon` control plane exposes `/daemon/v1/*` endpoints for IDEs, the TUI, and CLI frontends that attach to daemon-managed workers. If daemon auth is enabled, mutating control routes and project proxy routes require the `Bearer` token from `daemon.json`. `GET /daemon/v1/status` is intentionally public and read-only: clients may use it for liveness and version discovery before they have loaded local credentials, while authenticated clients still send Bearer when they have one.

## Tests

```bash
cd refact-agent/engine
cargo check
cargo test --lib
cargo test --doc
```

Python integration tests under `tests/` expect a running `refact-lsp` instance and are not part of the quick local check.

## Configuration

The engine uses these local locations by default:

| Location | Purpose |
| --- | --- |
| `~/.config/refact/` | User configuration, provider YAML files, privacy settings, global customization |
| `~/.config/refact/providers.d/*.yaml` | BYOK/local provider configs loaded by the provider registry |
| `~/.cache/refact/` | Logs, caches, shadow repositories, integration state |
| `.refact/` in a workspace | Project trajectories, knowledge, tasks, integrations, and customization overrides |

Provider setup is normally handled from the GUI, but the engine ultimately loads YAML files from `providers.d`. Current provider families include OpenAI-compatible APIs, Anthropic, OpenRouter, Ollama, LM Studio, vLLM, Groq, DeepSeek, Doubao, xAI, Google Gemini, Qwen, Kimi, Zhipu, MiniMax, GitHub Copilot, Claude Code, and custom endpoints. Available models are derived from provider config and provider/runtime catalogs instead of a fixed hard-coded model list.

Custom provider YAML keeps chat, completion, and embedding roles separate. Existing chat-only configs using `custom_models` and `enabled_models` remain valid for chat. Code completion uses `completion_models` plus `completion_default_model` in caps/defaults, and VecDB uses `embedding_model` plus the embedding default. Endpoint styles are string-backed: completion currently supports `openai_completions` and `openai_chat_completions`; embeddings currently support `openai` and `ollama_native`. Future or invalid styles are preserved as config strings but return explicit unsupported/invalid-style errors when used.

## API overview

Selected HTTP endpoints under `/v1`:

| Endpoint | Purpose |
| --- | --- |
| `/ping` | Health check and process identity |
| `/caps` | Current provider/model/tool capabilities |
| `/chats/{id}/commands` | Queue chat commands such as user messages, aborts, retries, and tool decisions |
| `/chats/subscribe` | SSE stream for chat snapshots, deltas, queue changes, and runtime updates |
| `/code-completion` | Fill-in-middle/code completion requests |
| `/tools` and `/tools-check-if-confirmation-needed` | Tool metadata and confirmation checks |
| `/ast-status`, `/ast-file-symbols` | AST index status and symbols |
| `/rag-status`, `/vecdb-search` | Semantic index status and search |
| `/integrations`, `/integration-get`, `/integration-save` | Integration configuration |
| `/knowledge/*`, `/knowledge-graph` | Memory and knowledge graph operations |
| `/tasks/*` | Task board operations |
| `/checkpoints-preview`, `/checkpoints-restore` | Workspace rollback preview and restore |
| `/`, `/index.html`, `/favicon.ico`, `/dist/chat/*` | Embedded chat GUI assets served by the HTTP server |

Chat clients use the commands API plus `/v1/chats/subscribe` SSE events rather than the legacy one-shot chat endpoint.

## Source pointers

| Path | Notes |
| --- | --- |
| `src/main.rs` | Process startup, HTTP/LSP selection, background tasks |
| `src/global_context.rs` | Shared state, CLI options, provider loading, workspace initialization |
| `src/http/routers/v1/` | HTTP route handlers |
| `src/chat/` | Chat sessions, queues, streaming, tools, trajectories, history limits |
| `src/chat/trajectory_ops.rs` | Handoff and trajectory selection helpers for model-switch, handoff preview/apply, and plan carry-over |
| `src/llm/` | Provider wire-format adapters and streaming conversions |
| `src/providers/` | Provider implementations and registry |
| `src/tools/` | Built-in tools and file-edit/search/task/agent tool implementations |
| `src/integrations/` | Integration configuration and runtime sessions |
| `src/ast/` | Tree-sitter parsing and AST index storage |
| `src/vecdb/` | SQLite/vec0 semantic indexing and search |
| `src/tasks/` | Task board storage and events |
| `src/yaml_configs/` | Default modes, toolbox commands, subagents, and provider templates |

## Supported AST languages

AST indexing currently covers C, C++, Python, Java, Kotlin, JavaScript, Rust, and TypeScript. Refact can still work with other languages using file, regex, semantic, and provider context, but language-aware AST features depend on parser support.

## Contributing

- Root repository: <https://github.com/JegernOUTT/refact>
- Docs: <https://github.com/JegernOUTT/refact/wiki>
- Issues: <https://github.com/JegernOUTT/refact/issues>
- Discussions: <https://github.com/JegernOUTT/refact/discussions>

Run `cargo fmt`, `cargo check`, and the relevant tests before submitting engine changes.
