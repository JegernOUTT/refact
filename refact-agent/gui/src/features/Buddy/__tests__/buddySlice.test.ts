import { describe, expect, it, vi } from "vitest";
import {
  addDraft,
  addOpportunity,
  beginBuddySettingsRequest,
  buddySlice,
  clearActiveSpeech,
  defaultBuddyPulse,
  defaultBuddySettings,
  dequeueRuntimeEvent,
  enqueueRuntimeEvent,
  expireOpportunities,
  failBuddySettingsRequest,
  finishBuddySettingsRequest,
  markBuddyNotificationSeen,
  recordChatBubbleImpression,
  resetBuddyForWorkspaceChange,
  resolveOpportunity,
  setActiveSpeech,
  setBuddySnapshot,
  setBuddyUnavailable,
  setPulse,
  snoozeChatBubbles,
  snoozeHomeNotifications,
} from "../buddySlice";
import type {
  BuddyDraft,
  BuddyOpportunity,
  BuddyPulse,
  BuddyRuntimeEvent,
  BuddySnapshot,
  BuddySpeechItem,
} from "../types";

const reducer = buddySlice.reducer;

function makeSnapshot(overrides?: Partial<BuddySnapshot>): BuddySnapshot {
  const opportunities = overrides?.opportunities ?? [];
  return {
    state: {
      identity: {
        name: "Buddy",
        created_at: "2024-01-01T00:00:00Z",
        palette_index: 0,
      },
      progression: {
        stage: 0,
        stage_name: "Egg",
        level: 1,
        xp: 0,
        xp_next: 20,
      },
      skills: { unlocked: [], locked: [] },
      workflow_summaries: [],
      semantic: {
        mood: "idle",
        focus: "helping",
        headline: "Ready",
        last_active: "2024-01-01T00:00:00Z",
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
        vibe: "Playful",
        summary: "An energetic helper.",
        prompt: "Playful",
        traits: {
          playfulness: 70,
          chaos: 35,
          sociability: 72,
          curiosity: 78,
          resilience: 66,
        },
      },
      active_quest: null,
      opportunities,
    },
    settings: defaultBuddySettings(),
    enabled: true,
    ...overrides,
  };
}

function makeRuntimeEvent(
  overrides?: Partial<BuddyRuntimeEvent>,
): BuddyRuntimeEvent {
  return {
    id: "runtime-1",
    signal_type: "workflow_progress",
    title: "Runtime event",
    source: "test",
    status: "info",
    priority: "normal",
    created_at: "2024-01-01T00:00:00Z",
    ...overrides,
  };
}

function makeOpportunity(
  overrides?: Partial<BuddyOpportunity>,
): BuddyOpportunity {
  return {
    id: "opp-1",
    kind: "diagnostic_investigation",
    summary: "Investigate errors",
    priority: "high",
    confidence: 0.9,
    fact_keys: [],
    cooldown_key: "diagnostic:opp-1",
    cooldown_secs: 1800,
    status: "new",
    proposed_actions: [],
    humor: null,
    humor_allowed: false,
    related: { chat_ids: [], task_ids: [], memory_ids: [], config_paths: [] },
    created_at: "2024-01-01T00:00:00Z",
    expires_at: "2099-01-01T00:00:00Z",
    resolved_at: null,
    ...overrides,
  };
}

function makeSpeech(overrides?: Partial<BuddySpeechItem>): BuddySpeechItem {
  return {
    id: "speech-1",
    text: "Hello",
    mood: "happy",
    scope: "global",
    persistent: false,
    ttl_seconds: 5,
    created_at: "2024-01-01T00:00:00Z",
    controls: [],
    ...overrides,
  };
}

function makeDraft(overrides?: Partial<BuddyDraft>): BuddyDraft {
  return {
    id: "draft-1",
    kind: "skill",
    title: "Draft skill",
    yaml_or_json: "{}",
    explanation: "Draft explanation",
    created_at: "2024-01-01T00:00:00Z",
    expires_at: "2099-01-01T00:00:00Z",
    ...overrides,
  };
}

function makePulse(): BuddyPulse {
  return {
    ...defaultBuddyPulse(),
    generated_at: "2024-01-02T00:00:00Z",
    tasks: { total: 4, stuck: 1, abandoned: 0, by_status: { active: 4 } },
  };
}

