# Browser Mode — Implementation Specification

## 1. Overview

Browser Mode is a new thread type in Refact that provides seamless browser integration. It launches a **real visible Chrome window** (separate OS window) controlled via CDP (Chrome DevTools Protocol) from the Rust engine. Both the **user and AI agent share the same browser session** — the user interacts with Chrome directly while the agent controls it through tools.

### Key characteristics
- **Real OS window**: Chrome runs visible (headless=false), not embedded in an IDE webview
- **Shared session**: User clicks/types in Chrome; agent sends CDP commands — both affect the same tabs
- **Action recording**: All user interactions are captured by an injected JS recorder and logged
- **Auto-context**: Browser state (actions, console, network, DOM mutations) is automatically prepended to user messages
- **Toolbar buttons**: One-click actions that perform browser operations and paste results into chat
- **Per-thread persistence**: Chrome profile, tabs, and window bounds survive engine restarts
- **No confirmations**: Power-user mode by default (all actions auto-approved)
- **Rust-only**: Built on the existing `headless_chrome` crate + CDP, no Node.js dependency

### Comparison with competitors
| Feature | Refact Browser Mode | Cursor | Cline | Claude Code |
|---------|-------------------|--------|-------|-------------|
| Browser engine | Real Chrome (CDP) | Playwright MCP | Chrome (CDP) | External MCP |
| Visibility | Visible OS window | IDE-embedded | Extension-embedded | N/A |
| User+Agent shared | ✅ | ❌ | ❌ | ❌ |
| Action recording | ✅ (injected JS) | ❌ | ❌ | ❌ |
| Confirmations | None (default) | Every action | Every action | N/A |
| Headless CI | Via config | IDE only | CLI with -y flag | Via MCP |

---

## 2. Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    User's Machine                           │
│                                                             │
│  ┌──────────────┐     CDP (WebSocket)    ┌───────────────┐ │
│  │ Chrome Window │◄─────────────────────►│ BrowserRuntime │ │
│  │ (visible)     │                       │ (Rust/Engine)  │ │
│  │               │  Injected recorder ──►│                │ │
│  │ User clicks,  │  Runtime.addBinding   │ Buffers:       │ │
│  │ types, scrolls│                       │  actions[]     │ │
│  └──────────────┘                        │  console[]     │ │
│                                          │  network[]     │ │
│                                          │  mutations[]   │ │
│                                          │  last_frame    │ │
│                                          └───────┬────────┘ │
│                                                  │          │
│                              SSE events ─────────┤          │
│                              HTTP endpoints ─────┤          │
│                                                  │          │
│  ┌───────────────────────────────────────────────▼────────┐ │
│  │                    Refact GUI                          │ │
│  │  ┌─────────────────────────────────────────────────┐  │ │
│  │  │ BrowserPanel                                     │  │ │
│  │  │  ┌──────────────────────────────────────────┐   │  │ │
│  │  │  │ BrowserToolbar (12+ buttons)              │   │  │ │
│  │  │  ├──────────────────────────────────────────┤   │  │ │
│  │  │  │ Status bar (URL + connection + timeline)  │   │  │ │
│  │  │  ├──────────────────────────────────────────┤   │  │ │
│  │  │  │ Latest frame (screenshot)                 │   │  │ │
│  │  │  ├──────────────────────────────────────────┤   │  │ │
│  │  │  │ ActionTimeline (collapsible)              │   │  │ │
│  │  │  ├──────────────────────────────────────────┤   │  │ │
│  │  │  │ BrowserContextGuard (when >100KB)         │   │  │ │
│  │  │  └──────────────────────────────────────────┘   │  │ │
│  │  ├─────────────────────────────────────────────────┤  │ │
│  │  │ Chat (normal chat interface below)               │  │ │
│  │  └─────────────────────────────────────────────────┘  │ │
│  └───────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
```

### Data flow
1. **User → Chrome**: Direct interaction (clicks, typing, scrolling)
2. **Chrome → Engine**: Recorder JS sends events via `Runtime.addBinding`; CDP `Log.entryAdded` / `Network.requestWillBeSent` events captured
3. **Engine → GUI**: SSE events (`BrowserFrame`, `BrowserStatus`, `BrowserClosed`, `BrowserTimeline`)
4. **GUI → Engine**: HTTP endpoint calls from toolbar buttons
5. **Engine → Chrome**: CDP commands from agent tools or toolbar actions
6. **On Send**: Engine auto-prepends browser context before user message

---

## 3. Thread Type & Mode Configuration

### browser.yaml
```yaml
schema_version: 6
id: browser
title: Browser
description: Browse the web with a real Chrome browser. Agent and user share the same browser session.
specific: false

