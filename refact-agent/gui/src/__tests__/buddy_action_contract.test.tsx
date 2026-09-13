import React from "react";
import { Provider } from "react-redux";
import { Theme } from "@radix-ui/themes";
import { renderHook } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "../utils/test-utils";
import { server } from "../utils/mockServer";
import { setUpStore, type AppStore } from "../app/store";
import { BuddyOpportunityCard } from "../features/Buddy/BuddyOpportunityCard";
import { BuddyChatCompanion } from "../features/Buddy/BuddyChatCompanion";
import { setBuddySnapshot } from "../features/Buddy/buddySlice";
import { useExecuteBuddyAction } from "../features/Buddy/hooks/useExecuteBuddyAction";
import { executeBuddyAction } from "../features/Buddy/executeBuddyAction";
import type {
  BuddyAction,
  BuddyActionResult,
  BuddyControl,
  BuddyOpportunity,
  BuddyRuntimeEvent,
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

function makeSnapshot(
  name = "Buddy",
  overrides?: Partial<BuddySnapshot>,
): BuddySnapshot {
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

function makeRuntimeEvent(
  overrides?: Partial<BuddyRuntimeEvent>,
): BuddyRuntimeEvent {
  return {
    id: "runtime-1",
    signal_type: "chat_error",
    title: "Runtime failed",
    source: "chat",
    status: "failed",
    priority: "high",
    created_at: "2024-01-01T00:00:00Z",
    chat_id: "chat-a",
    ...overrides,
  };
}

function acceptResponse(actionResult: BuddyActionResult) {
  return HttpResponse.json({
    snapshot: makeSnapshot("Accepted Snapshot"),
    action_result: actionResult,
  });
}

function makeNoopCanvasContext(): CanvasRenderingContext2D {
  const noopCanvasContext = {
    clearRect: vi.fn(),
    fillRect: vi.fn(),
    fillText: vi.fn(),
    getImageData: vi.fn(
      () => ({ data: new Uint8ClampedArray(4) }) as ImageData,
    ),
    putImageData: vi.fn(),
    restore: vi.fn(),
    save: vi.fn(),
    scale: vi.fn(),
    translate: vi.fn(),
    beginPath: vi.fn(),
    clip: vi.fn(),
    arc: vi.fn(),
    ellipse: vi.fn(),
    fill: vi.fn(),
    moveTo: vi.fn(),
    lineTo: vi.fn(),
    stroke: vi.fn(),
    imageSmoothingEnabled: false,
    globalAlpha: 1,
    fillStyle: "#000000",
    strokeStyle: "#000000",
    lineWidth: 1,
    lineCap: "butt" as CanvasLineCap,
    font: "",
    textAlign: "center" as CanvasTextAlign,
    textBaseline: "top" as CanvasTextBaseline,
  } satisfies Partial<CanvasRenderingContext2D>;
  return noopCanvasContext as unknown as CanvasRenderingContext2D;
}

function setupCompanionRender() {
  vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
    window.setTimeout(() => callback(0), 0);
    return 1;
  });
  vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => undefined);
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(
    makeNoopCanvasContext(),
  );
}

function setupCompanionApiHandlers() {
  server.use(
    http.get("*/v1/buddy/opportunities", () =>
      HttpResponse.json({ opportunities: [] }),
    ),
    http.post("*/v1/buddy/runtime/:id/dismiss", () =>
      HttpResponse.json({ dismissed: true }),
    ),
    http.post("*/v1/buddy/conversations", () =>
      HttpResponse.json({
        chat_id: "buddy-investigation-chat",
        title: "Buddy investigation",
        created_at: "2024-01-01T00:00:00Z",
        last_message_at: null,
        message_count: 0,
      }),
    ),
    http.post("*/v1/buddy/investigation-context", () =>
      HttpResponse.json({
        logs: "logs",
        internal_context: "context",
        repo_owner: "smallcloudai",
        repo_name: "refact",
      }),
    ),
    http.post("*/v1/chats/:id/commands", () => HttpResponse.json({ ok: true })),
  );
}

