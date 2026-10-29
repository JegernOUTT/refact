import { readFile } from "node:fs/promises";
import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "../../../../utils/test-utils";
import { http, HttpResponse } from "msw";
import { resolve } from "node:path";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { server } from "../../../../utils/mockServer";
import { setUpStore } from "../../../../app/store";
import {
  setBuddySnapshot,
  selectHomeSnoozedUntil,
  selectSeenNotificationIds,
} from "../../buddySlice";
import { BuddyHome } from "../../BuddyHome";
import { readBuddyPersistentState, resetBuddyPersistentState } from "../../ui";
import { BUDDY_REPORTS_VIEW_KEY } from "../buddyReportsLink";
import type {
  BuddyOpportunity,
  BuddyPulse,
  BuddyRuntimeEvent,
  BuddySnapshot,
} from "../../types";

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
    proposed_actions: [],
    humor_allowed: false,
    related: { chat_ids: [], task_ids: [], memory_ids: [], config_paths: [] },
    created_at: "2024-01-01T00:00:00Z",
    expires_at: "2099-12-31T00:00:00Z",
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

function readGuiSource(path: string): Promise<string> {
  return readFile(resolve(process.cwd(), "src", path), "utf8");
}

function readCssBlock(source: string, selector: string): string {
  const escapedSelector = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = new RegExp(`(^|\\n)\\s*${escapedSelector}\\s*{`).exec(source);
  if (match?.index === undefined) {
    throw new Error(`Missing CSS block for ${selector}`);
  }
  const open = source.indexOf("{", match.index);
  let depth = 0;
  for (let i = open; i < source.length; i += 1) {
    const char = source[i];
    if (char === "{") depth += 1;
    if (char === "}") depth -= 1;
    if (depth === 0) return source.slice(open + 1, i);
  }
  throw new Error(`Unclosed CSS block for ${selector}`);
}

