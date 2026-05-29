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
  BuddyStorageMetadata,
} from "./types";

const HOME_NOTIFICATION_SNOOZE_MS = 10 * 60 * 1000;
const BUDDY_SEEN_STORAGE_KEY = "refact.buddy.seenNotifications.v1";
const BUDDY_CHAT_BUBBLE_POLICY_STORAGE_KEY = "refact.buddy.chatBubblePolicy.v1";
const CHAT_BUBBLE_COOLDOWN_MIN_MS = 5 * 60 * 1000;
const CHAT_BUBBLE_COOLDOWN_MAX_MS = 10 * 60 * 1000;
const CHAT_BUBBLE_IMPRESSION_LIMIT = 20;

export type BuddyChatBubbleClass = "ambient" | "actionable" | "event_once";

export interface BuddyChatBubbleImpression {
  id: string;
  kind: BuddyChatBubbleClass;
  shown_at: number;
}

interface BuddyChatBubblePolicyStorage {
  snoozedUntil: number | null;
  impressions: BuddyChatBubbleImpression[];
}

function nowMs(): number {
  return Date.now();
}

function loadSeenNotificationIds(): Record<string, number> {
  if (typeof localStorage === "undefined") return {};
  try {
    const raw = localStorage.getItem(BUDDY_SEEN_STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as unknown;
    if (
      typeof parsed !== "object" ||
      parsed === null ||
      Array.isArray(parsed)
    ) {
      return {};
    }
    const result: Record<string, number> = {};
    for (const [id, value] of Object.entries(parsed)) {
      if (typeof value === "number" && Number.isFinite(value)) {
        result[id] = value;
      }
    }
    return result;
  } catch {
    return {};
  }
}

function persistSeenNotificationIds(seen: Record<string, number>): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(BUDDY_SEEN_STORAGE_KEY, JSON.stringify(seen));
  } catch {
    return;
  }
}

function pruneSeenNotificationIds(
  seen: Record<string, number>,
): Record<string, number> {
  const cutoff = nowMs() - 24 * 60 * 60 * 1000;
  return Object.fromEntries(
    Object.entries(seen)
      .filter(([, value]) => value >= cutoff)
      .sort((left, right) => right[1] - left[1])
      .slice(0, 200),
  );
}

function isBuddyChatBubbleClass(value: unknown): value is BuddyChatBubbleClass {
  return (
    value === "ambient" || value === "actionable" || value === "event_once"
  );
}

function loadChatBubblePolicy(): BuddyChatBubblePolicyStorage {
  if (typeof localStorage === "undefined") {
    return { snoozedUntil: null, impressions: [] };
  }
  try {
    const raw = localStorage.getItem(BUDDY_CHAT_BUBBLE_POLICY_STORAGE_KEY);
    if (!raw) return { snoozedUntil: null, impressions: [] };
    const parsed = JSON.parse(raw) as unknown;
    if (
      typeof parsed !== "object" ||
      parsed === null ||
      Array.isArray(parsed)
    ) {
      return { snoozedUntil: null, impressions: [] };
    }
    const candidate = parsed as {
      snoozedUntil?: unknown;
      impressions?: unknown;
    };
    const snoozedUntil =
      typeof candidate.snoozedUntil === "number" &&
      Number.isFinite(candidate.snoozedUntil) &&
      candidate.snoozedUntil > nowMs()
        ? candidate.snoozedUntil
        : null;
    const impressions = Array.isArray(candidate.impressions)
      ? candidate.impressions.flatMap((entry): BuddyChatBubbleImpression[] => {
          if (typeof entry !== "object" || entry === null) return [];
          const impression = entry as {
            id?: unknown;
            kind?: unknown;
            shown_at?: unknown;
          };
          if (
            typeof impression.id !== "string" ||
            !isBuddyChatBubbleClass(impression.kind) ||
            typeof impression.shown_at !== "number" ||
            !Number.isFinite(impression.shown_at)
          ) {
            return [];
          }
          return [
            {
              id: impression.id,
              kind: impression.kind,
              shown_at: impression.shown_at,
            },
          ];
        })
      : [];
    return {
      snoozedUntil,
      impressions: pruneChatBubbleImpressions(impressions),
    };
  } catch {
    return { snoozedUntil: null, impressions: [] };
  }
}