thread_defaults:
  include_project_info: false
  checkpoints_enabled: false
  auto_approve_editing_tools: true
  auto_approve_dangerous_commands: true

ui:
  order: 35
  tags:
    - browser

prompt: |
  You are an AI assistant operating in Browser Mode...
  [balanced prompt: act on fresh context, ask user to refresh when stale]

tools:
  - chrome
  - web
  - web_search

tool_confirm:
  rules:
    - match: "*"
      action: auto
```

### BrowserMeta (persisted in thread JSON)
```rust
pub struct BrowserMeta {
    pub browser_runtime_id: Option<String>,   // UUID linking to active runtime
    pub profile_dir: Option<String>,          // Chrome user-data-dir path
    pub tab_urls: Vec<String>,                // URLs to restore on restart
    pub active_tab_id: Option<String>,        // Currently focused tab
    pub window_bounds: Option<WindowBounds>,  // {x, y, width, height}
    pub attach_screenshot_on_send: bool,      // Toggle: auto-attach viewport JPEG on Send
    pub mask_passwords: bool,                 // Toggle: mask type=password inputs (default: true)
}
```

### ThreadParams extension
`BrowserMeta` is stored as an optional field on `ThreadParams`:
```rust
pub struct ThreadParams {
    // ... existing fields ...
    pub browser_meta: Option<BrowserMeta>,
}
```

### Creating a Browser thread
1. User clicks **"New Browser"** button in GUI toolbar
2. GUI creates a thread with `mode: "browser"`
3. GUI calls `POST /v1/browser/start` to launch Chrome
4. Engine creates `BrowserRuntime`, stores `browser_runtime_id` in thread's `BrowserMeta`

---

## 4. BrowserRuntime (Engine)

### BrowserRuntime struct
```rust
pub struct BrowserRuntime {
    pub runtime_id: String,                         // UUID
    pub attached_chat_id: Option<String>,            // Thread that owns this session
    pub browser: Browser,                            // headless_chrome::Browser
    pub tabs: HashMap<String, Arc<AMutex<ChromeTab>>>,
    pub profile_dir: PathBuf,
    pub window_bounds: Option<WindowBounds>,
    pub buffers: BrowserBuffers,                     // All event buffers + cursors
    pub idle_timeout: Duration,                      // Default: 600s
    pub is_connected: bool,
    pub last_activity: Instant,
}
```

### BrowserBuffers struct (extracted for safe testing)
```rust
pub struct BrowserBuffers {
    pub action_buffer: Vec<RecorderEvent>,
    pub console_buffer: Vec<ConsoleEntry>,
    pub network_buffer: Vec<NetworkEntry>,
    pub mutation_summary: Vec<MutationSummaryEntry>,
    // "Since last send" cursors
    pub last_send_action_cursor: usize,
    pub last_send_console_cursor: usize,
    pub last_send_network_cursor: usize,
    pub last_send_mutation_cursor: usize,
    // Frame diffing
    pub last_frame_hash: Option<u64>,
    pub last_frame_data: Option<Vec<u8>>,
    pub last_send_frame_hash: Option<u64>,  // For page_changed detection
}
```

### Lifecycle

| State | Trigger | Action |
|-------|---------|--------|
| **Launch** | `POST /v1/browser/start` | Create Chrome process (headless=false), persistent profile dir, inject recorder |
| **Connect** | CDP WebSocket established | Mark `is_connected = true`, start event listeners |
| **Run** | Normal operation | Process recorder events, capture screenshots, serve endpoints |
| **Disconnect** | User closes Chrome / CDP drops | Mark `is_connected = false`, emit `BrowserClosed` SSE event |
| **Restart** | User clicks "Restart" | Relaunch Chrome with same `profile_dir`, reopen `tab_urls` |
| **Idle timeout** | No activity for `idle_timeout` | Emit `BrowserClosed { reason: "timeout" }`, close Chrome |
| **Detach** | Handoff to new thread | Set `attached_chat_id = None`, transfer to new thread |
| **Cleanup** | Thread deleted / engine shutdown | Close Chrome, preserve profile dir |

### Storage
- Profile dirs: `.refact/browser_profiles/{thread_id}/`
- BrowserRuntime map: `GlobalContext.browser_runtimes: HashMap<String, Arc<AMutex<BrowserRuntime>>>`
- Background monitor task: `browser_monitor_background_task` — detects disconnects, enforces idle timeouts

---

## 5. Recorder System

### Injection mechanism
1. `Page.addScriptToEvaluateOnNewDocument` — injects recorder JS before any site code runs
2. `Runtime.addBinding("__refact_event")` — creates a bridge for JS → Rust communication
3. `Runtime.bindingCalled` — engine listens for events sent by the recorder

### RecorderEvent types
```rust
pub enum RecorderEvent {
    Navigation { url: String, title: String, timestamp: f64 },
    Click { selector: String, text: String, x: f64, y: f64, timestamp: f64 },
    Input { selector: String, value: String, masked: bool, timestamp: f64 },
    Keypress { key: String, modifiers: Vec<String>, timestamp: f64 },
    Submit { selector: String, action: String, method: String, timestamp: f64 },
    Scroll { scroll_x: f64, scroll_y: f64, timestamp: f64 },
    MutationSummary { added: u32, removed: u32, changed: u32, timestamp: f64 },
}
```

### Password masking
- The recorder JS checks each input element for `type="password"` or `autocomplete="current-password"`
- If matched AND `mask_passwords` is true (per-thread setting, default ON): sends `masked: true` and Rust replaces value with `"*".repeat(value.len())`
- Other inputs send `masked: false` with full values

### Scroll debouncing
- Consecutive scroll events within 300ms are collapsed into one (keeping the latest position)
- Prevents event floods during continuous scrolling

### Buffer management
- Max buffer size: 10,000 entries per buffer type
- When exceeded: oldest entries are drained, cursors adjusted via `saturating_sub`
- `flush_buffer_since(cursor)`: returns items from cursor to end, advances cursor

### Console capture
- CDP `Log.enable` → `Log.entryAdded` events → stored as `ConsoleEntry { timestamp, level, text }`

### Network capture
- CDP `Network.enable` → `Network.requestWillBeSent` + `Network.responseReceived`
- **Filtered**: only `Document` and `XHR`/`Fetch` resource types are stored
- Stored as `NetworkEntry { timestamp, method, url, resource_type, status }`

### DOM mutation capture
- Injected JS installs a `MutationObserver` on `document.body`
- Accumulates a summary: `MutationSummaryEntry { added, removed, changed, descriptions[] }`
- Descriptions are compact (e.g., "childList changed on #app")

---

## 6. SSE Events (Browser-specific)

All browser events are delivered through the **existing chat SSE stream** (`GET /v1/chats/subscribe?chat_id={id}`), reusing the same `seq` numbering. GUI ignores unknown event types, so these don't break existing subscribers.

### BrowserFrame
```json
{
  "type": "browser_frame",
  "tab_id": "ABC123",
  "mime": "image/jpeg",
  "data": "<base64>",
  "diff_boxes": [{"x": 10, "y": 20, "width": 100, "height": 50}],
  "changed_text": "Button clicked, modal appeared"
}
```
- Emitted after user/agent actions, **only if the image meaningfully changed**
- Debounced: max 1 frame per 500ms, max ~2 frames/second
- Diff detection: downscaled grayscale hash comparison with noise threshold
- `diff_boxes`: approximate bounding boxes of changed regions
- `changed_text`: derived from DOM mutation summary

### BrowserStatus
```json
{
  "type": "browser_status",
  "runtime_id": "uuid",
  "connected": true,
  "active_tab": "tab-id",
  "url": "https://example.com",
  "title": "Example Page",
  "tabs": [{"tab_id": "t1", "url": "...", "title": "..."}]
}
```
- Emitted on: tab open/close, navigation, attach/detach, connect/disconnect

### BrowserClosed
```json
{
  "type": "browser_closed",
  "runtime_id": "uuid",
  "reason": "user_closed"  // or "timeout" or "crash"
}
```

### BrowserTimeline
```json
{
  "type": "browser_timeline",
  "events": [
    {
      "timestamp": "2025-01-01T10:00:00Z",
      "source": "user",
      "type": "click",
      "summary": "Clicked #submit-btn",
      "details": {"selector": "#submit-btn", "x": 200, "y": 350}
    }
  ]
}
```

### BrowserContextOversize
```json
{
  "type": "browser_context_oversize",
  "total_bytes": 150000,
  "action_count": 500,
  "action_bytes": 80000,
  "console_count": 200,
  "console_bytes": 30000,
  "network_count": 100,
  "network_bytes": 25000,
  "mutation_bytes": 15000,
  "pending_message_id": "uuid"
}
```
- Emitted when auto-include context exceeds 100KB threshold
- Triggers the GUI guard dialog

---

## 7. HTTP Endpoints (Complete API Reference)

Base: `http://127.0.0.1:{port}/v1/browser/`

