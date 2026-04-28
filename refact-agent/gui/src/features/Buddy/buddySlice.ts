import { createSelector, createSlice, PayloadAction } from "@reduxjs/toolkit";
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
  BuddyOpportunity,
  OpportunityStatus,
  BuddyPulse,
  BuddyDraft,
} from "./types";

function defaultBuddyState(): BuddyState {
  return {
    identity: {
      name: "Buddy",
      created_at: "",
      palette_index: 0,
    },
    progression: {
      stage: 0,
      stage_name: "Egg",
      level: 1,
      xp: 0,
      xp_next: 20,
    },
    skills: {
      unlocked: [],
      locked: [],
    },
    workflow_summaries: [],
    semantic: {
      mood: "Playful",
      focus: "helping",
      headline: "",
      last_active: "",
    },
    recent_activities: [],
    suggestion_state: [],
    pet: {
      needs: {
        hunger: 80,
        energy: 85,
        hygiene: 80,
        boredom: 15,
        affection: 75,
      },
      condition: {
        sleeping: false,
        hungry: false,
        sleepy: false,
        dirty: false,
        bored: false,
        lonely: false,
      },
      evolution: {
        care_score: 0,
        neglect_score: 0,
        open_seconds: 0,
        last_evolved_at: null,
      },
    },
    personality: {
      archetype_id: "helper_sprite",
      archetype_label: "Helper Sprite",
      vibe: "Playful, quirky, helpful",
      summary: "An energetic helper with gentle mischief and warm humor.",
      prompt:
        "Playful, quirky, helpful. Think energetic pet meets curious assistant—gentle mischief, warm humor, celebration of small wins",
      traits: {
        playfulness: 70,
        chaos: 35,
        sociability: 72,
        curiosity: 78,
        resilience: 66,
      },
    },
    active_quest: null,
    opportunities: [],
  };
}

function defaultBuddySettings(): BuddySettings {
  return {
    enabled: true,
    auto_diagnostics: true,
    auto_issue_creation: false,
    personality_prompt: null,
    proactive_enabled: true,
    message_observation_enabled: false,
    housekeeping_enabled: true,
    humor_enabled: true,
    humor_level: "light",
    autonomy_level: "suggest",
    quiet_mode: false,
    observers: {
      task_health: true,
      trajectory_clutter: true,
      chat_pattern: false,
      customization_drift: true,
      memory_garden: true,
      mcp_auth: true,
      git_pressure: true,
      diagnostic_cluster: true,
      provider_health: true,
    },
  };
}

export function defaultBuddyPulse(): BuddyPulse {
  return {
    generated_at: null,
    tasks: { total: 0, stuck: 0, abandoned: 0, by_status: {} },
    trajectories: { total: 0, untitled: 0, oldest_age_days: 0 },
    memory: { total: 0, orphan: 0, stale_conflicts: 0 },
    providers: { defaults_ok: true, broken_refs: 0, quota_warnings: 0 },
    mcp: { total: 0, failing: 0, auth_expiring: 0 },
    customization: { modes: 0, skills: 0, commands: 0, subagents: 0, hooks: 0 },
    diagnostics: { last_hour: 0, top_error_types: [] },
    git: { uncommitted_files: 0, diff_lines_4h: 0, branches: 0 },
  };
}

function normalizeBuddyState(state: Partial<BuddyState>): BuddyState {
  const base = defaultBuddyState();
  return {
    ...base,
    ...state,
    identity: {
      ...base.identity,
      ...state.identity,
    },
    progression: {
      ...base.progression,
      ...state.progression,
    },
    skills: {
      ...base.skills,
      ...state.skills,
    },
    semantic: {
      ...base.semantic,
      ...state.semantic,
    },
    recent_activities: state.recent_activities ?? base.recent_activities,
    suggestion_state: state.suggestion_state ?? base.suggestion_state,
    workflow_summaries: state.workflow_summaries ?? base.workflow_summaries,
    pet: {
      ...base.pet,
      ...state.pet,
      needs: {
        ...base.pet.needs,
        ...state.pet?.needs,
      },
      condition: {
        ...base.pet.condition,
        ...state.pet?.condition,
      },
      evolution: {
        ...base.pet.evolution,
        ...state.pet?.evolution,
      },
    },
    personality: {
      ...base.personality,
      ...state.personality,
      traits: {
        ...base.personality.traits,
        ...state.personality?.traits,
      },
    },
    active_quest: state.active_quest ?? base.active_quest,
    opportunities: state.opportunities ?? base.opportunities,
  };
}

