import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "../../../../utils/test-utils";
import { http, HttpResponse } from "msw";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { server } from "../../../../utils/mockServer";
import { setUpStore } from "../../../../app/store";
import {
  setBuddySnapshot,
  clearNowPlaying,
  dequeueRuntimeEvent,
} from "../../buddySlice";
import { BuddyHome } from "../../BuddyHome";
import { BuddyActivityPanel } from "../../BuddyActivityPanel";
import { BuddyRecentErrorsPanel } from "../../BuddyRecentErrorsPanel";
import { buddyFailureLabel, resetBuddyPersistentState } from "../../ui";
import type {
  BuddyPulse,
  BuddyRuntimeEvent,
  BuddySnapshot,
  BuddyActivityEntry,
  DiagnosticContext,
} from "../../types";
import { act } from "react-dom/test-utils";

const operatorStatus = (
  mode: "disabled" | "enabled" | "unavailable",
  unavailableReason?: {
    kind: string;
    detail: string;
    retry_in_seconds?: number;
  },
) => ({
  schema_version: 1,
  mode,
  features: {
    storage: mode !== "disabled",
    actions: false,
    supervision: false,
    improvement: false,
    editorial: false,
  },
  project:
    mode === "enabled"
      ? { project_id: "fixture", canonical_root: "/fixture" }
      : null,
  worker:
    mode === "enabled"
      ? { worker_id: "worker", engine_instance_id: "engine", epoch: 1 }
      : null,
  ...(mode === "unavailable" && unavailableReason
    ? { unavailable_reason: unavailableReason }
    : {}),
});

// Existing legacy tests explicitly exercise the disabled compatibility contract.
afterEach(() => resetBuddyPersistentState());

beforeEach(() => {
  server.use(
    http.get("*/v1/buddy/operator/status", () =>
      HttpResponse.json(operatorStatus("disabled")),
    ),
  );
});

vi.mock("../../BuddyCharacter", async () => {
  const ReactModule = await vi.importActual<typeof import("react")>("react");

  return {
    BuddyCharacter: ({
      bubblePosition,
      randomizeBubblePosition,
      compactBubble,
      scenePose = "idle",
      sceneXPercent,
      sceneYPercent,
      sceneDepthScale,
      speechText,
      speechControls,
      onSpeechControl,
    }: {
      bubblePosition?: string;
      randomizeBubblePosition?: boolean;
      compactBubble?: boolean;
      scenePose?: string;
      sceneXPercent?: number;
      sceneYPercent?: number;
      sceneDepthScale?: number;
      speechText?: string | null;
      speechControls?: {
        id: string;
        label: string;
        action: string;
        action_param?: string;
        style: string;
      }[];
      onSpeechControl?: (control: {
        id: string;
        label: string;
        action: string;
        action_param?: string;
        style: string;
      }) => void;
    }) =>
      ReactModule.createElement(
        "div",
        {
          "data-bubble-position": bubblePosition,
          "data-depth-scale": sceneDepthScale,
          "data-compact-bubble": String(compactBubble),
          "data-pose": scenePose,
          "data-randomize-bubble-position": String(randomizeBubblePosition),
          "data-testid": "buddy-world-character",
          style:
            typeof sceneXPercent === "number" ||
            typeof sceneYPercent === "number"
              ? {
                  ...(typeof sceneXPercent === "number"
                    ? { left: `${sceneXPercent}%` }
                    : undefined),
                  ...(typeof sceneYPercent === "number"
                    ? { bottom: `${100 - sceneYPercent}%` }
                    : undefined),
                }
              : undefined,
        },
        speechText,
        ...(speechControls?.map((control) =>
          ReactModule.createElement(
            "button",
            {
              key: control.id,
              type: "button",
              onClick: () => onSpeechControl?.(control),
            },
            control.label,
          ),
        ) ?? []),
      ),
  };
});

const CONFIG_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

function readGuiSource(path: string): Promise<string> {
  return readFile(resolve(process.cwd(), "src", path), "utf8");
}