function persistChatBubblePolicy(
  snoozedUntil: number | null,
  impressions: BuddyChatBubbleImpression[],
): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(
      BUDDY_CHAT_BUBBLE_POLICY_STORAGE_KEY,
      JSON.stringify({
        snoozedUntil,
        impressions: pruneChatBubbleImpressions(impressions),
      }),
    );
  } catch {
    return;
  }
}

function pruneChatBubbleImpressions(
  impressions: BuddyChatBubbleImpression[],
): BuddyChatBubbleImpression[] {
  return [...impressions]
    .sort((left, right) => right.shown_at - left.shown_at)
    .slice(0, CHAT_BUBBLE_IMPRESSION_LIMIT);
}

function chatBubbleCooldownDurationMs(overrideMs: number | undefined): number {
  if (
    overrideMs !== undefined &&
    Number.isFinite(overrideMs) &&
    overrideMs > 0
  ) {
    return overrideMs;
  }
  const spanMs = CHAT_BUBBLE_COOLDOWN_MAX_MS - CHAT_BUBBLE_COOLDOWN_MIN_MS;
  return CHAT_BUBBLE_COOLDOWN_MIN_MS + Math.floor(Math.random() * spanMs);
}

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

export function defaultBuddySettings(): BuddySettings {
  return {
    enabled: true,
    auto_diagnostics: true,
    auto_issue_creation: false,
    personality_prompt: null,
    autonomous_chats_enabled: true,
    proactive_enabled: true,
    message_observation_enabled: true,
    chat_reactions_enabled: true,
    housekeeping_enabled: true,
    humor_enabled: true,
    humor_level: "light",
    autonomy_level: "suggest",
    quiet_mode: false,
    daily_digest_hour: 18,
    observers: {
      task_health: true,
      trajectory_clutter: true,
      chat_pattern: true,
      customization_drift: true,
      memory_garden: true,
      mcp_auth: true,
      git_pressure: true,
      diagnostic_cluster: true,
      provider_health: true,
    },
  };
}

export function normalizeBuddySettings(
  settings?: Partial<BuddySettings>,
): BuddySettings {
  const defaults = defaultBuddySettings();
  if (!settings) return defaults;
  return {
    ...defaults,
    ...settings,
    observers: {
      ...defaults.observers,
      ...settings.observers,
    },
  };
}

export type BuddySettingsPatch = Partial<BuddySettings> & {
  clear_personality_prompt?: boolean;
};

export type BuddySettingsPatchKey = keyof BuddySettings;

export type BuddySettingsResponse = BuddySettings & {
  storage?: BuddyStorageMetadata;
};

interface PendingBuddySettingsRequest {
  requestSeq: number;
  keys: BuddySettingsPatchKey[];
  patch: BuddySettingsPatch;
}

function applyBuddySettingsPatch(
  current: BuddySettings,
  patch: BuddySettingsPatch,
): BuddySettings {
  const { clear_personality_prompt, observers, ...settingsPatch } = patch;
  const next: Partial<BuddySettings> = {
    ...current,
    ...settingsPatch,
  };
  if (clear_personality_prompt) {
    next.personality_prompt = null;
  }
  if (observers) {
    next.observers = {
      ...current.observers,
      ...observers,
    };
  }
  return normalizeBuddySettings(next);
}

function applyBuddySettingsPatchToSnapshot(
  snapshot: BuddySnapshot,
  patch: BuddySettingsPatch,
): void {
  const normalized = applyBuddySettingsPatch(snapshot.settings, patch);
  snapshot.settings = normalized;
  snapshot.enabled = normalized.enabled;
}

