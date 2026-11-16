# Refact Agent GUI

React chat UI for AI coding assistant. Builds to `dist/chat/` (browser UMD) and `dist/events/` (Node.js types). Consumed by IDEs (VSCode, JetBrains) and standalone web.

## Tech Stack

React 18.2 · TypeScript 5.8 (strict) · Vite 5.0 · Redux Toolkit 2.2 (RTK Query) · Radix UI/Themes · CSS Modules · Vitest 3.1 · MSW 2.3

## Quick Start

```bash
npm run test:all        # CI
npm run lint            # eslint strict-type-checked
npm run types           # tsc --noEmit
DEBUG=* npm run dev     # debug logging
```

## Architecture

```
React App → Redux (RTK Query) → LSP Server (:8001)   [chat, tools, caps, models, rag-status]
                               → IDE (postMessage)     [file ops, theme, context]
```

CodeGraph status is a first-class GUI surface: RTK Query polls `/v1/rag-status`, stores the typed `RagStatus` payload in the Knowledge slice, and renders toolbar mini indicators for CodeGraph, VecDB, and the legacy AST compatibility status.

### Directory Layout

```
src/
├── app/              # Store (combineSlices), middleware (50+ listeners), storage
├── features/         # Redux slices + feature UIs
│   ├── Chat/Thread/  # Multi-thread: reducer, selectors (~40+), actions, types
│   ├── Checkpoints/  # Workspace rollback
│   ├── Config/       # Global settings + FeatureMenu
│   ├── Connection/   # SSE connection status
│   ├── Customization/# Agent modes, subagent forms, tool parameter editor
│   ├── AgentsPanel/  # Live background-agent tree, messaging, cancellation
│   ├── FIM/          # Fill-in-Middle debug
│   ├── History/      # Chat history
│   ├── Integrations/ # Integration config
│   ├── Knowledge/    # Memory system, VecDB/CodeGraph status, knowledge graph view
│   ├── Login/        # Login page
│   ├── Pages/        # Navigation stack
│   ├── PatchesAndDiffsTracker/
│   ├── Providers/    # LLM provider config + OAuth
│   ├── Privacy/      # Zone cards, destination access, audit matrix, inspector, shield, result cards
│   ├── Statistics/   # Usage charts
│   ├── Tasks/        # Task management
├── components/       # Reusable UI (50+ dirs)
│   ├── ChatContent/  # Message rendering (ChatContent, ToolsContent, DiffContent)
│   ├── ChatForm/     # Input form + ToolConfirmation
│   ├── FIMDebug/     # FIM debug panel
│   ├── IntegrationsView/ # Integration UI + Docker + MCP logs
│   ├── Providers/    # ProviderForm, ProviderOAuth, ModelCard
│   ├── Sidebar/      # Navigation
│   ├── Tour/         # Onboarding (Welcome, TourBubble)
│   ├── Trajectory/   # Trajectory popover
│   └── UsageCounter/ # Token tracking, streaming counter
├── hooks/            # 72+ custom hooks
├── services/         # RTK Query APIs (20+) + chat commands/subscription
│   ├── refact/       # LSP APIs (caps, tools, docker, integrations, etc.)
├── contexts/         # AbortControllers, InternalLink
├── events/           # IDE integration event types + setup
├── lib/              # Library entry (render + events export)
├── utils/            # Utilities (@-command parsing, token calc, test helpers)
├── __tests__/        # 15+ test files (SSE protocol, integration, slices)
└── __fixtures__/     # 20+ fixture files for tests
```

## Chat Flow (Command/Event SSE)

```
User sends → POST /v1/chats/{chatId}/commands {type: "user_message", content}
           → Backend processes, streams via SSE
           → GET /v1/chats/subscribe?chat_id={id}
           → Events: snapshot → stream_started → stream_delta* → stream_finished
           → dispatch(applyChatEvent) per event → reducer updates state → React re-renders
```

### SSE Event Types

| Event                           | Purpose                                                       |
| ------------------------------- | ------------------------------------------------------------- |
| `snapshot`                      | Full state sync (resets seq to 0)                             |
| `stream_started`                | AI response beginning                                         |
| `stream_delta`                  | Incremental content (DeltaOp[])                               |
| `stream_finished`               | Complete with usage stats                                     |
| `message_added/updated/removed` | Message CRUD, including hidden `event`/`goal`/`plan` messages |
| `messages_truncated`            | Messages trimmed                                              |
| `thread_updated`                | Thread metadata changed                                       |
| `runtime_updated`               | Runtime flags changed                                         |
| `pause_required/cleared`        | Tool confirmation                                             |
| `ide_tool_required`             | IDE tool execution needed                                     |
| `subchat_update`                | Nested chat update                                            |
| `background_agent_updated`      | Live background-agent summary update                          |
| `queue_updated`                 | Command queue changed                                         |
| `ack`                           | Command acknowledgment                                        |
| `subscription_closed`           | Multiplexed only, no `seq`: one chat's stream ended/failed    |

### Delta Operations

`append_content` · `append_reasoning` · `set_reasoning` · `set_tool_calls` · `set_thinking_blocks` · `add_citation` · `add_server_content_block` · `set_usage` · `merge_extra`

### Command Types (POST /v1/chats/{chatId}/commands)

`user_message` · `abort` · `regenerate` · `update_message` · `remove_message` · `tool_decision` · `tool_decisions` · `ide_tool_result` · `set_params` · `set_goal` (optional user-set `budget`; omitted/absent = unlimited) · `set_goal_budget` (user-set/clear budget) · `update_goal` · `goal_control` · `retry_from_index` · `branch_from_chat`

### Sequence Validation

Every event has a `seq` number. `snapshot` resets to 0, each subsequent increments by 1. Gap detected → immediate reconnect for fresh snapshot.

`subscription_closed` (`reason`: `ended` or `unavailable`; `retryable`, which defaults to `true` when absent; `message`; sent only when `/v1/chats/subscribe-many` carries `per_chat_close=1`) is handled before seq validation and never counts as a gap: only that chat's consumers fail, the shared stream stays up, and `retryable: false` stops automatic retries. `SSE connect timeout` before response headers fails every chat. When headers arrive, each chat still waiting for its snapshot gets its own full bootstrap window (`connectTimeoutMs`); a chat that misses it fails alone with `SSE chat snapshot timeout`, and re-subscribing it while still pending re-attaches without a rebuild and starts a fresh window. Per-chat failures do not mark the shared agent roster stale, and a partial chunked snapshot left by a timed-out chat does not turn a normal end of stream into an error. Network/HTTP errors and unexpected EOF still fail every chat. Failing chats back off on their own count, subscribe/unsubscribe bursts coalesce into one rebuild that starts before the old stream is stopped, and a scheduled rebuild is cancelled when the desired chats return to the live connection's set. If a chat's first snapshot completes while it has no consumer (bootstrap timeout backoff, refresh gap), the multiplexer parks that snapshot plus the chat's following events (at most 256 events / 8 MiB per chat, 32 MiB across chats; data that would exceed the total is not parked) and replays them in order to the next consumer on the same connection without a new fetch; overflow, `subscription_closed` or the connection stopping drops the parked data and the next subscribe rebuilds, and an overflow after a consumer attached, or a consumer throwing during the handover, schedules a rebuild for that chat's consumers. A chat closed with `retryable: false` before any snapshot records `subscription_failure` on its runtime, busy or not (busy chats also keep the existing `markThreadSseError` error state); `ChatContent` then shows "This chat could not be opened" with the reason and a Retry that dispatches `requestSseRefresh`, which clears the failure and resubscribes.

## State Management

**Store**: `src/app/store.ts` — `combineSlices` with 12+ slices + 20+ RTK Query APIs

### Key State (per-thread)

```typescript
state.chat.threads[id]: ChatThreadRuntime = {
  thread: ChatThread,         // id, messages, model, title, tool_use, boost_reasoning, reasoning_effort, temperature, mode, is_task_chat, task_meta, goal
  streaming: boolean,
  waiting_for_response: boolean,
  prevent_send: boolean,
  error: string | null,
  queued_items: QueuedItem[],
  attached_images: ImageFile[],
  confirmation: ThreadConfirmation,  // pause, pause_reasons, status
  snapshot_received: boolean,
}
```

**Navigation**: `current_thread_id`, `open_thread_ids` (tabs), `threads` map

### Redux Persist

Whitelist: `["tour", "userSurvey"]` (NOT chat/history — those are ephemeral)

### Key Selectors (features/Chat/Thread/selectors.ts, ~40+)

Always use selectors. Never access `state.chat.threads[id]` directly in components.

Hidden-role and privacy selector convention:

- `selectVisibleMessages(state, threadId)` excludes `event`, `goal`, and `plan`; use this for normal transcript rendering.
- `selectEventLog(state, threadId)` returns normalized `EventMessage[]` for EventLog surfaces and excludes `plan_delta`, `goal_delta`, and `goal_pursuit` events.
- `selectCurrentPlan(state, threadId)` returns the latest base `PlanMessage` by version/index for PlanBanner.
- `selectPlanDeltaEvents(state, threadId)` returns hidden `event(plan_delta)` messages in index order.
- `selectSynthesizedPlanText(state, threadId)` returns base plan text plus append-only plan-delta notes using the synthesis separator.
- `selectPlanHistory(state, threadId)` returns the current base plan followed by plan-delta events for history UI.
- `selectGoalById(state, threadId)` / `selectGoal(state)` read `thread.goal`, the `GoalSnapshot` projection supplied by `Snapshot.goal` and runtime updates.
- `selectGoalStatusById`, `selectGoalProgressById`, and related helpers must read the projection, not scan hidden `goal` messages in components.
- `selectPrivacyFilesById(state, threadId)` returns the deduplicated `extra.privacy.files` records for chat privacy surfaces.
- `selectPrivacyStepContextById(state, threadId, toolCallId)` returns the blocked step and clean-branch boundary for a privacy tool result.

If a new component needs hidden-role or chat privacy data, add or reuse a selector first instead of filtering `thread.messages` inside the component.

### RTK Query APIs

All generate hooks (`useGetCapsQuery`, etc.). Dynamic base URL from Redux state. Auto-injects auth.

