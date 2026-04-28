import { describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { render, screen, waitFor } from "../utils/test-utils";
import { server } from "../utils/mockServer";
import { setUpStore } from "../app/store";
import { actionLabel } from "../features/Buddy/buddyOpportunityActions";
import { BuddyHome } from "../features/Buddy/BuddyHome";
import {
  addOpportunity,
  buddySlice,
  selectUnreadOpportunities,
  setBuddySnapshot,
} from "../features/Buddy/buddySlice";
import { navigateFromBuddyPage } from "../features/Buddy/executeBuddyAction";
import { push } from "../features/Pages/pagesSlice";
import type {
  BuddyOpportunity,
  BuddyPage,
  BuddyPulse,
  BuddySnapshot,
} from "../features/Buddy/types";

const CONFIG_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

function makeOpportunity(
  overrides?: Partial<BuddyOpportunity>,
): BuddyOpportunity {
  return {
    id: "opp-1",
    kind: "diagnostic_investigation",
    summary: "Model config is broken",
    priority: "high",
    confidence: 0.9,
    fact_keys: [],
    cooldown_key: "opp-1",
    cooldown_secs: 1800,
    status: "new",
    proposed_actions: [],
    humor_allowed: false,
    related: { chat_ids: [], task_ids: [], memory_ids: [], config_paths: [] },
    created_at: "2024-01-01T00:00:00Z",
    expires_at: "2099-12-31T00:00:00Z",
    ...overrides,
  };
}

function makePulse(): BuddyPulse {
  return {
    generated_at: "2024-01-01T00:00:00Z",
    tasks: { total: 3, stuck: 1, abandoned: 2, by_status: {} },
    trajectories: { total: 10, untitled: 2, oldest_age_days: 14 },
    memory: { total: 50, orphan: 5, stale_conflicts: 1 },
    providers: { defaults_ok: true, broken_refs: 0, quota_warnings: 0 },
    mcp: { total: 4, failing: 1, auth_expiring: 0 },
    customization: { modes: 3, skills: 2, commands: 1, subagents: 0, hooks: 0 },
    diagnostics: { last_hour: 7, top_error_types: ["model_not_found"] },
    git: { uncommitted_files: 5, diff_lines_4h: 120, branches: 3 },
  };
}

function makeSnapshot(): BuddySnapshot {
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
        headline: "Ready to help",
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
    },
    enabled: true,
    pulse: makePulse(),
  };
}

describe("buddy UI polish", () => {
  it("actionLabel_humanizes_customization_kind", () => {
    expect(
      actionLabel({
        kind: "draft_customization_change",
        customization_kind: "delegate",
        id: "delegate-a",
        patch: {},
      }),
    ).toBe("Edit delegate");
  });

  it("actionLabel_humanizes_pulse_scope", () => {
    expect(actionLabel({ kind: "create_pulse_report", scope: "all" })).toBe(
      "Create system report",
    );
    expect(actionLabel({ kind: "create_pulse_report", scope: "mcp" })).toBe(
      "Create MCP report",
    );
  });

  it("actionLabel_humanizes_market_kind", () => {
    expect(
      actionLabel({
        kind: "offer_marketplace_install",
        market_kind: "mcp",
        item_id: "github",
      }),
    ).toBe("Install MCP");
  });

  it("unread_selector_matches_documented_shown_policy", () => {
    const reducer = buddySlice.reducer;
    const s1 = reducer(
      undefined,
      addOpportunity(makeOpportunity({ id: "o-new", status: "new" })),
    );
    const s2 = reducer(
      s1,
      addOpportunity(makeOpportunity({ id: "o-shown", status: "shown" })),
    );
    const unread = selectUnreadOpportunities({ buddy: s2 });
    expect(unread.map((o) => o.id)).toEqual(["o-new"]);
  });

  it("shared_navigation_helper_routes_all_pages", () => {
    const cases: [BuddyPage, string][] = [
      [{ type: "buddy" }, "buddy"],
      [{ type: "stats" }, "stats dashboard"],
      [{ type: "customization" }, "customization"],
      [{ type: "providers" }, "providers page"],
      [{ type: "default_models" }, "default models"],
      [{ type: "integrations" }, "integrations page"],
      [{ type: "extensions" }, "extensions"],
      [{ type: "marketplace_hub" }, "marketplace hub"],
      [{ type: "marketplace" }, "mcp marketplace"],
      [{ type: "skills_marketplace" }, "skills marketplace"],
      [{ type: "commands_marketplace" }, "commands marketplace"],
      [{ type: "delegates_marketplace" }, "subagents marketplace"],
      [{ type: "tasks_list" }, "tasks list"],
      [{ type: "task_workspace", task_id: "task-a" }, "task workspace"],
      [{ type: "knowledge_graph" }, "knowledge graph"],
    ];

    for (const [page, expectedName] of cases) {
      const dispatch = vi.fn();
      navigateFromBuddyPage(page, dispatch as never);
      const action = dispatch.mock.calls[0][0] as ReturnType<typeof push>;
      expect(action.payload).toMatchObject({ name: expectedName });
    }
  });

  it("BuddyHome_container_renders_split_subcomponents", async () => {
    server.use(
      http.get("http://127.0.0.1:8001/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities: [] }),
      ),
      http.get("http://127.0.0.1:8001/v1/buddy/conversations", () =>
        HttpResponse.json([]),
      ),
      http.get("http://127.0.0.1:8001/v1/stats/llm/summary", () =>
        HttpResponse.json({
          totals: { total_calls: 1, successful_calls: 1, total_tokens: 10 },
        }),
      ),
      http.get("http://127.0.0.1:8001/v1/setup/status", () =>
        HttpResponse.json({ configured: true }),
      ),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    render(<BuddyHome />, {
      preloadedState: { ...CONFIG_STATE, buddy: store.getState().buddy },
    });

    await waitFor(() => {
      expect(screen.getByTestId("buddy-hero")).toBeInTheDocument();
      expect(screen.getByTestId("buddy-summary-strip")).toBeInTheDocument();
      expect(screen.getByTestId("buddy-personality-panel")).toBeInTheDocument();
      expect(screen.getByTestId("buddy-activity-panel")).toBeInTheDocument();
      expect(
        screen.getByTestId("buddy-recent-errors-panel"),
      ).toBeInTheDocument();
    });
  });
});