function normalizeBuddySnapshot(snapshot: BuddySnapshot): BuddySnapshot {
  return {
    ...snapshot,
    settings: {
      ...defaultBuddySettings(),
      ...snapshot.settings,
    },
    state: normalizeBuddyState(snapshot.state),
    recent_diagnostics: snapshot.recent_diagnostics ?? [],
    runtime_queue: snapshot.runtime_queue ?? [],
    now_playing: snapshot.now_playing ?? null,
    active_speech: snapshot.active_speech ?? null,
    pulse: snapshot.pulse ?? defaultBuddyPulse(),
    active_drafts: snapshot.active_drafts ?? [],
  };
}

export interface BuddySliceState {
  snapshot: BuddySnapshot | null;
  /** true once the first snapshot event has been received (even if buddy is disabled) */
  loaded: boolean;
  conversations: BuddyConversationEntry[];
  recentDiagnostics: DiagnosticContext[];
  runtimeQueue: BuddyRuntimeEvent[];
  nowPlaying: BuddyRuntimeEvent | null;
  activeSpeech: BuddySpeechItem | null;
  opportunities: BuddyOpportunity[];
  pulse: BuddyPulse | null;
  activeDrafts: BuddyDraft[];
}

const initialState: BuddySliceState = {
  snapshot: null,
  loaded: false,
  conversations: [],
  recentDiagnostics: [],
  runtimeQueue: [],
  nowPlaying: null,
  activeSpeech: null,
  opportunities: [],
  pulse: null,
  activeDrafts: [],
};
const selectUnreadOpportunitiesFromSlice = createSelector(
  [(state: BuddySliceState) => state.opportunities],
  (opportunities) => opportunities.filter((o) => o.status === "new"),
);