### POST /v1/browser/start
Launch or reattach Chrome for a browser thread.

**Request:** `{ "chat_id": "string" }`
**Response:** `{ "runtime_id": "uuid", "status": "started" | "already_running" }`

### POST /v1/browser/stop
Stop Chrome for a thread.

**Request:** `{ "chat_id": "string" }`
**Response:** `{ "status": "stopped" }`

### POST /v1/browser/screenshot
Capture viewport (JPEG) or full page (PNG).

**Request:** `{ "chat_id": "string", "full_page": false }`
**Response:** `{ "mime": "image/jpeg", "data": "<base64>", "url": "https://...", "title": "Page Title" }`

### POST /v1/browser/context
Get browser context since last send.

**Request:** `{ "chat_id": "string", "max_bytes?": 102400, "last_n_actions?": 50 }`
**Response:**
```json
{
  "url": "https://example.com",
  "title": "Example",
  "actions": [/* RecorderEvent[] */],
  "console": [/* ConsoleEntry[] */],
  "network": [/* NetworkEntry[] */],
  "mutations": [/* MutationSummaryEntry[] */],
  "total_bytes": 45000
}
```

### POST /v1/browser/context/commit
Advance all "since last send" cursors (called after successful message send).

**Request:** `{ "chat_id": "string" }`
**Response:** `{ "status": "committed" }`

