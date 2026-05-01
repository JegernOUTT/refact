import { render, screen, waitFor } from "../utils/test-utils";
import { http, HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";
import { renderHook } from "@testing-library/react";
import { Provider } from "react-redux";
import { Theme } from "@radix-ui/themes";
import { server } from "../utils/mockServer";
import { setUpStore } from "../app/store";
import {
  setBuddySnapshot,
  setPulse,
  addOpportunity,
  addBuddySuggestion,
} from "../features/Buddy/buddySlice";
import { push } from "../features/Pages/pagesSlice";
import { BuddyPulseCard } from "../features/Buddy/BuddyPulseCard";
import { BuddyOpportunityCard } from "../features/Buddy/BuddyOpportunityCard";
import { BuddyOpportunitiesFeed } from "../features/Buddy/BuddyOpportunitiesFeed";
import { BuddyWorkshop } from "../features/Buddy/BuddyWorkshop";
import { BuddyDraftPreview } from "../features/Buddy/BuddyDraftPreview";
import { BuddySettingsPanel } from "../features/Buddy/BuddySettingsPanel";
import { BuddyPanel } from "../features/Buddy/BuddyPanel";
import { BuddyWorld } from "../features/Buddy/BuddyWorld";
import { buildBuddyWorldState } from "../features/Buddy/buddyWorldModel";
import { buildBuddySceneSpeech } from "../features/Buddy/buddySceneSpeech";
import { useExecuteBuddyAction } from "../features/Buddy/hooks/useExecuteBuddyAction";
import { executeBuddyNavigation } from "../features/Buddy/executeBuddyAction";
import { PALETTES, STAGES } from "../features/Buddy/constants";
import type {
  BuddyOpportunity,
  BuddySuggestion,
  BuddyDraft,
  BuddyPulse,
  BuddyRuntimeEvent,
  BuddySemanticState,
  BuddySnapshot,
} from "../features/Buddy/types";
import type React from "react";

const CONFIG_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

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

function makeSuggestion(overrides?: Partial<BuddySuggestion>): BuddySuggestion {
  return {
    id: "suggestion-1",
    suggestion_type: "quest_start_setup",
    title: "Warm up this workspace",
    description: "Kick off setup so Buddy can help proactively.",
    created_at: "2024-01-01T00:00:00Z",
    dismissed: false,
    controls: [],
    quest: null,
    ...overrides,
  };
}

function makeSemanticState(
  overrides?: Partial<BuddySemanticState>,
): BuddySemanticState {
  return {
    name: "Buddy",
    paletteIndex: 0,
    born: 0,
    mood: {
      happiness: 80,
      energy: 80,
      curiosity: 70,
      anxiety: 0,
      boredom: 10,
      affection: 80,
    },
    personality: {
      playfulness: 70,
      confidence: 60,
      clinginess: 70,
      resilience: 60,
      chaos: 30,
      sociability: 70,
      curiosity: 70,
    },
    progress: { xp: 0, stage: 2 },
    activity: {
      mood: "idle",
      animationType: "idle",
      lastSignalTime: 0,
      lastSignalType: null,
    },
    skills: [],
    log: [],
    ...overrides,
  };
}

function makeSnapshot(pulse?: BuddyPulse): BuddySnapshot {
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
    pulse,
  };
}

describe("BuddyPulseCard_renders_pulse", () => {
  it("shows pulse counts from store", async () => {
    const pulse = makePulse();
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setPulse(pulse));

    server.use(
      http.get("http://127.0.0.1:8001/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities: [] }),
      ),
    );

    render(<BuddyPulseCard />, {
      preloadedState: { ...CONFIG_STATE, buddy: store.getState().buddy },
    });

    await waitFor(() => {
      expect(screen.getByTestId("buddy-pulse-card")).toBeInTheDocument();
    });
    expect(screen.getByText(/3 open/)).toBeInTheDocument();
    expect(screen.getByText(/1 stuck/)).toBeInTheDocument();
    expect(screen.getByText(/2 abandoned/)).toBeInTheDocument();
    expect(screen.getByText(/7 in last hour/)).toBeInTheDocument();
    expect(screen.getByText(/5 files/)).toBeInTheDocument();
  });
});