function pinCompanionTime() {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  vi.setSystemTime(new Date("2024-01-01T00:00:00Z"));
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
  beforeEach(() => {
    localStorage.clear();
    vi.restoreAllMocks();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  const pulseControl: BuddyControl = {
    id: "worktree-report",
    label: "Create Worktrees pulse report",
    action: "create_worktrees_pulse",
    style: "primary",
  };
  const pulseContext = {
    triggerText: "Worktrees need attention",
    triggerSource: "suggestion" as const,
    suggestionId: "worktree-suggestion",
  };

  function installPulseHandlers(options?: {
    enabled?: boolean;
    readOnly?: boolean;
    failure?: "settings" | "pulse" | "draft" | "missing-id";
  }) {
    const calls = { pulse: 0, draft: 0, resolved: 0, body: null as unknown };
    server.use(
      http.get("*/v1/buddy/settings", () =>
        options?.failure === "settings"
          ? HttpResponse.json({ detail: "settings offline" }, { status: 503 })
          : HttpResponse.json({
              ...makeSnapshot().settings,
              enabled: options?.enabled ?? true,
              autonomy_level: options?.readOnly ? "read_only" : "suggest",
            }),
      ),
      http.get("*/v1/buddy/pulse", () => {
        calls.pulse += 1;
        return options?.failure === "pulse"
          ? HttpResponse.json({ detail: "pulse offline" }, { status: 503 })
          : HttpResponse.json({
              generated_at: "2026-09-05T00:00:00Z",
              worktrees: {
                total: 3,
                dirty: 1,
                clean: 2,
                disk_usage_bytes: null,
              },
              diagnostics: { private_text: "must not appear in report" },
            });
      }),
      http.post("*/v1/buddy/drafts/pulse_report", async ({ request }) => {
        calls.draft += 1;
        calls.body = await request.json();
        return options?.failure === "draft"
          ? HttpResponse.json({ detail: "draft offline" }, { status: 503 })
          : HttpResponse.json({
              id: options?.failure === "missing-id" ? " " : "worktree-draft",
              kind: "pulse_report",
            });
      }),
      http.post("*/v1/buddy/suggestions/:id/dismiss", () => {
        calls.resolved += 1;
        return HttpResponse.json({ dismissed: true });
      }),
    );
    return calls;
  }

  it("creates a worktree report draft before opening it and resolving the suggestion", async () => {
    const calls = installPulseHandlers();
    const store = setUpStore(CONFIG_STATE);
    const outcome = await executeBuddyAction(
      pulseControl,
      store.dispatch,
      pulseContext,
    );

    expect(outcome).toEqual({ status: "ok" });
    expect(calls.pulse).toBe(1);
    expect(calls.draft).toBe(1);
    expect(calls.resolved).toBe(1);
    expect(calls.body).toEqual({
      title: "Worktrees pulse report",
      yaml_or_json:
        "# Worktrees pulse report\n\nPulse generated at 2026-09-05T00:00:00Z.\n\n- Worktrees total: 3\n- Clean: 2\n- Dirty: 1",
      explanation: "Worktree counters read from the current Buddy pulse.",
    });
    expect(lastPage(store)).toMatchObject({
      name: "buddy",
      draftId: "worktree-draft",
    });
  });

  it.each(["settings", "pulse", "draft", "missing-id"] as const)(
    "keeps the suggestion unresolved and reports %s failure without navigation",
    async (failure) => {
      const calls = installPulseHandlers({ failure });
      const store = setUpStore(CONFIG_STATE);
      const before = store.getState().pages;
      const outcome = await executeBuddyAction(
        pulseControl,
        store.dispatch,
        pulseContext,
      );

      expect(outcome.status).toBe("failed");
      expect(calls.resolved).toBe(0);
      expect(calls.draft).toBe(
        failure === "settings" || failure === "pulse" ? 0 : 1,
      );
      expect(store.getState().pages).toEqual(before);
      expect(store.getState().buddy.activeSpeech?.text).toContain(
        "Could not create",
      );
    },
  );

  it.each([{ readOnly: true }, { enabled: false }])(
    "does not create drafts when current settings forbid it: %j",
    async (settings) => {
      const calls = installPulseHandlers(settings);
      const store = setUpStore(CONFIG_STATE);
      const outcome = await executeBuddyAction(
        pulseControl,
        store.dispatch,
        pulseContext,
      );

      expect(outcome.status).toBe("unsupported");
      expect(calls.pulse).toBe(0);
      expect(calls.draft).toBe(0);
      expect(calls.resolved).toBe(0);
      expect(store.getState().buddy.activeSpeech?.text).toContain(
        "allowed to create drafts",
      );
    },
  );

  it.each([
    "unsupported-control",
    "dismiss_suggestion",
    "dismiss_runtime_event",
    "accept_quest",
  ])(
    "reports unavailable or malformed %s without falsely resolving the suggestion",
    async (action) => {
      const calls = installPulseHandlers();
      const store = setUpStore(CONFIG_STATE);
      const outcome = await executeBuddyAction(
        { ...pulseControl, action },
        store.dispatch,
        pulseContext,
      );
      expect(outcome.status).toBe("unsupported");
      expect(calls.resolved).toBe(0);
      expect(store.getState().buddy.activeSpeech?.speech_intent).toBe(
        "error_alert",
      );
    },
  );

  it("supports dismiss_speech without resolving the underlying suggestion", async () => {
    const calls = installPulseHandlers();
    const store = setUpStore(CONFIG_STATE);
    expect(
      await executeBuddyAction(
        { ...pulseControl, action: "dismiss_speech" },
        store.dispatch,
        pulseContext,
      ),
    ).toEqual({ status: "ok" });
    expect(calls.resolved).toBe(0);
    expect(store.getState().buddy.activeSpeech).toBeNull();
  });

  it("click_second_action_sends_action_index_1", async () => {
    let requestBody: unknown = null;
    server.use(
      http.post("*/v1/buddy/opportunities/:id/accept", async ({ request }) => {
        requestBody = await request.json();
        return acceptResponse({
          kind: "open_page",
          navigate_to: { type: "buddy" },
        });
      }),
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
      http.post("*/v1/buddy/opportunities/:id/accept", () =>
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
      http.post("*/v1/buddy/opportunities/:id/accept", () =>
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
      http.post("*/v1/buddy/opportunities/:id/accept", () =>
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
      http.post("*/v1/buddy/opportunities/:id/accept", () => {
        acceptCalled = true;
        return acceptResponse({ kind: "dismiss" });
      }),
      http.post("*/v1/buddy/opportunities/:id/dismiss", () => {
        dismissCalled = true;
        return HttpResponse.json({
          snapshot: makeSnapshot("Dismiss Snapshot"),
        });
      }),
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
      http.post("*/v1/buddy/opportunities/:id/accept", () => {
        acceptCalls += 1;
        return HttpResponse.json(
          { detail: "marketplace_install_failed: denied" },
          { status: 502 },
        );
      }),
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
      http.post("*/v1/buddy/opportunities/:id/dismiss", () =>
        HttpResponse.json({ detail: "dismiss failed" }, { status: 409 }),
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

  it("chat_companion_failed_accept_keeps_notification_visible", async () => {
    setupCompanionRender();
    pinCompanionTime();
    const opportunity = makeOpportunity({
      id: "opp-companion-accept-fails",
      proposed_actions: [{ kind: "open_page", page: { type: "buddy" } }],
    });
    server.use(
      http.get("*/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities: [opportunity] }),
      ),
      http.post("*/v1/buddy/opportunities/:id/accept", () =>
        HttpResponse.json({ detail: "accept failed" }, { status: 500 }),
      ),
    );
    const store = setUpStore();
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot("Companion", { opportunities: [opportunity] }),
      ),
    );

    const { container } = render(<BuddyChatCompanion chatId="chat-a" />, {
      store,
    });
    await waitFor(() => {
      expect(
        container.querySelector(
          '[data-notification-id="opportunity:opp-companion-accept-fails"]',
        ),
      ).not.toBeNull();
    });
    const button = await screen.findByRole("button", {
      name: "Open Companion",
    });
    fireEvent.click(button);

    await waitFor(() => {
      expect(
        container.querySelector(
          '[data-notification-id="opportunity:opp-companion-accept-fails"]',
        ),
      ).not.toBeNull();
      expect(screen.getByText(/accept failed/i)).toBeInTheDocument();
    });
  });

  it("chat companion keeps a failed pulse report notification and shows its error", async () => {
    setupCompanionRender();
    setupCompanionApiHandlers();
    pinCompanionTime();
    const calls = installPulseHandlers({ failure: "draft" });
    const store = setUpStore(CONFIG_STATE);
    const runtime = makeRuntimeEvent({
      id: "runtime-pulse-fails",
      controls: [pulseControl],
    });
    store.dispatch(
      setBuddySnapshot(makeSnapshot("Companion", { runtime_queue: [runtime] })),
    );
    const { container } = render(<BuddyChatCompanion chatId="chat-a" />, {
      store,
    });

    fireEvent.click(
      await screen.findByRole("button", { name: pulseControl.label }),
    );

    await waitFor(() => {
      expect(
        screen.getByText(/Could not create the worktrees pulse report/),
      ).toBeInTheDocument();
    });
    expect(
      container.querySelector(
        '[data-notification-id="runtime:runtime-pulse-fails"]',
      ),
    ).not.toBeNull();
    expect(calls.draft).toBe(1);
    expect(calls.resolved).toBe(0);
    expect(lastPage(store)).not.toMatchObject({
      name: "buddy",
      draftId: "worktree-draft",
    });
  });

  it("guards repeated quest acceptance and retains the notification on failure", async () => {
    setupCompanionRender();
    setupCompanionApiHandlers();
    pinCompanionTime();
    let calls = 0;
    let finish: (() => void) | undefined;
    const responseReady = new Promise<void>((resolve) => {
      finish = resolve;
    });
    server.use(
      http.post("*/v1/buddy/quest/accept", async () => {
        calls += 1;
        await responseReady;
        return HttpResponse.json({ detail: "quest offline" }, { status: 503 });
      }),
    );
    const store = setUpStore(CONFIG_STATE);
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot("Companion", {
          runtime_queue: [
            makeRuntimeEvent({
              id: "runtime-quest-fails",
              controls: [
                {
                  id: "quest",
                  label: "Accept quest",
                  action: "accept_quest",
                  action_param: "quest-id",
                  style: "primary",
                },
              ],
            }),
          ],
        }),
      ),
    );
    const { container } = render(<BuddyChatCompanion chatId="chat-a" />, {
      store,
    });
    const button = await screen.findByRole("button", { name: "Accept quest" });
    fireEvent.click(button);
    fireEvent.click(button);
    await waitFor(() => expect(calls).toBe(1));
    finish?.();
    await waitFor(() =>
      expect(screen.getByText(/quest offline/)).toBeInTheDocument(),
    );
    expect(
      container.querySelector(
        '[data-notification-id="runtime:runtime-quest-fails"]',
      ),
    ).not.toBeNull();
    expect(calls).toBe(1);
  });

  it("chat_companion_failed_dismiss_keeps_notification_visible", async () => {
    setupCompanionRender();
    pinCompanionTime();
    const opportunity = makeOpportunity({
      id: "opp-companion-dismiss-fails",
      proposed_actions: [{ kind: "dismiss" }],
    });
    server.use(
      http.get("*/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities: [opportunity] }),
      ),
      http.post("*/v1/buddy/opportunities/:id/dismiss", () =>
        HttpResponse.json({ detail: "dismiss failed" }, { status: 409 }),
      ),
    );
    const store = setUpStore();
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot("Companion", { opportunities: [opportunity] }),
      ),
    );

    const { container } = render(<BuddyChatCompanion chatId="chat-a" />, {
      store,
    });
    await waitFor(() => {
      expect(
        container.querySelector(
          '[data-notification-id="opportunity:opp-companion-dismiss-fails"]',
        ),
      ).not.toBeNull();
    });
    const button = await screen.findByRole("button", { name: "Dismiss" });
    fireEvent.click(button);

    await waitFor(() => {
      expect(
        container.querySelector(
          '[data-notification-id="opportunity:opp-companion-dismiss-fails"]',
        ),
      ).not.toBeNull();
      expect(screen.getByText(/dismiss failed/i)).toBeInTheDocument();
    });
  });

  it("runtime_investigation_starts_when_runtime_dismiss_fails", async () => {
    setupCompanionRender();
    setupCompanionApiHandlers();
    pinCompanionTime();
    let conversationStarted = false;
    server.use(
      http.post("*/v1/buddy/runtime/:id/dismiss", () =>
        HttpResponse.json({ detail: "offline" }, { status: 503 }),
      ),
      http.post("*/v1/buddy/conversations", () => {
        conversationStarted = true;
        return HttpResponse.json({
          chat_id: "buddy-investigation-chat",
          title: "Buddy investigation",
          created_at: "2024-01-01T00:00:00Z",
          last_message_at: null,
          message_count: 0,
        });
      }),
    );
    const store = setUpStore(CONFIG_STATE);
    const runtime = makeRuntimeEvent({
      id: "runtime-investigate-dismiss-fails",
      controls: [
        {
          id: "investigate-runtime",
          label: "Investigate",
          action: "investigate_error",
          style: "primary",
        },
      ],
    });
    store.dispatch(
      setBuddySnapshot(makeSnapshot("Companion", { runtime_queue: [runtime] })),
    );

    render(<BuddyChatCompanion chatId="chat-a" />, { store });
    const button = await screen.findByRole("button", { name: "Investigate" });
    fireEvent.click(button);

    await waitFor(() => {
      expect(conversationStarted).toBe(true);
    });
  });

  it("runtime_dismiss_failure_is_handled", async () => {
    setupCompanionRender();
    setupCompanionApiHandlers();
    pinCompanionTime();
    server.use(
      http.post("*/v1/buddy/runtime/:id/dismiss", () =>
        HttpResponse.json({ detail: "offline" }, { status: 503 }),
      ),
    );
    const unhandled = vi.fn();
    window.addEventListener("unhandledrejection", unhandled);
    const store = setUpStore();
    const runtime = makeRuntimeEvent({
      id: "runtime-dismiss-fails",
      controls: [
        {
          id: "dismiss-runtime",
          label: "Dismiss",
          action: "dismiss_runtime_event",
          action_param: "runtime-dismiss-fails",
          style: "ghost",
        },
      ],
    });
    store.dispatch(
      setBuddySnapshot(makeSnapshot("Companion", { runtime_queue: [runtime] })),
    );

    try {
      const { container } = render(<BuddyChatCompanion chatId="chat-a" />, {
        store,
      });
      await waitFor(() => {
        expect(
          container.querySelector(
            '[data-notification-id="runtime:runtime-dismiss-fails"]',
          ),
        ).not.toBeNull();
      });
      const button = await screen.findByRole("button", {
        name: "Dismiss",
        hidden: true,
      });
      fireEvent.click(button);

      await waitFor(() => {
        expect(
          container.querySelector(
            '[data-notification-id="runtime:runtime-dismiss-fails"]',
          ),
        ).toBeNull();
      });
      await new Promise((resolve) => window.setTimeout(resolve, 0));
      expect(unhandled).not.toHaveBeenCalled();
    } finally {
      window.removeEventListener("unhandledrejection", unhandled);
    }
  });

  it("successful_marketplace_install_navigates_to_marketplace_hub", async () => {
    server.use(
      http.post("*/v1/buddy/opportunities/:id/accept", () =>
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
      http.post("*/v1/buddy/opportunities/:id/accept", () =>
        HttpResponse.text("action_not_implemented", { status: 501 }),
      ),
    );

    const { store, execute } = renderExecutor();
    const action: BuddyAction = {
      kind: "offer_marketplace_install",
      market_kind: "mcp",
      item_id: "server-1",
    };
    const result = await execute(
      action,
      makeOpportunity({ proposed_actions: [action] }),
      0,
    );
    expect(result.status).toBe("failed");
    expect("message" in result && typeof result.message === "string").toBe(
      true,
    );

    expect(lastPage(store)).toMatchObject({ name: "login page" });
    consoleError.mockRestore();
  });

  it("workshop_action_with_null_opp_executes_locally", async () => {
    let acceptCalled = false;
    server.use(
      http.post("*/v1/buddy/opportunities/:id/accept", () => {
        acceptCalled = true;
        return acceptResponse({
          kind: "open_page",
          navigate_to: { type: "buddy" },
        });
      }),
    );

    const { store, execute } = renderExecutor();
    await execute({ kind: "open_page", page: { type: "stats" } }, null, -1);

    expect(lastPage(store)).toMatchObject({ name: "stats dashboard" });
    expect(acceptCalled).toBe(false);
  });

  it("preserves partial memory effects instead of reporting success", async () => {
    server.use(
      http.post("*/v1/buddy/opportunities/:id/accept", () =>
        acceptResponse({
          kind: "memory_batch_applied",
          batch_key: "batch",
          applied: 2,
          failed: 1,
          remaining: 3,
        }),
      ),
    );
    const { store, execute } = renderExecutor();
    const action: BuddyAction = { kind: "open_page", page: { type: "buddy" } };
    const outcome = await execute(
      action,
      makeOpportunity({ proposed_actions: [action] }),
      0,
    );

    expect(outcome).toMatchObject({
      status: "partial",
      applied: 2,
      failed: 1,
      remaining: 3,
    });
    expect(lastPage(store)).toMatchObject({ name: "login page" });
  });

  it("does not apply a stale opportunity response snapshot or navigation", async () => {
    server.use(
      http.post("*/v1/buddy/opportunities/:id/accept", () =>
        acceptResponse({
          kind: "open_page",
          navigate_to: { type: "marketplace_hub" },
        }),
      ),
    );
    const { store, execute } = renderExecutor();
    const snapshot = store.getState().buddy.snapshot;
    const action: BuddyAction = { kind: "open_page", page: { type: "buddy" } };
    const outcome = await execute(
      action,
      makeOpportunity({ proposed_actions: [action] }),
      0,
      undefined,
      () => false,
    );

    expect(outcome).toEqual({ status: "ignored", reason: "stale" });
    expect(store.getState().buddy.snapshot).toEqual(snapshot);
    expect(lastPage(store)).toMatchObject({ name: "login page" });
  });
});
