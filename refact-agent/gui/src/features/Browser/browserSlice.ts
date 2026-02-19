import { createSlice, PayloadAction } from "@reduxjs/toolkit";
import { RootState } from "../../app/store";
import { applyChatEvent } from "../Chat/Thread/actions";

export type DiffBox = {
  x: number;
  y: number;
  width: number;
  height: number;
};

export type BrowserTabInfo = {
  tab_id: string;
  url: string;
  title: string;
};

export type BrowserFrame = {
  mime: string;
  data: string;
  diff_boxes: DiffBox[];
};

export type TimelineEntry = {
  timestamp: string;
  source: "user" | "agent";
  type: string;
  summary: string;
  details?: Record<string, unknown>;
};

export type TimelineFilterSource = "all" | "user" | "agent";

export type BrowserNotification = {
  type: "detached" | "attached" | "closed" | "timeout";
  message: string;
};

export type BrowserContextOversizeInfo = {
  pending_message_id: string;
  total_bytes: number;
  action_count: number;
  action_bytes: number;
  console_count: number;
  console_bytes: number;
  network_count: number;
  network_bytes: number;
  mutation_bytes: number;
};

export type BrowserToolbarActionType =
  | "screenshot"
  | "screenshot_full"
  | "pick_element"
  | "paste_actions"
  | "paste_console"
  | "paste_network"
  | "curl"
  | "summarize"
  | "extract_json"
  | "annotate"
  | "annotate_send"
  | "annotate_clear"
  | "rect_highlight";

export type BrowserRuntime = {
  runtime_id: string;
  connected: boolean;
  active_tab: string | null;
  url: string | null;
  title: string | null;
  tabs: BrowserTabInfo[];
  latest_frame: BrowserFrame | null;
  picker_active: boolean;
  annotate_active: boolean;
  attach_screenshot_on_send: boolean;
  timeline: TimelineEntry[];
  timeline_open: boolean;
  timeline_filter_source: TimelineFilterSource;
  timeline_filter_type: string | null;
  notification: BrowserNotification | null;
  oversize_info: BrowserContextOversizeInfo | null;
  pending_toolbar_actions: BrowserToolbarActionType[];
};

export type BrowserState = {
  runtimes: Record<string, BrowserRuntime | undefined>;
  browserUiOpen: Record<string, boolean>;
};

const initialState: BrowserState = {
  runtimes: {},
  browserUiOpen: {},
};

const TIMELINE_MAX = 2000;

const VALID_TOOLBAR_ACTIONS = [
  "screenshot",
  "screenshot_full",
  "pick_element",
  "paste_actions",
  "paste_console",
  "paste_network",
  "curl",
  "summarize",
  "extract_json",
  "annotate",
  "annotate_send",
  "annotate_clear",
  "rect_highlight",
] as const;

export function makeBrowserRuntime(runtime_id: string): BrowserRuntime {
  return {
    runtime_id,
    connected: true,
    active_tab: null,
    url: null,
    title: null,
    tabs: [],
    latest_frame: null,
    picker_active: false,
    annotate_active: false,
    attach_screenshot_on_send: true,
    timeline: [],
    timeline_open: true,
    timeline_filter_source: "all",
    timeline_filter_type: null,
    notification: null,
    oversize_info: null,
    pending_toolbar_actions: [],
  };
}

