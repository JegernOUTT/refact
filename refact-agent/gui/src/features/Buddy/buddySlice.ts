import { createSlice, PayloadAction } from "@reduxjs/toolkit";
import type {
  BuddySnapshot,
  BuddyState,
  BuddyActivityEntry,
  BuddySuggestion,
  BuddySettings,
  BuddyConversationMeta,
  DiagnosticContext,
} from "./types";

interface BuddySignalQueueItem {
  signalType: string;
  timestamp: number;
}

interface BuddySliceState {
  snapshot: BuddySnapshot | null;
  loading: boolean;
  conversations: BuddyConversationMeta[];
  recentDiagnostics: DiagnosticContext[];
  signalQueue: BuddySignalQueueItem[];
}

const initialState: BuddySliceState = {
  snapshot: null,
  loading: false,
  conversations: [],
  recentDiagnostics: [],
  signalQueue: [],
};

export const buddySlice = createSlice({
  name: "buddy",
  initialState,
  reducers: {
    setBuddySnapshot: (state, action: PayloadAction<BuddySnapshot>) => {
      state.snapshot = action.payload;
      state.loading = false;
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
      action: PayloadAction<BuddyConversationMeta[]>,
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
} = buddySlice.selectors;