function getBuddySettingsPatchFromSettings(
  settings: BuddySettings,
  keys: BuddySettingsPatchKey[],
): BuddySettingsPatch {
  const patch: BuddySettingsPatch = {};
  if (keys.includes("enabled")) patch.enabled = settings.enabled;
  if (keys.includes("auto_diagnostics")) {
    patch.auto_diagnostics = settings.auto_diagnostics;
  }
  if (keys.includes("auto_issue_creation")) {
    patch.auto_issue_creation = settings.auto_issue_creation;
  }
  if (keys.includes("personality_prompt")) {
    patch.personality_prompt = settings.personality_prompt;
  }
  if (keys.includes("autonomous_chats_enabled")) {
    patch.autonomous_chats_enabled = settings.autonomous_chats_enabled;
  }
  if (keys.includes("proactive_enabled")) {
    patch.proactive_enabled = settings.proactive_enabled;
  }
  if (keys.includes("message_observation_enabled")) {
    patch.message_observation_enabled = settings.message_observation_enabled;
  }
  if (keys.includes("chat_reactions_enabled")) {
    patch.chat_reactions_enabled = settings.chat_reactions_enabled;
  }
  if (keys.includes("housekeeping_enabled")) {
    patch.housekeeping_enabled = settings.housekeeping_enabled;
  }
  if (keys.includes("humor_enabled")) {
    patch.humor_enabled = settings.humor_enabled;
  }
  if (keys.includes("humor_level")) patch.humor_level = settings.humor_level;
  if (keys.includes("autonomy_level")) {
    patch.autonomy_level = settings.autonomy_level;
  }
  if (keys.includes("quiet_mode")) patch.quiet_mode = settings.quiet_mode;
  if (keys.includes("daily_digest_hour")) {
    patch.daily_digest_hour = settings.daily_digest_hour;
  }
  if (keys.includes("observers")) patch.observers = { ...settings.observers };
  return patch;
}