describe("BuddyOpportunityCard_renders_actions", () => {
  it("each action variant produces a button", () => {
    server.use(
      http.get("http://127.0.0.1:8001/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities: [] }),
      ),
      http.post("http://127.0.0.1:8001/v1/buddy/opportunities/:id/accept", () =>
        HttpResponse.json({ accepted: true }),
      ),
      http.post(
        "http://127.0.0.1:8001/v1/buddy/opportunities/:id/dismiss",
        () => HttpResponse.json({ snapshot: makeSnapshot() }),
      ),
    );

    const opp = makeOpportunity({
      proposed_actions: [
        { kind: "open_page", page: { type: "buddy" } },
        {
          kind: "launch_investigation_chat",
          preload: {
            fact_keys: [],
            diagnostic_ids: [],
            log_excerpt: "",
            config_summary: "",
            initial_user_message: "",
          },
        },
        { kind: "draft_skill", draft_id: "d1", label: "Add Skill" },
        {
          kind: "draft_defaults_change",
          defaults_kind: "chat_model",
          patch: {},
        },
        { kind: "dismiss" },
      ],
    });

    render(<BuddyOpportunityCard opportunity={opp} />, {
      preloadedState: CONFIG_STATE,
    });

    expect(screen.getByText("Open Buddy")).toBeInTheDocument();
    expect(screen.getByText("Investigate")).toBeInTheDocument();
    expect(screen.getByText("Add Skill")).toBeInTheDocument();
    expect(screen.getByText("Adjust defaults")).toBeInTheDocument();
    expect(screen.getByText("Dismiss")).toBeInTheDocument();
  });
});

describe("BuddyOpportunityCard_dismiss_calls_mutation", () => {
  it("clicking Dismiss calls dismiss mutation", async () => {
    let dismissed = false;
    server.use(
      http.get("http://127.0.0.1:8001/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities: [] }),
      ),
      http.post(
        "http://127.0.0.1:8001/v1/buddy/opportunities/:id/dismiss",
        () => {
          dismissed = true;
          return HttpResponse.json({ snapshot: makeSnapshot() });
        },
      ),
    );

    const opp = makeOpportunity({
      id: "opp-dismiss-1",
      proposed_actions: [{ kind: "dismiss" }],
    });

    const { user } = render(<BuddyOpportunityCard opportunity={opp} />, {
      preloadedState: CONFIG_STATE,
    });

    const dismissBtn = screen.getByText("Dismiss");
    await user.click(dismissBtn);

    await waitFor(() => {
      expect(dismissed).toBe(true);
    });
  });
});

describe("BuddyHome_renders_all_sections", () => {
  it("pulse, feed, and workshop are visible when snapshot present", async () => {
    const pulse = makePulse();
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(pulse)));

    server.use(
      http.get("http://127.0.0.1:8001/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities: [] }),
      ),
    );

    render(
      <>
        <BuddyPulseCard />
        <BuddyOpportunitiesFeed />
        <BuddyWorkshop />
      </>,
      {
        preloadedState: { ...CONFIG_STATE, buddy: store.getState().buddy },
      },
    );

    await waitFor(() => {
      expect(screen.getByTestId("buddy-pulse-card")).toBeInTheDocument();
      expect(
        screen.getByTestId("buddy-opportunities-feed"),
      ).toBeInTheDocument();
      expect(screen.getByTestId("buddy-workshop")).toBeInTheDocument();
    });
  });
});