describe("BuddyHome_renders_all_sections", () => {
  it("failed hero runtime dismiss remains local and snoozes notifications", async () => {
    let dismissCalled = false;
    const runtime = {
      id: "home-runtime-dismiss-fails",
      signal_type: "chat_error",
      title: "Runtime dismiss fails",
      source: "test",
      status: "failed",
      priority: "high",
      created_at: new Date().toISOString(),
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
      http.post("*/v1/buddy/runtime/:id/dismiss", () => {
        dismissCalled = true;
        return HttpResponse.json({ detail: "offline" }, { status: 503 });
      }),
    );
    const unhandled = vi.fn();
    window.addEventListener("unhandledrejection", unhandled);
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(
      setBuddySnapshot(makeSnapshot(makePulse(), { runtime_queue: [runtime] })),
    );

    try {
      render(<BuddyHome />, { store });
      const world = await screen.findByTestId("buddy-presence");
      const dismissButton = within(world)
        .getAllByRole("button", { hidden: true })
        .find((button) => button.textContent === "Shoo");
      expect(dismissButton).toBeDefined();
      if (!dismissButton) throw new Error("expected dismiss button");
      fireEvent.click(dismissButton);

      await waitFor(() => {
        expect(store.getState().buddy.nowPlaying?.id).toBe(runtime.id);
        expect(store.getState().buddy.nowPlaying?.dismissed).toBe(true);
        expect(dismissCalled).toBe(true);
      });
      expect(selectHomeSnoozedUntil(store.getState())).toBeGreaterThan(
        Date.now(),
      );
      expect(
        `runtime-${runtime.id}` in selectSeenNotificationIds(store.getState()),
      ).toBe(true);
      await new Promise((resolve) => window.setTimeout(resolve, 0));
      expect(unhandled).not.toHaveBeenCalled();
    } finally {
      window.removeEventListener("unhandledrejection", unhandled);
      vi.useRealTimers();
    }
  });

  it("failed hero opportunity action stays unseen and prevents duplicate requests", async () => {
    // Opportunity speech intentionally stays silent in some 30-second buckets.
    // Fix only the wall clock to a speaking bucket; keep request timers real.
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2024-01-01T12:00:00Z"));
    try {
      const opportunity = makeOpportunity({
        id: "hero-failed-action",
        proposed_actions: [{ kind: "open_page", page: { type: "providers" } }],
      });
      let calls = 0;
      let finish: (() => void) | undefined;
      const responseReady = new Promise<void>((resolve) => {
        finish = resolve;
      });
      server.use(
        http.get("*/v1/buddy/opportunities", () =>
          HttpResponse.json({ opportunities: [opportunity] }),
        ),
        http.get("*/v1/buddy/conversations", () => HttpResponse.json([])),
        http.get("*/v1/buddy/briefing", () => HttpResponse.json(null)),
        http.get("*/v1/stats/llm/summary", () =>
          HttpResponse.json({ totals: {} }),
        ),
        http.get("*/v1/setup/status", () =>
          HttpResponse.json({ configured: true }),
        ),
        http.post("*/v1/buddy/opportunities/:id/accept", async () => {
          calls += 1;
          await responseReady;
          return HttpResponse.json(
            { detail: "hero action offline" },
            { status: 503 },
          );
        }),
      );
      const store = setUpStore(CONFIG_STATE);
      store.dispatch(
        setBuddySnapshot(
          makeSnapshot(makePulse(), {
            opportunities: [opportunity],
            runtime_queue: [],
          }),
        ),
      );
      render(<BuddyHome />, { store });
      const world = await screen.findByTestId("buddy-presence");
      const button = within(world)
        .getAllByRole("button", { hidden: true })
        .find((candidate) => candidate.textContent === "Open Providers");
      if (!button) throw new Error("Expected hero opportunity control");
      fireEvent.click(button);
      fireEvent.click(button);
      await waitFor(() => expect(calls).toBe(1));
      finish?.();
      await waitFor(() =>
        expect(store.getState().buddy.activeSpeech?.text).toContain(
          "hero action offline",
        ),
      );
      expect(selectSeenNotificationIds(store.getState())).not.toHaveProperty(
        `opportunity-${opportunity.id}`,
      );
      expect(selectHomeSnoozedUntil(store.getState())).toBeNull();
      expect(calls).toBe(1);
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("BuddyHome_operator_surface", () => {
  const operatorProject = operatorStatus("enabled").project;
  const record = (value: unknown) => ({ value, revision: 1, change_seq: 1 });
  const envelope = (data: unknown) => ({
    schema_version: 1,
    project: operatorProject,
    data,
    next_cursor: null,
  });
  const workItem = (id: string, title: string) => ({
    schema_version: 1,
    id,
    project: operatorProject,
    subject: { resource_kind: "legacy_draft", resource_id: id, version: "1" },
    kind: "proposal",
    title,
    state: "needs_attention",
    native_sources: [],
    current_artifact_id: `${id}-report`,
    current_artifact_revision: 1,
    last_material_change_at: "2026-01-01T00:00:00Z",
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
  it("previews at most three saved items on Home without mounting the Desk", async () => {
    const workLimits: string[] = [];
    const items = Array.from({ length: 5 }, (_, index) =>
      record({
        ...workItem(`desk-${index}`, `Saved finding ${index}`),
        problem_summary: `Problem summary ${index}`,
        scope: `scope-${index}`,
      }),
    );
    server.use(
      ...surfaceHandlers,
      http.get("*/v1/buddy/operator/status", () =>
        HttpResponse.json(operatorStatus("enabled")),
      ),
      http.get("*/v1/buddy/work", ({ request }) => {
        const url = new URL(request.url);
        workLimits.push(
          `${url.searchParams.get("view")}:${url.searchParams.get("limit")}`,
        );
        if (url.searchParams.get("view") === "delivered")
          return HttpResponse.json(envelope([]));
        return HttpResponse.json({
          ...envelope(items),
          next_cursor: 3,
        });
      }),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));

    render(<BuddyHome />, { store });

    const preview = await screen.findByRole("list", {
      name: "Reports that need you",
    });
    expect(within(preview).getAllByRole("listitem")).toHaveLength(3);
    expect(within(preview).getByText("Problem summary 0")).toBeInTheDocument();
    expect(within(preview).getByText(/scope-0/)).toBeInTheDocument();
    expect(within(preview).queryByText("Saved finding 3")).toBeNull();
    expect(
      within(screen.getByTestId("buddy-home-saved-work")).getByRole("button", {
        name: "View all reports",
      }),
    ).toBeInTheDocument();
    expect(screen.queryByTestId("buddy-desk")).toBeNull();
    expect(workLimits).toContain("needs_you:3");
    expect(workLimits.some((entry) => entry.endsWith(":50"))).toBe(false);
    expect(
      await screen.findByText("No delivered reports yet."),
    ).toBeInTheDocument();
  });

  it("features a delivered report instead of cancelled runs or housekeeping", async () => {
    server.use(
      ...surfaceHandlers,
      http.get("*/v1/buddy/operator/status", () =>
        HttpResponse.json(operatorStatus("enabled")),
      ),
      http.get("*/v1/buddy/work", ({ request }) => {
        const url = new URL(request.url);
        if (url.searchParams.get("view") === "delivered")
          return HttpResponse.json(
            envelope([
              record({
                ...workItem("cancelled-run", "Cancelled run"),
                state: "cancelled",
              }),
              record({
                ...workItem("commit-run", "Generated commit message"),
                subject: {
                  resource_kind: "workflow",
                  resource_id: "commit_message",
                  version: "1",
                },
                state: "delivered",
              }),
              record({
                ...workItem("real-report", "Real delivered report"),
                state: "delivered",
              }),
            ]),
          );
        return HttpResponse.json(envelope([]));
      }),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));

    render(<BuddyHome />, { store });

    const latest = await screen.findByTestId("buddy-home-latest-report");
    expect(
      await within(latest).findByText("Real delivered report"),
    ).toBeInTheDocument();
    expect(within(latest).queryByText("Cancelled run")).toBeNull();
    expect(within(latest).queryByText("Generated commit message")).toBeNull();
    expect(
      await screen.findByText("Nothing needs you right now."),
    ).toBeInTheDocument();
  });

  it("reports a project mismatch rather than presenting foreign previews as empty", async () => {
    const foreignProject = {
      project_id: "foreign-project",
      canonical_root: "/scratch/foreign-project",
    };
    server.use(
      ...surfaceHandlers,
      http.get("*/v1/buddy/operator/status", () =>
        HttpResponse.json(operatorStatus("enabled")),
      ),
      http.get("*/v1/buddy/work", ({ request }) => {
        const delivered =
          new URL(request.url).searchParams.get("view") === "delivered";
        return HttpResponse.json({
          ...envelope([
            record({
              ...workItem("foreign-work", "Private foreign report"),
              project: foreignProject,
              state: delivered ? "delivered" : "needs_attention",
            }),
          ]),
          project: foreignProject,
        });
      }),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));
    render(<BuddyHome />, { store });
    expect(
      await screen.findByText("Couldn't load reports."),
    ).toBeInTheDocument();
    expect(
      await screen.findByText("Couldn't load delivered reports."),
    ).toBeInTheDocument();
    expect(screen.queryByText("Private foreign report")).toBeNull();
    expect(screen.queryByText("Nothing needs you right now.")).toBeNull();
  });

  it("keeps native questions on Home when saved work fails to load", async () => {
    server.use(
      ...surfaceHandlers,
      http.get("*/v1/buddy/operator/status", () =>
        HttpResponse.json(operatorStatus("enabled")),
      ),
      http.get(
        "*/v1/buddy/work",
        () => new HttpResponse(null, { status: 503 }),
      ),
      http.get(
        "*/v1/buddy/decisions/current",
        () => new HttpResponse(null, { status: 503 }),
      ),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));

    render(<BuddyHome />, { store });

    const saved = await screen.findByTestId("buddy-home-saved-work");
    expect(
      await within(saved).findByText("Couldn't load reports."),
    ).toBeInTheDocument();
    expect(within(saved).queryByText(/Nothing needs you/)).toBeNull();
    const native = screen.getByTestId("buddy-home-native");
    expect(
      await within(native).findByText("Couldn't load questions."),
    ).toBeInTheDocument();
    expect(screen.getByTestId("buddy-operator-status")).toHaveTextContent(
      "Enabled",
    );
  });

  it("merges simultaneous question and approval failures into one callout with one retry", async () => {
    let questionReads = 0;
    let approvalReads = 0;
    server.use(
      ...surfaceHandlers,
      http.get("*/v1/buddy/operator/status", () =>
        HttpResponse.json(operatorStatus("enabled")),
      ),
      http.get("*/v1/buddy/work", () => HttpResponse.json(envelope([]))),
      http.get("*/v1/buddy/decisions/current", () => {
        questionReads += 1;
        return new HttpResponse(null, { status: 503 });
      }),
      http.get("*/v1/buddy/approvals/current", () => {
        approvalReads += 1;
        return new HttpResponse(null, { status: 503 });
      }),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));
    render(<BuddyHome />, { store });

    const merged = await screen.findByTestId("buddy-home-requests-error");
    expect(merged).toHaveTextContent("Couldn't load questions and approvals.");
    const native = screen.getByTestId("buddy-home-native");
    expect(within(native).getAllByRole("alert")).toHaveLength(1);
    const seen = [questionReads, approvalReads];
    fireEvent.click(within(merged).getByRole("button", { name: "Retry" }));
    await waitFor(() => {
      expect(questionReads).toBeGreaterThan(seen[0]);
      expect(approvalReads).toBeGreaterThan(seen[1]);
    });
  });

  it("opens Reports on the matching view from the Home cards", async () => {
    resetBuddyPersistentState();
    server.use(
      ...surfaceHandlers,
      http.get("*/v1/buddy/operator/status", () =>
        HttpResponse.json(operatorStatus("enabled")),
      ),
      http.get("*/v1/buddy/work", () => HttpResponse.json(envelope([]))),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));
    render(<BuddyHome />, { store });

    const latest = await screen.findByTestId("buddy-home-latest-report");
    fireEvent.click(
      within(latest).getByRole("button", { name: "View delivered" }),
    );
    expect(readBuddyPersistentState(BUDDY_REPORTS_VIEW_KEY)).toBe("delivered");
    expect(screen.getByRole("tab", { name: "Reports" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  it("sizes the Home portrait and drops the nested scroll island", async () => {
    const css = await readGuiSource(
      "features/Buddy/sections/BuddyHomeSection.module.css",
    );
    expect(
      readCssBlock(css, '.portrait.portrait [data-testid="buddy-world"]'),
    ).toContain("height: clamp(160px, 22vh, 200px)");
    expect(css).not.toContain("nativeSlot");
    expect(css).not.toContain("max-height");
  });
});