| API                             | Key Endpoints                                                          |
| ------------------------------- | ---------------------------------------------------------------------- |
| capsApi                         | `/v1/caps`                                                             |
| commandsApi                     | `/v1/at-command-completion`, `/v1/at-command-preview`                  |
| toolsApi                        | `/v1/tools`, `/v1/tools/check_confirmation`                            |
| dockerApi                       | `/v1/docker-container-list`, `/v1/docker-container-action`             |
| integrationsApi                 | `/v1/integrations-list`, `/v1/integration-get`, `/v1/integration-save` |
| modelsApi, providersApi         | `/v1/customization`                                                    |
| checkpointsApi                  | `/v1/preview_checkpoints`, `/v1/restore_checkpoints`                   |
| linksApi                        | `/v1/links`                                                            |
| trajectoriesApi, trajectoryApi  | `/v1/trajectories/*`                                                   |
| tasksApi                        | Tasks CRUD                                                             |
| chatModesApi, customizationApi  | Agent modes/customization                                              |
| knowledgeApi, knowledgeGraphApi | Knowledge/memory                                                       |
| ragStatusApi                    | `/v1/rag-status`                                                       |
| privacyApi                      | `/v1/privacy/policy`, `/v1/privacy/status`, `/v1/privacy/inspect`      |

`RagStatus` includes `codegraph: { counts: { nodes, edges, files, fts_docs }, queued, state, error } | null`, `codegraph_alive`, and `codegraph_error`; CodeGraph states are `turned_off`, `indexing`, `working`, and `error`. Keep `services/refact/types.ts`, `features/Knowledge/knowledgeSlice.ts`, and `components/ConnectionStatus/RagStatusIndicators.tsx` in sync with the backend shape.

Chat uses **Commands API** + **SSE subscription**, not RTK Query.

Privacy policy, status, destination inspection, and policy saves belong to `privacyApi`. Privacy holds no Redux slice: the page is driven entirely by RTK Query plus local component state. Do not copy server policy/status data into Redux or scan chat messages inside Privacy components.

The Privacy & Access page is destination-first, not a zone × destination matrix. `features/Privacy/access.ts` is the single home for the shared predicates (`zoneAllowsDestination`, `mcpAllowedForProvider`, `SHELL_BEHAVIOR_OPTIONS`) so component files export components only and stay Fast-Refresh clean. `ZoneCard` owns pattern editing as chips (`PatternChips`), `DestinationAccess` groups destinations by kind and expands one row at a time, and `AccessMatrix` is the read-only audit grid with a sticky zone column, collapsed by default. `PrivacyPolicy.tool_access.providers[<provider>]` is optional by design: an absent provider may use every MCP server, so `match_counts` and `tool_access.providers` are typed with `| undefined` values and must be read with `??` defaults. The `policy` returned by `/v1/privacy/policy` is the **global-only** document that the page edits; `match_counts` and destinations reflect the merged effective policy, and `has_project_overrides` must be surfaced so users know a project file narrows what they see. `isCatchAllZone` guards the zone whose removal would leave an invisible engine-synthesized allow-all fallback.

## Key Hooks

| Hook                             | Purpose                                                                                  |
| -------------------------------- | ---------------------------------------------------------------------------------------- |
| `useChatActions`                 | submit, abort, regenerate, respondToToolConfirmation                                     |
| `useChatSubscription`            | Single chat SSE connection                                                               |
| `useAllChatsSubscription`        | Multi-tab SSE manager                                                                    |
| `useEnsureSubscriptionConnected` | Wait for snapshot before actions                                                         |
| `useEventBusForApp`              | IDE → GUI events (file context, new chat, tool approval)                                 |
| `useEventBusForIDE`              | GUI → IDE events (open file, paste, tool call)                                           |
| `usePostMessage`                 | Transport: VSCode `acquireVsCodeApi`, JetBrains `postIntellijMessage`, web `postMessage` |
| `useCheckpoints`                 | Checkpoint preview/restore                                                               |

## Components

### ChatContent (src/components/ChatContent/ChatContent.tsx)

Dispatches messages to specialized renderers. Iterative processing (not recursive). Groups assistant messages with related diffs + tools.

Compression visibility rules:

- `compression_report` messages render visibly as `SummarizationMessage` cards via `syntheticCompressionReportMessage()` and should not be hidden as internal roles.
- Assistant messages with `extra.compression.kind === "llm_segment_summary"` render visibly as compression/summarization cards via `syntheticSummarizationMessage()` instead of ordinary assistant transcript text; the card now distinguishes deterministic compaction, LLM summaries, merged-history summaries, and reactive compaction labels.
- Visible compression failure events are narrowly defined as `event` messages whose metadata is `subkind: "system_notice"`, `source: "chat.summarizer"`, and whose content starts with `Context compression failed:`; these render as visible error display items. Other hidden events, including `plan_delta`, `goal_delta`, and `goal_pursuit`, stay out of normal transcript display.
- The footer compression indicator is driven only by the thread runtime `is_compressing` selector. It appears while the backend reports an active compression attempt and disappears on terminal `RuntimeUpdated`/snapshot state when `is_compressing` becomes false or is cleared.
- Restore paths and SSE-error/reconnect cleanup must clear `is_compressing`, `compression_phase`, and `compression_reason` so stale compression progress does not survive thread restoration or subscription failures.

| Role           | Component                  | Notes                                                                                                              |
| -------------- | -------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| `user`         | UserInput                  | Editable, checkpoints badge, images, compression hint 🗜️                                                           |
| `assistant`    | AssistantInput             | ReasoningContent → Markdown → ToolsContent → DiffContent → Citations                                               |
| `tool`         | (inline in AssistantInput) | Skipped in top-level render                                                                                        |
| `diff`         | DiffContent                | Grouped by tool_call_id, apply/reject UI                                                                           |
| `context_file` | ContextFiles               | Memory/knowledge attachments 🗃️                                                                                    |
| `event`        | EventLog                   | Hidden from normal transcript; grouped under nearby assistant turns, excluding plan/goal deltas and pursuit events |
| `goal`         | TaskProgressWidget         | Hidden from normal transcript; projected through `thread.goal`                                                     |
| `plan`         | PlanBanner                 | Hidden from normal transcript; latest version pinned above chat                                                    |

### EventLog component pattern (src/components/ChatContent/EventLog/)

EventLog renders hidden `event` messages without polluting the main transcript.

- Feed it selector-normalized `EventMessage[]`; do not pass raw backend messages with only `extra.event`.
- Keep it collapsed by default and persist collapse/filter state per thread in localStorage.
- Show subkind icon/chip, source, one-line `content`, and expandable JSON payload.
- Use `EventLogEntry` for row-level behavior and `eventSubkind.ts` for the single source of icon mapping.
- Click behavior stays subkind-specific and explicit: `process_completed` scrolls to matching `[data-exec-process-id]`; `cron_fire` opens the scheduler page via `openScheduler`.
- Tests belong next to the component and should cover collapsed/expanded state, filters, localStorage persistence, and any subkind click behavior.

### PlanBanner component pattern (src/components/ChatContent/PlanBanner/)

PlanBanner is a read-only fallback for legacy plan roles when no nested goal plan is available. Current plans render inside TaskProgressWidget through GoalPlanSection and the goal snapshot; targeted goal deltas supply append-only plan history. Task workspaces read the canonical task goal through the task-goal query, including history on request. Never introduce a second authoritative plan editor.

- Read plan data with `selectCurrentPlan`, `selectSynthesizedPlanText`, and `selectPlanHistory`; do not scan messages directly.
- Header format: `📋 Plan — {mode} · v{version} · {humanizedAge}`.
- Body uses existing Markdown rendering, bounded scrolling, and a persisted collapse toggle. Keep v1 expanded by default unless the user toggles.
- Manual plan editing is not exposed in the banner; plan changes arrive as append-only `plan_delta` events.
- History modal lists the current base plan followed by each delta note in index order.
- Keep sticky styles in `PlanBanner.module.css`; avoid inline styles.

### TaskProgressWidget goal pattern (src/components/TaskProgressWidget/)

TaskProgressWidget is the GUI owner for goal display and controls. It reads `thread.goal` with `selectGoalById` and never scans hidden `goal`, `goal_delta`, or `goal_pursuit` messages directly. The widget shows the goal content, status, turn/token/no-progress counters, verifier attempts, pursuit events, and ownership transfer hints from `GoalSnapshot`. Goal budget hard limits are optional and unlimited by default; absent, `null`, or `0` limits render as plain usage plus `No budget limits` rather than a ratio.

Pause/resume/stop stay reachable while collapsed: compact icon controls (`IconButton`, labels `Pause goal` / `Resume goal` / `Stop goal`) render on the right side of the collapsed widget header and the collapsed goal header as siblings of the collapse trigger — never nested inside the trigger button. Exactly one control set is visible at a time (collapsed widget header, collapsed goal header, or the labeled buttons in the expanded goal body), and all share one availability rule (`goalControlAvailability`). Verifier attempts, goal events, and the expanded widget body are bounded scroll regions (`scrollbarThin` + `max-height`) so long histories scroll instead of clipping inside the dock's `overflow: hidden`.

Expanded goal controls precede the potentially lengthy goal and plan text. Compact headers summarize task progress with a nonshrinking count and bounded optional dots. Goal/plan reading regions and histories are bounded disclosures, with a full-reader Dialog for large content; the expanded editor moves the same draft into a Dialog and returns to the existing Save action. Editing an existing goal still appends an update rather than replacing its approved base.

Goal controls dispatch chat commands through `useChatActions`: `setGoal(content, budget?)` sends `set_goal` with optional user-set hard limits, `setGoalBudget(budget)` sends `set_goal_budget` to set or clear limits on an existing goal, `updateGoal(note?, plan?)` sends `update_goal` with outcome and/or nested plan changes, and `controlGoal(action)` sends `goal_control` with `pause`, `resume`, or `stop`. Blank budget editor inputs are unlimited and must omit hard-limit keys. The reducer updates `thread.goal` from `Snapshot.goal`; `runtime_updated` mirrors only mutate the runtime fields on an existing projection (`goal_active`, `goal_status`, `goal_turns_used`, `goal_tokens_used`, `goal_no_progress_turns`). Hidden `event(goal_delta)` and `event(goal_pursuit)` remain out of EventLog and the normal transcript.

Status semantics mirror the backend: `active=true` means this chat owns the goal; pursuit requires `active && status === "active"`. Terminal or held states include `verifying`, `paused`, `completed`, `stopped`, `budget_exhausted`, `no_progress`, and `transferred`. Transfer surfaces should show `transferred_from` / `transferred_to` from the snapshot rather than inferring ownership from chat mode.

### ToolsContent (src/components/ChatContent/ToolsContent.tsx)

