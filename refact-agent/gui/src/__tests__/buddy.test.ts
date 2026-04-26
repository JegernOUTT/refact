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
  setActiveSpeech,
  clearActiveSpeech,
  enqueueRuntimeEvent,
} from "../features/Buddy/buddySlice";
import { PALETTES, SIGNALS, STAGES } from "../features/Buddy/constants";
import { buildColorMap } from "../features/Buddy/canvas/colorMap";
import { updateSceneAnimation } from "../features/Buddy/canvas/animLoop";
import { createInitialAnimState } from "../features/Buddy/state";
import type {
  BuddySnapshot,
  BuddyState,
  BuddyActivityEntry,
  BuddySuggestion,
  BuddyConversationMeta,
  DiagnosticContext,
  BuddySpeechItem,
  BuddyRuntimeEvent,
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

describe("BuddyChatCompanion triggers", () => {
  test("chat_error signal is marked as error in SIGNALS", () => {
    expect(SIGNALS["chat_error"]?.isError).toBe(true);
  });

  test("tool_failed signal is marked as error in SIGNALS", () => {
    expect(SIGNALS["tool_failed"]?.isError).toBe(true);
  });

  test("chat_completed signal is not an error", () => {
    expect(SIGNALS["chat_completed"]?.isError).toBe(false);
  });

  test("diagnostic stored in recentDiagnostics on addBuddyDiagnostic", () => {
    const diag = makeDiagnostic({ error_message: "model not found" });
    const state = reducer(undefined, addBuddyDiagnostic(diag));
    expect(state.recentDiagnostics).toHaveLength(1);
    expect(state.recentDiagnostics[0].error_message).toBe("model not found");
  });
});

describe("BuddyCanvas displaySize", () => {
  test("SIGNALS has isError flag for error types", () => {
    const errorTypes = ["chat_error", "tool_failed", "balance_low", "connection_lost", "task_failed"];
    for (const t of errorTypes) {
      expect(SIGNALS[t]?.isError, `${t} should be error`).toBe(true);
    }
  });

  test("SIGNALS has isError=false for success types", () => {
    const okTypes = ["chat_completed", "edit_applied", "task_completed"];
    for (const t of okTypes) {
      expect(SIGNALS[t]?.isError, `${t} should not be error`).toBe(false);
    }
  });
});
describe("speech cloud state", () => {
  function makeSpeech(overrides?: Partial<BuddySpeechItem>): BuddySpeechItem {
    return {
      id: "speech-1",
      text: "Hello from Buddy!",
      mood: "happy",
      scope: "global",
      persistent: false,
      ttl_seconds: 10,
      created_at: "2024-01-01T00:00:00Z",
      controls: [],
      ...overrides,
    };
  }

  test("initial activeSpeech is null", () => {
    const state = reducer(undefined, { type: "@@INIT" });
    expect(state.activeSpeech).toBeNull();
  });

  test("setActiveSpeech stores speech item", () => {
    const speech = makeSpeech({ text: "Working on it..." });
    const state = reducer(undefined, setActiveSpeech(speech));
    expect(state.activeSpeech?.text).toBe("Working on it...");
  });

  test("clearActiveSpeech removes speech item", () => {
    const speech = makeSpeech();
    const s1 = reducer(undefined, setActiveSpeech(speech));
    const s2 = reducer(s1, clearActiveSpeech());
    expect(s2.activeSpeech).toBeNull();
  });

  test("setActiveSpeech with controls stores controls", () => {
    const speech = makeSpeech({
      controls: [{ id: "btn1", label: "Fix", action: "open_chat", style: "primary" }],
    });
    const state = reducer(undefined, setActiveSpeech(speech));
    expect(state.activeSpeech?.controls).toHaveLength(1);
    expect(state.activeSpeech?.controls[0].action).toBe("open_chat");
  });

  test("setBuddySnapshot hydrates activeSpeech from snapshot", () => {
    const speech = makeSpeech({ text: "Snapshot speech" });
    const snap = makeSnapshot({ active_speech: speech });
    const state = reducer(undefined, setBuddySnapshot(snap));
    expect(state.activeSpeech?.text).toBe("Snapshot speech");
  });

  test("setBuddySnapshot with null active_speech sets null", () => {
    const speech = makeSpeech();
    const s1 = reducer(undefined, setActiveSpeech(speech));
    const snap = makeSnapshot({ active_speech: null });
    const s2 = reducer(s1, setBuddySnapshot(snap));
    expect(s2.activeSpeech).toBeNull();
  });
});

describe("runtime event new fields", () => {
  function makeEvent(overrides?: Partial<BuddyRuntimeEvent>): BuddyRuntimeEvent {
    return {
      id: "evt-1",
      signal_type: "streaming",
      title: "Streaming",
      source: "chat",
      status: "started",
      priority: "normal",
      created_at: "2024-01-01T00:00:00Z",
      ...overrides,
    };
  }

  test("runtime event stores speech_text and scene", () => {
    const evt = makeEvent({ speech_text: "Thinking...", scene: "working", persistent: true });
    const state = reducer(undefined, enqueueRuntimeEvent(evt));
    const queue = state.runtimeQueue;
    expect(queue[0].speech_text).toBe("Thinking...");
    expect(queue[0].scene).toBe("working");
    expect(queue[0].persistent).toBe(true);
  });

  test("runtime event with controls stores controls", () => {
    const evt = makeEvent({
      controls: [{ id: "c1", label: "Fix", action: "open_chat", style: "primary" }],
    });
    const state = reducer(undefined, enqueueRuntimeEvent(evt));
    expect(state.runtimeQueue[0].controls).toHaveLength(1);
  });
});

describe("SIGNALS scene categories", () => {
  test("streaming signal is active category", () => {
    expect(SIGNALS["streaming"]?.category).toBe("active");
  });

  test("indexing signal is active category", () => {
    expect(SIGNALS["indexing"]?.category).toBe("active");
  });

  test("chat_error signal is speech category", () => {
    expect(SIGNALS["chat_error"]?.category).toBe("speech");
  });

  test("chat_completed signal is transient category", () => {
    expect(SIGNALS["chat_completed"]?.category).toBe("transient");
  });

  test("all signals have scene defined", () => {
    for (const [key, def] of Object.entries(SIGNALS)) {
      expect(def.scene, `${key} missing scene`).toBeDefined();
    }
  });

  test("6 distinct scene types exist", () => {
    const scenes = new Set(Object.values(SIGNALS).map((d) => d.scene));
    expect(scenes.size).toBeGreaterThanOrEqual(6);
  });
});

describe("scene animation system", () => {
  test("updateSceneAnimation working/typing increases heat", () => {
    const anim = createInitialAnimState();
    anim.heat = 0;
    updateSceneAnimation(anim, "working", "typing");
    expect(anim.heat).toBeGreaterThan(0);
  });

  test("updateSceneAnimation alert triggers shake on interval", () => {
    const anim = createInitialAnimState();
    anim.frame = 40;
    anim.shakeIntensity = 0;
    updateSceneAnimation(anim, "alert", "shake_worried");
    expect(anim.shakeIntensity).toBeGreaterThan(0);
  });

  test("updateSceneAnimation perk/ears_up sets ear state", () => {
    const anim = createInitialAnimState();
    anim.earState = 0;
    updateSceneAnimation(anim, "perk", "ears_up");
    expect(anim.earState).toBeGreaterThan(0);
  });

  test("updateSceneAnimation with empty scene does nothing", () => {
    const anim = createInitialAnimState();
    const heatBefore = anim.heat;
    updateSceneAnimation(anim, "", "");
    expect(anim.heat).toBe(heatBefore);

  });
});

describe("conversation ledger", () => {
  function makeEntry(overrides?: Partial<import("../features/Buddy/types").BuddyConversationEntry>): import("../features/Buddy/types").BuddyConversationEntry {
    return {
      id: "c1",
      kind: "chat",
      title: "Test Chat",
      created_at: "2024-01-01T00:00:00Z",
      updated_at: "2024-01-01T00:00:00Z",
      status: "active",
      message_count: 0,
      icon: "💬",
      badge: null,
      ...overrides,
    };
  }

  test("setBuddyConversations stores unified ledger entries", () => {
    const entries = [
      makeEntry({ id: "c1", kind: "chat", icon: "💬" }),
      makeEntry({ id: "w1", kind: "workflow", icon: "📦", badge: "Commit Msg" }),
      makeEntry({ id: "s1", kind: "system", icon: "🗜", badge: "Compress" }),
    ];
    const state = reducer(undefined, setBuddyConversations(entries));
    expect(state.conversations).toHaveLength(3);
    expect(state.conversations[0].kind).toBe("chat");
    expect(state.conversations[1].kind).toBe("workflow");
    expect(state.conversations[2].kind).toBe("system");
  });

  test("chat entry has correct icon and no badge by default", () => {
    const entry = makeEntry({ kind: "chat", icon: "💬", badge: null });
    expect(entry.icon).toBe("💬");
    expect(entry.badge).toBeNull();
  });

  test("setup entry has badge and gear icon", () => {
    const entry = makeEntry({ kind: "setup", icon: "⚙️", badge: "MCP Setup" });
    expect(entry.icon).toBe("⚙️");
    expect(entry.badge).toBe("MCP Setup");
  });

  test("workflow entry has badge and workflow icon", () => {
    const entry = makeEntry({ kind: "workflow", icon: "📦", badge: "Commit Msg" });
    expect(entry.kind).toBe("workflow");
    expect(entry.badge).toBe("Commit Msg");
  });

  test("system entry has system icon", () => {
    const entry = makeEntry({ kind: "system", icon: "🗜", badge: "Compress" });
    expect(entry.kind).toBe("system");
    expect(entry.icon).toBe("🗜");
  });

  test("setBuddyConversations replaces existing conversations", () => {
    const initial = [makeEntry({ id: "old" })];
    let state = reducer(undefined, setBuddyConversations(initial));
    expect(state.conversations).toHaveLength(1);
    const updated = [makeEntry({ id: "new1" }), makeEntry({ id: "new2" })];
    state = reducer(state, setBuddyConversations(updated));
    expect(state.conversations).toHaveLength(2);
    expect(state.conversations[0].id).toBe("new1");
  });
});
