import { configureStore, type Middleware } from "@reduxjs/toolkit";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import { server } from "../utils/mockServer";
import { reducer as configReducer } from "../features/Config/configSlice";
import {
  buddySlice,
  defaultBuddyPulse,
  enqueueRuntimeEvent,
  replaceOpportunities,
  setBuddySnapshot,
} from "../features/Buddy/buddySlice";
import { buddyApi } from "../services/refact/buddy";
import type {
  BuddyOpportunity,
  BuddyPulse,
  BuddyRuntimeEvent,
  BuddySnapshot,
} from "../features/Buddy/types";

function makeSnapshot(overrides?: Partial<BuddySnapshot>): BuddySnapshot {
  return {
    state: {
      identity: { name: "Buddy", created_at: "", palette_index: 0 },
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
      opportunities: [],
    },
    settings: {
      enabled: true,
      auto_diagnostics: true,
      auto_issue_creation: false,
      personality_prompt: null,
      autonomous_chats_enabled: true,
      proactive_enabled: true,
      message_observation_enabled: false,
      chat_reactions_enabled: false,
      housekeeping_enabled: true,
      humor_enabled: true,
      humor_level: "light",
      autonomy_level: "suggest",
      quiet_mode: false,
      daily_digest_hour: 18,
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
    },
    enabled: true,
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

function makeProvidedPulse(): BuddyPulse {
  return {
    ...defaultBuddyPulse(),
    generated_at: "2024-01-01T00:00:00Z",
    tasks: { total: 3, stuck: 1, abandoned: 0, by_status: { active: 3 } },
  };
}

function makeStore(actionSpy?: (action: unknown) => void) {
  const recorder: Middleware = () => (next) => (action) => {
    actionSpy?.(action);
    return next(action);
  };
  return configureStore({
    reducer: {
      buddy: buddySlice.reducer,
      config: configReducer,
      [buddyApi.reducerPath]: buddyApi.reducer,
    },
    middleware: (getDefault) =>
      getDefault().concat(recorder, buddyApi.middleware),
  });
}

describe("buddy frontend state hardening", () => {
  it("opportunities_query_unwraps_backend_envelope", async () => {
    const opportunities = [
      makeOpportunity({ id: "opp-1" }),
      makeOpportunity({ id: "opp-2" }),
    ];
    server.use(
      http.get("*/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities }),
      ),
    );
    const store = makeStore();

    const result = await store.dispatch(
      buddyApi.endpoints.getOpportunities.initiate(undefined),
    );

    expect(result.data).toEqual(opportunities);
  });

  it("opportunities_query_dispatches_replaceOpportunities", async () => {
    const actions: unknown[] = [];
    const opportunities = [
      makeOpportunity({ id: "opp-1" }),
      makeOpportunity({ id: "opp-2" }),
    ];
    server.use(
      http.get("*/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities }),
      ),
    );
    const store = makeStore((action) => actions.push(action));

    await store.dispatch(
      buddyApi.endpoints.getOpportunities.initiate(undefined),
    );

    expect(actions).toContainEqual(replaceOpportunities(opportunities));
    expect(store.getState().buddy.opportunities).toEqual(opportunities);
  });

  it("accept_invalidates_opportunities_tag", async () => {
    let getCount = 0;
    server.use(
      http.get("*/v1/buddy/opportunities", () => {
        getCount += 1;
        return HttpResponse.json({
          opportunities: [makeOpportunity({ id: `opp-${getCount}` })],
        });
      }),
      http.post("*/v1/buddy/opportunities/:id/accept", () =>
        HttpResponse.json({
          snapshot: makeSnapshot(),
          action_result: { kind: "open_page", navigate_to: { type: "buddy" } },
        }),
      ),
    );
    const store = makeStore();
    const subscription = store.dispatch(
      buddyApi.endpoints.getOpportunities.initiate(undefined),
    );
    await subscription;

    await store.dispatch(
      buddyApi.endpoints.acceptOpportunity.initiate({
        id: "opp-1",
        action_index: 0,
      }),
    );

    await waitFor(() => expect(getCount).toBeGreaterThanOrEqual(2));
    subscription.unsubscribe();
  });

  it("pulse_normalization_uses_default_when_missing", () => {
    const state = buddySlice.reducer(
      undefined,
      setBuddySnapshot(makeSnapshot()),
    );

    expect(state.pulse).toEqual(defaultBuddyPulse());
  });

  it("pulse_normalization_uses_provided_when_present", () => {
    const pulse = makeProvidedPulse();
    const state = buddySlice.reducer(
      undefined,
      setBuddySnapshot(makeSnapshot({ pulse })),
    );

    expect(state.pulse).toEqual(pulse);
  });

  it("BuddyOpportunity_has_cooldown_secs_resolved_at_fields", () => {
    const opportunity: BuddyOpportunity = makeOpportunity({
      cooldown_secs: 3600,
      resolved_at: "2024-01-02T00:00:00Z",
    });

    expect(opportunity.cooldown_secs).toBe(3600);
    expect(opportunity.resolved_at).toBe("2024-01-02T00:00:00Z");
  });

  it("signal_type_error_is_valid_runtime_event", () => {
    const event: BuddyRuntimeEvent = {
      id: "runtime-error",
      signal_type: "error",
      title: "Runtime error",
      source: "test",
      status: "failed",
      priority: "high",
      created_at: "2024-01-01T00:00:00Z",
    };
    const state = buddySlice.reducer(undefined, enqueueRuntimeEvent(event));

    expect(state.runtimeQueue[0].signal_type).toBe("error");
  });
});
