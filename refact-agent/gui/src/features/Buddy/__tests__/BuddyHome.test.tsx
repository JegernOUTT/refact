import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { render, screen, waitFor, within } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { setUpStore } from "../../../app/store";
import {
  beginBuddySettingsRequest,
  markBuddyNotificationSeen,
  resetBuddyForWorkspaceChange,
  setBuddySnapshot,
} from "../buddySlice";
import { BuddyHome } from "../BuddyHome";
import type {
  BuddyActivityEntry,
  BuddyConversationEntry,
  BuddyOpportunity,
  BuddyPulse,
  BuddyRuntimeEvent,
  BuddySettings,
  BuddySnapshot,
} from "../types";

const CONFIG_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

function makeSettings(overrides?: Partial<BuddySettings>): BuddySettings {
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
    ...overrides,
  };
}

function makePulse(overrides?: Partial<BuddyPulse>): BuddyPulse {
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
    ...overrides,
  };
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
    proposed_actions: [{ kind: "open_page", page: { type: "providers" } }],
    humor_allowed: false,
    related: { chat_ids: [], task_ids: [], memory_ids: [], config_paths: [] },
    created_at: "2024-01-01T00:00:00Z",
    expires_at: "2099-12-31T00:00:00Z",
    ...overrides,
  };
}

function makeActivity(
  overrides?: Partial<BuddyActivityEntry>,
): BuddyActivityEntry {
  return {
    icon: "⚙️",
    title: "Task coach noticed pattern",
    description: "A tiny nudge appeared.",
    timestamp: "2024-01-01T00:00:00Z",
    activity_type: "buddy_task_health",
    chat_id: null,
    ...overrides,
  };
}

function makeRuntimeEvent(
  overrides?: Partial<BuddyRuntimeEvent>,
): BuddyRuntimeEvent {
  return {
    id: "runtime-1",
    signal_type: "chat_error",
    title: "Model unavailable",
    description: "Default model is missing.",
    source: "provider",
    status: "failed",
    priority: "high",
    created_at: new Date().toISOString(),
    ...overrides,
  };
}

function makeConversation(
  overrides?: Partial<BuddyConversationEntry>,
): BuddyConversationEntry {
  return {
    id: "buddy-chat-1",
    kind: "chat",
    title: "Recent Buddy chat",
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T01:00:00Z",
    status: "completed",
    message_count: 3,
    icon: "💬",
    badge: null,
    ...overrides,
  };
}

function makeSnapshot(overrides?: Partial<BuddySnapshot>): BuddySnapshot {
  const settings = overrides?.settings ?? makeSettings();
  const opportunity = makeOpportunity();
  return {
    state: {
      identity: { name: "Buddy", created_at: "", palette_index: 0 },
      progression: {
        stage: 0,
        stage_name: "Egg",
        level: 1,
        xp: 8,
        xp_next: 20,
      },
      skills: { unlocked: [], locked: [] },
      workflow_summaries: [],
      semantic: {
        mood: "idle",
        focus: "helping",
        headline: "Keeping watch",
        last_active: "2024-01-01T00:00:00Z",
      },
      recent_activities: [makeActivity()],
      suggestion_state: [
        {
          id: "suggestion-1",
          suggestion_type: "quest_start_setup",
          title: "Warm up this workspace",
          description: "Kick off setup so Buddy can help proactively.",
          created_at: "2024-01-01T00:00:00Z",
          dismissed: false,
          controls: [],
          quest: null,
        },
      ],
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
          care_score: 4,
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
      opportunities: [opportunity],
    },
    settings,
    enabled: settings.enabled,
    pulse: makePulse(),
    opportunities: [opportunity],
    runtime_queue: [makeRuntimeEvent()],
    recent_diagnostics: [],
    active_speech: null,
    now_playing: null,
    active_drafts: [],
    ...overrides,
  };
}

function installBuddyHomeHandlers(settings = makeSettings()) {
  const conversation = makeConversation();
  server.use(
    http.get("*/v1/buddy/opportunities", () =>
      HttpResponse.json({ opportunities: [makeOpportunity()] }),
    ),
    http.get("*/v1/buddy/conversations", () =>
      HttpResponse.json([conversation]),
    ),
    http.get("*/v1/stats/llm/summary", () =>
      HttpResponse.json({
        totals: { total_calls: 12, successful_calls: 9, total_tokens: 3456 },
      }),
    ),
    http.get("*/v1/setup/status", () =>
      HttpResponse.json({
        configured: false,
        reasons: ["missing agents"],
        detail: {
          has_agents_md: false,
          has_knowledge: false,
          has_trajectories: false,
        },
      }),
    ),
    http.post("*/v1/buddy/settings", async ({ request }) => {
      const patch = (await request.json()) as Partial<BuddySettings>;
      return HttpResponse.json({ ...settings, ...patch });
    }),
  );
}