export const browserSlice = createSlice({
  name: "browser",
  initialState,
  reducers: {
    setBrowserRuntime(
      state,
      action: PayloadAction<{ chatId: string; runtime: BrowserRuntime }>,
    ) {
      state.runtimes[action.payload.chatId] = action.payload.runtime;
    },
    updateBrowserStatus(
      state,
      action: PayloadAction<{
        chatId: string;
        connected: boolean;
        url?: string | null;
        title?: string | null;
      }>,
    ) {
      const rt = state.runtimes[action.payload.chatId];
      if (rt) {
        rt.connected = action.payload.connected;
        if (action.payload.url !== undefined) rt.url = action.payload.url;
        if (action.payload.title !== undefined) rt.title = action.payload.title;
      }
    },
    updateBrowserFrame(
      state,
      action: PayloadAction<{ chatId: string; frame: BrowserFrame }>,
    ) {
      const rt = state.runtimes[action.payload.chatId];
      if (rt) {
        rt.latest_frame = action.payload.frame;
      }
    },
    removeBrowserRuntime(state, action: PayloadAction<{ chatId: string }>) {
      state.runtimes[action.payload.chatId] = undefined;
    },
    setPickerActive(
      state,
      action: PayloadAction<{ chatId: string; active: boolean }>,
    ) {
      const rt = state.runtimes[action.payload.chatId];
      if (rt) {
        rt.picker_active = action.payload.active;
      }
    },
    setAnnotateActive(
      state,
      action: PayloadAction<{ chatId: string; active: boolean }>,
    ) {
      const rt = state.runtimes[action.payload.chatId];
      if (rt) {
        rt.annotate_active = action.payload.active;
      }
    },
    toggleAttachScreenshotOnSend(
      state,
      action: PayloadAction<{ chatId: string }>,
    ) {
      const rt = state.runtimes[action.payload.chatId];
      if (rt) {
        rt.attach_screenshot_on_send = !rt.attach_screenshot_on_send;
      }
    },
    addTimelineEntries(
      state,
      action: PayloadAction<{
        chatId: string;
        entries: TimelineEntry[];
      }>,
    ) {
      const rt = state.runtimes[action.payload.chatId];
      if (rt) {
        rt.timeline.push(...action.payload.entries);
        if (rt.timeline.length > TIMELINE_MAX) {
          rt.timeline.splice(0, rt.timeline.length - TIMELINE_MAX);
        }
      }
    },
    clearTimeline(state, action: PayloadAction<{ chatId: string }>) {
      const rt = state.runtimes[action.payload.chatId];
      if (rt) {
        rt.timeline = [];
      }
    },
    toggleTimelineOpen(state, action: PayloadAction<{ chatId: string }>) {
      const rt = state.runtimes[action.payload.chatId];
      if (rt) {
        rt.timeline_open = !rt.timeline_open;
      }
    },
    setTimelineFilterSource(
      state,
      action: PayloadAction<{
        chatId: string;
        source: TimelineFilterSource;
      }>,
    ) {
      const rt = state.runtimes[action.payload.chatId];
      if (rt) {
        rt.timeline_filter_source = action.payload.source;
      }
    },
    setTimelineFilterType(
      state,
      action: PayloadAction<{ chatId: string; type: string | null }>,
    ) {
      const rt = state.runtimes[action.payload.chatId];
      if (rt) {
        rt.timeline_filter_type = action.payload.type;
      }
    },
    setBrowserNotification(
      state,
      action: PayloadAction<{
        chatId: string;
        notification: BrowserNotification | null;
      }>,
    ) {
      const rt = state.runtimes[action.payload.chatId];
      if (rt) {
        rt.notification = action.payload.notification;
      }
    },
    markBrowserDetached(state, action: PayloadAction<{ chatId: string }>) {
      const rt = state.runtimes[action.payload.chatId];
      if (rt) {
        rt.connected = false;
        rt.notification = {
          type: "detached",
          message: "Browser session detached",
        };
      }
    },
    markBrowserClosed(
      state,
      action: PayloadAction<{ chatId: string; reason: string }>,
    ) {
      const rt = state.runtimes[action.payload.chatId];
      if (rt) {
        rt.connected = false;
        rt.notification = {
          type: "closed",
          message: `Browser closed: ${action.payload.reason}`,
        };
      }
    },
    setBrowserContextOversize(
      state,
      action: PayloadAction<{
        chatId: string;
        info: BrowserContextOversizeInfo;
      }>,
    ) {
      const rt = state.runtimes[action.payload.chatId];
      if (rt) {
        rt.oversize_info = action.payload.info;
      }
    },
    clearBrowserContextOversize(
      state,
      action: PayloadAction<{ chatId: string }>,
    ) {
      const rt = state.runtimes[action.payload.chatId];
      if (rt) {
        rt.oversize_info = null;
      }
    },
    shiftPendingToolbarAction(
      state,
      action: PayloadAction<{ chatId: string }>,
    ) {
      const rt = state.runtimes[action.payload.chatId];
      if (rt && rt.pending_toolbar_actions.length > 0) {
        rt.pending_toolbar_actions.shift();
      }
    },
    openBrowserUi(state, action: PayloadAction<{ chatId: string }>) {
      state.browserUiOpen[action.payload.chatId] = true;
    },
    closeBrowserUi(state, action: PayloadAction<{ chatId: string }>) {
      state.browserUiOpen[action.payload.chatId] = false;
      state.runtimes[action.payload.chatId] = undefined;
    },
  },
  extraReducers: (builder) => {
    builder.addCase(applyChatEvent, (state, action) => {
      const event = action.payload;

      if (event.type === "browser_closed") {
        const rt = state.runtimes[event.chat_id];
        // If a runtime exists, only apply if runtime_id matches (ignore stale events from
        // a previous session). If runtime is already gone, close the UI unconditionally.
        if (rt?.runtime_id && event.runtime_id !== rt.runtime_id) return;
        state.browserUiOpen[event.chat_id] = false;
        state.runtimes[event.chat_id] = undefined;
        return;
      }

      if (!state.browserUiOpen[event.chat_id]) return;

      if (event.type === "browser_context_oversize") {
        const rt = state.runtimes[event.chat_id];
        if (!rt) return;
        rt.oversize_info = {
          pending_message_id: event.pending_message_id,
          total_bytes: event.total_bytes,
          action_count: event.action_count,
          action_bytes: event.action_bytes,
          console_count: event.console_count,
          console_bytes: event.console_bytes,
          network_count: event.network_count,
          network_bytes: event.network_bytes,
          mutation_bytes: event.mutation_bytes,
        };
      } else if (event.type === "browser_frame") {
        const rt = state.runtimes[event.chat_id];
        if (!rt) return;
        rt.latest_frame = {
          mime: event.mime,
          data: event.data,
          diff_boxes: event.diff_boxes ?? [],
        };
      } else if (event.type === "browser_status") {
        if (!state.runtimes[event.chat_id]) {
          if (event.connected && event.runtime_id) {
            state.runtimes[event.chat_id] = makeBrowserRuntime(
              event.runtime_id,
            );
          } else {
            return;
          }
        }
        const rt = state.runtimes[event.chat_id];
        if (!rt) return;
        if (rt.runtime_id && event.runtime_id !== rt.runtime_id) return;
        rt.connected = event.connected;
        if (!event.connected) {
          rt.url = event.url !== undefined ? event.url ?? null : null;
          rt.title = event.title !== undefined ? event.title ?? null : null;
          rt.active_tab =
            event.active_tab !== undefined ? event.active_tab ?? null : null;
          rt.tabs = event.tabs
            ? event.tabs.map((t) => ({
                tab_id: t.tab_id,
                url: t.url,
                title: t.title,
              }))
            : [];
        } else {
          if (event.url !== undefined) rt.url = event.url ?? null;
          if (event.title !== undefined) rt.title = event.title ?? null;
          if (event.active_tab !== undefined)
            rt.active_tab = event.active_tab ?? null;
          if (event.tabs !== undefined) {
            rt.tabs = event.tabs.map((t) => ({
              tab_id: t.tab_id,
              url: t.url,
              title: t.title,
            }));
          }
        }
      } else if (event.type === "browser_timeline") {
        const rt = state.runtimes[event.chat_id];
        if (!rt) return;
        const newEntries = event.events.map((e) => ({
          timestamp: e.timestamp,
          source: (e.source === "user" ? "user" : "agent") as "user" | "agent",
          type: e.type,
          summary: e.summary,
          details: e.details,
        }));
        rt.timeline.push(...newEntries);
        if (rt.timeline.length > TIMELINE_MAX) {
          rt.timeline.splice(0, rt.timeline.length - TIMELINE_MAX);
        }
      } else if (event.type === "browser_toolbar_action") {
        const rt = state.runtimes[event.chat_id];
        if (!rt) return;
        const toolbarAction =
          event.action as (typeof VALID_TOOLBAR_ACTIONS)[number];
        if (VALID_TOOLBAR_ACTIONS.includes(toolbarAction)) {
          rt.pending_toolbar_actions.push(toolbarAction);
        }
      }
    });
  },
});

