import { createSlice, PayloadAction } from "@reduxjs/toolkit";
import { RootState } from "../../app/store";

export type DiffBox = {
  x: number;
  y: number;
  width: number;
  height: number;
};

export type BrowserTabInfo = {
  id: string;
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

export type BrowserRuntime = {
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
  timeline_filter_source: TimelineFilterSource;
  timeline_filter_type: string | null;
};

export type BrowserState = {
  runtimes: Record<string, BrowserRuntime | undefined>;
};

const initialState: BrowserState = {
  runtimes: {},
};

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
        if (action.payload.title !== undefined)
          rt.title = action.payload.title;
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
      const { [action.payload.chatId]: _, ...rest } = state.runtimes;
      state.runtimes = rest;
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
        rt.timeline = [...rt.timeline, ...action.payload.entries];
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
  },
});

export const {
  setBrowserRuntime,
  updateBrowserStatus,
  updateBrowserFrame,
  removeBrowserRuntime,
  setPickerActive,
  toggleAttachScreenshotOnSend,
  addTimelineEntries,
  clearTimeline,
  toggleTimelineOpen,
  setTimelineFilterSource,
  setTimelineFilterType,
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

export const selectTimelineOpen = (
  state: RootState,
  chatId: string,
): boolean => state.browser.runtimes[chatId]?.timeline_open ?? false;

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
