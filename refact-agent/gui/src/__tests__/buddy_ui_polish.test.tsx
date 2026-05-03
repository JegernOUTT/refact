import { describe, expect, it, vi } from "vitest";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { http, HttpResponse } from "msw";
import { render, screen, waitFor } from "../utils/test-utils";
import { server } from "../utils/mockServer";
import { setUpStore } from "../app/store";
import { actionLabel } from "../features/Buddy/buddyOpportunityActions";
import { BuddyHome } from "../features/Buddy/BuddyHome";
import { BuddyDashboardScene } from "../features/Buddy/BuddyDashboardScene";
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

type CapturedThunk = (
  dispatch: (action: unknown) => unknown,
  getState: () => unknown,
  extra: unknown,
) => unknown;

type TestDispatch = (action: unknown) => unknown;

function readGuiSource(path: string): Promise<string> {
  return readFile(resolve(process.cwd(), "src", path), "utf8");
}

function isSetupModeCreateAction(action: unknown): boolean {
  if (typeof action !== "object" || action === null) return false;
  const candidate = action as { payload?: unknown; type?: unknown };
  if (candidate.type !== "chatThread/createWithId") return false;
  if (typeof candidate.payload !== "object" || candidate.payload === null) {
    return false;
  }
  const payload = candidate.payload as { mode?: unknown };
  return payload.mode === "setup_mcp";
}

