# Refact Agent GUI

React chat UI for AI coding assistant. Builds to `dist/chat/` (browser UMD) and `dist/events/` (Node.js types). Consumed by IDEs (VSCode, JetBrains) and standalone web.

## Tech Stack

React 18.2 · TypeScript 5.8 (strict) · Vite 5.0 · Redux Toolkit 2.2 (RTK Query) · Radix UI/Themes · CSS Modules · urql 4.2 (GraphQL, SmallCloud only) · Vitest 3.1 · MSW 2.3

## Quick Start

```bash
npm run test:all        # CI
npm run lint            # eslint strict-type-checked
npm run types           # tsc --noEmit
DEBUG=* npm run dev     # debug logging
```

## Architecture

```
React App → Redux (RTK Query) → LSP Server (:8001)   [chat, tools, caps, models]
                               → SmallCloud (GraphQL)  [auth, teams, surveys]
                               → IDE (postMessage)     [file ops, theme, context]
```

### Directory Layout

```
src/
├── app/              # Store (combineSlices), middleware (50+ listeners), storage
├── features/         # Redux slices + feature UIs
│   ├── Chat/Thread/  # Multi-thread: reducer, selectors (~40+), actions, types
│   ├── Checkpoints/  # Workspace rollback
│   ├── CoinBalance/  # Token/credit balance
│   ├── Config/       # Global settings + FeatureMenu
│   ├── Connection/   # SSE connection status
│   ├── Customization/# Agent modes, subagent forms, tool parameter editor
│   ├── FIM/          # Fill-in-Middle debug
│   ├── History/      # Chat history
│   ├── Integrations/ # Integration config
│   ├── Knowledge/    # Memory system + knowledge graph view
│   ├── Login/        # Login page
│   ├── Pages/        # Navigation stack
│   ├── PatchesAndDiffsTracker/
│   ├── Providers/    # LLM provider config + OAuth
│   ├── Statistics/   # Usage charts
│   ├── Tasks/        # Task management
│   ├── Teams/        # Team/group management
│   ├── ThreadHistory/# Thread history view
│   └── UserSurvey/
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
│   └── smallcloud/   # Cloud auth (GraphQL)
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

| Event                           | Purpose                           |
| ------------------------------- | --------------------------------- |
| `snapshot`                      | Full state sync (resets seq to 0) |
| `stream_started`                | AI response beginning             |
| `stream_delta`                  | Incremental content (DeltaOp[])   |
| `stream_finished`               | Complete with usage stats         |
| `message_added/updated/removed` | Message CRUD                      |
| `messages_truncated`            | Messages trimmed                  |
| `thread_updated`                | Thread metadata changed           |
| `runtime_updated`               | Runtime flags changed             |
| `pause_required/cleared`        | Tool confirmation                 |
| `ide_tool_required`             | IDE tool execution needed         |
| `subchat_update`                | Nested chat update                |
| `queue_updated`                 | Command queue changed             |
| `ack`                           | Command acknowledgment            |

### Delta Operations

`append_content` · `append_reasoning` · `set_tool_calls` · `set_thinking_blocks` · `add_citation` · `add_server_content_block` · `set_usage` · `merge_extra`

### Command Types (POST /v1/chats/{chatId}/commands)

`user_message` · `abort` · `regenerate` · `update_message` · `remove_message` · `tool_decision` · `tool_decisions` · `ide_tool_result` · `set_params` · `retry_from_index` · `branch_from_chat`

### Sequence Validation

Every event has a `seq` number. `snapshot` resets to 0, each subsequent increments by 1. Gap detected → immediate reconnect for fresh snapshot.

## State Management

**Store**: `src/app/store.ts` — `combineSlices` with 12+ slices + 20+ RTK Query APIs

### Key State (per-thread)

```typescript
state.chat.threads[id]: ChatThreadRuntime = {
  thread: ChatThread,         // id, messages, model, title, tool_use, boost_reasoning, reasoning_effort, temperature, mode, is_task_chat, task_meta
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
| telemetryApi                    | `/v1/telemetry/chat`                                                   |
| linksApi                        | `/v1/links`                                                            |
| trajectoriesApi, trajectoryApi  | `/v1/trajectories/*`                                                   |
| tasksApi                        | Tasks CRUD                                                             |
| chatModesApi, customizationApi  | Agent modes/customization                                              |
| knowledgeApi, knowledgeGraphApi | Knowledge/memory                                                       |
| smallCloudApi                   | `https://www.smallcloud.ai` (GraphQL)                                  |

Chat uses **Commands API** + **SSE subscription**, not RTK Query.

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
| `useActiveTeamsGroup`            | Teams group management                                                                   |

## Components

### ChatContent (src/components/ChatContent/ChatContent.tsx)

Dispatches messages to specialized renderers. Iterative processing (not recursive). Groups assistant messages with related diffs + tools.

| Role           | Component                  | Notes                                                                |
| -------------- | -------------------------- | -------------------------------------------------------------------- |
| `user`         | UserInput                  | Editable, checkpoints badge, images, compression hint 🗜️             |
| `assistant`    | AssistantInput             | ReasoningContent → Markdown → ToolsContent → DiffContent → Citations |
| `tool`         | (inline in AssistantInput) | Skipped in top-level render                                          |
| `diff`         | DiffContent                | Grouped by tool_call_id, apply/reject UI                             |
| `context_file` | ContextFiles               | Memory/knowledge attachments 🗃️                                      |

### ToolsContent (src/components/ChatContent/ToolsContent.tsx)

Largest component (~1180 lines). Handles 10+ tool types including nested subchats (max 5 deep), knowledge results, file browser, multi-modal results. OpenAI-specific tool components: AudioTool, ComputerCallTool, CodeInterpreterCallTool, FileSearchCallTool.

**Tool status**: ⏳ thinking · ✅ success · ❌ error · ☁️ server (`srvtoolu_*` prefix)

### Tool Confirmation

`pause_required` event → ToolConfirmation popup → Allow Once / Allow Chat / Stop.

Auto-approve for patch-like tools when `automatic_patch === true`: `patch`, `text_edit`, `create_textdoc`, `update_textdoc`, `replace_textdoc`, `update_textdoc_regex`, `update_textdoc_by_lines`.

## Styling

**Radix Themes** (design tokens) + **CSS Modules** (component-specific).

**Rules**: Use Radix primitives (`Flex`, `Box`, `Text`, `Card`, `Button`). Use design tokens (`var(--space-3)`, `var(--accent-9)`). CSS Modules for custom styles. No inline styles, no magic numbers, no hardcoded colors, no global CSS.

## IDE Integration (postMessage)

**Host modes**: `web` | `vscode` | `jetbrains` | `ide`

**IDE → GUI**: `updateConfig`, `setFileInfo`, `setSelectedSnippet`, `newChatAction`, `ideToolCallResponse`
**GUI → IDE**: `ideOpenFile`, `ideDiffPasteBack`, `ideToolCall`, `ideNewFile`, `ideAnimateFileStart/Stop`

## Multi-Tab & Background Threads

Threads continue processing even without open tabs. `closeThread` preserves busy runtimes (streaming, waiting, paused). Background thread needs confirmation → auto-switches user to that tab.

**Two SSE systems**: Chat subscription (per-thread, real-time state) + Trajectories subscription (global, metadata sync only).

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
- **Thinking Blocks**: `thinking_blocks: [{thinking, signature}]` on assistant messages. Collapsible UI. Signatures are opaque — never mutate.
- **Reasoning Content**: Separate `reasoning_content` field. Collapsible.
- **Knowledge/Memory**: `remember_how_to_use_tools` → vecdb → `context_file` messages. Knowledge graph view.
- **Customization**: Agent modes, subagent forms, tool parameter editor.
- **Tour/Onboarding**: Welcome screen, guided tour bubbles.
- **FIM Debug**: Fill-in-Middle debug panel with search context and symbol list.
- **CoinBalance**: Token/credit tracking with metering fields on messages.
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

### Adding Component

`Component.tsx` + `Component.module.css` + `index.ts`. Use Radix primitives + CSS Modules + design tokens.

### File Naming

Components: `PascalCase.tsx` · Hooks: `useCamelCase.ts` · Utils: `camelCase.ts` · CSS: `PascalCase.module.css`

## Testing

Vitest + React Testing Library + MSW + happy-dom. Custom render in `utils/test-utils.tsx` wraps Provider/Theme/Tour/AbortController. Fixtures in `__fixtures__/`. MSW handlers mock LSP endpoints.

## Agent Checklist

**When modifying chat flow**: Check state transitions, SSE event handling in reducer, command sending via `chatCommands.ts`, sequence validation, tool confirmation logic, type guards.

**When adding SSE events**: Type in `chatSubscription.ts` → handler in reducer's `applyChatEvent` → update `EventEnvelope` union → add tests.

**When touching Redux**: Use selectors. Register new slices/APIs in store. Add middleware for new APIs. Test state transitions.

**When modifying UI**: Radix primitives. CSS Modules. Design tokens. Test dark mode.

**Red flags**: Direct `state.chat.thread` (old pattern, use `threads[id]`), hardcoded colors/spacing, `any` types, missing sequence validation, missing `snapshot_received` checks, missing `useEffect` cleanup.
