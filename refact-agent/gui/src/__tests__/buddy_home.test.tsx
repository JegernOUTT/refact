import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "../utils/test-utils";
import { http, HttpResponse } from "msw";
import { readdir, readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "@testing-library/react";
import { Provider } from "react-redux";
import { QueryStatus } from "@reduxjs/toolkit/query";
import { Theme } from "@radix-ui/themes";
import { server } from "../utils/mockServer";
import { setUpStore } from "../app/store";
import {
  setBuddySnapshot,
  defaultBuddySettings,
} from "../features/Buddy/buddySlice";
import { push } from "../features/Pages/pagesSlice";
import { BuddyDraftPreview } from "../features/Buddy/BuddyDraftPreview";
import { BuddyChatCompanion } from "../features/Buddy/BuddyChatCompanion";
import { BuddyDashboardScene } from "../features/Buddy/BuddyDashboardScene";
import { BuddyHome } from "../features/Buddy/BuddyHome";
import { BuddyWorld } from "../features/Buddy/BuddyWorld";
import { BuddyRecentChats } from "../features/Buddy/BuddyRecentChats";
import { bubblePositionForSceneX } from "../features/Buddy/buddyWorldUtils";
import { buildBuddyWorldState } from "../features/Buddy/buddyWorldModel";
import {
  buildBuddySceneSpeech,
  formatBuddyRuntimeEventText,
  isBuddySpeechExpired,
  pickBuddySceneSpeechCandidate,
  type BuddySceneSpeech,
} from "../features/Buddy/buddySceneSpeech";
import { useExecuteBuddyAction } from "../features/Buddy/hooks/useExecuteBuddyAction";
import { executeBuddyNavigation } from "../features/Buddy/executeBuddyAction";
import { PALETTES, STAGES } from "../features/Buddy/constants";
import { SETUP_MODES } from "../features/Setup/setupModes";
import { buddyApi } from "../services/refact/buddy";
import {
  buddyOperatorApi,
  buddyOperatorIdentity,
} from "../services/refact/buddyOperator";
import type {
  BuddyOpportunity,
  BuddySuggestion,
  BuddyDraft,
  BuddyPulse,
  BuddyRuntimeEvent,
  BuddySemanticState,
  BuddySnapshot,
  BuddyConversationEntry,
  BuddySpeechItem,
} from "../features/Buddy/types";
import type React from "react";
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
beforeEach(() => {
  server.use(
    http.get("*/v1/buddy/operator/status", () =>
      HttpResponse.json(operatorStatus("disabled")),
    ),
  );
});

vi.mock("../features/Buddy/BuddyCharacter", async () => {
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

function makeConversation(
  overrides?: Partial<BuddyConversationEntry>,
): BuddyConversationEntry {
  return {
    id: "conversation-1",
    kind: "workflow",
    title: "Workflow chat",
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T01:00:00Z",
    status: "completed",
    message_count: 3,
    icon: "⚙️",
    badge: "refact_self_critic",
    ...overrides,
  };
}

function makeSpeech(overrides?: Partial<BuddySpeechItem>): BuddySpeechItem {
  return {
    id: "speech-1",
    text: "Hello from Buddy",
    mood: "happy",
    scope: "global",
    persistent: false,
    ttl_seconds: 60,
    created_at: "2024-01-01T00:00:00Z",
    controls: [],
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

describe("BuddyHome single work authority", () => {
  it.each(["enabled", "unavailable", "disabled"] as const)(
    "gates exact producer controls on Companion when %s",
    async (mode) => {
      server.use(
        http.get("*/v1/buddy/operator/status", () =>
          HttpResponse.json(operatorStatus(mode)),
        ),
        http.get("*/v1/buddy/opportunities", () =>
          HttpResponse.json({ opportunities: [] }),
        ),
      );
      const controls = [
        {
          id: "worktree-pulse",
          label: "Create Worktrees pulse report",
          action: "create_worktrees_pulse",
          style: "secondary" as const,
        },
        {
          id: "dismiss-worktree-cleanup",
          label: "Dismiss proposal",
          action: "dismiss_suggestion",
          action_param: "worktree_cleanup",
          style: "ghost" as const,
        },
        {
          id: "review-worktree-cleanup",
          label: "Review cleanup candidates",
          action: "review_worktree_cleanup",
          style: "primary" as const,
        },
      ];
      for (const [surfaceIndex, Surface] of (
        [() => <BuddyChatCompanion chatId="producer-fixture" />] as const
      ).entries()) {
        const store = setUpStore({ ...CONFIG_STATE });
        store.dispatch(
          setBuddySnapshot(
            makeSnapshot(undefined, {
              active_speech: makeSpeech({
                id: `speech-${mode}-${surfaceIndex}`,
                text: "Saved cleanup advice",
                created_at: new Date().toISOString(),
                persistent: true,
                controls,
              }),
            }),
          ),
        );
        await store
          .dispatch(
            buddyOperatorApi.endpoints.getBuddyOperatorStatus.initiate(
              buddyOperatorIdentity(store.getState()),
            ),
          )
          .unwrap();
        if (mode === "disabled") {
          await store
            .dispatch(buddyApi.endpoints.getOpportunities.initiate(undefined))
            .unwrap();
        }
        const view = render(<Surface />, { store });
        expect(
          await screen.findByText("Review cleanup candidates"),
        ).toBeInTheDocument();
        if (mode === "disabled") {
          expect(
            await screen.findByText("Create Worktrees pulse report"),
          ).toBeInTheDocument();
          expect(screen.getByText("Dismiss proposal")).toBeInTheDocument();
        } else {
          await waitFor(() =>
            expect(
              Object.values(store.getState().buddyApi.queries).some(
                (query) =>
                  query?.endpointName === "getBuddyOperatorStatus" &&
                  query.status === QueryStatus.fulfilled,
              ),
            ).toBe(true),
          );
          expect(
            screen.queryByText("Create Worktrees pulse report"),
          ).not.toBeInTheDocument();
          expect(
            screen.queryByText("Dismiss proposal"),
          ).not.toBeInTheDocument();
        }
        // The canvas bubble animates text via requestAnimationFrame (not driven
        // by this fixture); the advisory source must remain intact regardless.
        expect(store.getState().buddy.snapshot?.active_speech?.text).toBe(
          "Saved cleanup advice",
        );
        view.unmount();
      }
    },
  );

  it.each(["enabled", "unavailable"] as const)(
    "does not mount legacy lifecycle producers when %s",
    async (mode) => {
      let legacyRequests = 0;
      let workRequests = 0;
      server.use(
        http.get("*/v1/buddy/operator/status", () =>
          HttpResponse.json(operatorStatus(mode)),
        ),
        http.get("*/v1/buddy/work", () => {
          workRequests += 1;
          return HttpResponse.json({
            schema_version: 1,
            project: operatorStatus("enabled").project,
            data: [],
            next_cursor: null,
          });
        }),
        http.get("*/v1/buddy/briefing", () => {
          legacyRequests += 1;
          return HttpResponse.json({});
        }),
        http.get("*/v1/buddy/drafts/expired", () => {
          legacyRequests += 1;
          return new HttpResponse(null, { status: 404 });
        }),
      );
      const store = setUpStore({ ...CONFIG_STATE });
      store.dispatch(setBuddySnapshot(makeSnapshot()));
      store.dispatch(push({ name: "buddy", draftId: "expired" }));
      const { user } = render(<BuddyHome />, { store });
      expect(screen.getByRole("tab", { name: "Reports" })).toHaveAttribute(
        "aria-selected",
        "true",
      );
      if (mode === "enabled") await screen.findByTestId("buddy-desk");
      else await screen.findByRole("button", { name: "Check connection" });
      expect(
        screen.getByText(/Draft editing sessions may expire/),
      ).toBeTruthy();
      expect(screen.queryByTestId("buddy-pulse-card")).toBeNull();
      for (const name of ["Home", "Reports", "Activity", "Settings"]) {
        await user.click(screen.getByRole("tab", { name }));
        expect(screen.queryByTestId("buddy-opportunities-feed")).toBeNull();
        expect(screen.queryByTestId("buddy-briefing-card")).toBeNull();
        expect(
          screen.queryByRole("button", { name: "Dismiss draft" }),
        ).toBeNull();
      }
      expect(legacyRequests).toBe(0);
      if (mode === "enabled") expect(workRequests).toBeGreaterThan(0);
      else expect(workRequests).toBe(0);
    },
  );
});

describe("BuddyHome_renders_all_sections", () => {
  it("failed dashboard runtime dismiss remains local", async () => {
    let dismissCalled = false;
    const runtime = {
      id: "dashboard-runtime-dismiss-fails",
      signal_type: "chat_error",
      title: "Dashboard runtime dismiss fails",
      source: "test",
      status: "failed",
      priority: "high",
      created_at: new Date().toISOString(),
    } satisfies BuddyRuntimeEvent;
    server.use(
      http.get("*/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities: [] }),
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
      render(<BuddyDashboardScene />, { store });
      const world = await screen.findByTestId("buddy-world");
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
      await new Promise((resolve) => window.setTimeout(resolve, 0));
      expect(unhandled).not.toHaveBeenCalled();
    } finally {
      window.removeEventListener("unhandledrejection", unhandled);
      vi.useRealTimers();
    }
  });

  it("source keeps runtime dismiss best-effort outside chat companion", async () => {
    const home = await readGuiSource("features/Buddy/BuddyHome.tsx");
    const dashboard = await readGuiSource(
      "features/Buddy/BuddyDashboardScene.tsx",
    );
    const executor = await readGuiSource(
      "features/Buddy/executeBuddyAction.ts",
    );

    expect(home).toContain(
      "void dismissRuntimeMutation(heroSpeech.runtimeEventId)",
    );
    expect(home).toContain(".catch(() => undefined)");
    expect(home).toContain('ctrl.action === "dismiss_runtime_event"');
    expect(home).not.toContain(
      "await dismissRuntimeMutation(heroSpeech.runtimeEventId).unwrap()",
    );
    expect(dashboard).toContain("void dismissRuntimeMutation(runtimeEventId)");
    expect(dashboard).toContain(".catch(() => undefined)");
    expect(dashboard).toContain('control.action === "dismiss_runtime_event"');
    expect(dashboard).not.toContain(
      "await dismissRuntimeMutation(runtimeEventId).unwrap()",
    );
    expect(executor).toContain("dispatch(dismissRuntimeEvent(eventId))");
    expect(executor).toContain(".catch(() => undefined)");
  });

  it("renders a buddy-home-content scroll container", async () => {
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));
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

    render(<BuddyHome />, { store });

    expect(await screen.findByTestId("buddy-home-content")).toBeInTheDocument();
  });
});

describe("BuddyHome_layout_contract", () => {
  const layoutHandlers = [
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
  it("mounts one scene at a time: a portrait on Home and the full scene on Play", async () => {
    server.use(
      ...layoutHandlers,
      http.get("*/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities: [] }),
      ),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));

    const { user } = render(<BuddyHome />, { store });

    const overview = await screen.findByTestId("buddy-home-overview");
    expect(within(overview).getByTestId("buddy-world")).toBeInTheDocument();
    expect(screen.getAllByTestId("buddy-world")).toHaveLength(1);
    expect(
      screen
        .getByRole("tablist", { name: "Buddy sections" })
        .compareDocumentPosition(screen.getByTestId("buddy-home-content")),
    ).toBe(Node.DOCUMENT_POSITION_FOLLOWING);

    for (const name of ["Reports", "Activity", "Settings"]) {
      await user.click(screen.getByRole("tab", { name }));
      expect(screen.queryByTestId("buddy-world")).toBeNull();
    }
    await user.click(screen.getByRole("tab", { name: "Play" }));
    const play = screen.getByTestId("buddy-home-play");
    expect(within(play).getByTestId("buddy-world")).toBeInTheDocument();
    expect(screen.getAllByTestId("buddy-world")).toHaveLength(1);
  });

  it("keeps compliance prose and unstyled elements out of the page", async () => {
    const sectionFiles = (
      await readdir(resolve(process.cwd(), "src/features/Buddy/sections"))
    ).filter((file) => file.endsWith(".tsx"));
    const source = (
      await Promise.all(
        [
          "features/Buddy/BuddyHome.tsx",
          ...sectionFiles.map((file) => `features/Buddy/sections/${file}`),
        ].map(readGuiSource),
      )
    ).join("\n");

    expect(source).not.toContain("<p>");
    expect(source).not.toContain("<section");
    expect(source).not.toContain("Needs you above");
    expect(source).not.toContain("never permissions");
    expect(source).not.toContain("Play is optional");
    expect(source).not.toContain("Back to Desk");
  });

  it("exposes a real heading outline instead of styled spans", async () => {
    server.use(
      ...layoutHandlers,
      http.get("*/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities: [] }),
      ),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));

    render(<BuddyHome />, { store });
    await screen.findByTestId("buddy-home-content");

    const headings = screen.getAllByRole("heading");
    const levels = headings.map((heading) => Number(heading.tagName.slice(1)));

    expect(levels.filter((level) => level === 1)).toHaveLength(1);
    expect(levels[0]).toBe(1);
    expect(levels).toContain(2);
    expect(
      screen.getByRole("heading", { level: 1 }).textContent,
    ).not.toHaveLength(0);

    let previous = 1;
    for (const level of levels) {
      expect(level).toBeLessThanOrEqual(previous + 1);
      previous = level;
    }
  });

  it("keeps every Buddy stylesheet on the shared type and radius scale", async () => {
    const directory = resolve(process.cwd(), "src/features/Buddy");
    const sheets = [
      ...(await readdir(directory)),
      ...(await readdir(resolve(directory, "sections"))).map(
        (file) => `sections/${file}`,
      ),
    ].filter((file) => file.endsWith(".module.css"));
    expect(sheets.length).toBeGreaterThan(20);

    const offenders: string[] = [];
    for (const sheet of sheets) {
      const css = await readGuiSource(`features/Buddy/${sheet}`);
      for (const [declaration, value] of css.matchAll(
        /font-size:\s*([\d.]+)px/g,
      )) {
        if (Number(value) < 12) offenders.push(`${sheet}: ${declaration}`);
      }
      for (const dead of ["--rf-font-size-", "--rf-radius-control"]) {
        if (css.includes(dead)) offenders.push(`${sheet}: ${dead}`);
      }
    }

    expect(offenders).toEqual([]);
  });

  it("gives every Buddy control a hit-area floor", async () => {
    const css = await readGuiSource("features/Buddy/BuddyHome.module.css");

    expect(
      readCssBlock(
        css,
        '.page :is(button, [role="tab"], select, input, summary)',
      ),
    ).toContain("min-height: var(--rf-hit-min)");
    expect(css).toContain("--rf-hit-min: var(--rf-hit-min-touch)");
  });

  it("sizes the tab strip so narrow viewports scroll instead of truncating", async () => {
    const css = await readGuiSource("features/Buddy/BuddyHome.module.css");

    expect(readCssBlock(css, ".sections .tabsList")).toContain(
      "grid-auto-columns: minmax(max-content, 1fr)",
    );
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
  it("opens a Home preview row in Reports with its work id", async () => {
    const requested: string[] = [];
    const item = workItem("preview-open", "Preview finding");
    server.use(
      ...surfaceHandlers,
      http.get("*/v1/buddy/operator/status", () =>
        HttpResponse.json(operatorStatus("enabled")),
      ),
      http.get("*/v1/buddy/work", ({ request }) => {
        const url = new URL(request.url);
        if (url.searchParams.get("view") === "needs_you")
          return HttpResponse.json(envelope([record(item)]));
        return HttpResponse.json(envelope([]));
      }),
      http.get("*/v1/buddy/work/preview-open", () => {
        requested.push("preview-open");
        return HttpResponse.json(envelope(record(item)));
      }),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));

    const { user } = render(<BuddyHome />, { store });

    await user.click(
      await screen.findByRole("button", { name: "Open Preview finding" }),
    );
    expect(screen.getByRole("tab", { name: "Reports" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(await screen.findByTestId("buddy-desk")).toBeInTheDocument();
    await waitFor(() => expect(requested).toContain("preview-open"));
  });

  it("opens Reports first when the page deep-links a work id", async () => {
    const item = workItem("linked-work", "Linked finding");
    server.use(
      ...surfaceHandlers,
      http.get("*/v1/buddy/operator/status", () =>
        HttpResponse.json(operatorStatus("enabled")),
      ),
      http.get("*/v1/buddy/work", () => HttpResponse.json(envelope([]))),
      http.get("*/v1/buddy/work/linked-work", () =>
        HttpResponse.json(envelope(record(item))),
      ),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));
    store.dispatch(
      push({ name: "buddy", workId: "linked-work", workProjectId: "fixture" }),
    );

    render(<BuddyHome />, { store });

    expect(screen.getByRole("tab", { name: "Reports" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(await screen.findByTestId("buddy-desk")).toBeInTheDocument();
    expect(screen.queryByTestId("buddy-home-overview")).toBeNull();
  });

  it("returns to Home after a preview and honours a later deep link", async () => {
    const first = workItem("first-preview", "First preview");
    const linked = workItem("later-link", "Later link");
    const requested: string[] = [];
    server.use(
      ...surfaceHandlers,
      http.get("*/v1/buddy/operator/status", () =>
        HttpResponse.json(operatorStatus("enabled")),
      ),
      http.get("*/v1/buddy/work", ({ request }) => {
        const url = new URL(request.url);
        if (url.searchParams.get("view") === "needs_you")
          return HttpResponse.json(envelope([record(first)]));
        return HttpResponse.json(envelope([]));
      }),
      http.get("*/v1/buddy/work/:id", ({ params }) => {
        requested.push(String(params.id));
        return HttpResponse.json(
          envelope(record(params.id === "later-link" ? linked : first)),
        );
      }),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));

    const { user } = render(<BuddyHome />, { store });

    await user.click(
      await screen.findByRole("button", { name: "Open First preview" }),
    );
    await waitFor(() => expect(requested).toContain("first-preview"));
    await user.click(screen.getByRole("tab", { name: "Home" }));
    expect(screen.getByTestId("buddy-home-overview")).toBeInTheDocument();

    act(() => {
      store.dispatch(
        push({
          name: "buddy",
          workId: "later-link",
          workProjectId: "fixture",
        }),
      );
    });
    expect(screen.getByRole("tab", { name: "Reports" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await waitFor(() => expect(requested).toContain("later-link"));
  });

  it.each(["disabled", "enabled", "unavailable"] as const)(
    "renders the same five sections when %s and lands on Home",
    async (mode) => {
      server.use(
        ...surfaceHandlers,
        http.get("*/v1/buddy/operator/status", () =>
          HttpResponse.json(operatorStatus(mode)),
        ),
        http.get("*/v1/buddy/work", () => HttpResponse.json(envelope([]))),
      );
      const store = setUpStore({ ...CONFIG_STATE });
      store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));

      const { user } = render(<BuddyHome />, { store });

      await screen.findByTestId("buddy-home-content");
      expect(screen.getAllByRole("tab").map((tab) => tab.textContent)).toEqual([
        "Home",
        "Reports",
        "Activity",
        "Play",
        "Settings",
      ]);
      expect(screen.getByRole("tab", { selected: true })).toHaveTextContent(
        "Home",
      );
      await waitFor(() =>
        expect(screen.getByTestId("buddy-operator-status")).toHaveAttribute(
          "data-state",
          mode,
        ),
      );
      await user.click(screen.getByRole("tab", { name: "Reports" }));
      if (mode === "disabled") {
        expect(
          await screen.findByTestId("buddy-pulse-card"),
        ).toBeInTheDocument();
        expect(
          await screen.findByTestId("buddy-opportunities-feed"),
        ).toBeInTheDocument();
      } else {
        expect(screen.queryByTestId("buddy-pulse-card")).toBeNull();
        expect(screen.queryByTestId("buddy-opportunities-feed")).toBeNull();
      }
      if (mode === "unavailable")
        expect(
          within(screen.getByTestId("buddy-home-reports")).getByText(
            /This is a connection failure, not an empty list/,
          ),
        ).toBeInTheDocument();
    },
  );

  it("stops rendering cached progress and blocks setup runs when work authority fails", async () => {
    server.use(
      ...surfaceHandlers,
      http.get("*/v1/buddy/operator/status", () =>
        HttpResponse.json(operatorStatus("unavailable")),
      ),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));

    const { user } = render(<BuddyHome />, { store });

    const banner = await screen.findByTestId("buddy-operator-degraded");
    expect(banner).toHaveAttribute("role", "alert");
    expect(
      within(banner).getByRole("button", { name: "Check connection" }),
    ).toBeInTheDocument();

    await waitFor(() =>
      expect(screen.queryByTestId("buddy-world")).not.toBeInTheDocument(),
    );
    expect(
      within(screen.getByTestId("buddy-home-saved-work")).getByText(
        /connection failure, not an empty list/,
      ),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: "Play" }));
    expect(screen.queryByTestId("buddy-summary-strip")).not.toBeInTheDocument();
    expect(screen.queryByText("View stats")).not.toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "Settings" }));
    const setupStrip = screen.getByTestId("buddy-setup-strip");
    expect(setupStrip).toHaveAttribute("data-disabled", "true");
    const setupButtons = within(setupStrip).getAllByRole("button");
    expect(setupButtons).toHaveLength(SETUP_MODES.length);
    for (const button of setupButtons) expect(button).toBeDisabled();

    const pill = screen.getByTestId("buddy-operator-status");
    expect(pill).toHaveAttribute("data-state", "unavailable");
    expect(pill).toHaveTextContent("Not responding");
  });

  it("keeps the legacy surface whole when the operator is deliberately off", async () => {
    server.use(
      ...surfaceHandlers,
      http.get("*/v1/buddy/operator/status", () =>
        HttpResponse.json(operatorStatus("disabled")),
      ),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));

    const { user } = render(<BuddyHome />, { store });

    await screen.findByTestId("buddy-home-content");
    expect(screen.getByTestId("buddy-world")).toBeInTheDocument();
    expect(
      screen.queryByTestId("buddy-operator-degraded"),
    ).not.toBeInTheDocument();
    expect(
      within(screen.getByTestId("buddy-home-saved-work")).getByText(
        "Reports are off in this project.",
      ),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: "Play" }));
    expect(screen.getByTestId("buddy-summary-strip")).toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: "Settings" }));
    for (const button of within(
      screen.getByTestId("buddy-setup-strip"),
    ).getAllByRole("button")) {
      expect(button).toBeEnabled();
    }
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

  it("cleans up partial reduced-motion addEventListener mocks safely", () => {
    const originalMatchMedia = window.matchMedia;
    const addEventListener = vi.fn();
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: vi.fn(() => ({
        matches: false,
        addEventListener,
      })),
    });

    try {
      const view = render(
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
      );

      expect(addEventListener).toHaveBeenCalledWith(
        "change",
        expect.any(Function),
      );
      expect(() => view.unmount()).not.toThrow();
    } finally {
      Object.defineProperty(window, "matchMedia", {
        configurable: true,
        value: originalMatchMedia,
      });
    }
  });

  it("cleans up legacy reduced-motion addListener mocks safely", () => {
    const originalMatchMedia = window.matchMedia;
    const addListener = vi.fn();
    const removeListener = vi.fn();
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: vi.fn(() => ({
        matches: false,
        addListener,
        removeListener,
      })),
    });

    try {
      const view = render(
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
      );

      expect(addListener).toHaveBeenCalledWith(expect.any(Function));
      view.unmount();
      expect(removeListener).toHaveBeenCalledWith(expect.any(Function));
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

  it("turns serious provider pulse problems into stormy weather and interactive objects", () => {
    const pulse = {
      ...makePulse(),
      providers: { defaults_ok: true, broken_refs: 1, quota_warnings: 0 },
    };
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

  it("ramen_control_has_a_matching_label", async () => {
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
    expect(screen.getByTestId("buddy-world")).toHaveAttribute(
      "data-world-mood",
      "unstable",
    );
    expect(
      screen.getByTestId("buddy-world").getAttribute("data-world-layers"),
    ).toContain("provider_storm");
    expect(screen.getByTestId("buddy-world-canvas")).toBeInTheDocument();
    expect(screen.getByTestId("buddy-world-character")).toBeInTheDocument();
    expect(screen.getByTestId("buddy-world-character")).toHaveAttribute(
      "data-randomize-bubble-position",
      "false",
    );
    expect(screen.getByTestId("buddy-world-character")).toHaveAttribute(
      "data-compact-bubble",
      "false",
    );

    await user.click(screen.getByRole("button", { name: /play in sun/i }));
    expect(onCare).toHaveBeenCalledWith("play", "scroll");

    await user.click(screen.getByRole("button", { name: "Feed Buddy" }));
    expect(onCare).toHaveBeenCalledWith("feed", undefined);

    await user.click(screen.getByRole("button", { name: /open buddy home/i }));
    expect(onOpenPage).toHaveBeenCalledWith({ type: "buddy" });

    await user.click(screen.getByRole("button", { name: /open task grove/i }));
    expect(onOpenPage).toHaveBeenCalledWith({ type: "tasks_list" });
  });

  it("passes semantic state into the world model", () => {
    const state = makeSemanticState({
      activity: {
        mood: "happy",
        animationType: "perk",
        lastSignalTime: new Date("2024-01-01T13:58:00").getTime(),
        lastSignalType: "care_pet",
      },
    });
    const pet = makeSnapshot().state.pet;

    render(
      <BuddyWorld
        palette={PALETTES[0]}
        stage={STAGES[2]}
        state={state}
        pulse={{
          ...makePulse(),
          diagnostics: { last_hour: 0, top_error_types: [] },
          git: { uncommitted_files: 0, diff_lines_4h: 0, branches: 3 },
          mcp: { total: 4, failing: 0, auth_expiring: 0 },
          memory: { total: 10, orphan: 0, stale_conflicts: 0 },
        }}
        pet={{
          ...pet,
          needs: { ...pet.needs, affection: 35 },
        }}
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
    );

    expect(screen.getByTestId("buddy-world")).toHaveAttribute(
      "data-atmosphere-mood",
      "affectionate",
    );
  });

  it("updates scene time when care activity arrives without explicit now", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2024, 0, 1, 14, 0, 0));
    try {
      const quietPulse = {
        ...makePulse(),
        diagnostics: { last_hour: 0, top_error_types: [] },
        git: { uncommitted_files: 0, diff_lines_4h: 0, branches: 3 },
        mcp: { total: 4, failing: 0, auth_expiring: 0 },
        memory: { total: 10, orphan: 0, stale_conflicts: 0 },
      };
      const pet = makeSnapshot().state.pet;
      const calmPet = {
        ...pet,
        needs: { ...pet.needs, affection: 35 },
      };
      const { rerender } = render(
        <BuddyWorld
          palette={PALETTES[0]}
          stage={STAGES[2]}
          state={makeSemanticState()}
          pulse={quietPulse}
          pet={calmPet}
          nowPlaying={null}
          activeQuest={null}
          activeSpeech={null}
          setupNeeded={false}
          onCanvasEvent={vi.fn()}
          onCare={vi.fn()}
          onOpenPage={vi.fn()}
          onRunMode={vi.fn()}
          onDismissSetup={vi.fn()}
          onSpeechControl={vi.fn()}
        />,
        { preloadedState: CONFIG_STATE },
      );

      expect(screen.getByTestId("buddy-world")).toHaveAttribute(
        "data-atmosphere-mood",
        "curious",
      );

      await vi.advanceTimersByTimeAsync(30_000);
      const lastSignalTime = new Date(2024, 0, 1, 14, 0, 30).getTime();
      rerender(
        <BuddyWorld
          palette={PALETTES[0]}
          stage={STAGES[2]}
          state={makeSemanticState({
            activity: {
              mood: "happy",
              animationType: "perk",
              lastSignalTime,
              lastSignalType: "care_pet",
            },
          })}
          pulse={quietPulse}
          pet={calmPet}
          nowPlaying={null}
          activeQuest={null}
          activeSpeech={null}
          setupNeeded={false}
          onCanvasEvent={vi.fn()}
          onCare={vi.fn()}
          onOpenPage={vi.fn()}
          onRunMode={vi.fn()}
          onDismissSetup={vi.fn()}
          onSpeechControl={vi.fn()}
        />,
      );

      await vi.advanceTimersByTimeAsync(1);
      expect(screen.getByTestId("buddy-world")).toHaveAttribute(
        "data-atmosphere-mood",
        "affectionate",
      );
    } finally {
      vi.useRealTimers();
    }
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

  it("retries runtime showcase promptly when runtime cooldown expires", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2024-01-01T00:00:40Z"));
    const randomSpy = vi.spyOn(Math, "random").mockReturnValue(0.9);
    try {
      const quietPulse = {
        ...makePulse(),
        diagnostics: { last_hour: 0, top_error_types: [] },
        git: { uncommitted_files: 0, diff_lines_4h: 0, branches: 3 },
        mcp: { total: 4, failing: 0, auth_expiring: 0 },
        memory: { total: 10, orphan: 0, stale_conflicts: 0 },
      };
      const runtimeEvent = {
        id: "runtime-cooldown-a",
        signal_type: "memory_extract",
        title: "Memory extracted",
        source: "test",
        status: "completed",
        priority: "normal",
        created_at: "2024-01-01T00:00:00Z",
      } satisfies BuddyRuntimeEvent;
      const { rerender } = render(
        <BuddyWorld
          palette={PALETTES[0]}
          stage={STAGES[2]}
          state={makeSemanticState()}
          pulse={quietPulse}
          pet={makeSnapshot().state.pet}
          nowPlaying={runtimeEvent}
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

      await vi.advanceTimersByTimeAsync(12_900);
      expect(screen.getByTestId("buddy-world")).toHaveAttribute(
        "data-showcase",
        "none",
      );

      rerender(
        <BuddyWorld
          palette={PALETTES[0]}
          stage={STAGES[2]}
          state={makeSemanticState()}
          pulse={quietPulse}
          pet={makeSnapshot().state.pet}
          nowPlaying={{ ...runtimeEvent, id: "runtime-cooldown-b" }}
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
        "none",
      );
      await vi.advanceTimersByTimeAsync(4_900);
      expect(screen.getByTestId("buddy-world")).toHaveAttribute(
        "data-showcase",
        "none",
      );
      await vi.advanceTimersByTimeAsync(400);
      expect(screen.getByTestId("buddy-world")).toHaveAttribute(
        "data-showcase",
        "memory_firefly_night",
      );
    } finally {
      randomSpy.mockRestore();
      vi.useRealTimers();
    }
  });

  it("does not retry the same runtime event id after cooldown", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2024-01-01T00:00:40Z"));
    const randomSpy = vi.spyOn(Math, "random").mockReturnValue(0.9);
    try {
      const runtimeEvent = {
        id: "runtime-same-id",
        signal_type: "memory_extract",
        title: "Memory extracted",
        source: "test",
        status: "completed",
        priority: "normal",
        created_at: "2024-01-01T00:00:00Z",
      } satisfies BuddyRuntimeEvent;
      render(
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
          nowPlaying={runtimeEvent}
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

      await vi.advanceTimersByTimeAsync(12_900);
      expect(screen.getByTestId("buddy-world")).toHaveAttribute(
        "data-showcase",
        "none",
      );
      await vi.advanceTimersByTimeAsync(18_000);
      expect(screen.getByTestId("buddy-world")).toHaveAttribute(
        "data-showcase",
        "none",
      );
    } finally {
      randomSpy.mockRestore();
      vi.useRealTimers();
    }
  });

  it("lets a user care click preempt a running showcase with a visible world activity", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2024-01-01T00:00:40Z"));
    const onCare = vi.fn();
    try {
      render(
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
            id: "runtime-showcase-speech",
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
          onCare={onCare}
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

      expect(screen.getByTestId("buddy-world")).toHaveAttribute(
        "data-showcase-phase",
        "travel",
      );
      fireEvent.click(screen.getByRole("button", { name: /play in sun/i }));
      expect(onCare).toHaveBeenCalledWith("play", "scroll");

      expect(screen.getByTestId("buddy-world")).toHaveAttribute(
        "data-showcase",
        "none",
      );
      expect(screen.getByTestId("buddy-world")).toHaveAttribute(
        "data-care-activity",
        "play",
      );
      expect(screen.getByTestId("buddy-world")).toHaveAttribute(
        "data-speech-source",
        "care",
      );
      expect(screen.getByTestId("buddy-world")).toHaveAttribute(
        "data-speech-priority",
        "backend-care-session-arc-showcase-director-local",
      );
      expect(screen.getByTestId("buddy-world")).toHaveAttribute(
        "data-speech-text",
        "Buddy catches a warm sunbeam and opens the focus scroll.",
      );

      await vi.advanceTimersByTimeAsync(3_817);
      expect(screen.getByTestId("buddy-world-character")).toHaveAttribute(
        "data-pose",
        "pounce",
      );
      expect(screen.getByTestId("buddy-world")).toHaveAttribute(
        "data-care-activity",
        "play",
      );

      await vi.advanceTimersByTimeAsync(7_700);
      expect(screen.getByTestId("buddy-world")).toHaveAttribute(
        "data-care-activity",
        "none",
      );
    } finally {
      vi.useRealTimers();
    }
  });

  it("runtime world can set channel runtime director intent", async () => {
    vi.useFakeTimers();
    const now = new Date("2024-01-01T14:00:00");
    vi.setSystemTime(now);
    const runtimeEvent: BuddyRuntimeEvent = {
      id: "rt-director-1",
      signal_type: "indexing",
      title: "Indexing project files",
      source: "indexer",
      status: "progress",
      priority: "normal",
      created_at: now.toISOString(),
    };
    try {
      render(
        <BuddyWorld
          palette={PALETTES[0]}
          stage={STAGES[2]}
          state={makeSemanticState()}
          pulse={{
            ...makePulse(),
            providers: { defaults_ok: true, broken_refs: 0, quota_warnings: 0 },
            diagnostics: { last_hour: 0, top_error_types: [] },
            git: { uncommitted_files: 0, diff_lines_4h: 0, branches: 3 },
            mcp: { total: 4, failing: 0, auth_expiring: 0 },
            memory: { total: 10, orphan: 0, stale_conflicts: 0 },
          }}
          pet={makeSnapshot().state.pet}
          nowPlaying={runtimeEvent}
          activeQuest={null}
          activeSpeech={null}
          setupNeeded={false}
          now={now}
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
        "data-buddy-intent",
        "channel_runtime",
      );
      expect(screen.getByTestId("buddy-world")).toHaveAttribute(
        "data-speech-source",
        "director",
      );
      expect(screen.getByTestId("buddy-world-character")).toHaveAttribute(
        "data-pose",
        "meditate",
      );
    } finally {
      vi.useRealTimers();
    }
  });

  it("uses right bubble for left-side director targets", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2024-01-01T14:00:00Z"));
    try {
      const pet = makeSnapshot().state.pet;
      render(
        <BuddyWorld
          palette={PALETTES[0]}
          stage={STAGES[2]}
          state={makeSemanticState()}
          pulse={{
            ...makePulse(),
            providers: { defaults_ok: true, broken_refs: 0, quota_warnings: 0 },
            diagnostics: { last_hour: 0, top_error_types: [] },
            git: { uncommitted_files: 0, diff_lines_4h: 0, branches: 3 },
            mcp: { total: 4, failing: 0, auth_expiring: 0 },
            memory: { total: 10, orphan: 0, stale_conflicts: 0 },
          }}
          pet={{
            ...pet,
            condition: { ...pet.condition, hungry: true },
          }}
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
      );

      await vi.advanceTimersByTimeAsync(1);
      expect(screen.getByTestId("buddy-world")).toHaveAttribute(
        "data-buddy-intent",
        "seek_food",
      );
      expect(screen.getByTestId("buddy-world-character")).toHaveAttribute(
        "data-bubble-position",
        "right",
      );
    } finally {
      vi.useRealTimers();
    }
  });

  it("uses left bubble for right-side director targets", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2024-01-01T14:00:00Z"));
    try {
      render(
        <BuddyWorld
          palette={PALETTES[0]}
          stage={STAGES[2]}
          state={makeSemanticState()}
          pulse={{
            ...makePulse(),
            providers: { defaults_ok: true, broken_refs: 1, quota_warnings: 0 },
            diagnostics: { last_hour: 0, top_error_types: [] },
            git: { uncommitted_files: 0, diff_lines_4h: 0, branches: 3 },
            mcp: { total: 4, failing: 0, auth_expiring: 0 },
            memory: { total: 10, orphan: 0, stale_conflicts: 0 },
          }}
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
      );

      await vi.advanceTimersByTimeAsync(1);
      expect(screen.getByTestId("buddy-world")).toHaveAttribute(
        "data-buddy-intent",
        "stabilize_crystal",
      );
      expect(screen.getByTestId("buddy-world-character")).toHaveAttribute(
        "data-bubble-position",
        "left",
      );
    } finally {
      vi.useRealTimers();
    }
  });

  it("uses a side bubble for elevated center director targets so speech stays unclipped", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2024-01-01T08:00:00Z"));
    try {
      const pet = makeSnapshot().state.pet;
      render(
        <BuddyWorld
          palette={PALETTES[0]}
          stage={STAGES[2]}
          state={makeSemanticState()}
          pulse={{
            ...makePulse(),
            providers: { defaults_ok: true, broken_refs: 0, quota_warnings: 0 },
            diagnostics: { last_hour: 0, top_error_types: [] },
            git: { uncommitted_files: 0, diff_lines_4h: 0, branches: 3 },
            mcp: { total: 4, failing: 0, auth_expiring: 0 },
            memory: { total: 10, orphan: 0, stale_conflicts: 0 },
          }}
          pet={{
            ...pet,
            needs: { ...pet.needs, affection: 35 },
          }}
          nowPlaying={null}
          activeQuest={null}
          activeSpeech={null}
          setupNeeded={false}
          now={new Date("2024-01-01T08:00:00")}
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
        "data-buddy-intent",
        "morning_stretch",
      );
      expect(screen.getByTestId("buddy-world-character")).toHaveAttribute(
        "data-bubble-position",
        "right",
      );
    } finally {
      vi.useRealTimers();
    }
  });

  it("keeps side bubbles for ordinary speech and long compact speech", () => {
    const longSpeech =
      "The observatory has a very long update that should avoid short-scene top clipping in compact scenes.";

    expect(bubblePositionForSceneX(67, false, longSpeech)).toBe("left");
    expect(bubblePositionForSceneX(67, true, "Short update.")).toBe("left");
    expect(bubblePositionForSceneX(38, true, "Short update.")).toBe("right");
    expect(bubblePositionForSceneX(67, true, longSpeech)).toBe("left");
    expect(bubblePositionForSceneX(38, true, longSpeech)).toBe("right");
    expect(
      bubblePositionForSceneX(50, true, "A medium compact line here"),
    ).toBe("right");
  });

  it("keeps compact long active speech beside the buddy after moving to a side target", async () => {
    const longSpeech =
      "The observatory has a very long update that should avoid narrow side clipping in compact scenes.";
    const onOpenPage = vi.fn();

    const { user } = render(
      <BuddyWorld
        palette={PALETTES[0]}
        stage={STAGES[2]}
        state={makeSemanticState()}
        pulse={{
          ...makePulse(),
          providers: { defaults_ok: true, broken_refs: 0, quota_warnings: 0 },
        }}
        pet={makeSnapshot().state.pet}
        nowPlaying={null}
        activeQuest={null}
        activeSpeech={{ text: longSpeech, controls: [] }}
        setupNeeded={false}
        compact
        now={new Date("2024-01-01T14:00:00")}
        onCanvasEvent={vi.fn()}
        onCare={vi.fn()}
        onOpenPage={onOpenPage}
        onRunMode={vi.fn()}
        onDismissSetup={vi.fn()}
        onSpeechControl={vi.fn()}
      />,
      { preloadedState: CONFIG_STATE },
    );

    await user.click(
      screen.getByRole("button", { name: /open model observatory/i }),
    );

    await waitFor(() => {
      expect(screen.getByTestId("buddy-world-character")).toHaveStyle({
        left: "67%",
      });
    });
    expect(screen.getByTestId("buddy-world-character")).toHaveAttribute(
      "data-bubble-position",
      "left",
    );
    expect(screen.getByTestId("buddy-world-character")).toHaveAttribute(
      "data-compact-bubble",
      "true",
    );
    expect(screen.getByTestId("buddy-world")).toHaveAttribute(
      "data-speech-text",
      longSpeech,
    );
    expect(onOpenPage).toHaveBeenCalledWith({ type: "default_models" });
  });

  it("active speech immediately suppresses a rendered director intent", async () => {
    vi.useFakeTimers();
    const now = new Date("2024-01-01T14:00:00");
    vi.setSystemTime(now);
    const runtimeEvent: BuddyRuntimeEvent = {
      id: "rt-director-active-speech",
      signal_type: "indexing",
      title: "Indexing project files",
      source: "indexer",
      status: "progress",
      priority: "normal",
      created_at: now.toISOString(),
    };
    const quietPulse = makePulse({
      diagnostics: { last_hour: 0, top_error_types: [] },
      git: { uncommitted_files: 0, diff_lines_4h: 0, branches: 3 },
      mcp: { total: 4, failing: 0, auth_expiring: 0 },
      memory: { total: 10, orphan: 0, stale_conflicts: 0 },
    });
    try {
      const { rerender } = render(
        <BuddyWorld
          palette={PALETTES[0]}
          stage={STAGES[2]}
          state={makeSemanticState()}
          pulse={quietPulse}
          pet={makeSnapshot().state.pet}
          nowPlaying={runtimeEvent}
          activeQuest={null}
          activeSpeech={null}
          setupNeeded={false}
          now={now}
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
        "data-buddy-intent",
        "channel_runtime",
      );
      expect(screen.getByTestId("buddy-world-character")).toHaveAttribute(
        "data-pose",
        "meditate",
      );
      expect(screen.getByTestId("buddy-world-character")).toHaveStyle({
        left: "54%",
      });

      rerender(
        <BuddyWorld
          palette={PALETTES[0]}
          stage={STAGES[2]}
          state={makeSemanticState()}
          pulse={quietPulse}
          pet={makeSnapshot().state.pet}
          nowPlaying={runtimeEvent}
          activeQuest={null}
          activeSpeech={{ text: "Backend says hello.", controls: [] }}
          setupNeeded={false}
          now={now}
          onCanvasEvent={vi.fn()}
          onCare={vi.fn()}
          onOpenPage={vi.fn()}
          onRunMode={vi.fn()}
          onDismissSetup={vi.fn()}
          onSpeechControl={vi.fn()}
        />,
      );

      expect(screen.getByTestId("buddy-world")).toHaveAttribute(
        "data-buddy-intent",
        "none",
      );
      expect(screen.getByTestId("buddy-world")).toHaveAttribute(
        "data-speech-source",
        "active",
      );
      expect(screen.getByTestId("buddy-world")).toHaveAttribute(
        "data-speech-text",
        "Backend says hello.",
      );
      expect(screen.getByTestId("buddy-world-character")).toHaveAttribute(
        "data-pose",
        "idle",
      );
      expect(screen.getByTestId("buddy-world-character")).toHaveStyle({
        left: "50%",
      });
      expect(screen.getByTestId("buddy-world-character")).not.toHaveAttribute(
        "data-depth-scale",
      );
    } finally {
      vi.useRealTimers();
    }
  });

  it("honors director intent duration before replacing it", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2024-01-01T14:00:00"));
    const quietPulse = makePulse({
      providers: { defaults_ok: true, broken_refs: 0, quota_warnings: 0 },
      diagnostics: { last_hour: 0, top_error_types: [] },
      git: { uncommitted_files: 0, diff_lines_4h: 0, branches: 3 },
      mcp: { total: 4, failing: 0, auth_expiring: 0 },
      memory: { total: 10, orphan: 0, stale_conflicts: 0 },
    });
    const pet = makeSnapshot().state.pet;
    const sleepingPet = {
      ...pet,
      condition: {
        ...pet.condition,
        sleeping: true,
      },
    };
    const hungryPet = {
      ...pet,
      condition: {
        ...pet.condition,
        hungry: true,
      },
    };
    try {
      const { rerender } = render(
        <BuddyWorld
          palette={PALETTES[0]}
          stage={STAGES[2]}
          state={makeSemanticState()}
          pulse={quietPulse}
          pet={sleepingPet}
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
      );

      await vi.advanceTimersByTimeAsync(1);
      expect(screen.getByTestId("buddy-world")).toHaveAttribute(
        "data-buddy-intent",
        "rest_home",
      );

      await vi.advanceTimersByTimeAsync(7_100);
      vi.setSystemTime(new Date("2024-01-01T14:00:07.101"));
      rerender(
        <BuddyWorld
          palette={PALETTES[0]}
          stage={STAGES[2]}
          state={makeSemanticState()}
          pulse={quietPulse}
          pet={hungryPet}
          nowPlaying={null}
          activeQuest={null}
          activeSpeech={null}
          setupNeeded={false}
          now={new Date("2024-01-01T14:00:07.101")}
          onCanvasEvent={vi.fn()}
          onCare={vi.fn()}
          onOpenPage={vi.fn()}
          onRunMode={vi.fn()}
          onDismissSetup={vi.fn()}
          onSpeechControl={vi.fn()}
        />,
      );
      await vi.advanceTimersByTimeAsync(1);
      expect(screen.getByTestId("buddy-world")).toHaveAttribute(
        "data-buddy-intent",
        "rest_home",
      );

      vi.setSystemTime(new Date("2024-01-01T14:00:12.101"));
      rerender(
        <BuddyWorld
          palette={PALETTES[0]}
          stage={STAGES[2]}
          state={makeSemanticState()}
          pulse={quietPulse}
          pet={hungryPet}
          nowPlaying={null}
          activeQuest={null}
          activeSpeech={null}
          setupNeeded={false}
          now={new Date("2024-01-01T14:00:12.101")}
          onCanvasEvent={vi.fn()}
          onCare={vi.fn()}
          onOpenPage={vi.fn()}
          onRunMode={vi.fn()}
          onDismissSetup={vi.fn()}
          onSpeechControl={vi.fn()}
        />,
      );
      await vi.advanceTimersByTimeAsync(1);
      expect(screen.getByTestId("buddy-world")).toHaveAttribute(
        "data-buddy-intent",
        "seek_food",
      );
    } finally {
      vi.useRealTimers();
    }
  });

  it("showcase still wins over director", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2024-01-01T00:00:40Z"));
    try {
      render(
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
            id: "runtime-showcase-director",
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
      expect(screen.getByTestId("buddy-world")).toHaveAttribute(
        "data-buddy-intent",
        "none",
      );
      expect(screen.getByTestId("buddy-world")).toHaveAttribute(
        "data-speech-source",
        "showcase",
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
      created_at: "2024-01-01T13:59:30Z",
    };
    const world = buildBuddyWorldState({
      now: new Date("2024-01-01T14:00:00Z"),
      pulse: {
        ...makePulse(),
        diagnostics: { last_hour: 0, top_error_types: [] },
        git: { uncommitted_files: 0, diff_lines_4h: 0, branches: 3 },
        mcp: { total: 4, failing: 0, auth_expiring: 0 },
        memory: { total: 10, orphan: 0, stale_conflicts: 0 },
      },
      pet: makeSnapshot().state.pet,
      nowPlaying: runtimeEvent,
      activeQuest: null,
    });

    expect(world.weather).toBe("busy");
    expect(world.weatherDescription).toBe("Running browser checks");
  });

  it("ignores dismissed active runtime work for busy weather", () => {
    const world = buildBuddyWorldState({
      now: new Date("2024-01-01T14:00:00Z"),
      pulse: {
        ...makePulse(),
        diagnostics: { last_hour: 0, top_error_types: [] },
        git: { uncommitted_files: 0, diff_lines_4h: 0, branches: 3 },
        mcp: { total: 4, failing: 0, auth_expiring: 0 },
        memory: { total: 10, orphan: 0, stale_conflicts: 0 },
      },
      pet: makeSnapshot().state.pet,
      nowPlaying: {
        id: "rt-dismissed",
        signal_type: "tool_used",
        title: "Running browser checks",
        source: "test",
        status: "progress",
        priority: "normal",
        created_at: "2024-01-01T13:59:00Z",
        dismissed: true,
      },
      activeQuest: null,
    });

    expect(world.weather).not.toBe("busy");
    expect(world.headline).not.toContain("Running browser checks");
  });

  it("ignores expired non-persistent active runtime work for busy weather", () => {
    const world = buildBuddyWorldState({
      now: new Date("2024-01-01T14:00:00Z"),
      pulse: {
        ...makePulse(),
        diagnostics: { last_hour: 0, top_error_types: [] },
        git: { uncommitted_files: 0, diff_lines_4h: 0, branches: 3 },
        mcp: { total: 4, failing: 0, auth_expiring: 0 },
        memory: { total: 10, orphan: 0, stale_conflicts: 0 },
      },
      pet: makeSnapshot().state.pet,
      nowPlaying: {
        id: "rt-expired",
        signal_type: "tool_used",
        title: "Running browser checks",
        source: "test",
        status: "progress",
        priority: "normal",
        created_at: "2024-01-01T13:58:00Z",
        ttl_ms: 30_000,
        persistent: false,
      },
      activeQuest: null,
    });

    expect(world.weather).not.toBe("busy");
    expect(world.headline).not.toContain("Running browser checks");
  });

  it("keeps persistent active runtime work eligible for busy weather", () => {
    const world = buildBuddyWorldState({
      now: new Date("2024-01-01T14:00:00Z"),
      pulse: {
        ...makePulse(),
        diagnostics: { last_hour: 0, top_error_types: [] },
        git: { uncommitted_files: 0, diff_lines_4h: 0, branches: 3 },
        mcp: { total: 4, failing: 0, auth_expiring: 0 },
        memory: { total: 10, orphan: 0, stale_conflicts: 0 },
      },
      pet: makeSnapshot().state.pet,
      nowPlaying: {
        id: "rt-persistent",
        signal_type: "tool_used",
        title: "Running browser checks",
        source: "test",
        status: "progress",
        priority: "normal",
        created_at: "2024-01-01T13:58:00Z",
        ttl_ms: 30_000,
        persistent: true,
      },
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
      created_at: new Date().toISOString(),
      ...overrides,
    };
  }
  it("includes runtime descriptions for non-error notifications", () => {
    const runtime = makeRuntimeEvent({
      title: "Setup ready",
      description: "Connect GitHub to enable issue sync.",
    });
    const speech = buildBuddySceneSpeech({
      activeSpeech: null,
      nowPlaying: runtime,
      runtimeQueue: [],
      activeSuggestion: null,
    });

    expect(formatBuddyRuntimeEventText(runtime)).toBe(
      "Setup ready: Connect GitHub to enable issue sync.",
    );
    expect(speech?.text).toBe(formatBuddyRuntimeEventText(runtime));
  });

  it("uses speech text before runtime title or description", () => {
    const runtime = makeRuntimeEvent({
      title: "Hidden title",
      description: "Hidden description",
      speech_text: "  Server says this instead.  ",
    });
    const speech = buildBuddySceneSpeech({
      activeSpeech: null,
      nowPlaying: runtime,
      runtimeQueue: [],
      activeSuggestion: null,
    });

    expect(formatBuddyRuntimeEventText(runtime)).toBe(
      "Server says this instead.",
    );
    expect(speech?.text).toBe(formatBuddyRuntimeEventText(runtime));
  });

  it("strips noisy runtime prefixes consistently", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2024-01-01T00:00:01Z"));
    try {
      const runtime = makeRuntimeEvent({
        title: "generic: LLM error",
        description: "LLM error: upstream returned 429",
        status: "failed",
        priority: "high",
        created_at: "2024-01-01T00:00:00Z",
      });
      const speech = buildBuddySceneSpeech({
        activeSpeech: null,
        nowPlaying: runtime,
        runtimeQueue: [],
        activeSuggestion: null,
      });

      expect(formatBuddyRuntimeEventText(runtime)).toBe(
        "I hit an LLM snag: upstream returned 429",
      );
      expect(speech?.text).toBe(formatBuddyRuntimeEventText(runtime));
    } finally {
      vi.useRealTimers();
    }
  });

  it("ignores expired active speech before choosing home speech", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2024-01-01T00:01:00Z"));
    try {
      const expired = makeSpeech({
        id: "expired-home-speech",
        text: "Expired home speech",
        ttl_seconds: 5,
        created_at: "2024-01-01T00:00:00Z",
      });
      const speech = buildBuddySceneSpeech({
        activeSpeech: expired,
        nowPlaying: makeRuntimeEvent({
          id: "runtime-after-expired-speech",
          title: "Runtime after expired speech",
          created_at: "2024-01-01T00:00:59Z",
        }),
        runtimeQueue: [],
        activeSuggestion: null,
      });

      expect(isBuddySpeechExpired(expired)).toBe(true);
      expect(speech?.source).toBe("runtime");
      expect(speech?.text).toBe("Runtime after expired speech");
    } finally {
      vi.useRealTimers();
    }
  });

  it("prioritizes critical queued failures over low-priority now playing", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2024-01-01T00:00:01Z"));
    try {
      const freshCreatedAt = "2024-01-01T00:00:00Z";
      const speech = buildBuddySceneSpeech({
        activeSpeech: null,
        nowPlaying: makeRuntimeEvent({
          id: "now-playing",
          title: "Indexing quietly",
          priority: "low",
          status: "progress",
          created_at: freshCreatedAt,
        }),
        runtimeQueue: [
          makeRuntimeEvent({
            id: "critical-error",
            title: "Provider failed",
            description: "The default model key was rejected.",
            priority: "critical",
            status: "failed",
            created_at: freshCreatedAt,
          }),
        ],
        activeSuggestion: null,
      });

      expect(speech?.runtimeEventId).toBe("critical-error");
      expect(speech?.text).toBe(
        "Provider failed: The default model key was rejected.",
      );
    } finally {
      vi.useRealTimers();
    }
  });

  it("turns repeated context-window errors into Buddy language", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2024-01-01T00:00:01Z"));
    try {
      const runtime = makeRuntimeEvent({
        id: "context-error",
        title: "generic: LLM error",
        description:
          "LLM error: Your input exceeds the context window of this model. Please adjust your input and try again. LLM error: Your input exceeds the context window of this model.",
        priority: "high",
        status: "failed",
        created_at: "2024-01-01T00:00:00Z",
      });
      const speech = buildBuddySceneSpeech({
        activeSpeech: null,
        nowPlaying: runtime,
        runtimeQueue: [],
        activeSuggestion: null,
      });

      expect(formatBuddyRuntimeEventText(runtime)).toBe(
        "I ran out of context room. Want me to compress this and try again?",
      );
      expect(speech?.text).toBe(formatBuddyRuntimeEventText(runtime));
      expect(speech?.controls.map((control) => control.action)).toEqual([
        "investigate_error",
        "dismiss_runtime_event",
      ]);
      expect(speech?.controls[1]?.action_param).toBe("context-error");
    } finally {
      vi.useRealTimers();
    }
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

  it("does not silence actionable runtime speech", () => {
    const speech = buildBuddySceneSpeech({
      activeSpeech: null,
      nowPlaying: makeRuntimeEvent({
        id: "actionable-runtime",
        title: "Runtime needs action",
        status: "failed",
        priority: "high",
        created_at: new Date().toISOString(),
      }),
      runtimeQueue: [],
      activeSuggestion: null,
    });

    expect(speech).not.toBeNull();
    expect(speech?.mustShow).toBe(true);
    expect(pickBuddySceneSpeechCandidate(speech ? [speech] : [])).toBe(speech);
  });

  it("can silence non-error runtime speech with backend dismiss controls", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("1970-01-01T00:00:00Z"));
    try {
      const speech = buildBuddySceneSpeech({
        activeSpeech: null,
        nowPlaying: makeRuntimeEvent({
          id: "controlled-runtime-notice",
          title: "Runtime notice with controls",
          status: "info",
          priority: "normal",
          controls: [
            {
              id: "dismiss-controlled-runtime-notice",
              label: "Dismiss",
              action: "dismiss_runtime_event",
              action_param: "controlled-runtime-notice",
              style: "secondary",
            },
          ],
        }),
        runtimeQueue: [],
        activeSuggestion: null,
      });

      expect(speech?.source).toBe("runtime");
      expect(speech?.mustShow).toBe(false);
      expect(speech?.controls[0]?.action).toBe("dismiss_runtime_event");
      expect(pickBuddySceneSpeechCandidate(speech ? [speech] : [])).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("can silence suggestion speech with controls", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("1970-01-01T00:00:00Z"));
    try {
      const speech = buildBuddySceneSpeech({
        activeSpeech: null,
        nowPlaying: null,
        runtimeQueue: [],
        activeSuggestion: makeSuggestion({
          controls: [
            {
              id: "dismiss-controlled-suggestion",
              label: "Dismiss",
              action: "dismiss",
              style: "secondary",
            },
          ],
        }),
      });

      expect(speech?.source).toBe("suggestion");
      expect(speech).not.toHaveProperty("mustShow");
      expect(speech?.controls).toHaveLength(1);
      expect(pickBuddySceneSpeechCandidate(speech ? [speech] : [])).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("can silence opportunity speech with controls", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("1970-01-01T00:00:00Z"));
    try {
      const speech = buildBuddySceneSpeech({
        activeSpeech: null,
        nowPlaying: null,
        runtimeQueue: [],
        activeSuggestion: null,
        activeOpportunities: [
          makeOpportunity({
            id: "controlled-opportunity",
            proposed_actions: [{ kind: "dismiss" }],
          }),
        ],
      });

      expect(speech?.source).toBe("opportunity");
      expect(speech).not.toHaveProperty("mustShow");
      expect(speech?.controls).toHaveLength(1);
      expect(pickBuddySceneSpeechCandidate(speech ? [speech] : [])).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("prefers actionable runtime speech over suggestion speech with controls", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("1970-01-01T00:00:00Z"));
    try {
      const runtimeSpeech = buildBuddySceneSpeech({
        activeSpeech: null,
        nowPlaying: makeRuntimeEvent({
          id: "actionable-runtime-mixed",
          title: "Runtime needs action",
          status: "failed",
          priority: "high",
          created_at: new Date().toISOString(),
        }),
        runtimeQueue: [],
        activeSuggestion: null,
      });
      const suggestionSpeech = buildBuddySceneSpeech({
        activeSpeech: null,
        nowPlaying: null,
        runtimeQueue: [],
        activeSuggestion: makeSuggestion({
          controls: [
            {
              id: "dismiss-controlled-suggestion-mixed",
              label: "Dismiss",
              action: "dismiss",
              style: "secondary",
            },
          ],
        }),
      });

      expect(runtimeSpeech?.source).toBe("runtime");
      expect(suggestionSpeech?.source).toBe("suggestion");
      expect(
        pickBuddySceneSpeechCandidate(
          [suggestionSpeech, runtimeSpeech].filter(
            (speech): speech is BuddySceneSpeech => speech != null,
          ),
        ),
      ).toBe(runtimeSpeech);
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("BuddyDraftPreview_renders_draft_metadata", () => {
  it("shows title and explanation from draft without DOM nesting warnings", () => {
    const draft: BuddyDraft = {
      id: "draft-test",
      kind: "skill",
      title: "My Test Draft Title",
      yaml_or_json: "{}",
      explanation: "This is the explanation text",
      created_at: "2024-01-01T00:00:00Z",
      expires_at: "2099-12-31T00:00:00Z",
    };
    const consoleError = vi
      .spyOn(console, "error")
      .mockImplementation(() => undefined);

    try {
      render(<BuddyDraftPreview draft={draft} />, {
        preloadedState: CONFIG_STATE,
      });

      expect(screen.getByText(/My Test Draft Title/)).toBeInTheDocument();
      expect(
        screen.getByText(/This is the explanation text/),
      ).toBeInTheDocument();
      expect(
        consoleError.mock.calls.some((call) =>
          call.some((arg) => String(arg).includes("validateDOMNesting")),
        ),
      ).toBe(false);
    } finally {
      consoleError.mockRestore();
    }
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
      http.get("*/v1/buddy/opportunities", () =>
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

describe("BuddyHome_disabled_state", () => {
  it("disabled Buddy Home shows settings gear and Buddy enabled switch for re-enable", async () => {
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot(undefined, {
          enabled: false,
          settings: { ...makeSnapshot().settings, enabled: false },
        }),
      ),
    );

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

    const { user } = render(<BuddyHome />, { store });

    const settingsBtn = await screen.findByRole("button", {
      name: "Settings",
    });
    expect(settingsBtn).toBeInTheDocument();

    await user.click(settingsBtn);

    const enabledSwitch = await screen.findByRole("switch", {
      name: /buddy enabled/i,
    });
    expect(enabledSwitch).toBeInTheDocument();
    expect(enabledSwitch).not.toBeChecked();

    const settingsSection = await screen.findByTestId(
      "buddy-home-settings-section",
    );
    expect(settingsSection).toBeInTheDocument();
    expect(settingsSection).not.toHaveAttribute("style");
  });

  it("disabled Buddy Home enable button sends exact enabled patch", async () => {
    let capturedBody: unknown;
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot(undefined, {
          enabled: false,
          settings: { ...makeSnapshot().settings, enabled: false },
        }),
      ),
    );

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
      http.post("*/v1/buddy/settings", async ({ request }) => {
        capturedBody = await request.json();
        return HttpResponse.json(defaultBuddySettings());
      }),
    );

    const { user } = render(<BuddyHome />, { store });

    expect(
      await screen.findByTestId("buddy-home-disabled"),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Enable Buddy" }));

    await waitFor(() => {
      expect(capturedBody).toEqual({ enabled: true });
    });
  });

  it("settings.enabled false shows disabled UI when top-level enabled is stale", async () => {
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot(undefined, {
          enabled: true,
          settings: { ...makeSnapshot().settings, enabled: false },
        }),
      ),
    );

    server.use(
      http.get("*/v1/stats/llm/summary", () =>
        HttpResponse.json({
          totals: { total_calls: 0, successful_calls: 0, total_tokens: 0 },
        }),
      ),
      http.get("*/v1/setup/status", () =>
        HttpResponse.json({ configured: true, reasons: [], detail: {} }),
      ),
    );

    const { user } = render(<BuddyHome />, { store });

    expect(
      await screen.findByTestId("buddy-home-disabled"),
    ).toBeInTheDocument();
    expect(screen.queryByTestId("buddy-home-content")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Settings" }));

    const enabledSwitch = await screen.findByRole("switch", {
      name: /buddy enabled/i,
    });
    expect(enabledSwitch).not.toBeChecked();
  });
});

describe("BuddyRecentChats_opens_existing_chat", () => {
  it("system recent rows are not enabled focusable no-op buttons", async () => {
    let trajectoryCalled = false;
    server.use(
      http.get("*/v1/buddy/conversations", () =>
        HttpResponse.json([
          makeConversation({
            id: "system-row",
            kind: "system",
            title: "System maintenance note",
            badge: "System",
            status: "active",
            message_count: 0,
          }),
        ]),
      ),
      http.get("*/v1/trajectories/:id", () => {
        trajectoryCalled = true;
        return HttpResponse.json({});
      }),
    );

    render(<BuddyRecentChats showFilters={false} compact />, {
      preloadedState: CONFIG_STATE,
    });

    const title = await screen.findByText("System maintenance note");
    expect(title.closest("button")).toBeNull();
    expect(
      screen.queryByRole("button", { name: /system maintenance note/i }),
    ).not.toBeInTheDocument();

    fireEvent.click(title);

    expect(trajectoryCalled).toBe(false);
  });

  it("hides stale empty placeholders ahead of real Buddy chats", async () => {
    server.use(
      http.get("*/v1/buddy/conversations", () =>
        HttpResponse.json([
          makeConversation({
            id: "old-empty-placeholder",
            kind: "chat",
            title: "New Conversation",
            created_at: "2024-01-01T00:00:00Z",
            updated_at: "2024-01-01T00:00:00Z",
            status: "active",
            message_count: 0,
          }),
          makeConversation({
            id: "real-buddy-chat",
            kind: "chat",
            title: "Real Buddy Chat",
            created_at: "2026-01-01T00:00:00Z",
            updated_at: "2026-01-02T00:00:00Z",
            status: "active",
            message_count: 2,
          }),
        ]),
      ),
    );

    render(<BuddyRecentChats showFilters={false} compact maxItems={1} />, {
      preloadedState: CONFIG_STATE,
    });

    expect(await screen.findByText("Real Buddy Chat")).toBeInTheDocument();
    expect(screen.queryByText("New Conversation")).not.toBeInTheDocument();
  });

  it("clicking an existing chat row calls trajectory API and navigates to chat with buddy_meta", async () => {
    let trajectoryCalled = false;
    server.use(
      http.get("*/v1/buddy/conversations", () =>
        HttpResponse.json([
          makeConversation({
            id: "existing-buddy-chat",
            kind: "chat",
            title: "My Buddy Chat",
            message_count: 5,
          }),
        ]),
      ),
      http.get("*/v1/trajectories/existing-buddy-chat", () => {
        trajectoryCalled = true;
        return HttpResponse.json({
          id: "existing-buddy-chat",
          title: "My Buddy Chat",
          created_at: "2024-01-01T00:00:00Z",
          updated_at: "2024-01-02T00:00:00Z",
          model: "",
          mode: "buddy",
          tool_use: "agent",
          messages: [],
        });
      }),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    const { user } = render(<BuddyRecentChats showFilters={false} />, {
      store,
    });

    const chatRow = await screen.findByText("My Buddy Chat");
    await user.click(chatRow);

    await waitFor(() => {
      expect(trajectoryCalled).toBe(true);
    });

    const pages = store.getState().pages;
    expect(pages[pages.length - 1].name).toBe("chat");

    const rt = store.getState().chat.threads["existing-buddy-chat"];
    expect(rt?.thread.buddy_meta?.is_buddy_chat).toBe(true);
    expect(store.getState().chat.open_thread_ids).toContain(
      "existing-buddy-chat",
    );
  });

  it("clicking a workflow recent row opens the Buddy workflow trajectory", async () => {
    let trajectoryCalled = false;
    server.use(
      http.get("*/v1/buddy/conversations", () =>
        HttpResponse.json([
          makeConversation({
            id: "buddy_memory_garden",
            kind: "workflow",
            title: "Workflow Buddy Chat",
            badge: "Memory Garden",
            message_count: 4,
          }),
        ]),
      ),
      http.get("*/v1/trajectories/buddy_memory_garden", () => {
        trajectoryCalled = true;
        return HttpResponse.json({
          id: "buddy_memory_garden",
          title: "Workflow Buddy Chat",
          created_at: "2024-01-01T00:00:00Z",
          updated_at: "2024-01-02T00:00:00Z",
          model: "",
          mode: "buddy",
          tool_use: "agent",
          messages: [
            {
              role: "user",
              content: "Workflow saved message",
              message_id: "workflow-msg-1",
            },
          ],
        });
      }),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    const { user } = render(<BuddyRecentChats showFilters={false} />, {
      store,
    });

    const chatRow = await screen.findByText("Workflow Buddy Chat");
    await user.click(chatRow);

    await waitFor(() => {
      expect(trajectoryCalled).toBe(true);
    });

    const rt = store.getState().chat.threads.buddy_memory_garden;
    expect(rt?.thread.buddy_meta).toEqual({
      is_buddy_chat: true,
      buddy_chat_kind: "workflow",
      workflow_id: "buddy_memory_garden",
    });
    expect(rt?.thread.messages).toHaveLength(1);
    expect(store.getState().pages.at(-1)?.name).toBe("chat");
    expect(store.getState().chat.open_thread_ids).toContain(
      "buddy_memory_garden",
    );
  });

  it("New Chat button still creates a new conversation without trajectory fetch", async () => {
    let createConversationCalled = false;
    let trajectoryCalled = false;
    server.use(
      http.get("*/v1/buddy/conversations", () => HttpResponse.json([])),
      http.post("*/v1/buddy/conversations", () => {
        createConversationCalled = true;
        return HttpResponse.json({
          chat_id: "buddy-new-chat",
          title: "New Chat",
          created_at: "2024-01-01T00:00:00Z",
          last_message_at: null,
          message_count: 0,
        });
      }),
      http.get("*/v1/trajectories/:id", () => {
        trajectoryCalled = true;
        return HttpResponse.json({});
      }),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    const { user } = render(<BuddyRecentChats compact={false} />, { store });

    const newChatBtn = await screen.findByRole("button", { name: /new chat/i });
    await user.click(newChatBtn);

    await waitFor(() => {
      expect(createConversationCalled).toBe(true);
    });
    expect(trajectoryCalled).toBe(false);
  });
});

describe("BuddyHome_bottom_row_bounded_scroll", () => {
  it("recent chats panel sizes to its rows and paginates in the footer", async () => {
    const recentChatsCss = await readGuiSource(
      "features/Buddy/BuddyRecentChats.module.css",
    );
    const listBlock = readCssBlock(recentChatsCss, ".list");

    expect(listBlock).toContain("min-width: 0");
    expect(recentChatsCss).not.toContain("max-height: 220px");
    expect(recentChatsCss).not.toMatch(/(^|[^-])height: 100%/);
  });
});