export const {
  setBrowserRuntime,
  updateBrowserStatus,
  updateBrowserFrame,
  removeBrowserRuntime,
  setPickerActive,
  setAnnotateActive,
  toggleAttachScreenshotOnSend,
  addTimelineEntries,
  clearTimeline,
  toggleTimelineOpen,
  setTimelineFilterSource,
  setTimelineFilterType,
  setBrowserNotification,
  markBrowserDetached,
  markBrowserClosed,
  setBrowserContextOversize,
  clearBrowserContextOversize,
  shiftPendingToolbarAction,
  openBrowserUi,
  closeBrowserUi,
} = browserSlice.actions;

export const selectBrowserRuntime = (
  state: RootState,
  chatId: string,
): BrowserRuntime | undefined => state.browser.runtimes[chatId];

export const selectBrowserRuntimes = (state: RootState) =>
  state.browser.runtimes;

export const selectTimeline = (
  state: RootState,
  chatId: string,
): TimelineEntry[] => state.browser.runtimes[chatId]?.timeline ?? [];

export const selectTimelineOpen = (state: RootState, chatId: string): boolean =>
  state.browser.runtimes[chatId]?.timeline_open ?? false;

export const selectTimelineFilterSource = (
  state: RootState,
  chatId: string,
): TimelineFilterSource =>
  state.browser.runtimes[chatId]?.timeline_filter_source ?? "all";

export const selectTimelineFilterType = (
  state: RootState,
  chatId: string,
): string | null =>
  state.browser.runtimes[chatId]?.timeline_filter_type ?? null;

export const selectBrowserContextOversize = (
  state: RootState,
  chatId: string,
): BrowserContextOversizeInfo | null =>
  state.browser.runtimes[chatId]?.oversize_info ?? null;

export const selectBrowserUiOpen = (
  state: RootState,
  chatId: string,
): boolean => !!state.browser.browserUiOpen[chatId];