function makeActivity(
  overrides?: Partial<BuddyActivityEntry>,
): BuddyActivityEntry {
  return {
    icon: "⚙️",
    title: "Activity",
    description: "Activity description",
    timestamp: "2024-01-01T00:00:00Z",
    activity_type: "buddy_memory_garden",
    chat_id: null,
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

function makeSnapshot(
  pulse?: BuddyPulse,
  overrides?: Partial<BuddySnapshot>,
): BuddySnapshot {
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
    pulse,
    ...overrides,
  };
}

describe("ActivityFeed_filter_chips_filter_by_workflow_prefix", () => {
  it("filters activities by workflow prefix", async () => {
    const { user } = render(
      <BuddyActivityPanel
        activities={[
          makeActivity({
            title: "Compile sniffer",
            activity_type: "refact_compile_sniffer",
          }),
          makeActivity({
            title: "Memory garden",
            activity_type: "buddy_memory_garden",
          }),
        ]}
      />,
      { preloadedState: CONFIG_STATE },
    );

    await user.click(screen.getByRole("radio", { name: "Agent checks" }));

    expect(screen.getByText("Compile sniffer")).toBeInTheDocument();
    expect(screen.queryByText("Memory garden")).not.toBeInTheDocument();

    await user.click(screen.getByRole("radio", { name: "Buddy workflows" }));

    expect(screen.queryByText("Compile sniffer")).not.toBeInTheDocument();
    expect(screen.getByText("Memory garden")).toBeInTheDocument();
  });
});

describe("BuddyWorkflowFailureSummaries", () => {
  it("formats failure category labels with the shared Buddy label map", () => {
    expect(buddyFailureLabel("model_unavailable")).toBe("Model unavailable");
    expect(buddyFailureLabel("MODEL_UNAVAILABLE")).toBe("Model unavailable");
    expect(buddyFailureLabel(" context_too_large ")).toBe("Context too large");
    expect(buddyFailureLabel(" ")).toBe("Other");
  });

  it("shows structured failure categories in activity and recent error panels", () => {
    render(
      <>
        <BuddyActivityPanel
          activities={[
            makeActivity({
              title: "Dependency radar failed: Model unavailable",
              description: "OpenAI 404: model not found",
              failure_category: "model_unavailable",
              failure_summary:
                "Model unavailable — check Buddy/default model settings.",
            }),
          ]}
        />
        <BuddyRecentErrorsPanel
          recentErrors={[
            {
              id: "workflow-failure-1",
              signal_type: "buddy_dependency_radar_failed",
              title: "Dependency radar failed: Model unavailable",
              source: "buddy",
              status: "failed",
              priority: "high",
              created_at: "2024-01-01T00:00:00Z",
              failure_category: "model_unavailable",
              failure_summary:
                "Model unavailable — check Buddy/default model settings.",
            },
          ]}
          onInvestigate={vi.fn()}
          onDismiss={vi.fn()}
        />
      </>,
      { preloadedState: CONFIG_STATE },
    );

    expect(screen.getByText("Model unavailable")).toBeInTheDocument();
    expect(
      screen.getByText(
        "Model unavailable — check Buddy/default model settings.",
      ),
    ).toBeInTheDocument();
    expect(
      screen.getAllByText("Dependency radar failed: Model unavailable"),
    ).toHaveLength(2);
  });
});

describe("BuddyHome_renders_all_sections", () => {
  it("investigating a grouped recent error acknowledges every related runtime id", async () => {
    const dismissedIds: string[] = [];
    let conversationStarted = false;
    let requestedInvestigationChatId: string | undefined;
    const nowMs = Date.now();
    const runtimeA = {
      id: "grouped-error-a",
      signal_type: "chat_error",
      title: "Grouped provider failure",
      description: "Model returned 500",
      source: "provider",
      status: "failed",
      priority: "high",
      chat_id: "newest-chat-id",
      created_at: new Date(nowMs - 1_000).toISOString(),
    } satisfies BuddyRuntimeEvent;
    const runtimeB = {
      ...runtimeA,
      id: "grouped-error-b",
      chat_id: "older-chat-id",
      created_at: new Date(nowMs - 2_000).toISOString(),
    } satisfies BuddyRuntimeEvent;
    const runtimeC = {
      ...runtimeA,
      id: "grouped-error-c",
      chat_id: "oldest-chat-id",
      created_at: new Date(nowMs - 3_000).toISOString(),
    } satisfies BuddyRuntimeEvent;

    server.use(
      http.get("*/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities: [] }),
      ),
      http.get("*/v1/buddy/conversations", () => HttpResponse.json([])),
      http.get("*/v1/stats/llm/summary", () =>
        HttpResponse.json({
          totals: { total_calls: 0, successful_calls: 0, total_tokens: 0 },
        }),
      ),
      http.get("*/v1/setup/status", () =>
        HttpResponse.json({ configured: true, reasons: [], detail: {} }),
      ),
      http.post("*/v1/buddy/runtime/:id/dismiss", ({ params }) => {
        dismissedIds.push(String(params.id));
        return HttpResponse.json({ detail: "offline" }, { status: 503 });
      }),
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
      http.post("*/v1/buddy/investigation-context", async ({ request }) => {
        const body = (await request.json()) as { chat_id?: string };
        requestedInvestigationChatId = body.chat_id;
        return HttpResponse.json({
          logs: "logs",
          internal_context: "context",
          repo_owner: "smallcloudai",
          repo_name: "refact",
        });
      }),
      http.post("*/v1/chats/:id/commands", () =>
        HttpResponse.json({ ok: true }),
      ),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot(makePulse(), {
          runtime_queue: [runtimeA, runtimeB, runtimeC],
        }),
      ),
    );

    const { user } = render(<BuddyHome />, { store });
    await user.click(screen.getByRole("tab", { name: "Activity" }));
    const errorsPanel = await screen.findByTestId("buddy-recent-errors-panel");
    expect(within(errorsPanel).getByText("×3")).toBeInTheDocument();
    expect(within(errorsPanel).queryByText(/Sniff|Shoo/)).toBeNull();
    expect(dismissedIds).toHaveLength(0);

    await user.click(
      within(errorsPanel).getByRole("button", { name: "Investigate" }),
    );

    const buddyState = store.getState().buddy;
    const allRuntimeEvents = [
      buddyState.nowPlaying,
      ...buddyState.runtimeQueue,
    ].filter((event): event is BuddyRuntimeEvent => event != null);
    expect(allRuntimeEvents.find((event) => event.id === runtimeA.id)).toEqual(
      expect.objectContaining({ dismissed: true }),
    );
    expect(allRuntimeEvents.find((event) => event.id === runtimeB.id)).toEqual(
      expect.objectContaining({ dismissed: true }),
    );
    expect(allRuntimeEvents.find((event) => event.id === runtimeC.id)).toEqual(
      expect.objectContaining({ dismissed: true }),
    );

    await waitFor(() => {
      expect(conversationStarted).toBe(true);
      expect(requestedInvestigationChatId).toBe(runtimeA.chat_id);
      expect(new Set(dismissedIds)).toEqual(
        new Set([runtimeA.id, runtimeB.id, runtimeC.id]),
      );
    });
    expect(dismissedIds).toHaveLength(3);
  });

  it("recent error grouping keeps distinct structured failure details", async () => {
    const nowMs = Date.now();
    const base = {
      id: "structured-failure-model",
      signal_type: "buddy_dependency_radar_failed",
      title: "Dependency radar failed",
      description: "Workflow failed",
      source: "buddy",
      status: "failed",
      priority: "high",
      created_at: new Date(nowMs - 1_000).toISOString(),
    } satisfies BuddyRuntimeEvent;
    const modelFailure = {
      ...base,
      failure_category: "model_unavailable",
      failure_summary: "Model unavailable — switch model.",
    } satisfies BuddyRuntimeEvent;
    const contextFailure = {
      ...base,
      id: "structured-failure-context",
      created_at: new Date(nowMs - 2_000).toISOString(),
      failure_category: "context_too_large",
      failure_summary: "Context too large — compact first.",
    } satisfies BuddyRuntimeEvent;

    server.use(
      http.get("*/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities: [] }),
      ),
      http.get("*/v1/buddy/conversations", () => HttpResponse.json([])),
      http.get("*/v1/stats/llm/summary", () =>
        HttpResponse.json({
          totals: { total_calls: 0, successful_calls: 0, total_tokens: 0 },
        }),
      ),
      http.get("*/v1/setup/status", () =>
        HttpResponse.json({ configured: true, reasons: [], detail: {} }),
      ),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot(makePulse(), {
          runtime_queue: [modelFailure, contextFailure],
        }),
      ),
    );

    const { user } = render(<BuddyHome />, { store });
    await user.click(screen.getByRole("tab", { name: "Activity" }));
    const errorsPanel = await screen.findByTestId("buddy-recent-errors-panel");
    for (const trigger of within(errorsPanel).getAllByRole("button", {
      name: "Technical details",
    }))
      await user.click(trigger);
    expect(
      within(errorsPanel).getByText(/Category: model_unavailable/),
    ).toBeInTheDocument();
    expect(
      within(errorsPanel).getByText(/Category: context_too_large/),
    ).toBeInTheDocument();
    expect(within(errorsPanel).queryByText("×2")).not.toBeInTheDocument();
  });

  it("old dismissed terminal error is not shown in recent errors", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.setSystemTime(new Date("2024-01-01T12:00:00Z"));
    try {
      const oldDismissedError = {
        id: "old-dismissed-terminal-error",
        signal_type: "chat_error",
        title: "Old dismissed provider failure",
        description: "This should not linger.",
        source: "provider",
        status: "failed",
        priority: "high",
        dismissed: true,
        created_at: "2024-01-01T05:00:00Z",
      } satisfies BuddyRuntimeEvent;

      server.use(
        http.get("*/v1/buddy/opportunities", () =>
          HttpResponse.json({ opportunities: [] }),
        ),
        http.get("*/v1/buddy/conversations", () => HttpResponse.json([])),
        http.get("*/v1/stats/llm/summary", () =>
          HttpResponse.json({
            totals: { total_calls: 0, successful_calls: 0, total_tokens: 0 },
          }),
        ),
        http.get("*/v1/setup/status", () =>
          HttpResponse.json({ configured: true, reasons: [], detail: {} }),
        ),
      );
      const store = setUpStore({ ...CONFIG_STATE });
      store.dispatch(
        setBuddySnapshot(
          makeSnapshot(makePulse(), { runtime_queue: [oldDismissedError] }),
        ),
      );

      render(<BuddyHome />, { store });
      fireEvent.mouseDown(screen.getByRole("tab", { name: "Activity" }));
      const errorsPanel = await screen.findByTestId(
        "buddy-recent-errors-panel",
      );

      expect(
        within(errorsPanel).queryByText("Old dismissed provider failure"),
      ).not.toBeInTheDocument();
      expect(
        within(errorsPanel).getByText("No warnings in the last six hours"),
      ).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it("active streaming old event remains shown in recent errors", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.setSystemTime(new Date("2024-01-01T12:00:00Z"));
    try {
      const oldStreamingError = {
        id: "old-streaming-error",
        signal_type: "chat_error",
        title: "Old streaming provider issue",
        description: "Still running and actionable.",
        source: "provider",
        status: "streaming",
        priority: "high",
        created_at: "2024-01-01T05:00:00Z",
      } satisfies BuddyRuntimeEvent;

      server.use(
        http.get("*/v1/buddy/opportunities", () =>
          HttpResponse.json({ opportunities: [] }),
        ),
        http.get("*/v1/buddy/conversations", () => HttpResponse.json([])),
        http.get("*/v1/stats/llm/summary", () =>
          HttpResponse.json({
            totals: { total_calls: 0, successful_calls: 0, total_tokens: 0 },
          }),
        ),
        http.get("*/v1/setup/status", () =>
          HttpResponse.json({ configured: true, reasons: [], detail: {} }),
        ),
      );
      const store = setUpStore({ ...CONFIG_STATE });
      store.dispatch(
        setBuddySnapshot(
          makeSnapshot(makePulse(), { runtime_queue: [oldStreamingError] }),
        ),
      );

      render(<BuddyHome />, { store });
      fireEvent.mouseDown(screen.getByRole("tab", { name: "Activity" }));
      const errorsPanel = await screen.findByTestId(
        "buddy-recent-errors-panel",
      );

      expect(
        within(errorsPanel).getByText("Old streaming provider issue"),
      ).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("BuddyHome_operator_surface", () => {
  const operatorProject = operatorStatus("enabled").project;
  const envelope = (data: unknown) => ({
    schema_version: 1,
    project: operatorProject,
    data,
    next_cursor: null,
  });
  const surfaceHandlers = [
    http.get("*/v1/buddy/opportunities", () =>
      HttpResponse.json({ opportunities: [] }),
    ),
    http.get("*/v1/buddy/conversations", () => HttpResponse.json([])),
    http.get("*/v1/stats/llm/summary", () =>
      HttpResponse.json({
        totals: { total_calls: 50, successful_calls: 48, total_tokens: 900 },
      }),
    ),
    http.get("*/v1/setup/status", () =>
      HttpResponse.json({ configured: true, reasons: [], detail: {} }),
    ),
  ];
  it("keeps operator warnings visible while runtime events are dequeued and cleared", async () => {
    const createdAt = new Date().toISOString();
    const runtimeEvents = ["alpha", "beta", "gamma"].map((suffix) => ({
      id: `persistent-warning-${suffix}`,
      signal_type: "chat_error",
      title: `Persistent warning ${suffix}`,
      description: `Diagnostic details ${suffix}`,
      source: "provider",
      status: "failed",
      failure_summary: `Diagnostic details ${suffix}`,
      priority: "high",
      created_at: createdAt,
      chat_id: "persistent-warning-chat",
    })) satisfies BuddyRuntimeEvent[];
    const diagnostics = ["alpha", "beta", "gamma"].map((suffix) => ({
      error_type: "provider_error",
      error_message: `Diagnostic details ${suffix}`,
      source_file: null,
      tool_name: null,
      chat_id: "persistent-warning-chat",
      diagnostic_id: `persistent-diagnostic-${suffix}`,
      collected_at: createdAt,
      severity: "high",
    })) satisfies DiagnosticContext[];
    server.use(
      ...surfaceHandlers,
      http.get("*/v1/buddy/operator/status", () =>
        HttpResponse.json(operatorStatus("enabled")),
      ),
      http.get("*/v1/buddy/work", () => HttpResponse.json(envelope([]))),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot(makePulse(), {
          recent_diagnostics: diagnostics,
          runtime_queue: runtimeEvents,
          now_playing: runtimeEvents[0],
        }),
      ),
    );

    const actEnvironment = globalThis as typeof globalThis & {
      IS_REACT_ACT_ENVIRONMENT?: boolean;
    };
    const previousActEnvironment = actEnvironment.IS_REACT_ACT_ENVIRONMENT;
    actEnvironment.IS_REACT_ACT_ENVIRONMENT = true;
    try {
      const { user } = render(<BuddyHome />, { store });
      await user.click(screen.getByRole("tab", { name: "Activity" }));
      const list = await screen.findByRole("list", {
        name: "Recent warnings",
      });
      expect(within(list).getAllByRole("listitem")).toHaveLength(3);

      act(() => {
        for (const _event of runtimeEvents) {
          store.dispatch(dequeueRuntimeEvent());
          store.dispatch(clearNowPlaying());
        }
      });
      expect(store.getState().buddy.runtimeQueue).toHaveLength(0);
      expect(store.getState().buddy.nowPlaying).toBeNull();
      const currentList = screen.getByRole("list", {
        name: "Recent warnings",
      });
      expect(within(currentList).getAllByRole("listitem")).toHaveLength(3);
    } finally {
      actEnvironment.IS_REACT_ACT_ENVIRONMENT = previousActEnvironment;
    }
  });

  it("keeps diagnostics out of legacy Recent errors and dismisses runtime ids", async () => {
    const createdAt = new Date().toISOString();
    const runtimeEvents = ["first", "second"].map((suffix) => ({
      id: `legacy-runtime-${suffix}`,
      signal_type: "chat_error",
      title: `Legacy warning ${suffix}`,
      description: `Runtime details ${suffix}`,
      source: "provider",
      status: "failed",
      priority: "high",
      created_at: createdAt,
    })) satisfies BuddyRuntimeEvent[];
    const dismissedIds: string[] = [];
    server.use(
      http.get("*/v1/buddy/operator/status", () =>
        HttpResponse.json(operatorStatus("disabled")),
      ),
      http.get("*/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities: [] }),
      ),
      http.get("*/v1/buddy/conversations", () => HttpResponse.json([])),
      http.get("*/v1/stats/llm/summary", () =>
        HttpResponse.json({
          totals: { total_calls: 0, successful_calls: 0, total_tokens: 0 },
        }),
      ),
      http.get("*/v1/setup/status", () =>
        HttpResponse.json({ configured: true, reasons: [], detail: {} }),
      ),
      http.post("*/v1/buddy/runtime/:id/dismiss", ({ params }) => {
        dismissedIds.push(String(params.id));
        return HttpResponse.json({ detail: "dismissed" });
      }),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot(makePulse(), {
          runtime_queue: runtimeEvents,
          recent_diagnostics: [
            {
              error_type: "provider_error",
              error_message: "Diagnostic-only warning",
              source_file: null,
              tool_name: null,
              chat_id: "diagnostic-only-chat",
              diagnostic_id: "legacy-diagnostic-only",
              collected_at: createdAt,
              severity: "high",
            },
          ],
        }),
      ),
    );

    const { user } = render(<BuddyHome />, { store });
    await user.click(screen.getByRole("tab", { name: "Activity" }));
    const panel = await screen.findByTestId("buddy-recent-errors-panel");
    expect(panel.querySelectorAll("[data-priority]")).toHaveLength(2);
    expect(within(panel).getByText("Legacy warning first")).toBeInTheDocument();
    expect(
      within(panel).getByText("Legacy warning second"),
    ).toBeInTheDocument();
    expect(
      within(panel).queryByText("Diagnostic-only warning"),
    ).not.toBeInTheDocument();

    for (const event of runtimeEvents) {
      const row = within(panel)
        .getByText(event.title)
        .closest<HTMLElement>("[data-priority]");
      if (!row) throw new Error("Expected a legacy runtime warning row");
      await user.click(within(row).getByRole("button", { name: "Dismiss" }));
      await waitFor(() => {
        const currentEvent = store
          .getState()
          .buddy.runtimeQueue.find((candidate) => candidate.id === event.id);
        const activeEvent = store.getState().buddy.nowPlaying;
        expect(currentEvent?.dismissed ?? activeEvent?.dismissed).toBe(true);
      });
    }
    expect(dismissedIds).toEqual(runtimeEvents.map((event) => event.id));
  });

  it("limits operator diagnostics to six hours and merges matches with runtime warnings", async () => {
    const createdAt = new Date().toISOString();
    const runtimeEvent = {
      id: "diagnostic-runtime-match",
      signal_type: "chat_error",
      title: "Provider unavailable",
      description: "Model endpoint returned an error",
      source: "provider",
      status: "failed",
      failure_summary: "Model endpoint returned an error",
      priority: "high",
      created_at: createdAt,
      chat_id: "diagnostic-chat",
    } satisfies BuddyRuntimeEvent;
    const diagnostic = (
      id: string,
      overrides: Partial<DiagnosticContext> = {},
    ): DiagnosticContext => ({
      error_type: "provider_error",
      error_message: "Model endpoint returned an error",
      source_file: null,
      tool_name: null,
      chat_id: "diagnostic-chat",
      diagnostic_id: id,
      collected_at: createdAt,
      severity: "high",
      occurrences: 2,
      ...overrides,
    });
    server.use(
      ...surfaceHandlers,
      http.get("*/v1/buddy/operator/status", () =>
        HttpResponse.json(operatorStatus("enabled")),
      ),
      http.get("*/v1/buddy/work", () => HttpResponse.json(envelope([]))),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot(makePulse(), {
          runtime_queue: [runtimeEvent],
          recent_diagnostics: [
            diagnostic("matched-diagnostic"),
            diagnostic("old-diagnostic", {
              error_type: "old_provider_error",
              error_message: "Old diagnostic is outside the window",
              collected_at: new Date(
                Date.now() - 7 * 60 * 60 * 1000,
              ).toISOString(),
            }),
          ],
        }),
      ),
    );

    const { user } = render(<BuddyHome />, { store });
    await user.click(screen.getByRole("tab", { name: "Activity" }));
    const list = await screen.findByRole("list", { name: "Recent warnings" });
    expect(within(list).getAllByRole("listitem")).toHaveLength(1);
    expect(
      within(list).getByText("Provider unavailable", { exact: true }),
    ).toBeInTheDocument();
    expect(within(list).getByText("×2")).toBeInTheDocument();
    expect(
      within(list).queryByText("Old diagnostic is outside the window"),
    ).not.toBeInTheDocument();
  });
});

describe("BuddyHome_bottom_row_bounded_scroll", () => {
  const commonHandlers = [
    http.get("*/v1/buddy/opportunities", () =>
      HttpResponse.json({ opportunities: [] }),
    ),
    http.get("*/v1/buddy/conversations", () => HttpResponse.json([])),
    http.get("*/v1/stats/llm/summary", () =>
      HttpResponse.json({
        totals: { total_calls: 0, successful_calls: 0, total_tokens: 0 },
      }),
    ),
    http.get("*/v1/setup/status", () =>
      HttpResponse.json({ configured: true, reasons: [], detail: {} }),
    ),
  ];
  it("activity panel renders as a Buddy section with list rows", async () => {
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));
    server.use(...commonHandlers);

    const { user } = render(<BuddyHome />, { store });
    await user.click(screen.getByRole("tab", { name: "Activity" }));
    const panel = await screen.findByTestId("buddy-activity-panel");
    expect(panel.tagName).toBe("SECTION");
    expect(panel).toHaveAttribute("data-buddy-surface", "section");
    expect(
      within(panel).getByRole("heading", { name: "Activity" }),
    ).toBeInTheDocument();
  });

  it("recent warnings render as one Buddy section with a heading and count", async () => {
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));
    server.use(...commonHandlers);

    const { user } = render(<BuddyHome />, { store });
    await user.click(screen.getByRole("tab", { name: "Activity" }));
    const panel = await screen.findByTestId("buddy-recent-errors-panel");
    expect(panel).toHaveAttribute("data-buddy-surface", "section");
    expect(
      within(panel).getByRole("heading", { name: "Recent warnings" }),
    ).toBeInTheDocument();
    expect(panel.querySelector("details")).toBeNull();
  });

  it("recent chats render as a Buddy section", async () => {
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));
    server.use(...commonHandlers);

    const { user } = render(<BuddyHome />, { store });
    await user.click(screen.getByRole("tab", { name: "Activity" }));
    const panel = await screen.findByTestId("buddy-recent-chats");
    expect(panel).toHaveAttribute("data-buddy-surface", "section");
  });

  it("activity slots size to their content instead of fixed heights", async () => {
    const css = await readGuiSource(
      "features/Buddy/sections/BuddyActivitySection.module.css",
    );
    expect(css).not.toMatch(/height:\s*clamp/);
    expect(css).not.toContain("52vh");
    expect(css).not.toContain("max-height: 860px");
    expect(css).not.toContain("summary");
  });

  it("legacy panels have no inner scroll caps or opacity fades", async () => {
    for (const file of [
      "features/Buddy/BuddyActivityPanel.module.css",
      "features/Buddy/BuddyRecentErrorsPanel.module.css",
      "features/Buddy/BuddyRecentChats.module.css",
    ]) {
      const css = await readGuiSource(file);
      expect(css, file).not.toContain("overflow-y: auto");
      expect(css, file).not.toContain("opacity");
      expect(css, file).not.toContain("max-height: 220px");
    }
  });
});