Largest component (~1900 lines). Dispatches a `ToolCall` list to the right specialized `ToolCard` via a `processToolCalls` switch on `normalizeToolName(toolCall.function.name)`. Covers file ops (`cat`, `tree`, `search_pattern`, `search_semantic`, `search_symbol_definition`, `mv`, `rm`, `add_workspace_folder`), shells (`shell`, `shell_service`, `process_*`), agentic tools (`subagent`, `tasks_set`, `task_done`, `wait`, `schedule`, `pause`, `agent_lifecycle`, `agent_done`, `final_report`), knowledge (`knowledge`, `search_trajectories`, `get_trajectory_context`, `create_knowledge`), web (`web`, `web_search`), editing (`patch_*` raw-text-doc family, `text_edit`), Chrome/browser (`ask_questions`, `chrome_*`), and the OpenAI Responses tool family (`audio`, `web_search_call`, `file_search_call`, `code_interpreter_call`, `computer_call*`, `image_generation_call`, `mcp_call`, `mcp_list_tools`, `refusal`). Falls back to `GenericTool` for anything unhandled.

- New background work is created with `subagent`. Its results are decorated with a `BackgroundAgentCard` built from `BackgroundAgentSummary`, showing status, selected model, current tool, goal/plan, worktree/merge state, questions, usage, and edit/result summaries. Legacy delegate calls and records are normalized into the same card only for compatibility.
- Multimodal results (`MultiModalToolResult` with image content) are rendered by `MultiModalToolContent` and surface inline image previews via `DialogImage`.
- Per-tool-group collapsible state is persisted in `CollapsibleStore` (`useStoredOpen`); the `ToolUsageSummary` header shows call counts, the latest subchat step (`step: text` progress entries), and attached-file chips.
- Background-agent trajectory links dispatch `createChatWithId` + `switchToThread` so opening a child chat routes the user into the right tab.

### Background-agent state and panel

Snapshots and `background_agent_updated` events normalize into each thread's `background_agents` map;
newer `change_seq` wins, with terminal state breaking equal-sequence ties. `AgentsPanel` combines that live
state with `GET /v1/background-agents?chat_id=...`, renders the recursive tree with Active/All filters and
aggregate usage, and exposes trajectory navigation, messages, and confirmed subtree cancellation. The
compact chat card and panel preserve legacy `delegate` kinds for stored historical records, but the GUI
must present `subagent` as the sole current spawn tool.

**Tool status**: ⏳ thinking · ✅ success · ❌ error · ☁️ server (`srvtoolu_*` prefix)

CodeGraph tools render through the engine-analysis tool-card path. The recognized names are `codegraph_overview`, `code_health`, `git_risk`, `code_why`, `code_duplication`, `code_map`, `security_scan`, `pr_blast`, and `dead_code`; keep icons/status rendering aligned with the backend tool descriptions. Code Intelligence response severity unions no longer include `Info`; keep GUI unions to `Low`, `Medium`, `High`, and `Critical`. Page-level index-readiness banners should be driven from loaded response `index_state` data and disappear once `cross_file_ready` is true.

These nine tools return **JSON** shaped by `engine/src/codegraph/code_intel_api.rs` — the same response structs the `/v1/code-intel/*` HTTP endpoints serialize. Every result is `{ "tool": "<name>", "summary": "<one line>", ...payload }`. Never text-parse a tool result.

- `engineAnalysisJson.ts` owns the contract: per-tool TypeScript interfaces, `parseEngineAnalysisJson` (safe `JSON.parse`, returns `null` for non-object payloads such as tool error strings), and `buildAnalysisReport(toolName, value)` which narrows with runtime guards and maps into the presentational `AnalysisReport` view model.
- `AnalysisReport.tsx` is presentation only — warnings, `summary` headline, `index_state` chips, fact metric cards, and sections of rows (`title`, `detail`, `severity`, `lead`, `metrics`, `paths`, `tags`). Rows and sections carry a unique `line` counter used as the React key.
- `EngineAnalysisTool.tsx` renders the report when the payload parses, falls back to a `ShikiCodeBlock` of the raw string otherwise (tool errors), and renders `code_map.markdown` through `Markdown`.
- Adding a field: extend the Rust struct in `code_intel_api.rs`, the TS interface in `engineAnalysisJson.ts`, and the adapter that surfaces it — in the same change.
- Absolute paths are shortened by the report-wide common prefix (`shortenPath`), full path kept in `title`, prefix disclosed once at the bottom.

### Python tool UI (Code Mode)

`python` is a persistent per-chat script runner whose nested calls invoke ordinary tools. A run is one script invocation; each nested call has its own call ID and sequence number. Shared wire types/guards live in `services/refact/types.ts`, SSE parsing in `chatSubscription.ts`, and command senders in `chatCommands.ts`.

#### Events, state and cards