### POST /v1/browser/element-pick
Activate element picker in the page.

**Request:** `{ "chat_id": "string" }`
**Response:** `{ "status": "picker_active" }`

### GET /v1/browser/element-pick/result
Poll for picked element result.

**Request:** `{ "chat_id": "string" }`
**Response:** `{ "status": "waiting" }` or `{ "selector": "...", "innerText": "...", "bbox": {"x":0,"y":0,"width":100,"height":50} }`

### POST /v1/browser/curl
Get sanitized cURL for the last network request.

**Request:** `{ "chat_id": "string" }`
**Response:** `{ "curl": "curl -X GET '...'", "url": "...", "method": "GET", "status": 200 }`

### POST /v1/browser/eval
Evaluate JavaScript in the active tab.

**Request:** `{ "chat_id": "string", "expression": "document.title" }`
**Response:** `{ "result": "Page Title" }`

### POST /v1/browser/inject-css
Inject a CSS snippet into the page.

**Request:** `{ "chat_id": "string", "css": "body { border: 2px solid red; }", "id?": "my-style" }`
**Response:** `{ "style_id": "my-style" }`

### POST /v1/browser/remove-css
Remove a previously injected CSS snippet.

**Request:** `{ "chat_id": "string", "style_id": "my-style" }`
**Response:** `{ "status": "removed" }`

