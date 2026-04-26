import { describe, test, expect } from "vitest";
import {
  buddySlice,
  setBuddySnapshot,
  updateBuddyState,
  addBuddyActivity,
  addBuddySuggestion,
  dismissBuddySuggestion,
  addBuddyDiagnostic,
  setBuddyConversations,
  selectBuddySnapshot,
  selectIsBuddyEnabled,
} from "../features/Buddy/buddySlice";
import { PALETTES, STAGES } from "../features/Buddy/constants";
import { buildColorMap } from "../features/Buddy/canvas/colorMap";
import type {
  BuddySnapshot,
  BuddyState,
  BuddyActivityEntry,
  BuddySuggestion,
  BuddyConversationMeta,
  DiagnosticContext,
} from "../features/Buddy/types";

const reducer = buddySlice.reducer;

function makeState(): BuddyState {
  return {
    identity: { name: "Pixel", created_at: "2024-01-01T00:00:00Z", palette_index: 0 },
    progression: { stage: 0, stage_name: "Egg", level: 1, xp: 0, xp_next: 30 },
    skills: { unlocked: [], locked: [] },
    workflow_summaries: [],
    semantic: { mood: "idle", focus: "none", headline: "", last_active: "2024-01-01T00:00:00Z" },
    recent_activities: [],
    suggestion_state: [],
  };
}

function makeSnapshot(overrides?: Partial<BuddySnapshot>): BuddySnapshot {
  return {
    state: makeState(),
    settings: { enabled: true, auto_diagnostics: true, auto_issue_creation: false, personality_prompt: null },
    enabled: true,
    ...overrides,
  };
}

function makeActivity(overrides?: Partial<BuddyActivityEntry>): BuddyActivityEntry {
  return {
    icon: "🔧",
    title: "Test Activity",
    description: "desc",
    timestamp: "2024-01-01T00:00:00Z",
    activity_type: "workflow",
    ...overrides,
  };
}

function makeSuggestion(id: string): BuddySuggestion {
  return {
    id,
    suggestion_type: "setup",
    title: "Setup needed",
    description: "desc",
    created_at: "2024-01-01T00:00:00Z",
    dismissed: false,
  };
}

function makeDiagnostic(overrides?: Partial<DiagnosticContext>): DiagnosticContext {
  return {
    error_type: "model_not_found",
    error_message: "Model not found",
    source_file: null,
    tool_name: null,
    chat_id: null,
    collected_at: "2024-01-01T00:00:00Z",
    severity: "high",
    ...overrides,
  };
}

describe("buddySlice reducers", () => {
  test("setBuddySnapshot replaces snapshot state", () => {
    const snap = makeSnapshot();
    const state = reducer(undefined, setBuddySnapshot(snap));
    expect(state.snapshot).toEqual(snap);
    expect(state.loading).toBe(false);
  });

  test("updateBuddyState patches existing state", () => {
    const snap = makeSnapshot();
    const initial = reducer(undefined, setBuddySnapshot(snap));
    const updated = { ...makeState(), semantic: { mood: "happy", focus: "work", headline: "Working!", last_active: "2024-06-01T00:00:00Z" } };
    const next = reducer(initial, updateBuddyState(updated));
    expect(next.snapshot?.state.semantic.headline).toBe("Working!");
  });

  test("updateBuddyState does nothing without snapshot", () => {
    const state = reducer(undefined, updateBuddyState(makeState()));
    expect(state.snapshot).toBeNull();
  });

  test("addBuddyActivity prepends to activities list", () => {
    const snap = makeSnapshot();
    const initial = reducer(undefined, setBuddySnapshot(snap));
    const activity = makeActivity({ title: "First" });
    const state1 = reducer(initial, addBuddyActivity(activity));
    const activity2 = makeActivity({ title: "Second" });
    const state2 = reducer(state1, addBuddyActivity(activity2));
    expect(state2.snapshot?.state.recent_activities[0].title).toBe("Second");
    expect(state2.snapshot?.state.recent_activities[1].title).toBe("First");
  });

  test("addBuddySuggestion appends suggestion", () => {
    const snap = makeSnapshot();
    const initial = reducer(undefined, setBuddySnapshot(snap));
    const suggestion = makeSuggestion("s-1");
    const state = reducer(initial, addBuddySuggestion(suggestion));
    expect(state.snapshot?.state.suggestion_state).toHaveLength(1);
    expect(state.snapshot?.state.suggestion_state[0].id).toBe("s-1");
  });

  test("dismissBuddySuggestion marks as dismissed", () => {
    const snap = makeSnapshot();
    snap.state.suggestion_state = [makeSuggestion("s-1"), makeSuggestion("s-2")];
    const initial = reducer(undefined, setBuddySnapshot(snap));
    const state = reducer(initial, dismissBuddySuggestion("s-1"));
    const s1 = state.snapshot?.state.suggestion_state.find((s) => s.id === "s-1");
    const s2 = state.snapshot?.state.suggestion_state.find((s) => s.id === "s-2");
    expect(s1?.dismissed).toBe(true);
    expect(s2?.dismissed).toBe(false);
  });

  test("addBuddyDiagnostic stores diagnostic", () => {
    const diag = makeDiagnostic();
    const state = reducer(undefined, addBuddyDiagnostic(diag));
    expect(state.recentDiagnostics).toHaveLength(1);
    expect(state.recentDiagnostics[0].error_type).toBe("model_not_found");
  });

  test("addBuddyDiagnostic caps at 100 entries", () => {
    let state = reducer(undefined, { type: "@@INIT" });
    for (let i = 0; i < 105; i++) {
      state = reducer(state, addBuddyDiagnostic(makeDiagnostic({ error_message: `err-${i}` })));
    }
    expect(state.recentDiagnostics).toHaveLength(100);
  });
});

