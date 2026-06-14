# Refact Monorepo

AI coding assistant: Rust engine (LSP/HTTP server) + React chat UI + IDE plugins (VSCode, JetBrains) + cloud backend.

## Repository Map

| Subproject | Path | Language | AGENTS.md |
|---|---|---|---|
| Agent Engine | `refact-agent/engine/` | Rust 2021, async/tokio | ✅ `refact-agent/engine/AGENTS.md` |
| Agent GUI | `refact-agent/gui/` | TypeScript/React 18 | ✅ `refact-agent/gui/AGENTS.md` |
| VSCode Extension | `plugins/vscode/` | TypeScript | — |
| JetBrains Plugin | `plugins/intellij/` | Kotlin, Gradle | — |
| Documentation | GitHub Wiki | https://github.com/JegernOUTT/refact/wiki | — |
| Project pitch | `README.md` | Markdown | user-facing overview (local-first engine, Buddy, task planner, providers); defer to Wiki for setup, architecture, and per-mode details |
| IDE metadata | `.idea/` | IntelliJ project config | keep local/editor files out of commits; `.idea/workspace.xml` is ignored by `.gitignore` |
| Agent notes | `.agents/` | onboarding notes | checked for repo-specific guidance when present |
| Codex workspace | `.codex/` | Codex config/data | checked for repo-specific guidance when present |
| Claude Code workspace | `.claude/` | Claude Code config/agents | checked for repo-specific guidance when present |
| Playwright MCP cache | `.playwright-mcp/` | Playwright MCP server runtime/browser cache | local-only cache for the Playwright MCP server; do not commit |
| Local refact state | `.refact/` | per-project refact state | gitignored (`.refact/`, `.refact-*/`); holds `trajectories/`, `knowledge/`, `tasks/`, `integrations.d/`, and personal slash commands in `.refact/commands/` |
| Root `.gitignore` | `.gitignore` | repository ignore rules | includes local editor and build output exclusions; check before adding new generated files |
| JetBrains local release helper | `build-jb-plugin-local.sh` | shell script | local JetBrains packaging helper; root `.gitignore` excludes copied `/refact-*.zip` archives |

Sub-project `AGENTS.md` files contain detailed architecture, patterns, and checklists. Read them before working in those directories.

## Verification Commands

**Always verify your changes compile and pass tests before finishing.** Both engine and GUI builds are heavy — plan accordingly.

### Engine (`refact-agent/engine/`)

```bash
cd refact-agent/engine

# Fast check — type/borrow errors only (~1-3 min, no codegen)
cargo check

# Unit + doc tests (~3-8 min first build, ~1-3 min incremental)
cargo test --lib && cargo test --doc

# Full release build (~10-20 min cold, ~2-5 min incremental)
# LTO + opt-level=z + strip — very slow from scratch
cargo build --release
```

⚠️ **First build compiles ~85 crates + 7 tree-sitter parsers + SQLite. Expect 10-20 minutes cold.** Incremental builds are much faster. CI runs `cargo test --release` on 7 platform targets.

Python integration tests (`tests/*.py`) require a running `refact-lsp` instance — don't run them as a quick check.

### GUI (`refact-agent/gui/`)

```bash
cd refact-agent/gui

# All CI checks (~1-3 min total)
npm run test              # vitest (unit, excludes integration)
npm run format:check      # prettier — no code changes
npm run types             # tsc --noEmit
npm run lint              # eslint, 0 warnings allowed

# Full build (~30-60s)
npm run build
```

⚠️ **ESLint is strict-type-checked with `--max-warnings 0`.** Any new warning fails CI. Run `npm run lint` before committing TypeScript changes.

### Minimum pre-commit checks

If you changed **only engine Rust code**: `cd refact-agent/engine && cargo check && cargo test --lib`
If you changed **only GUI TypeScript**: `cd refact-agent/gui && npm run types && npm run lint && npm run test`
If you changed **both**: run both sets.

## CI Quality Gates (GitHub Actions)

| Workflow | Trigger paths | Checks |
|---|---|---|
| `agent_engine_build` | `refact-agent/engine/**` | `cargo test --release` on 7 targets (Win/Linux/macOS × x86_64/aarch64) |
| `agent_gui_build` | `refact-agent/gui/**` | `npm test` → `format:check` → `types` → `lint` → `build` (Node LTS + latest) |
| `server_build` | `refact-server/**` | Docker multi-arch build |
| `plugin_vscode_build` | `plugins/vscode/**`, engine, GUI | VS Code extension packaging against same-commit engine/GUI artifacts |
| `plugin_intellij_build` | `plugins/intellij/**`, engine, GUI | JetBrains plugin build against same-commit engine/GUI artifacts |

## Dev Workflow & Tooling

The development loop is **error-driven and worktree-based**: work happens in an
isolated worktree, then squash-merges to `main` as a single commit. Composable
slash commands (local, in `.refact/commands/`) orchestrate reusable scripts
(tracked, in `tools/dev/`). Use what you need; skip what you don't.

### Reusable scripts (`tools/dev/`, tracked)