describe("BuddyWorld_dynamic_environment", () => {
  it("renders when matchMedia is unavailable", () => {
    const originalMatchMedia = window.matchMedia;
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: undefined,
    });

    try {
      expect(() =>
        render(
          <BuddyWorld
            palette={PALETTES[0]}
            stage={STAGES[2]}
            state={makeSemanticState()}
            pulse={makePulse()}
            pet={makeSnapshot().state.pet}
            nowPlaying={null}
            activeQuest={null}
            activeSpeech={null}
            setupNeeded={false}
            now={new Date("2024-01-01T14:00:00")}
            onCanvasEvent={vi.fn()}
            onCare={vi.fn()}
            onOpenPage={vi.fn()}
            onRunMode={vi.fn()}
            onDismissSetup={vi.fn()}
            onSpeechControl={vi.fn()}
          />,
          { preloadedState: CONFIG_STATE },
        ),
      ).not.toThrow();
      expect(screen.getByTestId("buddy-world")).toHaveAttribute(
        "data-showcase",
        "none",
      );
    } finally {
      Object.defineProperty(window, "matchMedia", {
        configurable: true,
        value: originalMatchMedia,
      });
    }
  });

  it("builds time-dependent sun and moon phases", () => {
    const morning = buildBuddyWorldState({
      now: new Date("2024-01-01T08:00:00"),
      pulse: makePulse(),
      pet: makeSnapshot().state.pet,
      nowPlaying: null,
      activeQuest: null,
    });
    const night = buildBuddyWorldState({
      now: new Date("2024-01-01T23:00:00"),
      pulse: makePulse(),
      pet: makeSnapshot().state.pet,
      nowPlaying: null,
      activeQuest: null,
    });

    expect(morning.phase).toBe("morning");
    expect(morning.celestialLabel).toBe("Sunrise");
    expect(night.phase).toBe("night");
    expect(night.celestialLabel).toBe("Moon");
  });

  it("turns pulse problems into stormy weather and interactive objects", () => {
    const pulse = makePulse();
    const world = buildBuddyWorldState({
      now: new Date("2024-01-01T14:00:00"),
      pulse,
      pet: makeSnapshot().state.pet,
      nowPlaying: null,
      activeQuest: null,
    });

    expect(world.weather).toBe("storm");
    expect(world.objects.map((item) => item.label)).toEqual(
      expect.arrayContaining([
        "Task grove",
        "Memory fireflies",
        "Model observatory",
        "MCP satellites",
      ]),
    );
  });

  it("renders world controls and routes object clicks", async () => {
    const onCare = vi.fn();
    const onOpenPage = vi.fn();

    const { user } = render(
      <BuddyWorld
        palette={PALETTES[0]}
        stage={STAGES[2]}
        state={makeSemanticState()}
        pulse={makePulse()}
        pet={makeSnapshot().state.pet}
        nowPlaying={null}
        activeQuest={null}
        activeSpeech={null}
        setupNeeded={false}
        now={new Date("2024-01-01T14:00:00")}
        onCanvasEvent={vi.fn()}
        onCare={onCare}
        onOpenPage={onOpenPage}
        onRunMode={vi.fn()}
        onDismissSetup={vi.fn()}
        onSpeechControl={vi.fn()}
      />,
      { preloadedState: CONFIG_STATE },
    );

    expect(screen.getByTestId("buddy-world")).toHaveAttribute(
      "data-phase",
      "day",
    );
    expect(screen.getByTestId("buddy-world")).toHaveAttribute(
      "data-vitality",
      "tangled",
    );
    expect(screen.getByTestId("buddy-world")).toHaveAttribute(
      "data-showcase",
      "none",
    );
    expect(screen.getByTestId("buddy-world-canvas")).toBeInTheDocument();
    expect(screen.getByTestId("buddy-world-character")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /play in sun/i }));
    expect(onCare).toHaveBeenCalledWith("play", "scroll");

    await user.click(screen.getByRole("button", { name: /open buddy home/i }));
    expect(onOpenPage).toHaveBeenCalledWith({ type: "buddy" });

    await user.click(screen.getByRole("button", { name: /open task grove/i }));
    expect(onOpenPage).toHaveBeenCalledWith({ type: "tasks_list" });
  });

  it("keeps active showcase through ordinary headline changes", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2024-01-01T00:00:40Z"));
    try {
      const { rerender } = render(
        <BuddyWorld
          palette={PALETTES[0]}
          stage={STAGES[2]}
          state={makeSemanticState()}
          pulse={{
            ...makePulse(),
            diagnostics: { last_hour: 0, top_error_types: [] },
            git: { uncommitted_files: 0, diff_lines_4h: 0, branches: 3 },
            mcp: { total: 4, failing: 0, auth_expiring: 0 },
            memory: { total: 10, orphan: 0, stale_conflicts: 0 },
          }}
          pet={makeSnapshot().state.pet}
          nowPlaying={{
            id: "runtime-showcase-1",
            signal_type: "memory_extract",
            title: "Memory extracted",
            source: "test",
            status: "completed",
            priority: "normal",
            created_at: "2024-01-01T00:00:00Z",
          }}
          activeQuest={null}
          activeSpeech={null}
          setupNeeded={false}
          now={new Date("2024-01-01T14:00:00")}
          onCanvasEvent={vi.fn()}
          onCare={vi.fn()}
          onOpenPage={vi.fn()}
          onRunMode={vi.fn()}
          onDismissSetup={vi.fn()}
          onSpeechControl={vi.fn()}
        />,
        { preloadedState: CONFIG_STATE },
      );

      await vi.advanceTimersByTimeAsync(1);
      expect(screen.getByTestId("buddy-world")).toHaveAttribute(
        "data-showcase",
        "memory_firefly_night",
      );

      rerender(
        <BuddyWorld
          palette={PALETTES[0]}
          stage={STAGES[2]}
          state={makeSemanticState()}
          pulse={{
            ...makePulse(),
            diagnostics: { last_hour: 0, top_error_types: [] },
            git: { uncommitted_files: 0, diff_lines_4h: 0, branches: 3 },
            mcp: { total: 4, failing: 0, auth_expiring: 0 },
            memory: { total: 10, orphan: 0, stale_conflicts: 4 },
          }}
          pet={makeSnapshot().state.pet}
          nowPlaying={{
            id: "runtime-showcase-1",
            signal_type: "memory_extract",
            title: "Memory extracted",
            source: "test",
            status: "completed",
            priority: "normal",
            created_at: "2024-01-01T00:00:00Z",
          }}
          activeQuest={null}
          activeSpeech={null}
          setupNeeded={false}
          now={new Date("2024-01-01T14:00:00")}
          onCanvasEvent={vi.fn()}
          onCare={vi.fn()}
          onOpenPage={vi.fn()}
          onRunMode={vi.fn()}
          onDismissSetup={vi.fn()}
          onSpeechControl={vi.fn()}
        />,
      );

      expect(screen.getByTestId("buddy-world")).toHaveAttribute(
        "data-showcase",
        "memory_firefly_night",
      );
    } finally {
      vi.useRealTimers();
    }
  });

  it("uses active runtime work as busy weather", () => {
    const runtimeEvent: BuddyRuntimeEvent = {
      id: "rt-1",
      signal_type: "tool_used",
      title: "Running browser checks",
      source: "test",
      status: "progress",
      priority: "normal",
      created_at: "2024-01-01T00:00:00Z",
    };
    const world = buildBuddyWorldState({
      now: new Date("2024-01-01T14:00:00"),
      pulse: makePulse(),
      pet: makeSnapshot().state.pet,
      nowPlaying: runtimeEvent,
      activeQuest: null,
    });

    expect(world.weather).toBe("busy");
    expect(world.weatherDescription).toBe("Running browser checks");
  });
});

