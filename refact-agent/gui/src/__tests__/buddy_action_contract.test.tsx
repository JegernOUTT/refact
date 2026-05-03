import React from "react";
import { Provider } from "react-redux";
import { Theme } from "@radix-ui/themes";
import { renderHook } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "../utils/test-utils";
import { server } from "../utils/mockServer";
import { setUpStore, type AppStore } from "../app/store";
import { BuddyOpportunityCard } from "../features/Buddy/BuddyOpportunityCard";
import { useExecuteBuddyAction } from "../features/Buddy/hooks/useExecuteBuddyAction";
import type {
  BuddyAction,
  BuddyActionResult,
  BuddyOpportunity,
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

function makeSnapshot(name = "Buddy"): BuddySnapshot {
  return {
    state: {
      identity: { name, created_at: "", palette_index: 0 },
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

function acceptResponse(actionResult: BuddyActionResult) {
  return HttpResponse.json({
    snapshot: makeSnapshot("Accepted Snapshot"),
    action_result: actionResult,
  });
}

function renderExecutor() {
  const store = setUpStore({ ...CONFIG_STATE });
  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <Provider store={store}>
      <Theme>{children}</Theme>
    </Provider>
  );
  const { result } = renderHook(() => useExecuteBuddyAction(), { wrapper });
  return { store, execute: result.current };
}

function lastPage(store: AppStore) {
  const pages = store.getState().pages;
  return pages[pages.length - 1];
}

describe("buddy action execution contract", () => {
  it("click_second_action_sends_action_index_1", async () => {
    let requestBody: unknown = null;
    server.use(
      http.post(
        "http://127.0.0.1:8001/v1/buddy/opportunities/:id/accept",
        async ({ request }) => {
          requestBody = await request.json();
          return acceptResponse({
            kind: "open_page",
            navigate_to: { type: "buddy" },
          });
        },
      ),
    );

    const opp = makeOpportunity({
      proposed_actions: [
        { kind: "open_page", page: { type: "buddy" } },
        { kind: "open_page", page: { type: "stats" } },
      ],
    });
    const { user } = render(<BuddyOpportunityCard opportunity={opp} />, {
      preloadedState: CONFIG_STATE,
    });

    await user.click(screen.getByRole("button", { name: "Open Stats" }));

    await waitFor(() => {
      expect(requestBody).toEqual({ action_index: 1 });
    });
  });

  it("accept_response_dispatches_snapshot_to_redux", async () => {
    server.use(
      http.post("http://127.0.0.1:8001/v1/buddy/opportunities/:id/accept", () =>
        HttpResponse.json({
          snapshot: makeSnapshot("Backend Snapshot"),
          action_result: {
            kind: "open_page",
            navigate_to: { type: "buddy" },
          },
        }),
      ),
    );

    const { store, execute } = renderExecutor();
    const action: BuddyAction = { kind: "open_page", page: { type: "buddy" } };
    await execute(action, makeOpportunity({ proposed_actions: [action] }), 0);

    expect(store.getState().buddy.snapshot?.state.identity.name).toBe(
      "Backend Snapshot",
    );
  });

  it("draft_action_with_empty_draft_id_uses_returned_id", async () => {
    server.use(
      http.post("http://127.0.0.1:8001/v1/buddy/opportunities/:id/accept", () =>
        acceptResponse({
          kind: "draft",
          draft_kind: "skill",
          draft_id: "generated-uuid",
          label: "Generated Skill",
        }),
      ),
    );

    const { store, execute } = renderExecutor();
    const action: BuddyAction = {
      kind: "draft_skill",
      draft_id: "",
      label: "Draft Skill",
    };
    await execute(action, makeOpportunity({ proposed_actions: [action] }), 0);

    expect(lastPage(store)).toMatchObject({
      name: "extensions",
      tab: "skills",
      draftId: "generated-uuid",
    });
  });

  it("open_page_action_navigates_using_returned_navigate_to", async () => {
    server.use(
      http.post("http://127.0.0.1:8001/v1/buddy/opportunities/:id/accept", () =>
        acceptResponse({
          kind: "open_page",
          navigate_to: { type: "stats" },
        }),
      ),
    );

    const { store, execute } = renderExecutor();
    const beforePages = store.getState().pages.length;
    const action: BuddyAction = { kind: "open_page", page: { type: "buddy" } };
    await execute(action, makeOpportunity({ proposed_actions: [action] }), 0);

    expect(store.getState().pages.length - beforePages).toBe(1);
    expect(lastPage(store)).toMatchObject({ name: "stats dashboard" });
  });

  it("dismiss_action_uses_dismiss_route_not_accept", async () => {
    let acceptCalled = false;
    let dismissCalled = false;
    server.use(
      http.post(
        "http://127.0.0.1:8001/v1/buddy/opportunities/:id/accept",
        () => {
          acceptCalled = true;
          return acceptResponse({ kind: "dismiss" });
        },
      ),
      http.post(
        "http://127.0.0.1:8001/v1/buddy/opportunities/:id/dismiss",
        () => {
          dismissCalled = true;
          return HttpResponse.json({
            snapshot: makeSnapshot("Dismiss Snapshot"),
          });
        },
      ),
    );

    const { store, execute } = renderExecutor();
    const action: BuddyAction = { kind: "dismiss" };
    await execute(action, makeOpportunity({ proposed_actions: [action] }), 0);

    expect(dismissCalled).toBe(true);
    expect(acceptCalled).toBe(false);
    expect(store.getState().buddy.snapshot?.state.identity.name).toBe(
      "Dismiss Snapshot",
    );
  });

  it("failed_marketplace_install_shows_error_and_stays_retryable", async () => {
    let acceptCalls = 0;
    server.use(
      http.post(
        "http://127.0.0.1:8001/v1/buddy/opportunities/:id/accept",
        () => {
          acceptCalls += 1;
          return HttpResponse.json(
            { detail: "marketplace_install_failed: denied" },
            { status: 502 },
          );
        },
      ),
    );

    const action: BuddyAction = {
      kind: "offer_marketplace_install",
      market_kind: "mcp",
      item_id: "github",
    };
    const opp = makeOpportunity({ proposed_actions: [action] });
    const { user } = render(<BuddyOpportunityCard opportunity={opp} />, {
      preloadedState: CONFIG_STATE,
    });

    const button = screen.getByRole("button", { name: "Install MCP" });
    await user.click(button);

    await waitFor(() => {
      expect(screen.getByRole("alert")).toHaveTextContent(
        "marketplace_install_failed",
      );
    });
    expect(button).toBeEnabled();
    await user.click(button);

    await waitFor(() => {
      expect(acceptCalls).toBe(2);
    });
  });

  it("dismiss_failure_shows_error_and_keeps_button_visible", async () => {
    server.use(
      http.post(
        "http://127.0.0.1:8001/v1/buddy/opportunities/:id/dismiss",
        () => HttpResponse.json({ detail: "dismiss failed" }, { status: 409 }),
      ),
    );

    const action: BuddyAction = { kind: "dismiss" };
    const opp = makeOpportunity({ proposed_actions: [action] });
    const { user } = render(<BuddyOpportunityCard opportunity={opp} />, {
      preloadedState: CONFIG_STATE,
    });

    const button = screen.getByRole("button", { name: "Dismiss" });
    await user.click(button);

    await waitFor(() => {
      expect(screen.getByRole("alert")).toHaveTextContent("dismiss failed");
    });
    expect(button).toBeEnabled();
  });

  it("successful_marketplace_install_navigates_to_marketplace_hub", async () => {
    server.use(
      http.post("http://127.0.0.1:8001/v1/buddy/opportunities/:id/accept", () =>
        acceptResponse({
          kind: "marketplace_install",
          market_kind: "mcp",
          item_id: "github",
          success: true,
          error: null,
        }),
      ),
    );

    const { store, execute } = renderExecutor();
    const action: BuddyAction = {
      kind: "offer_marketplace_install",
      market_kind: "mcp",
      item_id: "github",
    };
    await execute(action, makeOpportunity({ proposed_actions: [action] }), 0);

    expect(lastPage(store)).toMatchObject({ name: "marketplace hub" });
  });

  it("accept_failure_surfaces_error_does_not_navigate", async () => {
    const consoleError = vi
      .spyOn(console, "error")
      .mockImplementation(() => undefined);
    server.use(
      http.post("http://127.0.0.1:8001/v1/buddy/opportunities/:id/accept", () =>
        HttpResponse.text("action_not_implemented", { status: 501 }),
      ),
    );

    const { store, execute } = renderExecutor();
    const action: BuddyAction = {
      kind: "offer_marketplace_install",
      market_kind: "mcp",
      item_id: "server-1",
    };
    await expect(
      execute(action, makeOpportunity({ proposed_actions: [action] }), 0),
    ).rejects.toBeTruthy();

    expect(lastPage(store)).toMatchObject({ name: "login page" });
    consoleError.mockRestore();
  });

  it("workshop_action_with_null_opp_executes_locally", async () => {
    let acceptCalled = false;
    server.use(
      http.post(
        "http://127.0.0.1:8001/v1/buddy/opportunities/:id/accept",
        () => {
          acceptCalled = true;
          return acceptResponse({
            kind: "open_page",
            navigate_to: { type: "buddy" },
          });
        },
      ),
    );

    const { store, execute } = renderExecutor();
    await execute({ kind: "open_page", page: { type: "stats" } }, null, -1);

    expect(lastPage(store)).toMatchObject({ name: "stats dashboard" });
    expect(acceptCalled).toBe(false);
  });
});