### POST /v1/browser/dom-snapshot
Get capped outerHTML for a selector.

**Request:** `{ "chat_id": "string", "selector": "#main", "max_chars?": 5000 }`
**Response:** `{ "html": "<div id='main'>...</div>", "truncated": false }`
- UTF-8 safe truncation (by char boundary, not byte)

### POST /v1/browser/accessibility
Get accessibility tree snapshot.

**Request:** `{ "chat_id": "string" }`
**Response:** `{ "tree": [/* AccessibilityNode[] */] }`

### POST /v1/browser/record-animation
Capture a burst of frames (default: 2 seconds at 5 FPS).

**Request:** `{ "chat_id": "string", "duration_ms?": 2000, "fps?": 5 }`
**Response:** `{ "frames": [{"mime": "image/jpeg", "data": "<base64>", "timestamp": 1234567890.0}] }`
- FPS clamped to [1, 60], duration to [100, 10000]ms

### POST /v1/browser/handoff
Transfer browser session to a different thread.

**Request:** `{ "from_chat_id": "old-thread", "to_chat_id": "new-thread" }`
**Response:** `{ "runtime_id": "uuid", "status": "transferred", "from_chat_id": "...", "to_chat_id": "..." }`

---

## 8. Auto-Include on Send

When a user sends a message in a Browser thread, the engine automatically prepends browser context.

### What gets included
- Current URL + page title
- User actions since last send (clicks, inputs, scrolls, navigation, etc.)
- Console log entries since last send
- Network requests since last send (Document + XHR/Fetch only)
- DOM mutation summary since last send
- Viewport JPEG screenshot (if `attach_screenshot_on_send` toggle is ON **and** page changed)

### "Since last send" cursor system
Each buffer type has a cursor tracking what was already included:
- `last_send_action_cursor`
- `last_send_console_cursor`
- `last_send_network_cursor`
- `last_send_mutation_cursor`

After a successful send, `POST /v1/browser/context/commit` advances all cursors.

### Context message format
```
[Browser Context]
URL: https://example.com
Title: Example Page

## User Actions (since last message)
[12:34:05] navigate → https://example.com
[12:34:08] click → button.submit "Submit Form" (x:200, y:350)
[12:34:10] input → input#email "user@example.com"
[12:34:12] submit → form#login POST /api/login

## Console (since last message)
[12:34:06] [error] Uncaught TypeError: Cannot read property 'x' of null
[12:34:09] [warn] Deprecated API usage

## Network (since last message)
[12:34:05] GET https://example.com → 200 (text/html, 15KB)
[12:34:06] POST /api/login → 401 (application/json, 0.2KB)

## DOM Changes (since last message)
Added: 3 elements (div.modal, span.error, button.retry)
Removed: 1 element (div.loading)
Changed: 2 elements (text content updated)
```

### 100KB threshold guard
1. Engine computes context payload size
2. If > 100KB (102,400 bytes):
   - Store user message as pending (not dropped)
   - Set session to `WaitingUserInput`
   - Emit `BrowserContextOversize` SSE event with byte breakdown