describe("buildBuddySceneSpeech", () => {
  function makeRuntimeEvent(
    overrides?: Partial<BuddyRuntimeEvent>,
  ): BuddyRuntimeEvent {
    return {
      id: "runtime-1",
      signal_type: "info",
      title: "Runtime notice",
      source: "test",
      status: "info",
      priority: "normal",
      created_at: "2024-01-01T00:00:00Z",
      ...overrides,
    };
  }

  it("includes runtime descriptions for non-error notifications", () => {
    const speech = buildBuddySceneSpeech({
      activeSpeech: null,
      nowPlaying: makeRuntimeEvent({
        title: "Setup ready",
        description: "Connect GitHub to enable issue sync.",
      }),
      runtimeQueue: [],
      activeSuggestion: null,
    });

    expect(speech?.text).toBe(
      "Setup ready: Connect GitHub to enable issue sync.",
    );
  });

  it("prioritizes critical queued failures over low-priority now playing", () => {
    const speech = buildBuddySceneSpeech({
      activeSpeech: null,
      nowPlaying: makeRuntimeEvent({
        id: "now-playing",
        title: "Indexing quietly",
        priority: "low",
        status: "progress",
        created_at: "2024-01-01T10:00:00Z",
      }),
      runtimeQueue: [
        makeRuntimeEvent({
          id: "critical-error",
          title: "Provider failed",
          description: "The default model key was rejected.",
          priority: "critical",
          status: "failed",
          created_at: "2024-01-01T09:00:00Z",
        }),
      ],
      activeSuggestion: null,
    });

    expect(speech?.runtimeEventId).toBe("critical-error");
    expect(speech?.text).toBe(
      "Provider failed: The default model key was rejected.",
    );
  });

  it("turns repeated context-window errors into Buddy language", () => {
    const speech = buildBuddySceneSpeech({
      activeSpeech: null,
      nowPlaying: makeRuntimeEvent({
        id: "context-error",
        title: "generic: LLM error",
        description:
          "LLM error: Your input exceeds the context window of this model. Please adjust your input and try again. LLM error: Your input exceeds the context window of this model.",
        priority: "high",
        status: "failed",
      }),
      runtimeQueue: [],
      activeSuggestion: null,
    });

    expect(speech?.text).toBe(
      "I ran out of context room. Want me to compress this and try again?",
    );
    expect(speech?.controls.map((control) => control.action)).toEqual([
      "investigate_error",
      "dismiss",
    ]);
  });

  it("converts suggestion dismiss controls into dismiss_suggestion actions", () => {
    const speech = buildBuddySceneSpeech({
      activeSpeech: null,
      nowPlaying: null,
      runtimeQueue: [],
      activeSuggestion: makeSuggestion({
        controls: [
          {
            id: "dismiss-suggestion",
            label: "Dismiss",
            action: "dismiss",
            style: "secondary",
          },
        ],
      }),
    });

    expect(speech?.source).toBe("suggestion");
    expect(speech?.controls[0]).toMatchObject({
      action: "dismiss_suggestion",
      action_param: "suggestion-1",
    });
  });
});