function doBuddySettingsKeysIntersect(
  left: BuddySettingsPatchKey[],
  right: BuddySettingsPatchKey[],
): boolean {
  return left.some((key) => right.includes(key));
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
    worktrees: {
      total_registered: 0,
      total_discovered: 0,
      total: 0,
      clean: 0,
      dirty: 0,
      unknown: 0,
      stale: 0,
      conflicted: 0,
      shared: 0,
      abandoned_clean: 0,
      changed_files: 0,
      additions: 0,
      deletions: 0,
      missing_registry_paths: 0,
      unregistered_cache_dirs: 0,
      merged_branches: 0,
    },
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
  const normalizedState = normalizeBuddyState(snapshot.state);
  const opportunities = snapshot.opportunities ?? normalizedState.opportunities;
  normalizedState.opportunities = opportunities;
  const normalizedSettings = normalizeBuddySettings(snapshot.settings);
  const enabled = snapshot.enabled !== false && normalizedSettings.enabled;
  if (!enabled) normalizedSettings.enabled = false;
  return {
    ...snapshot,
    enabled,
    settings: normalizedSettings,
    state: normalizedState,
    recent_diagnostics: snapshot.recent_diagnostics ?? [],
    runtime_queue: snapshot.runtime_queue ?? [],
    now_playing: snapshot.now_playing ?? null,
    active_speech: snapshot.active_speech ?? null,
    pulse: snapshot.pulse ?? defaultBuddyPulse(),
    opportunities,
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
  homeSnoozedUntil: number | null;
  seenNotificationIds: Record<string, number>;
  chatBubbleSnoozedUntil: number | null;
  chatBubbleImpressions: BuddyChatBubbleImpression[];
  pendingSettingsRequests: PendingBuddySettingsRequest[];
}

const EMPTY_BUDDY_ACTIVITIES: BuddyActivityEntry[] = [];
const EMPTY_BUDDY_SUGGESTIONS: BuddySuggestion[] = [];

function initialState(): BuddySliceState {
  const initialChatBubblePolicy = loadChatBubblePolicy();
  return {
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
    homeSnoozedUntil: null,
    seenNotificationIds: pruneSeenNotificationIds(loadSeenNotificationIds()),
    chatBubbleSnoozedUntil: initialChatBubblePolicy.snoozedUntil,
    chatBubbleImpressions: initialChatBubblePolicy.impressions,
    pendingSettingsRequests: [],
  };
}

function applyPendingSettingsRequests(state: BuddySliceState): void {
  if (!state.snapshot) return;
  for (const request of state.pendingSettingsRequests) {
    applyBuddySettingsPatchToSnapshot(state.snapshot, request.patch);
  }
}

function setSnapshotSettingsPreservingDisabled(
  state: BuddySliceState,
  settings: BuddySettings,
): void {
  if (!state.snapshot) return;
  const normalized = normalizeBuddySettings(settings);
  state.snapshot.settings = normalized;
  state.snapshot.enabled = normalized.enabled;
}
function syncSnapshotRuntime(state: BuddySliceState) {
  if (state.snapshot) {
    state.snapshot.runtime_queue = state.runtimeQueue;
    state.snapshot.now_playing = state.nowPlaying;
  }
}

function syncSnapshotDiagnostics(state: BuddySliceState) {
  if (state.snapshot) {
    state.snapshot.recent_diagnostics = state.recentDiagnostics;
  }
}

function syncSnapshotSpeech(state: BuddySliceState) {
  if (state.snapshot) {
    state.snapshot.active_speech = state.activeSpeech;
  }
}

function syncSnapshotOpportunities(state: BuddySliceState) {
  if (state.snapshot) {
    state.snapshot.state.opportunities = state.opportunities;
    state.snapshot.opportunities = state.opportunities;
  }
}

function syncSnapshotPulse(state: BuddySliceState) {
  if (state.snapshot) {
    state.snapshot.pulse = state.pulse ?? defaultBuddyPulse();
  }
}

function syncSnapshotDrafts(state: BuddySliceState) {
  if (state.snapshot) {
    state.snapshot.active_drafts = state.activeDrafts;
  }
}

const selectUnreadOpportunitiesFromSlice = createSelector(
  [(state: BuddySliceState) => state.opportunities],
  (opportunities) =>
    opportunities.filter((o) => o.status === "new" || o.status === "shown"),
);

export const buddySlice = createSlice({
  name: "buddy",
  initialState,
  reducers: {
    setBuddySnapshot: (state, action: PayloadAction<BuddySnapshot>) => {
      const raw = action.payload;
      const snapshot = normalizeBuddySnapshot(raw);
      state.snapshot = snapshot;
      applyPendingSettingsRequests(state);
      state.loaded = true;
      state.recentDiagnostics = state.snapshot?.recent_diagnostics ?? [];
      state.activeSpeech = state.snapshot?.active_speech ?? null;
      state.runtimeQueue = state.snapshot?.runtime_queue ?? [];
      state.nowPlaying = state.snapshot?.now_playing ?? null;
      state.opportunities = state.snapshot?.state.opportunities ?? [];
      state.pulse = state.snapshot?.pulse ?? defaultBuddyPulse();
      state.activeDrafts = state.snapshot?.active_drafts ?? [];
    },
    /** Called when SSE snapshot reports buddy as disabled/not-ready (no state). */
    setBuddyUnavailable: (state) => {
      state.loaded = true;
      state.snapshot = null;
      state.recentDiagnostics = [];
      state.runtimeQueue = [];
      state.nowPlaying = null;
      state.activeSpeech = null;
      state.opportunities = [];
      state.pulse = null;
      state.activeDrafts = [];
    },
    updateBuddyState: (state, action: PayloadAction<BuddyState>) => {
      state.loaded = true;
      const buddyState = normalizeBuddyState(action.payload);
      state.opportunities = buddyState.opportunities;
      if (state.snapshot) {
        state.snapshot.state = buddyState;
      } else {
        // Buddy became active while we had no snapshot (was disabled/not-ready).
        // Bootstrap a minimal snapshot so the UI recovers without a full reconnect.
        state.snapshot = {
          state: buddyState,
          settings: defaultBuddySettings(),
          enabled: true,
          recent_diagnostics: state.recentDiagnostics,
          runtime_queue: state.runtimeQueue,
          now_playing: state.nowPlaying,
          active_speech: state.activeSpeech,
          pulse: state.pulse ?? defaultBuddyPulse(),
          opportunities: state.opportunities,
          active_drafts: state.activeDrafts,
        };
      }
      syncSnapshotOpportunities(state);
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
        setSnapshotSettingsPreservingDisabled(state, action.payload);
        const storage = (action.payload as BuddySettingsResponse).storage;
        if (storage) state.snapshot.storage = storage;
        applyPendingSettingsRequests(state);
      }
      // If snapshot is null but buddy is being re-enabled, wait for the next
      // StateUpdated event which will bootstrap the full snapshot via updateBuddyState.
    },
    patchBuddySettings: (state, action: PayloadAction<BuddySettingsPatch>) => {
      if (state.snapshot) {
        applyBuddySettingsPatchToSnapshot(state.snapshot, action.payload);
      }
    },
    beginBuddySettingsRequest: (
      state,
      action: PayloadAction<{
        requestSeq: number;
        keys: BuddySettingsPatchKey[];
        patch: BuddySettingsPatch;
      }>,
    ) => {
      state.pendingSettingsRequests = [
        ...state.pendingSettingsRequests.filter(
          (request) =>
            !doBuddySettingsKeysIntersect(request.keys, action.payload.keys),
        ),
        action.payload,
      ].sort((left, right) => left.requestSeq - right.requestSeq);
      if (state.snapshot) {
        applyBuddySettingsPatchToSnapshot(state.snapshot, action.payload.patch);
      }
    },
    finishBuddySettingsRequest: (
      state,
      action: PayloadAction<{
        requestSeq: number;
        settings?: BuddySettingsResponse;
      }>,
    ) => {
      const request = state.pendingSettingsRequests.find(
        (entry) => entry.requestSeq === action.payload.requestSeq,
      );
      state.pendingSettingsRequests = state.pendingSettingsRequests.filter(
        (entry) => entry.requestSeq !== action.payload.requestSeq,
      );
      if (!state.snapshot || !request) return;
      if (action.payload.settings) {
        const settingsPatch = getBuddySettingsPatchFromSettings(
          action.payload.settings,
          request.keys,
        );
        applyBuddySettingsPatchToSnapshot(state.snapshot, settingsPatch);
        if (action.payload.settings.storage) {
          state.snapshot.storage = action.payload.settings.storage;
        }
      }
      applyPendingSettingsRequests(state);
    },
    failBuddySettingsRequest: (
      state,
      action: PayloadAction<{
        requestSeq: number;
        rollbackPatch: BuddySettingsPatch | null;
      }>,
    ) => {
      const request = state.pendingSettingsRequests.find(
        (entry) => entry.requestSeq === action.payload.requestSeq,
      );
      state.pendingSettingsRequests = state.pendingSettingsRequests.filter(
        (entry) => entry.requestSeq !== action.payload.requestSeq,
      );
      if (!state.snapshot || !request || !action.payload.rollbackPatch) return;
      const superseded = state.pendingSettingsRequests.some((entry) =>
        doBuddySettingsKeysIntersect(entry.keys, request.keys),
      );
      if (superseded) return;
      applyBuddySettingsPatchToSnapshot(
        state.snapshot,
        action.payload.rollbackPatch,
      );
      applyPendingSettingsRequests(state);
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
      syncSnapshotDiagnostics(state);
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
          syncSnapshotRuntime(state);
          return;
        }
        if (state.nowPlaying?.dedupe_key === event.dedupe_key) {
          const wasDismissed =
            (state.nowPlaying.dismissed ?? false) || (event.dismissed ?? false);
          state.nowPlaying = { ...event, dismissed: wasDismissed };
          syncSnapshotRuntime(state);
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
      syncSnapshotRuntime(state);
    },
    dequeueRuntimeEvent: (state) => {
      const next = state.runtimeQueue.shift();
      if (next !== undefined) {
        state.nowPlaying = next;
      }
      syncSnapshotRuntime(state);
    },
    clearNowPlaying: (state) => {
      state.nowPlaying = null;
      syncSnapshotRuntime(state);
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
      syncSnapshotRuntime(state);
    },
    setActiveSpeech: (state, action: PayloadAction<BuddySpeechItem>) => {
      state.activeSpeech = action.payload;
      syncSnapshotSpeech(state);
    },
    clearActiveSpeech: (state) => {
      state.activeSpeech = null;
      syncSnapshotSpeech(state);
    },
    /** Mark a runtime event as dismissed by id (optimistic; server confirms via SSE). */
    dismissRuntimeEvent: (state, action: PayloadAction<string>) => {
      const id = action.payload;
      const item = state.runtimeQueue.find((e) => e.id === id);
      if (item) item.dismissed = true;
      if (state.nowPlaying?.id === id) {
        state.nowPlaying = { ...state.nowPlaying, dismissed: true };
      }
      syncSnapshotRuntime(state);
    },
    addOpportunity: (state, action: PayloadAction<BuddyOpportunity>) => {
      const opp = action.payload;
      const idx = state.opportunities.findIndex((o) => o.id === opp.id);
      if (idx >= 0) {
        state.opportunities[idx] = opp;
        syncSnapshotOpportunities(state);
        return;
      }
      state.opportunities.push(opp);
      if (state.opportunities.length > 200) {
        state.opportunities.shift();
      }
      syncSnapshotOpportunities(state);
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
      syncSnapshotOpportunities(state);
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
      syncSnapshotOpportunities(state);
    },
    setPulse: (state, action: PayloadAction<BuddyPulse>) => {
      state.pulse = action.payload;
      syncSnapshotPulse(state);
    },
    addDraft: (state, action: PayloadAction<BuddyDraft>) => {
      const draft = action.payload;
      const idx = state.activeDrafts.findIndex((d) => d.id === draft.id);
      if (idx >= 0) {
        state.activeDrafts[idx] = draft;
      } else {
        state.activeDrafts.push(draft);
      }
      syncSnapshotDrafts(state);
    },
    consumeDraft: (state, action: PayloadAction<string>) => {
      state.activeDrafts = state.activeDrafts.filter(
        (d) => d.id !== action.payload,
      );
      syncSnapshotDrafts(state);
    },
    removeDraft: (state, action: PayloadAction<string>) => {
      state.activeDrafts = state.activeDrafts.filter(
        (d) => d.id !== action.payload,
      );
      syncSnapshotDrafts(state);
    },
    snoozeHomeNotifications: (
      state,
      action: PayloadAction<number | undefined>,
    ) => {
      const durationMs = action.payload ?? HOME_NOTIFICATION_SNOOZE_MS;
      state.homeSnoozedUntil = nowMs() + durationMs;
    },
    markBuddyNotificationSeen: (state, action: PayloadAction<string>) => {
      state.seenNotificationIds = pruneSeenNotificationIds({
        ...state.seenNotificationIds,
        [action.payload]: nowMs(),
      });
      persistSeenNotificationIds(state.seenNotificationIds);
    },
    recordChatBubbleImpression: (
      state,
      action: PayloadAction<{ id: string; kind: BuddyChatBubbleClass }>,
    ) => {
      if (
        state.chatBubbleImpressions.some(
          (impression) => impression.id === action.payload.id,
        )
      ) {
        return;
      }
      state.chatBubbleImpressions = pruneChatBubbleImpressions([
        {
          id: action.payload.id,
          kind: action.payload.kind,
          shown_at: nowMs(),
        },
        ...state.chatBubbleImpressions.filter(
          (impression) => impression.id !== action.payload.id,
        ),
      ]);
      persistChatBubblePolicy(
        state.chatBubbleSnoozedUntil,
        state.chatBubbleImpressions,
      );
    },
    snoozeChatBubbles: (state, action: PayloadAction<number | undefined>) => {
      state.chatBubbleSnoozedUntil =
        nowMs() + chatBubbleCooldownDurationMs(action.payload);
      persistChatBubblePolicy(
        state.chatBubbleSnoozedUntil,
        state.chatBubbleImpressions,
      );
    },
    clearExpiredChatBubbleSnooze: (state) => {
      if (
        state.chatBubbleSnoozedUntil != null &&
        state.chatBubbleSnoozedUntil <= nowMs()
      ) {
        state.chatBubbleSnoozedUntil = null;
        persistChatBubblePolicy(null, state.chatBubbleImpressions);
      }
    },
    clearExpiredBuddyNotificationSnooze: (state) => {
      if (state.homeSnoozedUntil != null && state.homeSnoozedUntil <= nowMs()) {
        state.homeSnoozedUntil = null;
      }
    },
    replaceOpportunities: (
      state,
      action: PayloadAction<BuddyOpportunity[]>,
    ) => {
      state.opportunities = action.payload;
      syncSnapshotOpportunities(state);
    },
  },
  selectors: {
    selectBuddySnapshot: (state) => state.snapshot,
    selectBuddyLoaded: (state) => state.loaded,
    selectBuddyState: (state) => state.snapshot?.state ?? null,
    selectBuddySettings: (state) => state.snapshot?.settings ?? null,
    selectBuddyStorage: (state) => state.snapshot?.storage ?? null,
    selectBuddyActivities: (state) =>
      state.snapshot?.state.recent_activities ?? EMPTY_BUDDY_ACTIVITIES,
    selectBuddySuggestions: (state) =>
      state.snapshot?.state.suggestion_state ?? EMPTY_BUDDY_SUGGESTIONS,
    selectBuddyConversations: (state) => state.conversations,
    selectIsBuddySnapshotAvailable: (state) => state.snapshot !== null,
    selectIsBuddyUserEnabled: (state) =>
      state.snapshot?.settings.enabled === true,
    selectIsBuddyInteractiveEnabled: (state) =>
      state.snapshot?.enabled === true && state.snapshot.settings.enabled,
    selectIsBuddyEnabled: (state) =>
      state.snapshot?.enabled === true && state.snapshot.settings.enabled,
    selectBuddyDiagnostics: (state) => state.recentDiagnostics,

    selectRuntimeQueue: (state) => state.runtimeQueue,
    selectNowPlaying: (state) => state.nowPlaying,
    selectActiveSpeech: (state) => state.activeSpeech,
    selectOpportunities: (state) => state.opportunities,
    selectUnreadOpportunities: selectUnreadOpportunitiesFromSlice,
    selectPulse: (state) => state.pulse,
    selectActiveDrafts: (state) => state.activeDrafts,
    selectHomeSnoozedUntil: (state) => state.homeSnoozedUntil,
    selectSeenNotificationIds: (state) => state.seenNotificationIds,
    selectChatBubbleSnoozedUntil: (state) => state.chatBubbleSnoozedUntil,
    selectChatBubbleImpressions: (state) => state.chatBubbleImpressions,
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
  patchBuddySettings,
  beginBuddySettingsRequest,
  finishBuddySettingsRequest,
  failBuddySettingsRequest,
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
  removeDraft,
  snoozeHomeNotifications,
  markBuddyNotificationSeen,
  recordChatBubbleImpression,
  snoozeChatBubbles,
  clearExpiredChatBubbleSnooze,
  clearExpiredBuddyNotificationSnooze,
  replaceOpportunities,
} = buddySlice.actions;

export const {
  selectBuddySnapshot,
  selectBuddyLoaded,
  selectBuddyState,
  selectBuddySettings,
  selectBuddyStorage,
  selectBuddyActivities,
  selectBuddySuggestions,
  selectBuddyConversations,
  selectIsBuddySnapshotAvailable,
  selectIsBuddyUserEnabled,
  selectIsBuddyInteractiveEnabled,
  selectIsBuddyEnabled,
  selectBuddyDiagnostics,
  selectRuntimeQueue,
  selectNowPlaying,
  selectActiveSpeech,
  selectOpportunities,
  selectUnreadOpportunities,
  selectPulse,
  selectActiveDrafts,
  selectHomeSnoozedUntil,
  selectSeenNotificationIds,
  selectChatBubbleSnoozedUntil,
  selectChatBubbleImpressions,
} = buddySlice.selectors;

export const selectOpportunityById = (
  state: { buddy: BuddySliceState },
  id: string,
) => state.buddy.opportunities.find((o) => o.id === id);

export const selectDraftById = (
  state: { buddy: BuddySliceState },
  id: string,
) => state.buddy.activeDrafts.find((d) => d.id === id);

export const selectChatReactionDebug = (state: { buddy: BuddySliceState }) =>
  state.buddy.snapshot?.chat_reaction_debug ?? null;