describe("palette fallback", () => {
  test("invalid palette index falls back to index 0", () => {
    const map = buildColorMap(999);
    const expected = buildColorMap(0);
    expect(map.body).toBe(expected.body);
    expect(map.light).toBe(expected.light);
  });

  test("negative palette index falls back to index 0", () => {
    const map = buildColorMap(-1);
    const expected = buildColorMap(0);
    expect(map.body).toBe(expected.body);
  });

  test("valid palette index 0 returns Ocean colors", () => {
    const map = buildColorMap(0);
    expect(map.body).toBe(PALETTES[0].body);
  });
});

describe("stage fallback", () => {
  test("invalid stage falls back to Egg", () => {
    const stage = STAGES[999] ?? STAGES[0];
    expect(stage.name).toBe("Egg");
  });

  test("negative stage falls back to Egg", () => {
    const stage = STAGES[-1] ?? STAGES[0];
    expect(stage.name).toBe("Egg");
  });

  test("valid stage 0 is Egg", () => {
    expect(STAGES[0].name).toBe("Egg");
  });

  test("valid stage 1 is Hatch", () => {
    expect(STAGES[1].name).toBe("Hatch");
  });
});

describe("recent chats", () => {
  function makeConversation(id: string, lastMessageAt: string | null): BuddyConversationMeta {
    return {
      chat_id: id,
      title: `Chat ${id}`,
      created_at: "2024-01-01T00:00:00Z",
      last_message_at: lastMessageAt,
      message_count: 1,
    };
  }

  test("recent chats render newest first", () => {
    const conversations: BuddyConversationMeta[] = [
      makeConversation("c-3", "2024-03-01T00:00:00Z"),
      makeConversation("c-2", "2024-02-01T00:00:00Z"),
      makeConversation("c-1", "2024-01-01T00:00:00Z"),
    ];
    const state = reducer(undefined, setBuddyConversations(conversations));
    expect(state.conversations[0].chat_id).toBe("c-3");
    expect(state.conversations[1].chat_id).toBe("c-2");
    expect(state.conversations[2].chat_id).toBe("c-1");
  });

  test("empty conversations list renders gracefully", () => {
    const state = reducer(undefined, setBuddyConversations([]));
    expect(state.conversations).toEqual([]);
    expect(state.conversations).toHaveLength(0);
  });
});

describe("loading state and identity hydration", () => {
  test("initial state has snapshot null — loading placeholder", () => {
    const state = reducer(undefined, { type: "@@INIT" });
    expect(selectBuddySnapshot.call(null, state)).toBeNull();
    expect(selectIsBuddyEnabled.call(null, state)).toBe(false);
  });

  test("snapshot arrival sets correct identity", () => {
    const snap = makeSnapshot();
    snap.state.identity.name = "Byte";
    snap.state.identity.palette_index = 3;
    const state = reducer(undefined, setBuddySnapshot(snap));
    const loaded = selectBuddySnapshot.call(null, state);
    expect(loaded).not.toBeNull();
    expect(loaded?.state.identity.name).toBe("Byte");
    expect(loaded?.state.identity.palette_index).toBe(3);
  });

  test("palette comes from state.identity not settings", () => {
    const snap = makeSnapshot();
    snap.state.identity.palette_index = 5;
    const state = reducer(undefined, setBuddySnapshot(snap));
    const loaded = selectBuddySnapshot.call(null, state);
    expect(loaded?.state.identity.palette_index).toBe(5);
    const settingsJson = JSON.stringify(loaded?.settings ?? {});
    expect(settingsJson).not.toContain("palette_index");
  });
});