describe("buddySlice", () => {
  it("setBuddySnapshot normalizes optional state and setBuddyUnavailable clears workspace data", () => {
    const snapshot = makeSnapshot({ enabled: false });
    const loaded = reducer(undefined, setBuddySnapshot(snapshot));

    expect(loaded.loaded).toBe(true);
    expect(loaded.snapshot?.enabled).toBe(false);
    expect(loaded.snapshot?.settings.enabled).toBe(false);
    expect(loaded.runtimeQueue).toEqual([]);
    expect(loaded.nowPlaying).toBeNull();
    expect(loaded.activeSpeech).toBeNull();
    expect(loaded.pulse).toEqual(defaultBuddyPulse());

    const unavailable = reducer(loaded, setBuddyUnavailable());

    expect(unavailable.loaded).toBe(true);
    expect(unavailable.snapshot).toBeNull();
    expect(unavailable.runtimeQueue).toEqual([]);
    expect(unavailable.pulse).toBeNull();
  });

  it("enqueues runtime events with a cap and dequeues into nowPlaying", () => {
    let state = reducer(undefined, setBuddySnapshot(makeSnapshot()));
    for (let index = 0; index < 101; index += 1) {
      state = reducer(
        state,
        enqueueRuntimeEvent(
          makeRuntimeEvent({ id: `runtime-${index}`, priority: "high" }),
        ),
      );
    }

    expect(state.runtimeQueue).toHaveLength(100);
    expect(state.runtimeQueue.some((event) => event.id === "runtime-0")).toBe(
      false,
    );
    expect(state.runtimeQueue.some((event) => event.id === "runtime-100")).toBe(
      true,
    );

    const queued = reducer(
      reducer(
        undefined,
        enqueueRuntimeEvent(makeRuntimeEvent({ id: "first" })),
      ),
      enqueueRuntimeEvent(makeRuntimeEvent({ id: "second" })),
    );
    const dequeued = reducer(queued, dequeueRuntimeEvent());

    expect(dequeued.nowPlaying?.id).toBe("first");
    expect(dequeued.runtimeQueue.map((event) => event.id)).toEqual(["second"]);
  });

  it("beginBuddySettingsRequest applies a patch and finish applies server settings by requestSeq", () => {
    const serverSettings = {
      ...defaultBuddySettings(),
      quiet_mode: false,
    };
    let state = reducer(undefined, setBuddySnapshot(makeSnapshot()));

    state = reducer(
      state,
      beginBuddySettingsRequest({
        requestSeq: 1,
        keys: ["quiet_mode"],
        patch: { quiet_mode: true },
      }),
    );

    expect(state.snapshot?.settings.quiet_mode).toBe(true);
    expect(state.pendingSettingsRequests).toHaveLength(1);

    state = reducer(
      state,
      finishBuddySettingsRequest({ requestSeq: 1, settings: serverSettings }),
    );

    expect(state.snapshot?.settings.quiet_mode).toBe(false);
    expect(state.pendingSettingsRequests).toEqual([]);
  });

  it("failBuddySettingsRequest rolls back when no newer intersecting request remains", () => {
    let state = reducer(undefined, setBuddySnapshot(makeSnapshot()));
    state = reducer(
      state,
      beginBuddySettingsRequest({
        requestSeq: 1,
        keys: ["auto_diagnostics"],
        patch: { auto_diagnostics: false },
      }),
    );

    state = reducer(
      state,
      failBuddySettingsRequest({
        requestSeq: 1,
        rollbackPatch: { auto_diagnostics: true },
      }),
    );

    expect(state.snapshot?.settings.auto_diagnostics).toBe(true);
    expect(state.pendingSettingsRequests).toEqual([]);
  });

  it("failBuddySettingsRequest does not roll back a superseded intersecting request", () => {
    let state = reducer(undefined, setBuddySnapshot(makeSnapshot()));
    state = reducer(
      state,
      beginBuddySettingsRequest({
        requestSeq: 1,
        keys: ["auto_diagnostics"],
        patch: { auto_diagnostics: false },
      }),
    );
    state = reducer(
      state,
      beginBuddySettingsRequest({
        requestSeq: 2,
        keys: ["auto_diagnostics"],
        patch: { auto_diagnostics: true },
      }),
    );

    state = reducer(
      state,
      failBuddySettingsRequest({
        requestSeq: 1,
        rollbackPatch: { auto_diagnostics: true },
      }),
    );

    expect(state.snapshot?.settings.auto_diagnostics).toBe(true);
    expect(
      state.pendingSettingsRequests.map((request) => request.requestSeq),
    ).toEqual([2]);
  });

  it("resetBuddyForWorkspaceChange clears workspace data and preserves UI preferences", () => {
    const nowSpy = vi.spyOn(Date, "now").mockReturnValue(1_000_000);
    let state = reducer(undefined, setBuddySnapshot(makeSnapshot()));
    state = reducer(state, enqueueRuntimeEvent(makeRuntimeEvent()));
    state = reducer(state, setActiveSpeech(makeSpeech()));
    state = reducer(state, addOpportunity(makeOpportunity()));
    state = reducer(state, setPulse(makePulse()));
    state = reducer(state, addDraft(makeDraft()));
    state = reducer(
      state,
      beginBuddySettingsRequest({
        requestSeq: 1,
        keys: ["quiet_mode"],
        patch: { quiet_mode: true },
      }),
    );
    state = reducer(state, snoozeHomeNotifications(30_000));
    state = reducer(state, markBuddyNotificationSeen("content:speech:hello"));
    state = reducer(state, snoozeChatBubbles(45_000));
    state = reducer(
      state,
      recordChatBubbleImpression({ id: "bubble-1", kind: "ambient" }),
    );

    const seenNotificationIds = state.seenNotificationIds;
    const homeSnoozedUntil = state.homeSnoozedUntil;
    const chatBubbleSnoozedUntil = state.chatBubbleSnoozedUntil;
    const reset = reducer(state, resetBuddyForWorkspaceChange());

    expect(reset.snapshot).toBeNull();
    expect(reset.loaded).toBe(false);
    expect(reset.runtimeQueue).toEqual([]);
    expect(reset.nowPlaying).toBeNull();
    expect(reset.activeSpeech).toBeNull();
    expect(reset.opportunities).toEqual([]);
    expect(reset.pulse).toBeNull();
    expect(reset.activeDrafts).toEqual([]);
    expect(reset.pendingSettingsRequests).toEqual([]);
    expect(reset.chatBubbleImpressions).toEqual([]);
    expect(reset.seenNotificationIds).toEqual(seenNotificationIds);
    expect(reset.homeSnoozedUntil).toBe(homeSnoozedUntil);
    expect(reset.chatBubbleSnoozedUntil).toBe(chatBubbleSnoozedUntil);
    nowSpy.mockRestore();
  });

  it("adds opportunities, caps them, resolves one, and expires active old entries", () => {
    let state = reducer(undefined, setBuddySnapshot(makeSnapshot()));
    for (let index = 0; index < 201; index += 1) {
      state = reducer(
        state,
        addOpportunity(
          makeOpportunity({
            id: `opp-${index}`,
            cooldown_key: `diagnostic:opp-${index}`,
            expires_at:
              index === 200 ? "2024-01-01T00:00:00Z" : "2099-01-01T00:00:00Z",
          }),
        ),
      );
    }

    expect(state.opportunities).toHaveLength(200);
    expect(state.opportunities[0].id).toBe("opp-1");

    state = reducer(
      state,
      resolveOpportunity({ id: "opp-2", status: "accepted" }),
    );
    state = reducer(state, expireOpportunities("2024-01-02T00:00:00Z"));

    expect(
      state.opportunities.find((opportunity) => opportunity.id === "opp-2")
        ?.status,
    ).toBe("accepted");
    expect(
      state.opportunities.find((opportunity) => opportunity.id === "opp-200")
        ?.status,
    ).toBe("expired");
    expect(state.snapshot?.opportunities).toBe(state.opportunities);
  });

  it("sets and clears active speech in slice and snapshot", () => {
    const speech = makeSpeech({ id: "speech-active", text: "Heads up" });
    let state = reducer(undefined, setBuddySnapshot(makeSnapshot()));

    state = reducer(state, setActiveSpeech(speech));

    expect(state.activeSpeech).toEqual(speech);
    expect(state.snapshot?.active_speech).toEqual(speech);

    state = reducer(state, clearActiveSpeech());

    expect(state.activeSpeech).toBeNull();
    expect(state.snapshot?.active_speech).toBeNull();
  });
});