3. GUI shows interactive dialog (see §9.5)
4. User sends `BrowserContextDecision` command with caps
5. Engine builds capped context, inserts it + original message, resumes

### Page-changed detection
- `last_send_frame_hash`: hash of frame at time of last context commit
- `last_frame_hash`: hash of most recently captured frame
- `page_changed = last_frame_hash != last_send_frame_hash`
- Updated in `commit_browser_cursors()`

---

## 9. GUI Components

### 9.1 "New Browser" Button
- Located in `Toolbar.tsx` alongside "New Chat" and "New Task"
- Creates a thread with `mode: "browser"`
- Auto-calls `POST /v1/browser/start` on creation

### 9.2 BrowserLayout
- Wrapper component that renders `BrowserPanel` + normal `Chat` in a vertical split
- Active when `thread.mode === "browser"`

### 9.3 BrowserPanel
- **Frame display**: Shows latest `BrowserFrame` from SSE as an `<img>` tag
- **Status bar**: Connection dot (green/red) + current URL + "Timeline" toggle button
- **Notifications**: Inline banner for detached/closed/timeout with "Restart" and "✕" buttons
- **Integrates**: BrowserToolbar, ActionTimeline, BrowserContextGuard

### 9.4 BrowserToolbar
All buttons call engine endpoints and paste results into chat:

| Button | Endpoint | Chat insertion |
|--------|----------|---------------|
| ▶️ Start / ⏹️ Stop | `/browser/start` or `/stop` | Status message |
| 🔄 Handoff | `/browser/handoff` | Detach/attach notifications |
| 📷 Screenshot | `/browser/screenshot?full_page=false` | Multimodal image (JPEG) |
| 📄 Full Page | `/browser/screenshot?full_page=true` | Multimodal image (PNG) |
| 📋 Actions | `/browser/context?fields=actions` | Formatted text |
| ⚠️ Console | `/browser/context?fields=console` | Formatted text |
| 🌐 Network | `/browser/context?fields=network` | Formatted text |
| 🔗 cURL | `/browser/curl` | cURL command text |
| 🎯 Pick Element | `/browser/element-pick` → poll `/element-pick/result` | Selector + text + bbox |
| 📎 Auto-Screenshot | Local toggle | Updates `attach_screenshot_on_send` |
| 📽️ Record | `/browser/record-animation` | Gallery of frames |
| 📝 Summarize | Screenshot + "Summarize this page" prompt | Agent response |
| 📊 Extract JSON | Screenshot + "Extract data as JSON" prompt | Agent response |

Each button shows loading state while the endpoint is being called.

### 9.5 BrowserContextGuard
Interactive dialog triggered when auto-include context exceeds 100KB:

- **Header**: "Browser context is large (X KB)"
- **Breakdown**: per-category byte counts and item counts
- **Slider**: "Include last N actions" — dynamically recalculates total size
- **Checkboxes**: ☑ Actions ☑ Console ☑ Network ☑ Mutations ☐ Screenshot
- **Live total**: "Estimated: XX KB"
- **Buttons**: [Include All] [Include Selected] [Skip Context] [Cancel Send]

Sends `BrowserContextDecision` command back to engine.

### 9.6 ActionTimeline
Collapsible chronological event list:

- **User events** (blue): from recorder (clicks, input, navigation, scroll)
- **Agent events** (green): from tool call messages in chat
- Each entry: timestamp + icon + one-line summary
- Click to expand: full details (selector, URL, coordinates, etc.)
- **Filters**: by source (all/user/agent) and by type
- Auto-scrolls to latest entry

---

## 10. Redux State (browserSlice)

