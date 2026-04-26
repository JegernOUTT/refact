import { createSlice, PayloadAction } from "@reduxjs/toolkit";
import type {
  BuddySnapshot,
  BuddyState,
  BuddyActivityEntry,
  BuddySuggestion,
  BuddySettings,
  BuddyConversationEntry,
  DiagnosticContext,
  BuddyRuntimeEvent,
  BuddySpeechItem,
} from "./types";

interface BuddySignalQueueItem {
  signalType: string;
  timestamp: number;
}

interface BuddySliceState {
  snapshot: BuddySnapshot | null;
  loading: boolean;
  conversations: BuddyConversationEntry[];
  recentDiagnostics: DiagnosticContext[];
  signalQueue: BuddySignalQueueItem[];
  runtimeQueue: BuddyRuntimeEvent[];
  nowPlaying: BuddyRuntimeEvent | null;
  activeSpeech: BuddySpeechItem | null;
}

const initialState: BuddySliceState = {
  snapshot: null,
  loading: false,
  conversations: [],
  recentDiagnostics: [],
  signalQueue: [],
  runtimeQueue: [],
  nowPlaying: null,
  activeSpeech: null,
};

export const buddySlice = createSlice({
  name: "buddy",
  initialState,
  reducers: {
    setBuddySnapshot: (state, action: PayloadAction<BuddySnapshot>) => {
      state.snapshot = action.payload;
      state.loading = false;
      state.activeSpeech = action.payload.active_speech ?? null;
    },
    updateBuddyState: (state, action: PayloadAction<BuddyState>) => {
      if (state.snapshot) {
        state.snapshot.state = action.payload;
      }
    },
    addBuddyActivity: (state, action: PayloadAction<BuddyActivityEntry>) => {
      if (state.snapshot) {
        state.snapshot.state.recent_activities.unshift(action.payload);
      }
    },
    addBuddySuggestion: (state, action: PayloadAction<BuddySuggestion>) => {
      if (state.snapshot) {
        state.snapshot.state.suggestion_state.push(action.payload);
      }
    },
    dismissBuddySuggestion: (state, action: PayloadAction<string>) => {
      if (state.snapshot) {
        const found = state.snapshot.state.suggestion_state.find(
          (s) => s.id === action.payload,
        );
        if (found) found.dismissed = true;
      }
    },
    updateBuddySettings: (state, action: PayloadAction<BuddySettings>) => {
      if (state.snapshot) {
        state.snapshot.settings = action.payload;
        state.snapshot.enabled = action.payload.enabled;
      }
    },
    setBuddyConversations: (
      state,
      action: PayloadAction<BuddyConversationEntry[]>,
    ) => {
      state.conversations = action.payload;
    },
    addBuddyDiagnostic: (state, action: PayloadAction<DiagnosticContext>) => {
      state.recentDiagnostics.unshift(action.payload);
      if (state.recentDiagnostics.length > 100) {
        state.recentDiagnostics.splice(100);
      }
    },
    enqueueBuddySignal: (state, action: PayloadAction<string>) => {
      state.signalQueue.push({ signalType: action.payload, timestamp: Date.now() });
      if (state.signalQueue.length > 50) state.signalQueue.shift();
    },
    consumeBuddySignal: (state) => {
      state.signalQueue.shift();
    },
    enqueueRuntimeEvent: (state, action: PayloadAction<BuddyRuntimeEvent>) => {
      const event = action.payload;
      if (event.dedupe_key) {
        const idx = state.runtimeQueue.findIndex(
          (e) => e.dedupe_key === event.dedupe_key,
        );
        if (idx >= 0) {
          state.runtimeQueue[idx] = event;
          return;
        }
        if (state.nowPlaying?.dedupe_key === event.dedupe_key) {
          state.nowPlaying = event;
          return;
        }
      }
      if (event.priority === "critical" || event.priority === "high") {
        state.runtimeQueue.unshift(event);
      } else {
        state.runtimeQueue.push(event);
      }
      if (state.runtimeQueue.length > 100) {
        state.runtimeQueue.splice(100);
      }
    },
    dequeueRuntimeEvent: (state) => {
      if (state.runtimeQueue.length > 0) {
        state.nowPlaying = state.runtimeQueue.shift()!;
      }
    },
    clearNowPlaying: (state) => {
      state.nowPlaying = null;
    },
    updateRuntimeProgress: (
      state,
      action: PayloadAction<{ dedupe_key: string; progress: number }>,
    ) => {
      const { dedupe_key, progress } = action.payload;
      const item = state.runtimeQueue.find((e) => e.dedupe_key === dedupe_key);
      if (item) {
        item.progress = progress;
      } else if (state.nowPlaying?.dedupe_key === dedupe_key) {
        state.nowPlaying.progress = progress;
      }
    },
    setActiveSpeech: (state, action: PayloadAction<BuddySpeechItem>) => {
      state.activeSpeech = action.payload;
    },
    clearActiveSpeech: (state) => {
      state.activeSpeech = null;
    },
  },
  selectors: {
    selectBuddySnapshot: (state) => state.snapshot,
    selectBuddyState: (state) => state.snapshot?.state ?? null,
    selectBuddySettings: (state) => state.snapshot?.settings ?? null,
    selectBuddyActivities: (state) =>
      state.snapshot?.state.recent_activities ?? [],
    selectBuddySuggestions: (state) =>
      state.snapshot?.state.suggestion_state ?? [],
    selectBuddyConversations: (state) => state.conversations,
    selectIsBuddyEnabled: (state) => state.snapshot?.enabled ?? false,
    selectBuddyDiagnostics: (state) => state.recentDiagnostics,
    selectBuddySignalQueue: (state) => state.signalQueue,
    selectRuntimeQueue: (state) => state.runtimeQueue,
    selectNowPlaying: (state) => state.nowPlaying,
    selectActiveSpeech: (state) => state.activeSpeech,
  },
});

export const {
  setBuddySnapshot,
  updateBuddyState,
  addBuddyActivity,
  addBuddySuggestion,
  dismissBuddySuggestion,
  updateBuddySettings,
  setBuddyConversations,
  addBuddyDiagnostic,
  enqueueBuddySignal,
  consumeBuddySignal,
  enqueueRuntimeEvent,
  dequeueRuntimeEvent,
  clearNowPlaying,
  updateRuntimeProgress,
  setActiveSpeech,
  clearActiveSpeech,
} = buddySlice.actions;

export const {
  selectBuddySnapshot,
  selectBuddyState,
  selectBuddySettings,
  selectBuddyActivities,
  selectBuddySuggestions,
  selectBuddyConversations,
  selectIsBuddyEnabled,
  selectBuddyDiagnostics,
  selectBuddySignalQueue,
  selectRuntimeQueue,
  selectNowPlaying,
  selectActiveSpeech,
} = buddySlice.selectors;