describe("BuddySettingsPanel_local_state_save", () => {
  it("editing input does not update store until Save clicked", async () => {
    server.use(
      http.post("http://127.0.0.1:8001/v1/buddy/settings", () =>
        HttpResponse.json({ enabled: true }),
      ),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    const { user } = render(<BuddySettingsPanel />, {
      preloadedState: { ...CONFIG_STATE, buddy: store.getState().buddy },
    });

    const panel = await screen.findByTestId("buddy-settings-panel");
    expect(panel).toBeInTheDocument();

    const quietSwitch = screen.getByRole("switch", { name: /quiet mode/i });
    const currentChecked = quietSwitch.getAttribute("data-state") === "checked";
    await user.click(quietSwitch);

    const afterClick = quietSwitch.getAttribute("data-state") === "checked";
    expect(afterClick).toBe(!currentChecked);

    const storeSettings = store.getState().buddy.snapshot?.settings;
    expect(storeSettings?.quiet_mode).toBe(currentChecked);
  });
});

describe("BuddyDraftPreview_renders_draft_metadata", () => {
  it("shows title and explanation from draft", () => {
    const draft: BuddyDraft = {
      id: "draft-test",
      kind: "skill",
      title: "My Test Draft Title",
      yaml_or_json: "{}",
      explanation: "This is the explanation text",
      created_at: "2024-01-01T00:00:00Z",
      expires_at: "2099-12-31T00:00:00Z",
    };

    render(<BuddyDraftPreview draft={draft} />, {
      preloadedState: CONFIG_STATE,
    });

    expect(screen.getByText(/My Test Draft Title/)).toBeInTheDocument();
    expect(
      screen.getByText(/This is the explanation text/),
    ).toBeInTheDocument();
  });
});

describe("useExecuteBuddyAction_open_page", () => {
  it("dispatches expected pushPage for open_page action via executeBuddyNavigation", () => {
    const dispatch = vi.fn();
    executeBuddyNavigation({ type: "customization" }, dispatch as never);
    expect(dispatch).toHaveBeenCalledTimes(1);
    const action = dispatch.mock.calls[0][0] as ReturnType<typeof push>;
    expect(action.payload).toMatchObject({ name: "customization" });
  });

  it("useExecuteBuddyAction hook pushes page via buddy navigation", async () => {
    server.use(
      http.get("http://127.0.0.1:8001/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities: [] }),
      ),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    const wrapper = ({ children }: { children: React.ReactNode }) => (
      <Provider store={store}>
        <Theme>{children}</Theme>
      </Provider>
    );

    const { result } = renderHook(() => useExecuteBuddyAction(), { wrapper });
    const execFn = result.current as unknown as (
      a: unknown,
      b: unknown,
      c: number,
    ) => Promise<void>;
    await execFn({ kind: "open_page", page: { type: "buddy" } }, null, -1);

    const pages = store.getState().pages;
    const last = pages[pages.length - 1];
    expect(last.name).toBe("buddy");
  });
});

describe("BuddyPanel_opportunity_notifications", () => {
  it("renders unread opportunities as Buddy speech controls without a badge", async () => {
    server.use(
      http.get("http://127.0.0.1:8001/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities: [] }),
      ),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));
    store.dispatch(
      addOpportunity(
        makeOpportunity({
          status: "new",
          proposed_actions: [{ kind: "dismiss" }],
        }),
      ),
    );

    render(<BuddyPanel />, {
      preloadedState: { ...CONFIG_STATE, buddy: store.getState().buddy },
    });

    expect(screen.queryByTestId("buddy-unread-badge")).not.toBeInTheDocument();
    await waitFor(() => {
      expect(screen.getByText("Dismiss")).toBeInTheDocument();
    });
  });

  it("does not render badge chrome when no unread opportunities", () => {
    server.use(
      http.get("http://127.0.0.1:8001/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities: [] }),
      ),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    render(<BuddyPanel />, {
      preloadedState: { ...CONFIG_STATE, buddy: store.getState().buddy },
    });

    expect(screen.queryByTestId("buddy-unread-badge")).not.toBeInTheDocument();
  });
});

describe("BuddyOpportunitiesFeed_suggestions", () => {
  it("shows active Buddy suggestions when detector opportunities are empty", async () => {
    server.use(
      http.get("http://127.0.0.1:8001/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities: [] }),
      ),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));
    store.dispatch(addBuddySuggestion(makeSuggestion()));

    render(<BuddyOpportunitiesFeed />, {
      preloadedState: { ...CONFIG_STATE, buddy: store.getState().buddy },
    });

    await waitFor(() => {
      expect(screen.getByText("Warm up this workspace")).toBeInTheDocument();
    });
  });
});