### Types
```typescript
type BrowserRuntime = {
  runtime_id: string;
  connected: boolean;
  active_tab: string | null;
  url: string | null;
  title: string | null;
  tabs: BrowserTabInfo[];
  latest_frame: BrowserFrame | null;
  picker_active: boolean;
  attach_screenshot_on_send: boolean;
  timeline: TimelineEntry[];
  timeline_open: boolean;
  timeline_filter_source: TimelineFilterSource;  // "all" | "user" | "agent"
  timeline_filter_type: string | null;
  notification: BrowserNotification | null;
  oversize_info: BrowserContextOversizeInfo | null;
};

type BrowserState = {
  runtimes: Record<string, BrowserRuntime | undefined>;
};
```

### Actions
| Action | Payload | Purpose |
|--------|---------|---------|
| `setBrowserRuntime` | `{chatId, runtime}` | Set/replace entire runtime |
| `updateBrowserStatus` | `{chatId, connected, url?, title?}` | Update connection/URL |
| `updateBrowserFrame` | `{chatId, frame}` | Set latest frame |
| `removeBrowserRuntime` | `{chatId}` | Remove runtime |
| `setPickerActive` | `{chatId, active}` | Toggle picker mode |
| `toggleAttachScreenshotOnSend` | `{chatId}` | Toggle auto-screenshot |
| `addTimelineEntries` | `{chatId, entries[]}` | Append timeline events |
| `clearTimeline` | `{chatId}` | Clear all events |
| `toggleTimelineOpen` | `{chatId}` | Toggle panel visibility |
| `setTimelineFilterSource` | `{chatId, source}` | Filter by user/agent/all |
| `setTimelineFilterType` | `{chatId, type}` | Filter by event type |
| `setBrowserNotification` | `{chatId, notification}` | Set/clear notification |
| `markBrowserDetached` | `{chatId}` | Mark disconnected + detached notice |
| `markBrowserClosed` | `{chatId, reason}` | Mark disconnected + closed notice |
| `setOversizeInfo` | `{chatId, info}` | Set oversize guard data |
| `clearOversizeInfo` | `{chatId}` | Clear after decision |

### Selectors
- `selectBrowserRuntime(state, chatId)` → `BrowserRuntime | undefined`
- `selectBrowserRuntimes(state)` → all runtimes map
- `selectTimeline(state, chatId)` → `TimelineEntry[]`
- `selectTimelineOpen(state, chatId)` → `boolean`
- `selectTimelineFilterSource(state, chatId)` → `TimelineFilterSource`
- `selectTimelineFilterType(state, chatId)` → `string | null`

### SSE → Redux mapping
| SSE Event | Redux Action |
|-----------|-------------|
| `browser_frame` | `updateBrowserFrame` |
| `browser_status` | `updateBrowserStatus` |
| `browser_closed` | `markBrowserClosed` |
| `browser_timeline` | `addTimelineEntries` |
| `browser_context_oversize` | `setOversizeInfo` |

---

## 11. Concurrency & Shared Control

### Policy: Mixed (user-preemptive)
- **User input**: applied immediately (user interacts directly with Chrome window)
- **Agent actions**: queued; only execute when no user activity for ~800ms
- **Status display**: "User active" / "Agent executing: click(#submit)"

### Implementation
- BrowserRuntime tracks `last_activity: Instant` (updated on recorder events)
- Agent tool calls check `last_activity.elapsed() > idle_threshold` before executing
- If user is active: agent action is deferred (retried after brief delay)

---

## 12. Session Handoff

### Flow
1. User clicks "🔄 Handoff" → prompted for target chat ID
2. GUI calls `POST /v1/browser/handoff { from_chat_id, to_chat_id }`
3. Engine:
   - Sets old thread's `browser_meta.browser_runtime_id = None`
   - Transfers runtime ownership: `runtime.attached_chat_id = to_chat_id`
   - Updates new thread's `browser_meta` with runtime_id, profile_dir, tab_urls
   - Does **NOT** restart Chrome
4. Old thread: `markBrowserDetached` → notification "Browser session detached"
5. New thread: `setBrowserRuntime` → notification "Browser session attached"