describe("BuddyHome", () => {
  it("renders major enabled home regions from store and RTK Query data", async () => {
    installBuddyHomeHandlers();
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    render(<BuddyHome />, { store });

    expect(await screen.findByTestId("buddy-home-content")).toBeInTheDocument();
    expect(screen.getByTestId("buddy-home-hero")).toBeInTheDocument();
    expect(screen.getByTestId("buddy-world")).toBeInTheDocument();
    expect(screen.getByTestId("buddy-summary-strip")).toBeInTheDocument();
    expect(screen.getByText("Project setup")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Run Setup" }),
    ).toBeInTheDocument();
    expect(await screen.findByTestId("buddy-pulse-card")).toBeInTheDocument();
    expect(screen.getByTestId("buddy-opportunities-feed")).toBeInTheDocument();
    expect(screen.getByText("Model config is broken")).toBeInTheDocument();
    expect(screen.getByTestId("buddy-personality-panel")).toBeInTheDocument();
    expect(screen.getByText("Helper Sprite")).toBeInTheDocument();
    expect(screen.getByTestId("buddy-activity-panel")).toBeInTheDocument();
    expect(screen.getByText("Task coach noticed pattern")).toBeInTheDocument();
    expect(screen.getByTestId("buddy-recent-errors-panel")).toBeInTheDocument();
    expect(screen.getByText("Model unavailable")).toBeInTheDocument();
    expect(await screen.findByText("Recent Buddy chat")).toBeInTheDocument();
  });

  it("shows disabled state and enables Buddy through the settings mutation", async () => {
    let requestBody: unknown;
    const disabledSettings = makeSettings({ enabled: false });
    installBuddyHomeHandlers(disabledSettings);
    server.use(
      http.post("*/v1/buddy/settings", async ({ request }) => {
        requestBody = await request.json();
        return HttpResponse.json(makeSettings({ enabled: true }));
      }),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot({ enabled: false, settings: disabledSettings }),
      ),
    );

    const { user } = render(<BuddyHome />, { store });

    const disabledHome = await screen.findByTestId("buddy-home-disabled");
    expect(disabledHome).toHaveTextContent(
      "Buddy is disabled. Still here, just politely lurking.",
    );

    await user.click(
      within(disabledHome).getByRole("button", { name: "Enable Buddy" }),
    );

    await waitFor(() => {
      expect(requestBody).toEqual({ enabled: true });
    });
  });

  it("shows the loading state before the Buddy snapshot is loaded", () => {
    installBuddyHomeHandlers();

    render(<BuddyHome />, { preloadedState: CONFIG_STATE });

    expect(screen.getByText("Loading Buddy")).toBeInTheDocument();
  });

  it("toggles the settings section from the settings gear", async () => {
    installBuddyHomeHandlers();
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    const { user } = render(<BuddyHome />, { store });

    expect(await screen.findByTestId("buddy-home-content")).toBeInTheDocument();
    expect(
      screen.queryByTestId("buddy-home-settings-section"),
    ).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Settings" }));

    expect(
      await screen.findByTestId("buddy-home-settings-section"),
    ).toBeInTheDocument();
    expect(screen.getByTestId("buddy-settings-panel")).toBeInTheDocument();
  });

  it("resets workspace-scoped Buddy state while preserving seen notifications", () => {
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));
    store.dispatch(markBuddyNotificationSeen("opp-1"));
    store.dispatch(
      beginBuddySettingsRequest({
        requestSeq: 1,
        keys: ["enabled"],
        patch: { enabled: false },
      }),
    );

    store.dispatch(resetBuddyForWorkspaceChange());

    const buddyState = store.getState().buddy;
    expect(buddyState.snapshot).toBeNull();
    expect(buddyState.loaded).toBe(false);
    expect(buddyState.pendingSettingsRequests).toEqual([]);
    expect(buddyState.seenNotificationIds["opp-1"]).toBeGreaterThan(0);
  });

  it("shows memory operation and candidate counters in the pulse card", async () => {
    installBuddyHomeHandlers();
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot({
          pulse: makePulse({
            memory: {
              total: 50,
              orphan: 5,
              stale_conflicts: 1,
              merge_candidates: 2,
              archive_candidates: 3,
              review_candidates: 4,
              conflict_candidates: 1,
              pending_ops: 6,
              applied_ops: 7,
              failed_ops: 1,
            },
          }),
        }),
      ),
    );

    render(<BuddyHome />, { store });

    expect(await screen.findByTestId("buddy-pulse-card")).toHaveTextContent(
      "50 docs · 5 orphan · 1 conflict · 6 pending · 7 applied · 1 failed · 10 candidates",
    );
  });
});