| Script | Purpose |
|---|---|
| `changed.sh [base]` | Print changed components (engine/gui/vscode/intellij/infra) vs base (default `origin/main`) |
| `check.sh [components...]` | Run pre-push checks only for changed components. Auto-detects; runs in the current worktree |
| `ci-status.sh <run-url\|id>` | GitHub Actions run status: per-job pass/fail + pinpointed failed steps |
| `ci-logs.sh <run-url\|id> [N]` | Tail (default 300) of each **failed** job's log. Use this, not `gh run view --log-failed` (unreliable for reusable-workflow jobs) |
| `release.sh <ver> <build\|plugins\|engine> [--push]` | Bump all manifests, commit, tag, optionally push |
| `setup-cache.sh [--status]` | Enable/inspect the shared sccache build cache |

### Slash commands (`.refact/commands/`, local/personal)

- **`/ship`** — review ×2 → `check.sh` → land. In a worktree: `merge_worktree`
  (squash, single commit) then **`git push` separately** (merge is local-only).
  On main: commit + push. Creates a PR only via `/pr`, not by default.
- **`/fix-ci`** — paste a run URL → `ci-status.sh` + `ci-logs.sh` → fix iteratively
  → re-check (schedule via `cron`, don't busy-wait). Caps at 3 rounds.
- **`/issue`** — file a **high-level** labeled issue (feature/bug/initiative).
- **`/pr`** — push the worktree branch + open a PR; **keep the worktree alive**
  until the PR merges (for review changes), then clean up.
- **`/release`** — bump + tag + push from `main`. Tag taxonomy below.

### Worktree → main (the normal land flow)

1. Work in the worktree (branch `refact/chat/...` or `refact/task/...`).
2. `merge_worktree` squashes it into `main` as one commit and deletes the worktree.
3. **Merge ≠ push.** `git push origin main` from the main checkout is a separate step.
4. Run review and `check.sh` **in the worktree** before merging — not against the
   main checkout (a stale main checkout yields false review findings).

### Release tags (what triggers what)

| Tag | Triggers |
|---|---|
| `v<ver>` (any tag matches `*`) | All CI builds (engine, gui, vscode, intellij) — no publish |
| `release/v<ver>` | VS Code + JetBrains **publish** |
| `engine/v<ver>` | Engine release |

`tools/bump_release_version.py <ver>` bumps all 6 manifests (intellij, vscode×2,
gui×2, engine) in one shot; `release.sh` wraps it with tagging.

### GitHub CLI & CI logs

- `gh` is authenticated (`repo` + `workflow` scope) — covers issues, PRs, runs,
  job logs, releases. Projects v2 board would need `gh auth refresh -s read:project,project`.
- For failed CI: use `tools/dev/ci-logs.sh` (walks the jobs API + tails each failed
  job). `gh run view --log-failed` returns nothing for the reusable-workflow matrix.
- Parse run/job ids straight from pasted URLs
  (`.../actions/runs/<id>[/job/<jid>]`).

### Build cache across worktrees (sccache)

Each worktree has its own `target/` (cold build = 10-20 min). **sccache** caches
compiled crates globally so fresh worktree builds become cache hits, and it's
parallel-safe (unlike a shared `CARGO_TARGET_DIR`, which locks).

- Enable/check: `tools/dev/setup-cache.sh` (writes `~/.cargo/config.toml`
  `rustc-wrapper = "sccache"` + `CARGO_INCREMENTAL=0` + `SCCACHE_CACHE_SIZE`).
- `CARGO_INCREMENTAL=0` is **required** for sccache to cache. Trade-off: slightly
  slower single-worktree incremental rebuilds, much faster cold cross-worktree builds.
- npm cache (`~/.npm`) is already shared; use `npm ci --prefer-offline` in worktrees.
- Optional: `mold` linker (uncomment in `refact-agent/engine/.cargo/config.toml`).

### Issues = WHAT, task planner = HOW

GitHub issues are **high-level** (a feature, bug, or initiative — roughly one
task-planner's *name*). The technical breakdown (T-1, T-2, … cards) stays in the
**internal task planner**: isolated, disposable, never synced to GitHub. Label
taxonomy: `component/*`, `type/*`, `P0-critical`/`P1-important`/`P2-nice`,
`needs-triage`, `blocked`.

## Architecture Overview

```
┌─────────────────┐     postMessage      ┌──────────────────┐
│  IDE Plugins    │◄────────────────────►│   Agent GUI      │
│  (VSCode/JB)    │                      │   (React webview)│
└────────┬────────┘                      └────────┬─────────┘
         │ LSP (stdin/stdout)                     │ HTTP + SSE
         │ or HTTP                                │
         └──────────────┬─────────────────────────┘
                        ▼
              ┌─────────────────────┐
              │   Agent Engine      │
              │   (refact-lsp)      │
              │   HTTP :8001 + LSP  │
              └──────────┬──────────┘
                         │
       ┌─────────────────┼──────────────────┬──────────────────┐
       ▼                 ▼                  ▼                  ▼
┌─────────────┐  hidden roles wire map  ┌──────────────┐  ┌──────────────┐
│ Chat Layer  │────────────────────────►│   LLM APIs   │  │  Scheduler   │
│ event/plan  │                         │ 15+ providers│  │ cron + sleep │
└──────┬──────┘                         └──────────────┘  └──────┬───────┘
       │                                                         │
       ├──────────────► Local indexes (AST, VecDB) ◄─────────────┘
       └──────────────► Integrations (GitHub, MCP, shell, browser, DBs)
```

- **Engine ↔ GUI**: HTTP REST + SSE streaming (`/v1/chats/subscribe`). GUI sends commands via `POST /v1/chats/{id}/commands`, receives state via SSE events with monotonic `seq` numbers.
- **Engine ↔ IDE**: LSP protocol (tower-lsp) for completions/code-lens, plus HTTP for chat and tools.
- **IDE ↔ GUI**: `postMessage` bridge (VSCode `acquireVsCodeApi`, JetBrains `postIntellijMessage`). Events: file context, theme, tool calls.

### Hidden Plan Roles

- `set_plan` installs one hidden base `plan` only, using exactly one of inline `content` or an absolute `.md` `path`; never call it twice in one chat.
- `update_plan` records append-only hidden `event(plan_delta)` notes. `get_plan` reads the synthesized current plan from the base plan plus deltas.
- Hidden `plan` and `plan_delta` are `Never` compression-exempt. Provider wire adapters lower the base as `<plan>` and deltas as `<plan-update>` user-context blocks so the base plan remains cache-safe.
- Plan bodies are capped at 96KB chars, and transitions into Task Planner auto-create a pinned `initial-plan` task document when an initial plan is provided.

## Cross-Project Conventions

### Rust (Engine)

- **Formatting**: `rustfmt.toml` — 100 char lines, 4-space indent, Unix newlines, `reorder_imports = false`.
- **Async discipline**: All shared state through `GlobalContext` (`Arc<ARwLock<>>`). Drop read guards before `.await`. Never hold `gcx.read()` across await points.
- **Shutdown**: Check `shutdown_flag.load(Ordering::Relaxed)` in loops. Use `select!` with shutdown arm for channel receivers. Never `loop { sleep }` without a shutdown check. Store `JoinHandle` for spawned tasks — no fire-and-forget `tokio::spawn`.
- **Lock ordering**: Always acquire `gcx` ARwLock before inner mutexes. Reversing order risks deadlocks in background threads.
- **Error handling**: `Result<>` with contextual errors. `.ok_or_else()` over `.unwrap()` for runtime data.

### TypeScript/React (GUI)

- **Linting**: ESLint strict-type-checked, 0 warnings. Prettier enforced in CI.
- **State**: Redux Toolkit + RTK Query. Always use selectors from `features/Chat/Thread/selectors.ts`. Never access `state.chat.threads[id]` directly.
- **Styling**: Radix UI primitives + CSS Modules + design tokens. No inline styles, no hardcoded colors, no magic numbers.
- **Design system**: GUI design-system architecture, tokens, motion, responsiveness, overlays, and guardrails are defined in `refact-agent/gui/AGENTS.md` → `## Design System (Refact UI)`.
- **File naming**: `PascalCase.tsx` (components), `useCamelCase.ts` (hooks), `camelCase.ts` (utils), `PascalCase.module.css`.
- **No `any` types.**

### Kotlin (JetBrains Plugin)

- Java 17 target. Gradle build with IntelliJ Platform Plugin. Communicates with engine via HTTP + JCEF webview for chat.

### Python (Backend)

- Python 3.10+. FastAPI + Uvicorn. Type hints expected.

## Project Config Locations

| Scope | Path | Contents |
|---|---|---|
| User config | `~/.config/refact/` | `default_privacy.yaml`, `providers.d/*.yaml` |
| Cache | `~/.cache/refact/` | Shadow repos, logs, telemetry, integrations |
| Project | `.refact/` | `trajectories/`, `knowledge/`, `tasks/`, `integrations.d/` |
| System prompts | `refact-agent/engine/yaml_configs/defaults/` | Modes, subagents, toolbox commands |

### AGENTS.md Scoping Rules

AGENTS.md files can appear at any directory level. Scope = entire directory tree rooted at that folder. More-deeply-nested files take precedence on conflicts. Direct user instructions override all AGENTS.md content.

## Common Pitfalls

- **Shutdown hangs**: `loop {}` without `shutdown_flag`, bare `.recv().await`/`.changed().await` without `select!` + timeout, `tokio::spawn` without stored handle.
- **Lock inversion**: `gcx.read().await` → inner mutex is safe order. Reversing (inner mutex → gcx) causes deadlocks under load.
- **SSE sequence gaps**: Every event has monotonic `seq`. Gap → client reconnects for fresh snapshot. Never skip or reorder events.
- **Thinking block signatures**: Anthropic thinking blocks with cryptographic signatures must be preserved byte-for-byte. No JSON rebuilding, no field reordering.
- **GUI state**: Chat/history state is ephemeral (not persisted). Only `tour` and `userSurvey` survive Redux persist.