### What transfers
- Running Chrome process (same PID, same CDP connection)
- All open tabs and their state
- Profile directory reference
- Latest frame
- **Buffers are NOT transferred** (new thread starts with empty "since last send")

---

## 13. Builtin web_search Tool

### Overview
A new `web_search` tool that performs web searches via DuckDuckGo HTML scraping. Requires no API key.

### Gating logic
- Only registered when the current model's `supports_web_search == false`
- When provider supports server-side web_search (e.g., Anthropic): builtin tool is NOT registered
- This avoids tool name collision

### Tool definition
```
name: "web_search"
description: "Search the web using DuckDuckGo. Returns titles, URLs, and snippets."
parameters:
  - query: string (required)
  - num_results: integer (optional, default 8, max 20)
```

### Implementation
1. `GET https://html.duckduckgo.com/html/?q={query}` with browser-like User-Agent
2. Parse HTML response using regex/string parsing for result entries
3. Extract: title, URL (decode DuckDuckGo redirect), snippet
4. Return formatted results

### Output format
```
Web search results for "rust async tutorial":

1. [Async Programming in Rust](https://rust-lang.github.io/async-book/)
   The async book provides a comprehensive guide to async programming in Rust...

2. [Tokio Tutorial](https://tokio.rs/tokio/tutorial)
   Learn how to use Tokio for async I/O in Rust...
```

### Error handling
- Rate limit / captcha: retry once with different User-Agent
- If still fails: return error message suggesting retry later
- Timeout: 10 seconds per request

---

## 14. Security Considerations

### Password masking
- Per-thread toggle (`mask_passwords`), default **ON**
- Recorder JS checks `type="password"` and `autocomplete="current-password"`
- Masked values replaced with `"*".repeat(length)` before storing in buffers
- When OFF: full values are logged (power-user choice)

### CSS injection safety
- CSS is injected via `element.textContent = css` (safe, no HTML parsing)
- Rust side uses `serde_json::to_string()` to safely escape CSS content in JS template
- Avoids backtick/template literal injection

### Path traversal prevention
- Profile directories use thread IDs (UUIDs)
- Stored under `.refact/browser_profiles/{thread_id}/`
- Thread IDs are validated as UUIDs before path construction

### No confirmations (by design)
- All browser actions (click, type, eval, CSS inject) execute without user approval
- Recommended: add a configurable safety switch in future:
  - `browser.confirmations = off | write_only | all`

### Data exposure
- Auto-included context sends all recorded data to the LLM
- Network request URLs, console logs, and input values are included
- Users should be aware that sensitive page content will be sent to the model

---

## 15. Known Limitations & Future Work

### Current limitations
- **Cross-origin iframes**: Recorder only captures top-frame events; interactions inside cross-origin iframes are not recorded
- **Single active tab**: Most endpoint handlers use `first()` tab; multi-tab workflows need explicit tab ID parameter support
- **CSP-hostile sites**: Some sites may block recorder injection despite `addScriptToEvaluateOnNewDocument` (rare)
- **SPA event floods**: Single-page apps with heavy DOM mutations can generate large buffers quickly; the 100KB guard mitigates this

### Future work
- **PDF export**: Use Playwright's `page.pdf()` equivalent via CDP `Page.printToPDF`
- **HAR export**: Build HAR files from captured network entries
- **Stealth mode**: Anti-detection measures for sites that block automation
- **Continuous streaming**: Higher-FPS frame streaming for remote-desktop-like experience
- **Multi-tab UI**: Tab bar in BrowserPanel for explicit tab management
- **Visual diff overlays**: Render diff bounding boxes on top of frame images in GUI
- **File downloads**: Handle Chrome download events and surface in chat
- **Authentication persistence**: Better cookie/session management across restarts
- **Configurable safety levels**: `off | write_only | all` confirmation modes
