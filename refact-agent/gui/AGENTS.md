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
React App → Redux (RTK Query) → LSP Server (:8001)   [chat, tools, caps, models]
                               → IDE (postMessage)     [file ops, theme, context]
```

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
│   ├── ThreadHistory/# Thread history view
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

| Event                           | Purpose                                                |
| ------------------------------- | ------------------------------------------------------ |
| `snapshot`                      | Full state sync (resets seq to 0)                      |
| `stream_started`                | AI response beginning                                  |
| `stream_delta`                  | Incremental content (DeltaOp[])                        |
| `stream_finished`               | Complete with usage stats                              |
| `message_added/updated/removed` | Message CRUD, including hidden `event`/`plan` messages |
| `messages_truncated`            | Messages trimmed                                       |
| `thread_updated`                | Thread metadata changed                                |
| `runtime_updated`               | Runtime flags changed                                  |
| `pause_required/cleared`        | Tool confirmation                                      |
| `ide_tool_required`             | IDE tool execution needed                              |
| `subchat_update`                | Nested chat update                                     |
| `queue_updated`                 | Command queue changed                                  |
| `ack`                           | Command acknowledgment                                 |

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

Hidden-role selector convention:

- `selectVisibleMessages(state, threadId)` excludes `event` and `plan`; use this for normal transcript rendering.
- `selectEventLog(state, threadId)` returns normalized `EventMessage[]` for EventLog surfaces and excludes `plan_delta` events.
- `selectCurrentPlan(state, threadId)` returns the latest base `PlanMessage` by version/index for PlanBanner.
- `selectPlanDeltaEvents(state, threadId)` returns hidden `event(plan_delta)` messages in index order.
- `selectSynthesizedPlanText(state, threadId)` returns base plan text plus append-only plan-delta notes using the synthesis separator.
- `selectPlanHistory(state, threadId)` returns the current base plan followed by plan-delta events for history UI.

If a new component needs hidden-role data, add or reuse a selector first instead of filtering `thread.messages` inside the component.

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

## Components

### ChatContent (src/components/ChatContent/ChatContent.tsx)

Dispatches messages to specialized renderers. Iterative processing (not recursive). Groups assistant messages with related diffs + tools.

Compression visibility rules:

- `compression_report` messages render visibly as `SummarizationMessage` cards via `syntheticCompressionReportMessage()` and should not be hidden as internal roles.
- Assistant messages with `extra.compression.kind === "llm_segment_summary"` render visibly as compression/summarization cards via `syntheticSummarizationMessage()` instead of ordinary assistant transcript text; the card now distinguishes deterministic compaction, LLM summaries, merged-history summaries, and reactive compaction labels.
- Visible compression failure events are narrowly defined as `event` messages whose metadata is `subkind: "system_notice"`, `source: "chat.summarizer"`, and whose content starts with `Context compression failed:`; these render as visible error display items. Other hidden events, including `plan_delta`, stay out of normal transcript display.
- The footer compression indicator is driven only by the thread runtime `is_compressing` selector. It appears while the backend reports an active compression attempt and disappears on terminal `RuntimeUpdated`/snapshot state when `is_compressing` becomes false or is cleared.
- Restore paths and SSE-error/reconnect cleanup must clear `is_compressing`, `compression_phase`, and `compression_reason` so stale compression progress does not survive thread restoration or subscription failures.

| Role           | Component                  | Notes                                                                |
| -------------- | -------------------------- | -------------------------------------------------------------------- |
| `user`         | UserInput                  | Editable, checkpoints badge, images, compression hint 🗜️             |
| `assistant`    | AssistantInput             | ReasoningContent → Markdown → ToolsContent → DiffContent → Citations |
| `tool`         | (inline in AssistantInput) | Skipped in top-level render                                          |
| `diff`         | DiffContent                | Grouped by tool_call_id, apply/reject UI                             |
| `context_file` | ContextFiles               | Memory/knowledge attachments 🗃️                                      |
| `event`        | EventLog                   | Hidden from normal transcript; grouped under nearby assistant turns  |
| `plan`         | PlanBanner                 | Hidden from normal transcript; latest version pinned above chat      |

### EventLog component pattern (src/components/ChatContent/EventLog/)

EventLog renders hidden `event` messages without polluting the main transcript.

- Feed it selector-normalized `EventMessage[]`; do not pass raw backend messages with only `extra.event`.
- Keep it collapsed by default and persist collapse/filter state per thread in localStorage.
- Show subkind icon/chip, source, one-line `content`, and expandable JSON payload.
- Use `EventLogEntry` for row-level behavior and `eventSubkind.ts` for the single source of icon mapping.
- Click behavior stays subkind-specific and explicit: `process_completed` scrolls to matching `[data-exec-process-id]`; `cron_fire` opens the scheduler page via `openScheduler`.
- Tests belong next to the component and should cover collapsed/expanded state, filters, localStorage persistence, and any subkind click behavior.

### PlanBanner component pattern (src/components/ChatContent/PlanBanner/)

PlanBanner renders synthesized plan text from the latest hidden base `plan` role plus hidden `event(plan_delta)` notes as sticky context above the virtualized transcript.

- Read plan data with `selectCurrentPlan`, `selectSynthesizedPlanText`, and `selectPlanHistory`; do not scan messages directly.
- Header format: `📋 Plan — {mode} · v{version} · {humanizedAge}`.
- Body uses existing Markdown rendering, bounded scrolling, and a persisted collapse toggle. Keep v1 expanded by default unless the user toggles.
- Manual plan editing is not exposed in the banner; plan changes arrive as append-only `plan_delta` events.
- History modal lists the current base plan followed by each delta note in index order.
- Keep sticky styles in `PlanBanner.module.css`; avoid inline styles.

### ToolsContent (src/components/ChatContent/ToolsContent.tsx)

Largest component (~1180 lines). Handles 10+ tool types including nested subchats (max 5 deep), knowledge results, file browser, multi-modal results. OpenAI-specific tool components: AudioTool, ComputerCallTool, CodeInterpreterCallTool, FileSearchCallTool.

**Tool status**: ⏳ thinking · ✅ success · ❌ error · ☁️ server (`srvtoolu_*` prefix)

### Tool Confirmation

`pause_required` event → ToolConfirmation popup → Allow Once / Allow Chat / Stop.

Auto-approve for patch-like tools when `automatic_patch === true`: `patch`, `text_edit`, `create_textdoc`, `update_textdoc`, `replace_textdoc`, `update_textdoc_regex`, `update_textdoc_by_lines`.

## Styling

**Radix Themes** (design tokens) + **CSS Modules** (component-specific).

**Rules**: Use Radix primitives (`Flex`, `Box`, `Text`, `Card`, `Button`). Use design tokens (`var(--space-3)`, `var(--accent-9)`). CSS Modules for custom styles. No inline styles, no magic numbers, no hardcoded colors, no global CSS.

### Responsive doctrine

- App-level horizontal overflow safety belongs in `src/styles/responsive.css` and the app root only; do not add global `*`, `body`, or `html` overflow clamps.
- `.scrollX` is the only sanctioned horizontal-overflow escape hatch for code, diff, table, and kanban islands.
- Prefer shared utilities `.rf-min-w-0`, `.rf-truncate`, `.rf-wrap-anywhere`, and `.rf-container-fluid` before adding one-off responsive CSS.
- Do not relayout feature surfaces while adding responsive infrastructure; feature migrations need their own characterization/parity tests.

## Design System (Refact UI)

Refact UI rules are contributor contracts. Any change that introduces a new design-system rule, primitive, token, or guardrail MUST update this section in the same PR/card.

### Architecture and boundary

- Reusable kit code lives in `src/components/ui/*`; design tokens and shared global utilities live in `src/styles/*`.
- `src/components/ui/**` and `src/styles/**` MUST NOT import from `features`, `services`, or `app`. This is enforced by `npm run lint:boundaries` through the scoped ESLint override in `.eslintrc.cjs`.
- Connected widgets must split into presentational kit pieces in `src/components/ui/*` and feature-connected wrappers in the owning feature folder.
- Core presentational primitives live in `src/components/ui/*`: `Surface` is panel-less by default, `Card` is reserved for restrained containment/selected/overlay use, and `Badge`/`Chip`/`StatusDot` provide the shared label/status language.
- Settings scaffolding primitives live in `src/components/ui/*`: `Field`/`FieldRow`/`FieldStack` own label, helper, error, and controlled-control layout; `FieldText`, `FieldTextarea`, `FieldSelect`, `FieldSwitch`, and `FieldSlider` stay controlled and presentational so parents can implement either blur-save via `onCommit` or submit-save by gathering state. Field text inputs, textareas, selects, and sliders fill their control block by default; `--rf-input-max` is an opt-in cap for compact contexts. `SaveStatus` is the shared idle/saving/saved/error indicator. `SettingsShell` owns only the responsive two-pane section shell and narrow section selector; `SettingItem` owns row/stack setting copy plus control layout and optional save status, with only row-layout controls capped compactly. Do not import features, services, or app code into these primitives.
- Settings content sections use the settings-specific `features/Settings/SettingsSection` primitive, not ad hoc page wrappers. It owns the section header (`title`, one-line `description`, optional right-aligned `actions`, optional `subNav`) and a max-width readable body column (`default` around 720px, `wide` around 1040px). Use `SettingsGroup` inside it for uppercase muted group labels plus `SettingItem` rows. Primary section actions belong in the header `actions` slot; embedded sections must not nest another `SettingsShell`.
- Table/list primitives live in `src/components/ui/*`: `DataTable` owns generic tabular display with narrow stacked-card rendering by default and uses explicit `wide` mode only for intentional `.scrollX` islands; `EditableTable` owns controlled editable cell grids with add/remove, Enter-to-advance down the active column, and per-cell validation display; `VirtualList` is the thin generic `react-virtuoso` wrapper for long lists with optional header/footer. They stay pure and generic, use kit controls, and must not migrate feature table behavior directly.
- Tool shell primitives live in `src/components/ui/*`: `ToolCard` owns the pure presentational tool-card shell with title, optional Lucide icon, status tone, actions slot, and controlled/uncontrolled collapse. It stays panel-less/glass by default, uses `rf-expand-grid` for collapse motion, and wide code/diff previews must use explicit `.scrollX` islands instead of nested vertical scrolling.
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
- App-level horizontal overflow safety belongs in `src/styles/responsive.css` and the app root only; do not add global `*`, `body`, or `html` overflow clamps.
- Prefer shared utilities `.rf-min-w-0`, `.rf-truncate`, `.rf-wrap-anywhere`, and `.rf-container-fluid` before adding one-off responsive CSS.

### Overlays

- Use five overlay primitives only: Tooltip, Popover, Menu, Dialog, and Sheet.
- Overlay implementations must provide viewport clamping, focus trap/restore, Escape handling, and Portal-into-theme-root behavior.
- Overlay blur uses `--rf-blur-overlay` and must have a reduced-transparency fallback; JetBrains host mode disables blur.
- The stabilized UI kit overlay set is exported from `src/components/ui`: `Dialog`, `Menu`, `Popover`, `Sheet`, and `Tooltip`. They share `open`, `defaultOpen`, `onOpenChange`, anchored `side`/`align`/`sideOffset`/`collisionPadding` where applicable, `modal` where applicable, and content `maxWidth`/`maxHeight` props.
- Overlay content clamps with `width: min(ideal, calc(100vw - 2 * var(--rf-space-3)))` and `max-height: min(ideal, calc(100dvh - var(--rf-space-5)))`; vertical overflow stays inside the overlay and horizontal overflow must use explicit `.scrollX` islands.
- `Popover` is responsive by default and renders as a bottom `Sheet` below the narrow viewport threshold; callers may set `responsive={false}` or `forceSheet` for deterministic behavior.

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

| Item        | Values                                                                 |
| ----------- | ---------------------------------------------------------------------- |
| Controls    | default `30px`, small `26px`, large `36px`                             |
| Switches    | track `36×20px`, thumb `16px`, inset `2px`, visual travel `18px`       |
| Icons       | default `15px`, small `13px`, large `18px`, tap target at least `28px` |
| Spacing     | `4 / 8 / 12 / 16 / 22 / 32px`                                          |
| Radii       | chip `6px`, control `8px`, card/popover `10px`, pill `999px`           |
| Lines/rings | hairline `1px`, focus ring `2px`                                       |
| Type        | `11.5 / 12.5 / 13.5 / 15 / 19px`                                       |
| Layout      | nav `220px` with `180px` min, content max `640px`                      |
| Overlays    | popover `210–360px`, dialog `340px`, tooltip max `280px`               |

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

### Scheduler feature

The scheduler UI is opened by the `scheduler` page (`openScheduler({ taskId? })`) and by EventLog clicks on `cron_fire` events. Keep the GUI model aligned with backend cron tools:

- `cron_create`: 5-field cron, prompt, description, recurring flag, durable flag.
- `cron_list`: scope filter (`session`, `durable`, `all`), list rows with human schedule, next fire, fire count, and scope chips.
- `cron_delete`: remove by id and update the visible list.

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
- **Hidden Roles**: `event` messages feed EventLog except `plan_delta`; `plan` plus `plan_delta` messages feed PlanBanner. Both stay out of `selectVisibleMessages`.
- **Thinking Blocks**: `thinking_blocks: [{thinking, signature}]` on assistant messages. Collapsible UI. Signatures are opaque — never mutate.
- **Reasoning Content**: Separate `reasoning_content` field. Collapsible.
- **Knowledge/Memory**: `remember_how_to_use_tools` → vecdb → `context_file` messages. Knowledge graph view.
- **Customization**: Agent modes, subagent forms, tool parameter editor.
- **Tour/Onboarding**: Welcome screen, guided tour bubbles.
- **FIM Debug**: Fill-in-Middle debug panel with search context and symbol list.
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