function makeThunkDispatch() {
  const innerDispatch = vi.fn<TestDispatch>((action) => action);
  const dispatch = vi.fn<TestDispatch>((action) => {
    if (typeof action === "function") {
      return (action as CapturedThunk)(
        innerDispatch,
        () => ({ config: { lspPort: 0, apiKey: null } }),
        undefined,
      );
    }
    return innerDispatch(action);
  });
  return { dispatch, innerDispatch };
}

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
    worktrees: {
      total_registered: 3,
      total_discovered: 1,
      total: 4,
      clean: 2,
      dirty: 1,
      unknown: 0,
      stale: 1,
      conflicted: 0,
      shared: 1,
      abandoned_clean: 2,
      changed_files: 3,
      additions: 10,
      deletions: 2,
      missing_registry_paths: 1,
      unregistered_cache_dirs: 1,
      merged_branches: 2,
    },
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

  it("unread_selector_treats_new_and_shown_as_unread", () => {
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
    expect(unread.map((o) => o.id)).toEqual(["o-new", "o-shown"]);
  });

  it("shared_navigation_helper_routes_pages", () => {
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
      [{ type: "worktrees" }, "tasks list"],
    ];

    for (const [page, expectedName] of cases) {
      const dispatch = vi.fn();
      navigateFromBuddyPage(page, dispatch as never);
      const action = dispatch.mock.calls[0][0] as ReturnType<typeof push>;
      expect(action.payload).toMatchObject({ name: expectedName });
    }
  });

  it("shared_navigation_helper_routes_setup_mode", () => {
    const { dispatch, innerDispatch } = makeThunkDispatch();
    navigateFromBuddyPage(
      { type: "setup_mode", mode: "setup_mcp" },
      dispatch as never,
    );
    const dispatchedActions = innerDispatch.mock.calls.map((call) => call[0]);
    expect(dispatchedActions.some(isSetupModeCreateAction)).toBe(true);
  });

  it("shared_navigation_helper_keeps_worktrees_inside_tasks", () => {
    const dispatch = vi.fn();
    navigateFromBuddyPage({ type: "worktrees" }, dispatch as never);
    const action = dispatch.mock.calls[0][0] as ReturnType<typeof push>;
    expect(action.payload).toEqual({ name: "tasks list" });
  });

  it("BuddyWorld_keeps_scene_level_motion_without_roam_boosts", async () => {
    const source = await readGuiSource("features/Buddy/BuddyWorld.tsx");

    const forbiddenTarget = ["roam", "TargetX"].join("");
    const forbiddenBoost = ["roam", "Boost"].join("");

    expect(source).not.toContain(forbiddenTarget);
    expect(source).not.toContain(forbiddenBoost);
  });

  it("BuddyWorld_uses_deterministic_edge_aware_bubbles", async () => {
    const source = await readGuiSource("features/Buddy/BuddyWorld.tsx");
    const utilSource = await readGuiSource("features/Buddy/buddyWorldUtils.ts");

    expect(utilSource).toContain("function bubblePositionForSceneX");
    expect(utilSource).toContain("LONG_COMPACT_SPEECH_LENGTH");
    expect(utilSource).toContain("compact && (speechText?.length ?? 0)");
    expect(utilSource).toContain('if (x < 42) return "right"');
    expect(utilSource).toContain('if (x > 58) return "left"');
    expect(utilSource).toContain('return "top"');
    expect(source).toContain("bubblePosition={bubblePosition}");
    expect(source).toContain("randomizeBubblePosition={false}");
  });

  it("BuddyCharacter_splits_anchor_and_body_motion", async () => {
    const characterSource = await readGuiSource(
      "features/Buddy/BuddyCharacter.tsx",
    );
    const canvasSource = await readGuiSource("features/Buddy/BuddyCanvas.tsx");
    const styleSource = await readGuiSource(
      "features/Buddy/BuddyWorld.module.css",
    );

    expect(characterSource).toContain("styles.characterAnchor");
    expect(characterSource).toContain("styles.characterBody");
    expect(characterSource).toContain("compactBubble={compactBubble}");
    expect(canvasSource).toContain(
      "compactBubbleOverride || displaySize <= 180",
    );
    expect(styleSource).toContain(".characterAnchor");
    expect(styleSource).toContain("bottom 3.8s cubic-bezier");
    expect(styleSource).toContain("transform 3.8s cubic-bezier");
    expect(styleSource).toContain(".characterBody[data-pose");
    expect(styleSource).not.toContain(".character[data-pose");
  });

  it("BuddyWorld_keeps_narrow_object_tooltips_available", async () => {
    const source = await readGuiSource("features/Buddy/BuddyWorld.module.css");

    expect(source).toContain("@media (max-width: 720px)");
    expect(source).toContain("max-width: min(108px, 32vw)");
    expect(source).not.toMatch(/\.objectTooltip\s*\{[^}]*display:\s*none/u);
  });

  it("BuddyWorld_preserves_compact_height_in_narrow_media", async () => {
    const source = await readGuiSource("features/Buddy/BuddyWorld.module.css");

    expect(source).toContain(".compact {\n  height: 210px;");
    expect(source).toContain("@media (max-width: 720px)");
    expect(source).toContain(".scene:not(.compact) {\n    height: 250px;");
  });

  it("BuddyWorld_reschedules_idle_loop_after_noop_branch", async () => {
    const source = await readGuiSource("features/Buddy/BuddyWorld.tsx");

    expect(source).toContain("const [idleTick, setIdleTick]");
    expect(source).toContain("setIdleTick((tick) => tick + 1)");
    expect(source).toContain("activeSpeech,");
    expect(source).toContain("idleTick,");
    expect(source).toContain("reaction,");
    expect(source).toContain("showcaseRun,");
    expect(source).toContain("startShowcase,");
    expect(source).toContain("waypoints,");
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
      expect(screen.getByTestId("buddy-world")).toBeInTheDocument();
      expect(screen.getByTestId("buddy-world-canvas")).toBeInTheDocument();
      expect(screen.getByTestId("buddy-world-character")).toBeInTheDocument();
      expect(screen.getByTestId("buddy-summary-strip")).toBeInTheDocument();
      expect(screen.getByTestId("buddy-personality-panel")).toBeInTheDocument();
      expect(screen.getByTestId("buddy-activity-panel")).toBeInTheDocument();
      expect(
        screen.getByTestId("buddy-recent-errors-panel"),
      ).toBeInTheDocument();
    });
  });

  it("BuddyDashboardScene_renders_shared_canvas_scene", async () => {
    server.use(
      http.get("http://127.0.0.1:8001/v1/setup/status", () =>
        HttpResponse.json({ configured: true }),
      ),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    render(<BuddyDashboardScene />, {
      preloadedState: { ...CONFIG_STATE, buddy: store.getState().buddy },
    });

    await waitFor(() => {
      expect(screen.getByTestId("buddy-world")).toBeInTheDocument();
      expect(screen.getByTestId("buddy-world-canvas")).toBeInTheDocument();
      expect(screen.getByTestId("buddy-world-character")).toBeInTheDocument();
    });
  });
});