export const buddySlice = createSlice({
  name: "buddy",
  initialState,
  reducers: {
    setBuddySnapshot: (state, action: PayloadAction<BuddySnapshot>) => {
      const raw = action.payload;
      const snapshot = normalizeBuddySnapshot(raw);
      state.snapshot = snapshot;
      state.loaded = true;
      state.recentDiagnostics = snapshot.recent_diagnostics ?? [];
      state.activeSpeech = snapshot.active_speech ?? null;
      state.runtimeQueue = snapshot.runtime_queue ?? [];
      state.nowPlaying = snapshot.now_playing ?? null;
      state.opportunities = snapshot.state.opportunities;
      state.pulse = snapshot.pulse ?? defaultBuddyPulse();
      state.activeDrafts = snapshot.active_drafts ?? [];
    },
    /** Called when SSE snapshot reports buddy as disabled/not-ready (no state). */
    setBuddyUnavailable: (state) => {
      state.loaded = true;
      state.snapshot = null;
      state.recentDiagnostics = [];
      state.runtimeQueue = [];
      state.nowPlaying = null;
      state.activeSpeech = null;
    },
    updateBuddyState: (state, action: PayloadAction<BuddyState>) => {
      state.loaded = true;
      if (state.snapshot) {
        state.snapshot.state = normalizeBuddyState(action.payload);
      } else {
        // Buddy became active while we had no snapshot (was disabled/not-ready).
        // Bootstrap a minimal snapshot so the UI recovers without a full reconnect.
        state.snapshot = {
          state: normalizeBuddyState(action.payload),
          settings: defaultBuddySettings(),
          enabled: true,
          runtime_queue: state.runtimeQueue,
          now_playing: state.nowPlaying,
          active_speech: state.activeSpeech,
        };
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
        state.snapshot.settings = {
          ...defaultBuddySettings(),
          ...action.payload,
        };
        state.snapshot.enabled = action.payload.enabled;
      }
      // If snapshot is null but buddy is being re-enabled, wait for the next
      // StateUpdated event which will bootstrap the full snapshot via updateBuddyState.
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

    enqueueRuntimeEvent: (state, action: PayloadAction<BuddyRuntimeEvent>) => {
      const event = action.payload;
      if (event.dedupe_key) {
        const idx = state.runtimeQueue.findIndex(
          (e) => e.dedupe_key === event.dedupe_key,
        );
        if (idx >= 0) {
          // Sticky dismissal on coalesce — see runtime_queue.rs::enqueue.
          const wasDismissed =
            (state.runtimeQueue[idx].dismissed ?? false) ||
            (event.dismissed ?? false);
          state.runtimeQueue[idx] = { ...event, dismissed: wasDismissed };
          return;
        }
        if (state.nowPlaying?.dedupe_key === event.dedupe_key) {
          const wasDismissed =
            (state.nowPlaying.dismissed ?? false) || (event.dismissed ?? false);
          state.nowPlaying = { ...event, dismissed: wasDismissed };
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
      const next = state.runtimeQueue.shift();
      if (next !== undefined) {
        state.nowPlaying = next;
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
    /** Mark a runtime event as dismissed by id (optimistic; server confirms via SSE). */
    dismissRuntimeEvent: (state, action: PayloadAction<string>) => {
      const id = action.payload;
      const item = state.runtimeQueue.find((e) => e.id === id);
      if (item) item.dismissed = true;
      if (state.nowPlaying?.id === id) {
        state.nowPlaying = { ...state.nowPlaying, dismissed: true };
      }
    },
    addOpportunity: (state, action: PayloadAction<BuddyOpportunity>) => {
      const opp = action.payload;
      const idx = state.opportunities.findIndex((o) => o.id === opp.id);
      if (idx >= 0) {
        state.opportunities[idx] = opp;
        return;
      }
      state.opportunities.push(opp);
      if (state.opportunities.length > 200) {
        state.opportunities.shift();
      }
    },
    resolveOpportunity: (
      state,
      action: PayloadAction<{ id: string; status: OpportunityStatus }>,
    ) => {
      const { id, status } = action.payload;
      const opp = state.opportunities.find((o) => o.id === id);
      if (opp) {
        opp.status = status;
      }
    },
    expireOpportunities: (state, action: PayloadAction<string>) => {
      const now = action.payload;
      for (const opp of state.opportunities) {
        if (opp.status === "new" || opp.status === "shown") {
          if (opp.expires_at <= now) {
            opp.status = "expired";
          }
        }
      }
    },
    setPulse: (state, action: PayloadAction<BuddyPulse>) => {
      state.pulse = action.payload;
    },
    addDraft: (state, action: PayloadAction<BuddyDraft>) => {
      const draft = action.payload;
      const idx = state.activeDrafts.findIndex((d) => d.id === draft.id);
      if (idx >= 0) {
        state.activeDrafts[idx] = draft;
      } else {
        state.activeDrafts.push(draft);
      }
    },
    consumeDraft: (state, action: PayloadAction<string>) => {
      state.activeDrafts = state.activeDrafts.filter(
        (d) => d.id !== action.payload,
      );
    },
    replaceOpportunities: (
      state,
      action: PayloadAction<BuddyOpportunity[]>,
    ) => {
      state.opportunities = action.payload;
    },
  },
  selectors: {
    selectBuddySnapshot: (state) => state.snapshot,
    selectBuddyLoaded: (state) => state.loaded,
    selectBuddyState: (state) => state.snapshot?.state ?? null,
    selectBuddySettings: (state) => state.snapshot?.settings ?? null,
    selectBuddyActivities: (state) =>
      state.snapshot?.state.recent_activities ?? [],
    selectBuddySuggestions: (state) =>
      state.snapshot?.state.suggestion_state ?? [],
    selectBuddyConversations: (state) => state.conversations,
    selectIsBuddyEnabled: (state) => state.snapshot?.enabled ?? false,
    selectBuddyDiagnostics: (state) => state.recentDiagnostics,

    selectRuntimeQueue: (state) => state.runtimeQueue,
    selectNowPlaying: (state) => state.nowPlaying,
    selectActiveSpeech: (state) => state.activeSpeech,
    selectOpportunities: (state) => state.opportunities,
    selectUnreadOpportunities: selectUnreadOpportunitiesFromSlice,
    selectPulse: (state) => state.pulse,
    selectActiveDrafts: (state) => state.activeDrafts,
  },
});

export const {
  setBuddySnapshot,
  setBuddyUnavailable,
  updateBuddyState,
  addBuddyActivity,
  addBuddySuggestion,
  dismissBuddySuggestion,
  updateBuddySettings,
  setBuddyConversations,
  addBuddyDiagnostic,
  enqueueRuntimeEvent,
  dequeueRuntimeEvent,
  clearNowPlaying,
  updateRuntimeProgress,
  setActiveSpeech,
  clearActiveSpeech,
  dismissRuntimeEvent,
  addOpportunity,
  resolveOpportunity,
  expireOpportunities,
  setPulse,
  addDraft,
  consumeDraft,
  replaceOpportunities,
} = buddySlice.actions;

export const {
  selectBuddySnapshot,
  selectBuddyLoaded,
  selectBuddyState,
  selectBuddySettings,
  selectBuddyActivities,
  selectBuddySuggestions,
  selectBuddyConversations,
  selectIsBuddyEnabled,
  selectBuddyDiagnostics,
  selectRuntimeQueue,
  selectNowPlaying,
  selectActiveSpeech,
  selectOpportunities,
  selectUnreadOpportunities,
  selectPulse,
  selectActiveDrafts,
} = buddySlice.selectors;

export const selectOpportunityById = (
  state: { buddy: BuddySliceState },
  id: string,
) => state.buddy.opportunities.find((o) => o.id === id);

export const selectDraftById = (
  state: { buddy: BuddySliceState },
  id: string,
) => state.buddy.activeDrafts.find((d) => d.id === id);
