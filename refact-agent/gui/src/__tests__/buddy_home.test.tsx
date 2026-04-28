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
} from "../features/Buddy/buddySlice";
import { push } from "../features/Pages/pagesSlice";
import { BuddyPulseCard } from "../features/Buddy/BuddyPulseCard";
import { BuddyOpportunityCard } from "../features/Buddy/BuddyOpportunityCard";
import { BuddyOpportunitiesFeed } from "../features/Buddy/BuddyOpportunitiesFeed";
import { BuddyWorkshop } from "../features/Buddy/BuddyWorkshop";
import { BuddyDraftPreview } from "../features/Buddy/BuddyDraftPreview";
import { BuddySettingsPanel } from "../features/Buddy/BuddySettingsPanel";
import { BuddyPanel } from "../features/Buddy/BuddyPanel";
import { useExecuteBuddyAction } from "../features/Buddy/hooks/useExecuteBuddyAction";
import { executeBuddyNavigation } from "../features/Buddy/executeBuddyAction";
import type {
  BuddyOpportunity,
  BuddyDraft,
  BuddyPulse,
  BuddySnapshot,
} from "../features/Buddy/types";
import type React from "react";

const CONFIG_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
    addressURL: "Refact",
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
    status: "new",
    proposed_actions: [],
    humor_allowed: false,
    related: { chat_ids: [], task_ids: [], memory_ids: [], config_paths: [] },
    created_at: "2024-01-01T00:00:00Z",
    expires_at: "2099-12-31T00:00:00Z",
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
        HttpResponse.json([]),
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
        HttpResponse.json([]),
      ),
      http.post("http://127.0.0.1:8001/v1/buddy/opportunities/:id/accept", () =>
        HttpResponse.json({ accepted: true }),
      ),
      http.post(
        "http://127.0.0.1:8001/v1/buddy/opportunities/:id/dismiss",
        () => HttpResponse.json({ dismissed: true }),
      ),
      http.post("http://127.0.0.1:8001/v1/buddy/investigations", () =>
        HttpResponse.json({ chat_id: "inv-1" }),
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
        HttpResponse.json([]),
      ),
      http.post(
        "http://127.0.0.1:8001/v1/buddy/opportunities/:id/dismiss",
        () => {
          dismissed = true;
          return HttpResponse.json({ dismissed: true });
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
        HttpResponse.json([]),
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
        HttpResponse.json([]),
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
    ) => Promise<void>;
    await execFn({ kind: "open_page", page: { type: "buddy" } }, null);

    const pages = store.getState().pages;
    const last = pages[pages.length - 1];
    expect(last.name).toBe("buddy");
  });
});

describe("BuddyPanel_unread_badge_appears", () => {
  it("badge is visible when there are unread opportunities", async () => {
    server.use(
      http.get("http://127.0.0.1:8001/v1/buddy/opportunities", () =>
        HttpResponse.json([]),
      ),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));
    store.dispatch(addOpportunity(makeOpportunity({ status: "new" })));

    render(<BuddyPanel />, {
      preloadedState: { ...CONFIG_STATE, buddy: store.getState().buddy },
    });

    await waitFor(() => {
      expect(screen.getByTestId("buddy-unread-badge")).toBeInTheDocument();
    });
  });

  it("badge is not visible when no unread opportunities", async () => {
    server.use(
      http.get("http://127.0.0.1:8001/v1/buddy/opportunities", () =>
        HttpResponse.json([]),
      ),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    render(<BuddyPanel />, {
      preloadedState: { ...CONFIG_STATE, buddy: store.getState().buddy },
    });

    await waitFor(() => {
      expect(
        screen.queryByTestId("buddy-unread-badge"),
      ).not.toBeInTheDocument();
    });
  });
});