- SSE `python_run_update` carries the outer `tool_call_id`, `run_id` and a `kind`: `started`, `call_started`, `call_finished`, `stdout`, `display`, `detached` or `finished`. Approval events are `python_approval_requested` and `python_approval_resolved`. Reconnect snapshots replace live state from `runtime.python_runs` and `runtime.python_approvals`; they are not reconstructed from rendered text.
- `features/Chat/Thread` stores `pythonRuns` keyed by outer tool-call ID and `pythonApprovals` keyed by request ID. Run state is `running | background | finished`, with a separate `finalStatus`. Calls upsert by call ID and retain the last 200; stdout retains a UTF-8-safe 16 KiB tail. `display` updates append to the run's `displays` by `index` (a duplicate index is ignored, order is by index, at most 12); snapshot runs replace them from `runtime.python_runs[].displays`. `selectPythonRun` reads a call's live run; memoized `selectPythonApprovals` returns oldest requests first. Use these selectors rather than new component scans. A valid stored `extra.python` with the same run ID replaces finished transient state; truncation prunes removed/orphaned finished runs without dropping unrelated active runs. `clearPythonLiveState` clears both maps.
- The engine flattens `ChatMessage.extra` on the wire, so tool messages and background `tool_job_result` deliveries carry `python` at the top level. `normalizeMessage` in `features/Chat/Thread/reducer.ts` (snapshot, `message_added`, `message_updated`, history restore) lifts it into `extra.python` only for tool-role messages and `tool_job_result` deliveries and drops it from any other message; `flattenedWireExtra` in `ChatContentDisplayItems.ts` lifts it for verified job deliveries.
- Read stored metadata with `normalizePythonRunExtra` (`isPythonRunExtra` is its boolean form): optional `description`, `code_preview`, `error.line` and per-call `line`/`col` are dropped when mistyped (older engines omit them); unknown run status, call status or approval values from a newer engine become `unknown`, so one new enum value never drops the card to raw text; malformed structure still fails closed. Optional `state.note` is the engine's own sentence about the session after the run. `reducer.python.test.ts` pins this with real messages in `__tests__/pythonWireFixtures.ts`.
- `ToolCard/PythonTool.tsx` is the **script card**: an ordinary `TranscriptItem` row (`expandable={false}`, always open, `data-testid="script-card"`, `data-python-run-id`), verb `Script`, never the word "Python" in visible text or accessible names. Title from `scriptTitle` (`pythonDisplay.ts`): the non-blank `description` argument, else `extra.python.description`, else the code's title line (`codeTitleLine`, the same rule as the engine's `code_preview`: the first non-blank line when it is a `#` comment with text (not `#!`), shown without the `#` in the plain font; else the first non-blank unindented line not starting with `#`, `import `, `from `, `def `, `async def `, `class `, `@` or a triple quote; else the first non-blank non-comment line; monospace, cut to 120 characters), else `code_preview`, else "Run script". The shared vectors are `TITLE_VECTORS` in `scriptDisplay.test.ts`; the panel's `runTitle` reads `code_preview` through the same `previewTitle`. Meta: `N calls · N files · duration` (shared `formatCallDuration`, ticking in a memoized `Elapsed`), plus "ran in background" for a finished background run; the meta never shrinks, the title truncates. Status goes through `scriptStatusView` (one table for the card, rows and panel: Running, Background, Waiting for approval → `needs_you`, Failed, Stopped = `cancelled`, Timed out, Limit exceeded, Result not available, Unknown status; Completed is quiet). Without a result or live run the shared `useUnrecordedResultState` decides. Quick actions: Stop while live (same "Stopping…"/10 s/clear-on-end behaviour) and a details chevron (constant name "Details", `aria-expanded`, `aria-controls` the body) where other rows draw theirs; it toggles the per-tool-call stored-open state `script:<id>`.
- Body, top to bottom: one state line (waiting, `ErrorType: message` in danger tone above any output, the limit as one sentence from `limitSentence`, stopped, live background; `role="status"`/`"alert"` only when the card saw the run live, never for history); the **code view** (`ScriptCodeView` in `PythonBlocks.tsx`); "Code truncated" when the arguments were cut (Copy code is then disabled); "Showing N of M calls" whenever `calls_truncated`; "Tool calls · N" only for calls without a usable `line`; the outcome preview (Result, else Output; both when expanded) bounded by CSS height (4 lines compact with a fade and "Show more" that expands the card when clipped, 12 lines + scroller and "Expand output" expanded, at most 20 000 characters cut on a code-point boundary, "Preview truncated · full output 1 MB", live output at the reducer's 16 KiB cap says "showing the last 16 KB"; small JSON results ≤ 4 000 characters are highlighted; `extra.python.result_truncated: true` adds "Result cut by the engine" next to the Result label, compact and expanded); "Printed output hidden · N lines" when a Result hides printed output in the compact state; Changes (`scriptChanges`: call diffs and written files deduplicated by normalized path, artifacts excluded, standard collapsed `GroupedDiffs` rows, 5 shown + "Show N more files"); Artifacts (`scriptArtifacts`, basename first, size, capability-gated Open, Copy path). Expanded only: Traceback, `TranscriptFacts` (duration, tool calls, `sessionSummary`, run ID), actions (Copy code/output, Open code/full output for `<run_dir>/code.py|stdout.txt` when the host can open files, Show workspace) and `TranscriptRaw` (Arguments, Raw result).
- The code view highlights once with `useShiki`, splits with `extractCodeLines` and turns Shiki spans into React text runs (`htmlLineTokens`/`CodeLineText`); theme colours are applied as inline `color` (the same data Shiki emits). Tool callees get the accent treatment: from a record's `line`/`col` (col counts Unicode characters), else lexically (`lexicalCallees`: identifier followed by `(`, known tool names of this run's records and pending approvals, outside comments, not a method). A line with calls is a button whose name is its code and annotation and whose description is the status (visually hidden text, "Failed here" on the error line); it carries an aggregated status icon (`aggregateCallStatus`), tinted background and a never-cut annotation (`×N`, approval label, `+a −r` counted like the Changes row, duration); it opens the call details inline (one call: `CallDetails`; several: `NestedToolCalls` with a limit of 20). `error.line` gets the danger tint. Folding (`foldAnchors` + `foldCodeRows`): ≤ 10 lines whole; else anchors in priority order (error line, waiting, running, failed/denied/stopped, other call lines by number, then the last non-empty line) within 12 rows, with clickable "⋯ N lines" fold rows that expand the card and focus the first revealed line; no anchors → first 8 + one fold. A full-width "Show all · N lines"/"Show less" appears only when something is folded. Expanded code renders 400 lines per "Show N more lines" step; a displayed line is cut at 2 000 characters with "… N more characters" (also expanded); `calleeAtColumn` walks code points. Rows are memoised with stable callbacks and per-line inputs that keep identity when unchanged (`useStableByLine`), so a live call update re-renders only its line. The gutter widens for 4- and 5-digit line numbers (`data-digits`). Token colours from Shiki are accepted only as hex, `rgb()`/`hsl()` or `var(--…)`.
- Detached runs initially have a tool-job receipt, not final Python metadata. The card looks in `ToolJobResultsContext` for the newest delivered tool-role message with valid `extra.python`, preferring that over the ordinary supplied/activity result. Keep this path when changing tool-job rendering: the final result may arrive through a hidden job-delivery event rather than replacing the receipt.
- **Model calls.** Call records with `tool: "llm"` are model calls made by the script (`model?`, `tokens_in?`, `tokens_out?`; `args_preview` is `{prompt, model?, schema}`). They share the `seq` sequence and are counted in `calls_total`; tool calls = `calls_total` − `extra.python.llm.calls` (missing = 0, clamped). Helpers in `ToolCard/scriptDisplays.ts` (`isModelCall`, `callCounts`, `callsTokens`, `compactCount`, `readLlmPreview`, `callModel`, `tokensInOut`). `llm` and `show` are always highlighted as callees (`HELPER_CALLEES`, also lexically). A line's annotation adds tokens (`×12 · 41.2k tokens · 13.7 s`). Line detail per call: Model, Tokens (`in 3k · out 433`), Duration, Status, Prompt and Answer/Error. Card meta tells both kinds apart (`1 call · 12 model calls · 9.6 s`); a card row narrower than 560 px (container query) shows `1 + 12 model` instead so at least 12 title characters stay visible (full wording stays in a visually hidden span and the tooltip). Details add one "Model calls" totals line from `extra.python.llm` (calls · in · out · models). Unplaced calls are labelled "Tool calls", "Model calls" or "Calls".
- **Displays.** `PythonDisplay` (`index`, `kind`, `title?`, `columns?`, `rows?`, `rows_total?`, `truncated`, `x?`, `y?`, `text?`, `path`) is read with `normalizePythonDisplay(s)`: index/kind/path required, other bad optionals dropped, non-plain cells become null. `extra.python.displays`/`displays_total`/`displays_truncated`/`llm` are tolerant optionals of `normalizePythonRunExtra`. A live card reads `run.displays`, a finished card `extra.python.displays`. `ToolCard/ScriptDisplays.tsx` renders them between the code view and Output in index order, first 3 + "Show N more displays" (and "Showing N of M displays" when the engine kept fewer). Each display: caption row (kind icon, title or kind label, size such as "12,000 rows × 4 columns", icon actions Show as table (charts), Copy (TSV with header row; markdown copies its text) and Open full data via `useOpenFileInApp`) and a surface. Tables reuse the chat markdown table tokens with a sticky header, numeric columns right-aligned in tabular numerals, `null` as a muted dash, cells over 80 characters clamped to two lines and expanding on click, a focusable named scroll region (12 rows card / 8 rows rail) and "Showing 200 of 12,000 rows". Charts (`bar`, `line`, `area`, `scatter`, `pie`; `chartData` maps `columns`/`rows` with `x`/`y`, a non-number y is skipped for that series) render through `ScriptChart.tsx`, loaded with `React.lazy` so ECharts stays out of the chat entry chunk; options come from `scriptChartOption.ts` with the shared `chartTheme` helpers (palette from `--rf-chart-*`, legend only for several series, no animation under reduced motion). The chart frame has a fixed height (220 px card, 160 px rail) used by the placeholder too, `role="img"` with a generated label, and an error boundary that falls back to the table. Markdown renders with the chat `Markdown` (the assistant options) inside `TranscriptClamp`. Unknown kinds show only the caption with Open full data.
- **Engine reads and changes.** Scripts call `await engine("<function>", ...)`. Its call records are ordinary records with `tool: "engine.<function>"` (a change's `summary` is "label: old → new"). `engine` is a helper callee like `llm` and `show`. Helpers live in `ToolCard/engineChanges.ts` (`isEngineTool`, `engineFunction`, `engineValueView`, `isTextPair`, `engineWarning`, `engineCallClass`, `engineChangeCount`, `approvalAnnouncement`, `EngineChangesContext`). Line detail of an engine call (`EngineCallDetails` in `NestedToolCalls.tsx`): Function, Kind in words (Read / Change / Risky change / Destructive change from the applied change with the same `seq`; else "Change (not applied)" for `approval: denied` or `cancelled` (the engine marks a change it refused before asking `denied`, so the approval label is the neutral "Denied", never "Denied by you"; the reason is in the summary); else Read for `approval: not_needed`; else omitted; the same in the nested rows), Duration, Status, Result (`summary`, through `visibleText` like the row summary), Arguments. `extra.python.engine_changes` (applied changes only, ≤ 50) and `engine_changes_total` render as "Engine changes · N" (`EngineChangesSection.tsx`) just before the file Changes section: one compact row per change (risky/destructive marker with icon and words; an entry with `outcome: "unknown"` (the engine started it but could not confirm it finished, e.g. a timeout; absent = applied, other values dropped) adds a dashed-outline "Outcome unknown" marker with a help icon and the line "The engine could not confirm whether this change was applied.", and still counts as an engine change; sentence-case title, scope chip, function, dense items), bounded at the transcript clamp height and scrolling inside, "Showing 50 of N" when the total is larger. Card meta adds "N engine changes" only when there are some (narrow: "N changes"); a live run counts approved, finished engine calls. Panel run meta adds `· N engine changes` from the workspace view's `engine_changes`. Values follow one rule in the bar and the card (`EngineItems.tsx`): scalars and short JSON inline (`label: old → new`), absent old a muted italic "not set", absent new "removed"; a string pair with a line break or over 80 characters, unless the engine marked the item `truncated`, is shown as computed line hunks (`textPairHunks`, the `diff` package's `structuredPatch` with 2 context lines) through `DiffBlock` (bar only; the card stays dense); other long or structured values are labelled "Was"/"Now" blocks clamped with "Show N more lines"; `truncated` adds "Value cut by the engine".
- **Untrusted text.** Engine- and script-provided strings (engine titles, scopes, labels, functions, warnings, values; display titles, column names and cells; the run title in the bar) go through `ToolCard/safeText.ts`: `visibleText` replaces bidi controls, zero-width characters and other invisible format characters with a visible `⟨U+XXXX⟩` marker, `shortText` cuts to 120 code points; `ShortText.tsx` renders a cut text with "Show all N characters". Display markdown renders with `<Markdown inert>`: images become "alt (url)" text, puzzle links stay text, artifact and diagram fences (`html`, `svg`, `mermaid`, …) render as plain code, and only http(s)/mailto links remain (in-app `refact://` links become text). The default (`inert` off) is unchanged and pinned by `Markdown.inert.test.tsx`. "Open full data" is enabled only for a path inside the run's `workspace_dir` (card) or the view's `workspace_dir` (panel).
- **Tables.** At most 50 rows are mounted, with "Show N more rows of M" in steps of 50. Cells hold no state: a cell longer than 80 characters is cut in the DOM and opens on click through one handler per table (opened cells kept in one map, at most 5,000 characters shown). Header cells beyond `columns` are named "Column N" for screen readers. A failed chart (including a failed chunk load) falls back to the table with "Try again", which creates a fresh lazy component. The user's choices (shown as table, shown rows, opened cells, show all displays) survive the virtual list unmounting the card: `ScriptDisplays` gets `runKey` (the script's tool call id) and keeps them through `useRememberedState` (`ToolCard/useRememberedState.ts`), keyed `script-displays:<run>[:<display index>]:<choice>` and stored per transcript next to the `CollapsibleStore` (reset with it on chat switch); only values the user set are stored.
- **Normalisation cost.** `normalizePythonRunExtra` caches its result per input object (`WeakMap`); store messages are immutable, so the activity rail and run pruning, which read it on every store update, normalise each stored metadata object once.
- `NestedToolCalls.tsx` renders rows ordered by `seq`: status icon, tool name, a one-line argument summary (`description` for shell-like tools, then command, then path/paths/file_path, pattern, query, url, symbols, else the first string; about 80 characters; never raw JSON), the approval label unless `not_needed`, and duration. A row whose call ID has a pending approval shows "Waiting for your approval". Rows expand to key/value arguments, the result summary and guarded `DiffChunk` values through `GroupedDiffs`. Lists longer than 20 show the newest 20 with "Show N earlier calls". Rows are keyed by `seq` (call IDs may repeat) and memoized; file and artifact keys include the index. This is bounded call-record rendering, not a second top-level ToolsContent dispatch of arbitrary nested results.

#### Approvals and Stop

A request whose `tool_name` starts with `engine.`, or whose `engine` part is unreadable, is never shown as an ordinary request: `normalizePythonApprovalRequest` gives it a risky engine part (title "change the engine", no items, a warning that the details could not be read). Engine items without a usable label are kept as "(unnamed)"; an item list that is not a list sets `items_dropped` and the bar says how many entries are not shown; an item with neither side reads "no value given". The bar's details scroll inside a bounded area (`.content`, at most 60dvh) so the actions stay on screen. For risky and destructive requests the callout is `role="alert"` and Deny and Allow once are described by the heading and the callout. Requests with `engine` (`PythonEngineRequest`: `function`, `class` change/risky/destructive with any other value read as risky, `title`, `items`, optional `scope` and `warning`; read with `normalizePythonApprovalRequest`, which drops an invalid `engine` part instead of the request) get their own body: heading "Script wants to <title>" with a scope chip (cut only past 160 characters, wrapping to at most three lines, with the full scope in its `title` and `aria-description`), the full `engine.<function>` name in small monospace under it, the run line, a warning callout for risky (warning tone, shield icon, "Risky change.") and destructive (danger tone, trash icon, "Destructive change.", plus "This cannot be undone." unless the text already says it cannot be undone, reversed or recovered, or calls the change irreversible (not negated); "permanent" wording still gets the sentence) — told apart by icon and words, not only colour — then the items. Their actions are only Allow once, Deny and Stop script, whatever `editing_tool` says; for risky and destructive, Deny becomes the primary button and gets initial focus once the swap guard ends, and Allow once is soft. Focus is never moved (neither to Deny nor to the heading on a swap) while a pointer press is in progress or while focus is on an element outside the prompt (the user is typing or working elsewhere); then the `role="alert"` callout and the live region carry the request. The live region announces the title, function and the warning. `PythonApprovalBar` actions follow the patch confirmation's `PatchActionRow`: an "Allow" group (Allow once primary; Allow for this run and Allow for this chat, soft, only when `editing_tool`) on the left and a "Refuse" group (Deny soft, Stop script danger with an icon) on the right; groups wrap as units at narrow widths. A request with `needs_person: true` (read tolerantly, any value other than `true` is dropped; the engine sets it for protected configuration edits, human-only automation approvals and every engine change) needs the user every time: no scoped allows, and the hint line is `PROTECTED_HINT` ("Protected file: this change needs your approval every time."). `PythonApprovalBar` shows that hint only for `needs_person` requests without an `engine` part; engine requests get no such hint (their card says what they change). `PythonApprovalBar` above the composer is the only surface with approval buttons; cards never render them. Like the card, the bar never shows the word "Python" in visible text or accessible names; the workspace panel and its rail action are labelled "Python workspace". It shows the oldest pending request with a muted "+N more waiting". Title: "Script wants to run `<tool>`" (a focusable heading), then a muted `In run: “<script title>”` when the outer tool call is found (quoted because the title is model-written; looked up through a memoised per-chat index of tool-call arguments, not a scan per store update); the region is labelled "Script approval for <tool>" (tool name bounded). Nothing being approved hides behind a click: the command of a command tool appears first, in full, in a scrollable block of bounded height with a line count when it overflows (`command` is hidden when it is null or equals the tool name). Then key/value rows with path keys first and project-relative; other multi-line values clamp to 8 lines with "Show N more lines". An uncut edit pair (`old_str`/`replacement` and equivalents) renders through `DiffBlock`; a cut value (engine marker `… (+N chars)`) falls back to the rows. With nothing to show the bar says "No arguments". When the shown request changes in place (or within 1.5 s of the previous one resolving) the buttons are disabled for 500 ms and focus moves to the heading, and the previous request's "no longer pending" notice is cleared. The live region announces only "Approval needed: <tool>". If `args_preview` is not a JSON object it is shown as bounded preformatted text. `rule` appears only when non-empty and not `default`. The scoped allows appear only for editing tools that are neither `needs_person` nor engine requests; their hint (`RUN_PERMIT_HINT`) reads "This run: every file edit this script makes, until it ends. This chat: edits by any script in this chat." Decisions go through `usePythonApprovalDecision`: a Redux-store-scoped registry of chat → request entries holds the in-flight lock/error, subscribes to the store only while entries exist, reads pending requests through the memoized selector (no key parsing per update), and drops entries when their prompt unmounts idle or their request leaves the store, including requests that never reached it. Successful HTTP submission stays pending until SSE removes the request. An unknown-request 400 expires/removes it with a notice; another 4xx with a message (`ChatCommandHttpError.body`, JSON `error` or plain text, not HTML) shows "The engine refused this decision: <message>" as plain text cut to 300 characters, without a retry hint; network failures, 5xx and a 4xx without a message show the generic "Please try again" line. Errors unlock the buttons.

`sendPythonApprovalDecision` sends `python_approval_decision { request_id, decision }`, where decision is `allow_once | allow_for_run | allow_for_chat | deny | stop`. No permit (`allow_for_run`, `allow_for_chat`) is ever offered for an engine request. `allow_for_run` (call approval `allowed_for_run`, shown as "Allowed for run") permits every file edit the current script makes until it ends. `sendPythonRunControl` sends `python_run_control { tool_call_id, action: "stop" }`. The card deduplicates Stop locally and explains an inactive-run 400. Deny rejects one nested call; Stop cancels the whole script but is not an undo or proof that every external effect stopped. Child-agent approvals use the existing parent child-approval UI/flow, not a second root Python approval bar.

#### Workspace rail and panel

The Workspace `python` dock section uses `PythonWorkspacePanel` for the focused chat. The rail's `Python workspace` action toggles/selects it. Its badge counts **artifacts**, not variables, runs or live calls. The rail queries only when that chat has Python live state, a guarded stored Python result, or the Python dock is open; a chat that never used Python does not cause a background query just to draw the rail.

`services/refact/pythonWorkspace.ts` owns `pythonWorkspaceApi` and `useGetPythonWorkspaceQuery({chatId})` for `GET /v1/python/workspace?chat_id=...`, using the shared project-aware URL builder and auth. `dropInvalidOptionals` first removes optional fields with unexpected values (`description`, `code_preview`, `llm_calls`, `displays_total`, `engine_changes`, `variables_total`, `last_run_restored`, `displays_truncated`; each entry of the top-level `displays` that fails its own check is dropped one by one), then `isPythonWorkspace` validates schema, records, nonnegative safe-integer counts, nullable fields and arrays; query acceptance also requires the returned chat ID to match. Variables/artifacts/runs are capped to 500 client-side with truncation flags (the engine currently supplies at most 200 variable previews). Keep `currentData`, not stale prior-chat `data`, when focus changes.

The panel skips the query without a focused chat, refetches on mount/argument change, once per window focus (its own listener; RTK `refetchOnFocus` is not used) and Refresh, keeps the data while refreshing, re-renders relative times once a minute without refetching, and renders "Loading workspace…", "Couldn't load workspace" with "Try again", and "No script runs yet". Its sections are disclosures with counts, open state remembered in `localStorage` (`refact.pythonWorkspace.sections`), in this order (Displays sits between Recent runs and Artifacts, is hidden when the view has no `displays`, newest first, 4 + "Show all N displays"; each is a compact row — kind icon, title, size · relative time — that expands in place to a rail-sized preview with Show in chat and Open full data): Session (status dot + Idle/Running/Background + session label — "Live session", "Restored from disk" per optional `last_run_restored`, "Saved on disk, not loaded", "No saved state" — then "Saved 1h ago · 12.4 KB", reset reason, and the live run's title with "Show in chat"); Recent runs (newest first, 5 + "Show all N runs"; the row itself is "Show in chat" via `revealScriptRun`, `aria-disabled` with a described reason when its card is not rendered, re-checked on hover, focus, click and menu open; title from `runTitle` = description, else `code_preview` through `previewTitle`, else "Run <id>"; one meta line ordered to lose least when cut, e.g. "Completed · 4.2 s · 1h ago · 3 calls · 12 model calls · 2 displays · background", no "0 calls", "background" only for finished runs; a "⋯" `Menu` with Show in chat, Open code, Open output (host can open files and the run ID is a plain name) and Copy run ID); Artifacts (basename first with muted directory, size and age, Open when the host can open files, Copy path; 8 + "Show all N"); Variables (open by default only for ≤ 12; "Filter variables" above 8, kept while it holds text; single-line 32 px button rows — name, type, preview — expanding to selectable wrapped text; 12 + "Show all N loaded variables"; "200 of 230 variables loaded. Filtering searches loaded variables only." when capped); then one "Storage · 20.1 KB of 5 GB" line with the progress bar. A terminal run without a finish timestamp stays terminal ("duration unavailable"); do not infer Running solely from a missing timestamp. On entering the narrow layout the dock closes itself by design (`WorkspaceShell`). The view is metadata, not an editor or an authenticated snapshot-load result.

`app/middleware.ts` invalidates the chat's `PythonWorkspace` tag on run `started` and `finished`, and on `call_finished` at most once per two seconds per chat/store. Stdout and other call progress do not invalidate it. The throttle prunes expired entries; do not replace it with invalidation on every SSE chunk or a component poll loop.

Rules when changing this surface:

- Validate untrusted wire shapes before reducers/rendering, including nested records. Unknown enum values must be tolerated safely: normalize to an explicit unknown state where supported, or reject the unsupported payload without crashing or inventing a known state. Keep guards, normalizers and humanized fallback display aligned; never bypass a guard with a cast.
- Never render code, paths, stdout, previews, errors, commands or variable values as untrusted HTML. Use text/code/highlighting and existing guarded result/diff components; Open actions must retain host capability checks.
- Cap live call/stdout state and workspace lists; preserve snapshot replacement, final-result pruning, stale-approval cleanup and per-chat isolation. Do not duplicate large run artifacts in Redux or render full output files as live progress.
- Update types/guards, subscription parsing, reducers/selectors, command helpers and characterization tests together when the protocol changes. Keep the approval bar the single action surface, tool-job final-result lookup intact, and workspace invalidation bounded.

### Tool Confirmation

`pause_required` event → ToolConfirmation popup → Allow Once / Allow Chat / Stop.

Auto-approve for patch-like tools when `automatic_patch === true`: `patch`, `text_edit`, `create_textdoc`, `update_textdoc`, `replace_textdoc`, `update_textdoc_regex`, `update_textdoc_by_lines`.

## Styling

**Radix Themes** (design tokens) + **CSS Modules** (component-specific).

**Rules**: Use Radix primitives (`Flex`, `Box`, `Text`, `Card`, `Button`). Use design tokens (`var(--space-3)`, `var(--accent-9)`). CSS Modules for custom styles. No inline styles, no magic numbers, no hardcoded colors, no global CSS.

### Global CSS contract (cascade + base resets)

- Global stylesheets (radix themes, `styles/tokens.css`, `styles/base.css`, glass/motion/responsive/scrollbar, theme-config, shared tokens) are imported FIRST in `src/lib/index.ts` and mirrored in `.storybook/preview.tsx`. Component CSS modules win equal-specificity battles against Radix defaults (e.g. `.rt-Box { display: block }` vs a module's `display: flex`) only because of this order; `src/lib/cssOrder.test.ts` locks it. Never make a global sheet reachable only through a component module.
- `styles/base.css` owns the global `box-sizing: border-box` reset (KaTeX subtree exempted), the 13px ambient base, theme-root-scoped `:where()` maps for bare `h1`-`h6`, `code/kbd/samp/pre`, and form controls, and the gray-text adapter that routes Radix `<Text color="gray">` (`--gray-a11`, an unowned color) onto `--rf-color-muted`. New code must not add per-component `box-sizing` declarations; the legacy sweep is complete and only `ui/Tabs` keeps a deliberate belt-and-braces declaration for story isolation.

### Control-height recipe (the Button pattern)

Every interactive control uses **fixed `height` (or a `min-height` floor plus stretch protection) + border-box + zero vertical padding**: `height: var(--rf-control-h-sm|h|h-lg)` for single-line controls; `min-height` alone is forbidden for stretchable controls because grid/flex parents inflate them off the 26/28/30/36 ladder (measured: Select 30→56.6, SegmentedControl 30→33.4). Badge sizes are 16/18/22 total; Chip floor is `var(--rf-control-h-icon-sm)` (28px, also the tap-target floor — undersized visuals get an expanded `::before` hit area). Small square icon actions (`.actionButton` convention) are 28×28 with `var(--rf-radius-chip)`.

### Typography tokens

- Weights come only from `--rf-weight-regular/medium/semibold/bold` (400/500/650/700); zero `font-weight:` literals outside `tokens.css`.
- Line heights pair with sizes via `--rf-line-1..5` (18/20/21/22/28px) to avoid half-pixel line boxes; the app-wide sweep is done, so any rule block that declares a scale font-size must use the paired token. Unitless `--rf-line` remains only on blocks without their own font-size (it inherits as a factor; pinning those to px would change descendant metrics).
- Never use `em` font sizes or Radix `var(--radius-*)` / `var(--space-*)` tokens in module CSS — use `--rf-text-*` and `--rf-radius-*`/`--rf-space-*`.

### Identifier display

Internal identifiers (chat-mode ids, goal statuses, event kinds, engine error categories such as Rust `UserErrorCategory` variants) must never render verbatim in chrome; route NEW identifier display through `humanizeIdentifier` in `src/utils/displayNames.ts` (curated labels + acronym-aware sentence-case fallback). A few older per-feature humanizers still exist and are consolidated opportunistically.

### Responsive doctrine

- App-level horizontal overflow safety belongs in `src/styles/responsive.css` and the app root only; do not add global `*`, `body`, or `html` overflow clamps.
- `.scrollX` is the only sanctioned horizontal-overflow escape hatch for code, diff, table, and kanban islands.
- Prefer shared utilities `.rf-min-w-0`, `.rf-truncate`, `.rf-wrap-anywhere`, and `.rf-container-fluid` before adding one-off responsive CSS.
- Do not relayout feature surfaces while adding responsive infrastructure; feature migrations need their own characterization/parity tests.
- Scrollbar styling (width/color) may be global, but `scrollbar-gutter: stable` must never use universal or broad selectors; it is a per-scroll-container opt-in because `stable` reserves a phantom inline-end gutter on every `overflow: hidden` element, including truncating labels.

## Design System (Refact UI)

Refact UI rules are contributor contracts. Any change that introduces a new design-system rule, primitive, token, or guardrail MUST update this section in the same PR/card.

### Architecture and boundary

- Reusable kit code lives in `src/components/ui/*`; design tokens and shared global utilities live in `src/styles/*`.
- `src/components/ui/**` and `src/styles/**` MUST NOT import from `features`, `services`, or `app`. This is enforced by `npm run lint:boundaries` through the scoped ESLint override in `.eslintrc.cjs`.
- Connected widgets must split into presentational kit pieces in `src/components/ui/*` and feature-connected wrappers in the owning feature folder.
- Core presentational primitives live in `src/components/ui/*`: `Surface` is panel-less by default, `Card` is reserved for restrained containment/selected/overlay use, and `Badge`/`Chip`/`StatusDot` provide the shared label/status language.
- Settings scaffolding primitives live in `src/components/ui/*`: `Field`/`FieldRow`/`FieldStack` own label, helper, error, and controlled-control layout; `FieldText`, `FieldTextarea`, `FieldSelect`, `FieldSwitch`, and `FieldSlider` stay controlled and presentational so parents can implement either blur-save via `onCommit` or submit-save by gathering state. Field text inputs, textareas, selects, and sliders fill their control block by default; `--rf-input-max` is an opt-in cap for compact contexts. `SaveStatus` is the shared idle/saving/saved/error indicator. `SettingsShell` owns only the responsive two-pane section shell and narrow section selector; `SettingItem` owns row/stack setting copy plus control layout and optional save status, with only row-layout controls capped compactly. Do not import features, services, or app code into these primitives.
- Settings content sections use the settings-specific `features/Settings/SettingsSection` primitive, not ad hoc page wrappers. It owns the section header (`title`, one-line `description`, optional right-aligned `actions`, optional `subNav`) and a fluid full-width body (`width="default"` and `width="wide"` both resolve to 100%; readability comes from `SettingItem` row layout and the `68ch` description cap, not a section clamp). `subNav` is a plain full-width slot — content placed there (tabs, segmented controls, buttons) brings its own chrome; the slot itself adds no glass panel. Use `SettingsGroup` inside it for uppercase muted group labels plus `SettingItem` rows. Primary section actions belong in the header `actions` slot; embedded sections must not nest another `SettingsShell`.
- Table/list primitives live in `src/components/ui/*`: `DataTable` owns generic tabular display with narrow stacked-card rendering by default and uses explicit `wide` mode only for intentional `.scrollX` islands; `EditableTable` owns controlled editable cell grids with add/remove, Enter-to-advance down the active column, and per-cell validation display; `VirtualList` is the thin generic `react-virtuoso` wrapper for long lists with optional header/footer. They stay pure and generic, use kit controls, and must not migrate feature table behavior directly.
- `VirtualList density="compact"` removes only item-wrapper padding for consumers such as Git trees that already own their row height. Default density retains the list's existing item padding; crossing a virtualization threshold must not change a consumer's row pitch.
- Tool shell primitives live in `src/components/ui/*`: `ToolCard` owns the pure presentational tool-card shell with title, optional Lucide icon, status tone, actions slot, and controlled/uncontrolled collapse. It stays panel-less/glass by default, uses `rf-expand-grid` for collapse motion, and wide code/diff previews must use explicit `.scrollX` islands instead of nested vertical scrolling.
- ToolCard `density="compact"` is opt-in through the kit and ChatContent bridge: the collapsed shell is one `--rf-space-6` (32px) row with no outer grid gap or header block padding and `--rf-text-2` (13px) type. Default density and open-state ownership are unchanged. In compact ChatContent rows, `summary` truncates before nonshrinking `meta` essential state/outcome text and the chevron; callers supply concise truthful state text. The bridge forwards `actions` as independent siblings of the toggle, never nested buttons.
- ToolCard `.body` is the canonical outer body contract for chat tool padding and indentation. The shell owns outer left offset and vertical rhythm; feature tool bodies should avoid competing padded wrapper shells unless they are true inner panels or scroll islands.
- `SettingItem` rows are FLUID (the settings width doctrine): `.item` declares no `max-width` at all — rows fill the parent section track so every control shares one right edge; readability comes from the copy column's absolute cap (68ch family) and the row-layout control cap (`--rf-setting-item-control-max`), never from a row-level measure. When a cap is genuinely needed anywhere in this primitive it must be an absolute length; a percentage `max-width` such as `100%` or `min(100%, …)` is forbidden. A percentage `max-width` on these nested grid boxes makes each row's intrinsic-size contribution cyclic, so Blink re-runs intrinsic layout per row: a 60-row settings list measured ~25ms of layout **per row** (≈1.7s per layout pass, main thread ~85% blocked, every interaction multi-second) versus ~0.6ms per row with absolute lengths, at pixel-identical geometry from 360px to 1400px. Containment inside the row comes from `min-width: 0` plus the parent grid track (`minmax(0, 1fr)` / `minmax(0, var(--rf-setting-item-control-max))`), not from percentage caps. Prefer absolute caps over percentage `max-width` in any primitive that renders in long repeated lists.

- Redesign the skin, keep the behavior: do not rewrite Redux selectors, RTK Query services, SSE contracts, virtualization, backend contracts, or tool execution logic as part of visual migration. Risky surfaces migrate only after characterization/parity tests pass.

### Tokens are the only visual truth

- Use `var(--rf-*)` tokens for colors, spacing, radii, shadows, typography, sizing, z-index, blur, and motion. Components must not introduce hardcoded colors, spacing, or radii.
- `src/styles/tokens.css` is canonical. It defines primitive tokens, semantic light/dark values, and theme adapters including `[data-host="jetbrains"]`; the Theme root carries both `data-host` and `data-appearance` so these selectors activate deterministically.
- The accent token binds to Radix via `--rf-color-accent: var(--accent-9, ...)`; `color-mix(...)` token overrides must keep static fallback values declared first for older engines.
- Canvas, chart, graph, or third-party theme code must read tokens through `useToken` / `useTokens` instead of duplicating visual constants.
- Legacy aliases such as `--z-*` and `--motion-*` may remain only as adapters to `--rf-*` tokens.

### Surface model

- Panel-less by default: inline content, especially tool cards and transcript content, should not gain boxes, fills, or heavy borders.
- Kit Menu, Select, and Combobox item rows follow the ModelSelector row language: transparent idle rows, no per-row border/box/radius, `--rf-surface-1` hover, and `--rf-color-accent-soft` selected tint with accent-colored marker. Preserve the glass overlay container and keep overlay content as the single scroll owner.
- Neutral-gray glass is the shared inline panel treatment. Use `Surface variant="glass"` when JSX can use the kit primitive, or `.rf-glass-panel` for global/module CSS surfaces that need the same recipe. Both read from `--rf-surface-glass` and `--rf-elev-panel`, with blur driven by `--rf-blur-overlay`.
- Glassy panel backgrounds must stay neutral gray, never periwinkle or blue. `--rf-color-accent-soft` is reserved for selected/active states and must not be used as a panel or card fill.
- Surfaces are reserved for overlays, fields, selected/active state, and true containment. When used, keep them barely visible with tokenized borders/backgrounds.
- JCEF/JetBrains (`[data-host="jetbrains"]`) forces solid overlay/glass surfaces and disables blur for performance. `prefers-reduced-transparency: reduce` also resolves overlay/glass surfaces to solid readable panels.

### Motion

- Prefer CSS-only motion. Chat transcript hot paths must not use JS animation.
- Animate `transform` and `opacity` first; `grid-template-rows` is allowed for expand/collapse.
- Motion must use `--rf-dur-*`, `--rf-ease-*`, and `--rf-stagger`, and must honor both `prefers-reduced-motion` and `prefers-reduced-transparency`.
- Shared utilities live in `src/styles/motion.css`: `.rf-enter`, `.rf-enter-rise`, `.rf-stagger`, `.rf-popover-motion`, `.rf-expand-grid`, `.rf-pressable`, `.rf-status-pulse`, and `.rf-shimmer`.
- Use `useReducedMotion()` only for rare JS-side decisions that cannot be expressed in CSS.

### Icons

- Use Lucide outline icons through the `<Icon>` wrapper.
- Icons inherit `currentColor` and use `strokeWidth={1.5}`. State is communicated by icon color only; do not add icon fills.
- Do not use emoji as icons. Data/content emoji are exempt, such as UserInput `🗜️` detection and TaskDocuments `★` content markers.

### Responsiveness doctrine

- No stray page-level horizontal scroll ever. The Playwright gate checks dashboard and chat at `240`, `360`, `768`, and `1280` px.
- Every shrinkable flex/grid child gets `min-width: 0`; grids use `minmax(0, 1fr)` where columns must shrink.
- Do not put `min-width: <N>px` on layout containers, cards, or tables.
- Do not use `nowrap` without truncation.
- Overlays clamp with viewport-aware sizing such as `width: min(ideal, calc(100vw - 2 * gutter))` and `max-height: min(ideal, calc(100dvh - gutter))`; narrow layouts should become a Sheet.
- `overflow-x` belongs only inside explicit `.scrollX` islands for code, diffs, tables, and kanban-like content.
- Prefer container queries for component layout changes.
- Composer views: expand when the editor receives focus or a pointer, while typing, or while a composer menu is open; collapse when focus and pointer leave and the composer holds no text or attachments (mount-time autofocus does not expand). Collapsed shows the baseline set (model, mode, branch, streaming, quota, context, Send/Stop) plus pinned exceptions (dangerous approval while ON, an active skill, a recording mic); expanded adds permissions/project controls above the editor opposite streaming/quota/context/trajectory, and action icons beside Send/Stop below. Width is governed only by `components/ChatForm/composerLayout.ts`: measured widths and one hardcoded sequence that shrinks labels first (quota, skills, branch, mode, model) and then hides rarest-first with mode last; model and Send/Stop never hide (model ellipsizes only as a last resort). No wrapping, More menu, corner toggle or inline warning text: dangerous approval and context warnings surface as pinned controls. Terminal, queue and goal/tasks rows live inside the composer surface and use `TranscriptItem` styling and animation. Keep one mounted instance of every stateful control and the textarea across layout changes, and pin controls whose menus are open.
- App-level horizontal overflow safety belongs in `src/styles/responsive.css` and the app root only; do not add global `*`, `body`, or `html` overflow clamps.
- Prefer shared utilities `.rf-min-w-0`, `.rf-truncate`, `.rf-wrap-anywhere`, and `.rf-container-fluid` before adding one-off responsive CSS.

### Overlays

- Use five overlay primitives only: Tooltip, Popover, Menu, Dialog, and Sheet.
- Overlay implementations must provide viewport clamping, focus trap/restore, Escape handling, and Portal-into-theme-root behavior.
- Overlay blur uses `--rf-blur-overlay` and must have a reduced-transparency fallback; JetBrains host mode disables blur.
- The stabilized UI kit overlay set is exported from `src/components/ui`: `Dialog`, `Menu`, `Popover`, `Sheet`, and `Tooltip`. They share `open`, `defaultOpen`, `onOpenChange`, anchored `side`/`align`/`sideOffset`/`collisionPadding` where applicable, `modal` where applicable, and content `maxWidth`/`maxHeight` props.
- Dense list popovers (for example, @-command completion) may use `--rf-panel-pad-compact` for content padding; all other overlay content uses `--rf-space-3`.
- Overlay content clamps with `width: min(ideal, calc(100vw - 2 * var(--rf-space-3)))` and `max-height: min(ideal, calc(100dvh - var(--rf-space-5)))`; vertical overflow stays inside the overlay and horizontal overflow must use explicit `.scrollX` islands.
- Portaled transient overlays opened from inside `Dialog` or `Sheet` use modal-layer context and the modal overlay tokens so they remain above their owning modal without lifting unrelated page overlays above every modal.
- `Popover` is responsive by default and renders as a bottom `Sheet` below the narrow viewport threshold; callers may set `responsive={false}` or `forceSheet` for deterministic behavior.
- Responsive popovers restore focus to their original trigger when their Sheet closes. Anchored `Popover.Content forceMount` supports single-instance stateful overflow compositions; callers must hide closed content and manage opening focus. Optional outside-interaction hooks retain Radix dismissal semantics by default. Vite deduplicates Radix dismissal and focus-scope runtimes so nested popovers and sheets share one ordered layer stack.

### Model selector

- `ModelSelector` in `src/components/ui/ModelSelector` is the reusable presentational selector for model picking. It accepts only prop data and callbacks: `models`, `value`, `onSelect`, optional `groups`, `allowUnset`, `disabled`, `onAddNewModel`, and `variant: "popover" | "inline"`.
- `unsetLabel?: string` customizes the empty-state row and trigger label when `allowUnset` is enabled; it defaults to `No model selected`.
- `triggerSize?: "sm" | "md" | "lg"` lets connected compact wrappers keep small triggers while the selector internals stay shared.
- `ModelOption` carries render-ready fields only: `value`, `displayName`, optional `group`, `disabled`, `pricing: { prompt, output }`, `contextWindow`, `badges: Array<"default" | "reasoning" | "light" | "buddy" | "task-agent" | "chat2">`, and `capabilities: ReactNode`.
- The kit selector must stay pure: no caps hooks, Redux, RTK Query, services, provider utilities, or feature imports. Connected feature code owns enrichment, grouping, pricing formatting, capability icons, and persistence.
- Use `variant="popover"` for compact pickers backed by the kit `Popover` responsive Sheet behavior, and `variant="inline"` for settings surfaces that render the searchable list directly.
- Model rows are panel-less and glass-friendly: no per-row bordered/card boxes, transparent idle rows, subtle `--rf-surface-1` hover tint, and selected state as `--rf-color-accent-soft` background + accent name + check icon only.
- Model names must truncate with ellipsis before badges wrap; badge chips stay in a compact grouped flex container so labels such as `Task Agent` fit on the name line in normal widths and wrap as a group only when unavoidable.
- `ModelSelector` popovers and settings compositions must have exactly one vertical scroll owner. Use `Popover.Content scrollable={false}` when an inner selector/list owns scrolling; keep search and footer actions pinned outside the `.scrollArea`.
- `Popover.Content` supports `scrollable={false}` for flex-column, overflow-hidden overlay content. Use it for composed popovers/sheets that need sticky top/bottom regions with a single inner scroller.

### Sizing contract

| Item        | Values                                                                                                             |
| ----------- | ------------------------------------------------------------------------------------------------------------------ |
| Controls    | default `30px`, small `26px`, large `36px`; icon-only small `28px` via `--rf-control-h-icon-sm` (tap-target floor) |
| Switches    | track `36×20px`, thumb `16px`, inset `2px`, visual travel `18px`                                                   |
| Icons       | default `15px`, small `13px`, large `18px`, tap target at least `28px`                                             |
| Spacing     | scale tokens documented below                                                                                      |
| Radii       | chip `6px`, control `8px`, card/popover `10px`, pill `999px`                                                       |
| Lines/rings | hairline `1px`, focus ring `2px`                                                                                   |
| Type        | `12 / 13 / 14 / 15 / 19px`                                                                                         |
| Layout      | nav `220px` with `180px` min, content max `640px`                                                                  |
| Overlays    | popover `210–360px`, dialog `340px`, tooltip max `280px`                                                           |

Canonical spacing rhythm:

- Full spacing scale: `--rf-space-2xs: 2px`, `--rf-space-1: 4px`, `--rf-space-xs: 6px`, `--rf-space-2: 8px`, `--rf-space-3: 12px`, `--rf-space-4: 16px`, `--rf-space-5: 22px`, `--rf-space-6: 32px`.

- Semantic spacing tokens resolve to the `--rf-space-*` scale and are the preferred contract tokens for layout surfaces.
- Panel/card inner padding uses `--rf-panel-pad`; compact panels use `--rf-panel-pad-compact`; card-specific APIs may use the `--rf-card-pad` alias.
- Inter-card and section gaps use `--rf-section-gap`.
- Rows use `--rf-row-pad` padding and `--rf-row-gap` internal gap.
- Tight label/value stacks use `--rf-space-2xs`.
- Chip and badge padding uses `var(--rf-space-2xs) var(--rf-space-xs)`; pill chips use `var(--rf-space-1) var(--rf-space-2)`.
- Header title-to-subtitle gaps use `--rf-space-2xs` or `--rf-space-1`; header-to-body gaps use `--rf-space-2`.
- Canvas and sprite pixel coordinates in BuddyWorld/BuddyCanvas, plus BuddyDemo showcase layouts, are exempt from spacing-token normalization.

### Guardrails and verification

These scripts exist in `refact-agent/gui/package.json` and are the current merged guardrails:

| Command                   | Purpose                                                                                                                 |
| ------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| `npm run types`           | TypeScript `tsc --noEmit`                                                                                               |
| `npm run lint`            | Strict ESLint over TS/TSX with zero warnings                                                                            |
| `npm run lint:css`        | Stylelint scoped to `src/components/ui/**/*.css` and `src/styles/*.css` for now; global CSS expansion belongs to DS-704 |
| `npm run lint:boundaries` | ESLint boundary check for `src/components/ui/**/*.{ts,tsx}` and `src/styles/**/*.{ts,tsx}`                              |
| `npm run test`            | Vitest unit suite excluding integration tests                                                                           |
| `npm run build`           | TypeScript and Vite chat/events builds                                                                                  |
| `npm run build-storybook` | Storybook static build                                                                                                  |
| `npm run test:e2e`        | Playwright no-horizontal-scroll gate via `tests/e2e/no-horizontal-scroll.spec.ts`                                       |

## IDE Integration (postMessage)

**Host modes**: `web` | `vscode` | `jetbrains` | `ide`

**IDE → GUI**: `updateConfig`, `setFileInfo`, `setSelectedSnippet`, `newChatAction`, `ideToolCallResponse`
**GUI → IDE**: `ideOpenFile`, `ideDiffPasteBack`, `ideToolCall`, `ideNewFile`, `ideAnimateFileStart/Stop`

## Multi-Tab & Background Threads

Threads continue processing even without open tabs. `closeThread` preserves busy runtimes (streaming, waiting, paused). Background thread needs confirmation → auto-switches user to that tab.

**Two SSE systems**: Chat subscription (per-thread, real-time state) + Trajectories subscription (global, metadata sync only). Sidebar v2 also carries section snapshots/updates plus notification envelopes for cross-thread prompts.

### Workspace dock / main / drawer contract

The web Workspace is one shell with three authorities. Keep the grammar stable:

- **Dock lists things.** It is a 240–400px project-persisted left rail with capability-filtered `Files | Git | Tasks` sections. Files owns the tree, Git owns changes/staging/commit, and Tasks projects in-flight cards. Below 768px the same dock renders through the kit `Sheet`; opening a file, Git change, or task closes the narrow Sheet instead of changing lifecycle.
- **Main shows things.** The top tab bar owns chats plus `file:{path}` read-only viewers, the singleton `git:main` surface, and existing `task:{id}` workspaces. File tabs deduplicate by exact path. Tasks remain per-task surfaces; do not invent `tasks:main`. Files and Terminal are not live center-tab kinds.
- **Drawer executes things.** Terminal is the bottom, height-persisted drawer under Main. It stays mounted when collapsed so PTY sessions survive; the open height is clamped to 50dvh and the collapsed strip keeps the session count and `StatusDot` indicators visible.

Workspace layout state is project-scoped as `dock: { open, width, section }` and `drawer: { open, height }`. Persistence accepts `files:main` and `terminal:main` only as legacy migration inputs: Files is silently dropped, Terminal opens the drawer, and neither may be emitted as a new tab. `git:main` remains the only panel-like live Main surface. Remove obsolete registries, launchers, splitters, CSS, and persistence keys when moving a surface; compatibility parsing is not permission to keep a dead render path.

Web-only keyboard shortcuts use stable section numbers: `Ctrl/Cmd+B` toggles the dock, `Ctrl/Cmd+J` toggles the terminal drawer, and `Ctrl/Cmd+1/2/3` opens and selects Files/Git/Tasks respectively when that section is available. The Workspace handler must not register for IDE hosts and must ignore composing/repeated/modified events plus targets inside inputs, textareas, selects, contenteditable regions, and `.xterm`.

The shell renders dock chrome only for web hosts with Files or Git capability, and terminal chrome only with Terminal capability. VS Code, JetBrains, and generic IDE hosts retain zero new Workspace chrome even if stale persisted layout exists. Changes to this shell must keep the IDE characterization suite, capability matrix, project-scoped legacy fixture, Workspace unit suite, boundary lint, and the Playwright no-horizontal-scroll route with dock + drawer open green at 240/360/768/1280px. Use kit `Badge`/`StatusDot` for Git, Tasks, and terminal indicators; preserve `min-width: 0`, Sheet clamping, and explicit `.scrollX` islands.

### Scheduler feature

The scheduler UI is opened by the `scheduler` page (`openScheduler({ taskId? })`) and by EventLog clicks on `cron_fire` events. Keep the GUI model aligned with the consolidated schedule and pause/resume operations:

- `schedule(action="create")`: 5-field cron, prompt, description, recurring flag, durable flag.
- `schedule(action="list")`: scope filter (`session`, `durable`, `all`), list rows with human schedule, next fire, fire count, and scope chips.
- `schedule(action="delete")`: remove by id and update the visible list.
- `pause` / `resume` with a schedule target control future enabled state; existing scheduler feature HTTP routes share the same operations.

Scheduler state should live in a feature slice or RTK Query service, not in ChatContent. Cron fire visibility comes from hidden `event(cron_fire)` messages and should route through EventLog first. Durable jobs are project-scoped; session jobs are engine-memory-scoped, so UI copy must make that distinction clear.

### Notifications + toast pattern

Notification events must stay semantically separate from task events. Sidebar SSE envelopes use:

```typescript
{ type: "notification", notification: NotificationEvent }
```

Current notification payloads are `task_done` and `ask_questions`; the parser validates them in `services/refact/sidebarSubscription.ts` and `useSidebarSubscription` routes them to IDE/window notifications instead of task reducers. Chat hidden events such as `event(process_completed)` appear through `message_added` and should be surfaced by EventLog or feature-specific toast middleware.

If adding a new toast source:

1. Define the envelope or hidden-event payload type in `services/refact/types.ts` or `sidebarSubscription.ts`.
2. Validate it at the parser boundary.
3. Dispatch a typed action from middleware/hook code.
4. Render with Radix + CSS modules, `role="status"`/`aria-live="polite"`, stable IDs for dismiss/dedupe, and click handlers that navigate or scroll to the relevant chat/process/card.
5. Add tests for parsing, dedupe, dismiss, and click behavior.

Dedicated `ProcessCompleted` chat envelopes are not active in this tree; process completion is currently `message_added` with `event(process_completed)`. If the dedicated envelope returns, GUI AGENTS and `EventEnvelope`/reducer tests must document it before use.

### State Machine (per thread)

```
IDLE → [submit] → WAITING → [first chunk] → STREAMING → [finish] → IDLE
                                           → [pause_required] → PAUSED → [confirm] → IDLE
                                           → [error/abort] → STOPPED
```

### Send Invariants

Chat can proceed when ALL true: `snapshot_received && !streaming && !waiting_for_response && !prevent_send && !error && !confirmation.pause`

## Special Features

- **Checkpoints**: Workspace rollback via git commits. Preview → Restore. Per-message reset button.
- **Hidden Roles**: `event` messages feed EventLog except `plan_delta`, `goal_delta`, and `goal_pursuit`; legacy `plan` plus `plan_delta` messages feed the history-only PlanBanner fallback; `goal` plus goal delta/pursuit projection feeds TaskProgressWidget through `thread.goal`. All hidden roles stay out of `selectVisibleMessages`.
- **Thinking Blocks**: `thinking_blocks: [{thinking, signature}]` on assistant messages. Collapsible UI. Signatures are opaque — never mutate.
- **Reasoning Content**: Separate `reasoning_content` field. Collapsible.
- **Knowledge/Memory**: `remember_how_to_use_tools` → vecdb → `context_file` messages. Knowledge graph view.
- **Customization**: Agent modes, subagent forms, tool parameter editor.
- **Tour/Onboarding**: Welcome screen, guided tour bubbles.
- **FIM Debug**: Fill-in-Middle debug panel with search context and symbol list.
- **CodeGraph status**: Toolbar mini indicators surface CodeGraph, VecDB, and legacy AST compatibility state from `/v1/rag-status`; CodeGraph settings are host-provided feature flags and status lives in the Knowledge slice.
- **Performance**: The Toolbar menu opens the `performance` page, implemented in `features/Performance`. `PerformancePage` fetches `GET /v1/performance/telemetry` with visible-tab five-second polling and uses its POST enable/disable and reset endpoints for process-local aggregate collection. Its trajectory settings panel fetches `GET /v1/trajectory-settings` and posts the complete server-loaded configuration to `POST /v1/trajectory-settings`; preserve every returned key, including fields without a descriptor, because the endpoint treats omitted fields as defaults and a metadata-rebuilt payload can silently lose settings.
- **Privacy & Access**: Settings owns destination-first access editing (zone cards, blocked patterns, per-destination zone and MCP toggles, read-only audit matrix) and observation status; chat owns the model shield, destination inspector, blocked-send card, and local-only withheld-output reveal; FileTree shows each resolved zone and persists exact-path moves through the complete policy.
- **Docker**: Container list, start/stop/kill/remove, env vars, smart links.
- **Compression Hints**: 🗜️ icon when context approaches limit. `compression_strength: "absent" | "weak" | "strong"`.
- **Queued Messages**: Send while streaming. Priority queue bypasses tool wait.
- **Multi-Modal**: Images in user messages and tool results. `DialogImage` lightbox.
- **Usage Tracking**: `UsageCounter` (circular progress), `StreamingTokenCounter` (live), `TokensMapContent` (breakdown).
- **Provider OAuth**: OAuth2 flow for provider authentication.
- **MCP Logs**: MCP integration logging in IntegrationsView.

## Development Patterns

### Adding Redux Slice

1. Create `features/MyFeature/myFeatureSlice.ts` with `createSlice`
2. Register in `combineSlices` in `store.ts`
3. Use `useAppSelector`/`useAppDispatch` in components

### Adding RTK Query API

1. Create `services/refact/myApi.ts` with `createApi`
2. Register in `combineSlices` + add `.middleware` in store
3. Use auto-generated hooks

### True tab appearance

True tabsets use `components/ui/Tabs`; legacy LongTail Tabs forwards to it.
`Tabs/TabAppearance.module.css` is the shared compact editor appearance contract:
compose `strip` for one `--rf-control-h` band, `item` for regular text-2 tab
geometry with upper-rounded transparent selection, `active` for selection on custom tab wrappers, and
`action` / `close` for overflow and close controls. Custom workspace strips own
layout, drag, closing and measurement, not a separate glass/pill skin. Keep hidden
measurement widths identical to rendered tab padding, gaps, icons and separators.
Canonical Tabs retains its measured indicator and Radix behavior. Local classes
may adapt layout/width, not tab typography, roundness, active or hover appearance.
This contract does not apply to Settings navigation or segmented filters/toggles.

### Adding Component

`Component.tsx` + `Component.module.css` + `index.ts`. Use Radix primitives + CSS Modules + design tokens.

### File Naming

Components: `PascalCase.tsx` · Hooks: `useCamelCase.ts` · Utils: `camelCase.ts` · CSS: `PascalCase.module.css`

## Testing

Vitest + React Testing Library + MSW + happy-dom. Custom render in `utils/test-utils.tsx` wraps Provider/Theme/Tour/AbortController. Fixtures in `__fixtures__/`. MSW handlers mock LSP endpoints.

## Agent Checklist

**When modifying chat flow**: Check state transitions, SSE event handling in reducer, command sending via `chatCommands.ts`, goal command dispatchers/projection updates, sequence validation, tool confirmation logic, type guards.

**When adding SSE events**: Type in `chatSubscription.ts` → handler in reducer's `applyChatEvent` → update `EventEnvelope` union → add tests.

**When touching Redux**: Use selectors. Register new slices/APIs in store. Add middleware for new APIs. Test state transitions.

**When modifying UI**: Radix primitives. CSS Modules. Design tokens. Test dark mode.

**Red flags**: Direct `state.chat.thread` (old pattern, use `threads[id]`), hardcoded colors/spacing, `any` types, missing sequence validation, missing `snapshot_received` checks, missing `useEffect` cleanup.
