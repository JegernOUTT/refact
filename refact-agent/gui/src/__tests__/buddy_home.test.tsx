import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "../utils/test-utils";
import { http, HttpResponse } from "msw";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
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
  defaultBuddySettings,
  selectHomeSnoozedUntil,
  selectSeenNotificationIds,
} from "../features/Buddy/buddySlice";
import { push } from "../features/Pages/pagesSlice";
import { BuddyPulseCard } from "../features/Buddy/BuddyPulseCard";
import { BuddyOpportunityCard } from "../features/Buddy/BuddyOpportunityCard";
import { BuddyOpportunitiesFeed } from "../features/Buddy/BuddyOpportunitiesFeed";
import { BuddyWorkshop } from "../features/Buddy/BuddyWorkshop";
import { BuddyDraftPreview } from "../features/Buddy/BuddyDraftPreview";
import { BuddySettingsPanel } from "../features/Buddy/BuddySettingsPanel";
import { BuddyPanel } from "../features/Buddy/BuddyPanel";
import { BuddyDashboardScene } from "../features/Buddy/BuddyDashboardScene";
import { BuddyHome } from "../features/Buddy/BuddyHome";
import { BuddyWorld } from "../features/Buddy/BuddyWorld";
import { AutonomousChats } from "../features/Buddy/AutonomousChats";
import { BuddyRecentChats } from "../features/Buddy/BuddyRecentChats";
import { UserActivityCard } from "../features/Buddy/UserActivityCard";
import { BuddyActivityPanel } from "../features/Buddy/BuddyActivityPanel";
import { BuddyRecentErrorsPanel } from "../features/Buddy/BuddyRecentErrorsPanel";
import { BuddySpeechCloud } from "../features/Buddy/BuddySpeechCloud";
import { bubblePositionForSceneX } from "../features/Buddy/buddyWorldUtils";
import { buildBuddyWorldState } from "../features/Buddy/buddyWorldModel";
import {
  buildBuddySceneSpeech,
  formatBuddyRuntimeEventText,
  isBuddySpeechExpired,
  pickBuddySceneSpeechCandidate,
  type BuddySceneSpeech,
} from "../features/Buddy/buddySceneSpeech";
import { formatFailureLabel } from "../features/Buddy/buddyUtils";
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
  BuddyConversationEntry,
  BuddyActivityEntry,
  BuddySpeechItem,
} from "../features/Buddy/types";
import type React from "react";

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

function readCssMediaBlock(source: string, query: string): string {
  const marker = `@media ${query}`;
  const start = source.indexOf(marker);
  if (start === -1) {
    throw new Error(`Missing CSS media block for ${query}`);
  }
  const open = source.indexOf("{", start);
  let depth = 0;
  for (let i = open; i < source.length; i += 1) {
    const char = source[i];
    if (char === "{") depth += 1;
    if (char === "}") depth -= 1;
    if (depth === 0) return source.slice(open + 1, i);
  }
  throw new Error(`Unclosed CSS media block for ${query}`);
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

describe("AutonomousChats_groups_by_workflow_id", () => {
  it("groups conversations by workflow id", () => {
    render(
      <AutonomousChats
        conversations={[
          makeConversation({ id: "refact-a", title: "Refact A" }),
          makeConversation({ id: "refact-b", title: "Refact B" }),
          makeConversation({
            id: "buddy-a",
            title: "Buddy A",
            badge: "buddy_memory_garden",
          }),
        ]}
      />,
      { preloadedState: CONFIG_STATE },
    );

    expect(screen.getByText("refact self critic")).toBeInTheDocument();
    expect(screen.getByText("buddy memory garden")).toBeInTheDocument();
    expect(screen.getByText("Refact A")).toBeInTheDocument();
    expect(screen.getByText("Refact B")).toBeInTheDocument();
    expect(screen.getByText("Buddy A")).toBeInTheDocument();
  });

  it("renders invalid timestamps safely", () => {
    render(
      <AutonomousChats
        conversations={[
          makeConversation({
            id: "invalid-time-chat",
            title: "Invalid Time Chat",
            updated_at: "not-a-date",
          }),
        ]}
      />,
      { preloadedState: CONFIG_STATE },
    );

    expect(screen.getByText("unknown")).toBeInTheDocument();
    expect(screen.queryByText(/NaNd ago/)).not.toBeInTheDocument();
  });
});

describe("UserActivityCard_renders_24h_heatmap_cells", () => {
  it("renders one heatmap cell for each hour", () => {
    render(
      <UserActivityCard
        activity={{
          actions: [
            { type: "file_opened", ts: "2024-01-01T01:00:00Z" },
            { type: "file_opened", ts: "2024-01-01T01:20:00Z" },
            { type: "tool_approved", ts: "2024-01-01T05:00:00Z" },
          ],
          time_of_day_pattern: "morning focus",
        }}
      />,
      { preloadedState: CONFIG_STATE },
    );

    expect(screen.getAllByTestId("user-activity-hour-cell")).toHaveLength(24);
    expect(screen.getByText("file opened · 2")).toBeInTheDocument();
    expect(screen.getByText("morning focus")).toBeInTheDocument();
  });
});

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

    await user.click(screen.getByRole("radio", { name: "refact_*" }));

    expect(screen.getByText("Compile sniffer")).toBeInTheDocument();
    expect(screen.queryByText("Memory garden")).not.toBeInTheDocument();

    await user.click(screen.getByRole("radio", { name: "buddy_*" }));

    expect(screen.queryByText("Compile sniffer")).not.toBeInTheDocument();
    expect(screen.getByText("Memory garden")).toBeInTheDocument();
  });
});

describe("BuddyWorkflowFailureSummaries", () => {
  it("formats failure category labels with the shared Buddy utility", () => {
    expect(formatFailureLabel("model_unavailable")).toBe("Model Unavailable");
    expect(formatFailureLabel("MODEL_UNAVAILABLE")).toBe("Model Unavailable");
    expect(formatFailureLabel("context_TOO-large")).toBe("Context Too Large");
    expect(formatFailureLabel(" context-too-large ")).toBe("Context Too Large");
    expect(formatFailureLabel(" ")).toBeNull();
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

    expect(screen.getAllByText("Model Unavailable")).toHaveLength(2);
    expect(
      screen.getAllByText(
        "Model unavailable — check Buddy/default model settings.",
      ),
    ).toHaveLength(2);
  });
});

describe("Speech_shows_intent_badge_when_field_present", () => {
  it("shows the speech intent badge", () => {
    render(
      <BuddySpeechCloud
        speech={makeSpeech({ speech_intent: "Humor" })}
        onControl={vi.fn()}
      />,
      { preloadedState: CONFIG_STATE },
    );

    expect(screen.getByText("Humor")).toBeInTheDocument();
  });
});

describe("BuddyPulseCard_renders_pulse", () => {
  it("shows pulse counts from store", async () => {
    const pulse = makePulse();
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setPulse(pulse));

    server.use(
      http.get("*/v1/buddy/opportunities", () =>
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

describe("BuddyOpportunityCard_dismiss_calls_mutation", () => {
  it("clicking Dismiss calls dismiss mutation", async () => {
    let dismissed = false;
    server.use(
      http.get("*/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities: [] }),
      ),
      http.post("*/v1/buddy/opportunities/:id/dismiss", () => {
        dismissed = true;
        return HttpResponse.json({ snapshot: makeSnapshot() });
      }),
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
      http.get("*/v1/buddy/opportunities", () =>
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
    const errorsPanel = await screen.findByTestId("buddy-recent-errors-panel");
    expect(within(errorsPanel).getByText("×3")).toBeInTheDocument();
    expect(dismissedIds).toHaveLength(0);

    await user.click(
      within(errorsPanel).getByRole("button", { name: "Sniff logs" }),
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

    render(<BuddyHome />, { store });
    const errorsPanel = await screen.findByTestId("buddy-recent-errors-panel");
    expect(
      within(errorsPanel).getByText("Model Unavailable"),
    ).toBeInTheDocument();
    expect(
      within(errorsPanel).getByText("Context Too Large"),
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
      const errorsPanel = await screen.findByTestId(
        "buddy-recent-errors-panel",
      );

      expect(
        within(errorsPanel).queryByText("Old dismissed provider failure"),
      ).not.toBeInTheDocument();
      expect(
        within(errorsPanel).getByText(/No errors recorded/),
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
    const panel = await readGuiSource("features/Buddy/BuddyPanel.tsx");
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
    expect(panel).toContain('ctrl.action === "dismiss_runtime_event"');
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

  it("settings section uses a CSS class wrapper when settings toggled on", async () => {
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

    const { user } = render(<BuddyHome />, { store });

    await screen.findByTestId("buddy-home-content");
    await user.click(screen.getByRole("button", { name: "Settings" }));

    const settingsSection = await screen.findByTestId(
      "buddy-home-settings-section",
    );
    expect(settingsSection).toBeInTheDocument();
    expect(settingsSection).not.toHaveAttribute("style");
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
    vi.setSystemTime(new Date("2024-01-01T14:00:00Z"));
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
      vi.setSystemTime(new Date("2024-01-01T14:00:07.101Z"));
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
          now={new Date("2024-01-01T14:00:07.101Z")}
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

      vi.setSystemTime(new Date("2024-01-01T14:00:12.101Z"));
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
          now={new Date("2024-01-01T14:00:12.101Z")}
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

describe("BuddySettingsPanel_autosave", () => {
  it("toggling a switch immediately sends partial patch to server", async () => {
    let capturedBody: unknown;
    server.use(
      http.post("*/v1/buddy/settings", async ({ request }) => {
        capturedBody = await request.json();
        return HttpResponse.json({
          ...makeSnapshot().settings,
          quiet_mode: true,
        });
      }),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    const { user } = render(<BuddySettingsPanel />, { store });

    const quietSwitch = await screen.findByRole("switch", {
      name: /quiet mode/i,
    });
    await user.click(quietSwitch);

    expect(quietSwitch).toBeChecked();
    await waitFor(() => {
      expect(capturedBody).toEqual({ quiet_mode: true });
    });
  });

  it("rapid switch toggles keep the last value after out-of-order responses", async () => {
    const capturedBodies: unknown[] = [];
    const responseResolvers: ((settings: BuddySnapshot["settings"]) => void)[] =
      [];
    server.use(
      http.post("*/v1/buddy/settings", async ({ request }) => {
        capturedBodies.push(await request.json());
        const settings = await new Promise<BuddySnapshot["settings"]>(
          (resolve) => responseResolvers.push(resolve),
        );
        return HttpResponse.json(settings);
      }),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    const { user } = render(<BuddySettingsPanel />, { store });

    const quietSwitch = await screen.findByRole("switch", {
      name: /quiet mode/i,
    });
    await user.click(quietSwitch);
    await waitFor(() => {
      expect(capturedBodies).toHaveLength(1);
    });
    expect(quietSwitch).toBeChecked();

    await user.click(quietSwitch);
    await waitFor(() => {
      expect(capturedBodies).toHaveLength(2);
    });
    expect(quietSwitch).not.toBeChecked();
    expect(capturedBodies).toEqual([
      { quiet_mode: true },
      { quiet_mode: false },
    ]);

    responseResolvers[1]?.({ ...makeSnapshot().settings, quiet_mode: false });
    await waitFor(() => {
      expect(store.getState().buddy.snapshot?.settings.quiet_mode).toBe(false);
    });

    responseResolvers[0]?.({ ...makeSnapshot().settings, quiet_mode: true });
    await new Promise((resolve) => window.setTimeout(resolve, 0));

    expect(store.getState().buddy.snapshot?.settings.quiet_mode).toBe(false);
    expect(quietSwitch).not.toBeChecked();
  });

  it("clicking a segmented enum button immediately sends partial patch", async () => {
    let capturedBody: unknown;
    server.use(
      http.post("*/v1/buddy/settings", async ({ request }) => {
        capturedBody = await request.json();
        return HttpResponse.json({
          ...makeSnapshot().settings,
          humor_level: "normal",
        });
      }),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    const { user } = render(<BuddySettingsPanel />, { store });

    const normalButton = screen.getByRole("radio", { name: /normal/i });
    await user.click(normalButton);

    expect(normalButton).toBeChecked();
    await waitFor(() => {
      expect(capturedBody).toEqual({ humor_level: "normal" });
    });
  });

  it("daily digest saves valid and empty partial patches", async () => {
    const capturedBodies: unknown[] = [];
    server.use(
      http.post("*/v1/buddy/settings", async ({ request }) => {
        const body = (await request.json()) as {
          daily_digest_hour?: number | null;
        };
        capturedBodies.push(body);
        return HttpResponse.json({
          ...makeSnapshot().settings,
          daily_digest_hour:
            "daily_digest_hour" in body
              ? body.daily_digest_hour
              : makeSnapshot().settings.daily_digest_hour,
        });
      }),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    render(<BuddySettingsPanel />, { store });

    const digestInput = screen.getByRole("spinbutton", {
      name: /daily digest hour/i,
    });
    fireEvent.change(digestInput, { target: { value: "7" } });

    expect(digestInput).toHaveValue(7);
    await waitFor(() => {
      expect(capturedBodies).toEqual([{ daily_digest_hour: 7 }]);
    });

    fireEvent.change(digestInput, { target: { value: "" } });

    expect(digestInput).toHaveValue(null);
    await waitFor(() => {
      expect(capturedBodies).toEqual([
        { daily_digest_hour: 7 },
        { daily_digest_hour: null },
      ]);
    });
  });

  it.each(["1e2", "7.5", "24", "-1"])(
    "daily digest invalid value %s does not save or clobber usable state",
    (invalidValue) => {
      const capturedBodies: unknown[] = [];
      server.use(
        http.post("*/v1/buddy/settings", async ({ request }) => {
          capturedBodies.push(await request.json());
          return HttpResponse.json(makeSnapshot().settings);
        }),
      );

      const store = setUpStore({ ...CONFIG_STATE });
      store.dispatch(setBuddySnapshot(makeSnapshot()));

      render(<BuddySettingsPanel />, { store });

      const digestInput = screen.getByRole("spinbutton", {
        name: /daily digest hour/i,
      });
      fireEvent.change(digestInput, { target: { value: invalidValue } });

      expect(capturedBodies).toEqual([]);
      expect(store.getState().buddy.snapshot?.settings.daily_digest_hour).toBe(
        18,
      );
      expect(digestInput).toHaveValue(18);
    },
  );

  it("mutation success updates Redux settings via onQueryStarted", async () => {
    const updatedSettings = {
      ...makeSnapshot().settings,
      humor_level: "normal" as const,
    };
    server.use(
      http.post("*/v1/buddy/settings", () =>
        HttpResponse.json(updatedSettings),
      ),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    const { user } = render(<BuddySettingsPanel />, { store });

    await user.click(screen.getByRole("radio", { name: /normal/i }));

    await waitFor(() => {
      expect(store.getState().buddy.snapshot?.settings.humor_level).toBe(
        "normal",
      );
    });
  });

  it("prompt change after debounce period sends personality_prompt patch", async () => {
    vi.useFakeTimers();
    let capturedBody: unknown;
    server.use(
      http.post("*/v1/buddy/settings", async ({ request }) => {
        capturedBody = await request.json();
        return HttpResponse.json(makeSnapshot().settings);
      }),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    try {
      render(<BuddySettingsPanel />, { store });

      const textarea = screen.getByRole("textbox", {
        name: /personality prompt/i,
      });
      fireEvent.change(textarea, { target: { value: "Be more chaotic" } });

      expect(capturedBody).toBeUndefined();

      await vi.advanceTimersByTimeAsync(750);

      await waitFor(() => {
        expect(capturedBody).toEqual({
          personality_prompt: "Be more chaotic",
        });
      });
    } finally {
      vi.useRealTimers();
    }
  });

  it("prompt blur saves immediately without waiting for debounce", async () => {
    let capturedBody: unknown;
    server.use(
      http.post("*/v1/buddy/settings", async ({ request }) => {
        capturedBody = await request.json();
        return HttpResponse.json(makeSnapshot().settings);
      }),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    render(<BuddySettingsPanel />, { store });

    const textarea = screen.getByRole("textbox", {
      name: /personality prompt/i,
    });
    fireEvent.change(textarea, { target: { value: "Be calm" } });
    fireEvent.blur(textarea);

    await waitFor(() => {
      expect(capturedBody).toEqual({ personality_prompt: "Be calm" });
    });
  });

  it("delete all prompt text and debounce sends clear_personality_prompt flag", async () => {
    vi.useFakeTimers();
    const capturedBodies: unknown[] = [];
    server.use(
      http.post("*/v1/buddy/settings", async ({ request }) => {
        capturedBodies.push(await request.json());
        return HttpResponse.json({
          ...makeSnapshot().settings,
          personality_prompt: null,
        });
      }),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot(undefined, {
          settings: {
            ...makeSnapshot().settings,
            personality_prompt: "Custom personality text",
          },
        }),
      ),
    );

    try {
      render(<BuddySettingsPanel />, { store });

      const textarea = screen.getByRole("textbox", {
        name: /personality prompt/i,
      });
      fireEvent.change(textarea, { target: { value: "" } });

      expect(capturedBodies).toHaveLength(0);

      await vi.advanceTimersByTimeAsync(750);

      await waitFor(() => {
        expect(capturedBodies).toEqual([{ clear_personality_prompt: true }]);
      });
    } finally {
      vi.useRealTimers();
    }
  });

  it("delete all prompt text and blur sends clear_personality_prompt flag", async () => {
    const capturedBodies: unknown[] = [];
    server.use(
      http.post("*/v1/buddy/settings", async ({ request }) => {
        capturedBodies.push(await request.json());
        return HttpResponse.json({
          ...makeSnapshot().settings,
          personality_prompt: null,
        });
      }),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot(undefined, {
          settings: {
            ...makeSnapshot().settings,
            personality_prompt: "Custom personality text",
          },
        }),
      ),
    );

    render(<BuddySettingsPanel />, { store });

    const textarea = screen.getByRole("textbox", {
      name: /personality prompt/i,
    });
    fireEvent.change(textarea, { target: { value: "" } });
    fireEvent.blur(textarea);

    await waitFor(() => {
      expect(capturedBodies).toEqual([{ clear_personality_prompt: true }]);
    });
  });

  it("prompt draft survives unrelated live settings update before debounce fires", async () => {
    vi.useFakeTimers();
    const capturedBodies: unknown[] = [];
    server.use(
      http.post("*/v1/buddy/settings", async ({ request }) => {
        capturedBodies.push(await request.json());
        return HttpResponse.json({
          ...makeSnapshot().settings,
          quiet_mode: true,
          personality_prompt: "Server prompt",
        });
      }),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot(undefined, {
          settings: {
            ...makeSnapshot().settings,
            personality_prompt: "Server prompt",
          },
        }),
      ),
    );

    try {
      render(<BuddySettingsPanel />, { store });

      const textarea = screen.getByRole("textbox", {
        name: /personality prompt/i,
      });
      fireEvent.focus(textarea);
      fireEvent.change(textarea, { target: { value: "Unsaved local prompt" } });
      store.dispatch(
        setBuddySnapshot(
          makeSnapshot(undefined, {
            settings: {
              ...makeSnapshot().settings,
              quiet_mode: true,
              personality_prompt: "Server prompt",
            },
          }),
        ),
      );

      expect(textarea).toHaveValue("Unsaved local prompt");

      await vi.advanceTimersByTimeAsync(750);

      await waitFor(() => {
        expect(capturedBodies).toEqual([
          { personality_prompt: "Unsaved local prompt" },
        ]);
      });
      expect(textarea).toHaveValue("Unsaved local prompt");
    } finally {
      vi.useRealTimers();
    }
  });

  it("prompt clear button sends clear_personality_prompt flag", async () => {
    let capturedBody: unknown;
    server.use(
      http.post("*/v1/buddy/settings", async ({ request }) => {
        capturedBody = await request.json();
        return HttpResponse.json({
          ...makeSnapshot().settings,
          personality_prompt: null,
        });
      }),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot(undefined, {
          settings: {
            ...makeSnapshot().settings,
            personality_prompt: "Custom personality text",
          },
        }),
      ),
    );

    const { user } = render(<BuddySettingsPanel />, { store });

    const clearBtn = await screen.findByTestId("buddy-clear-prompt");
    await user.click(clearBtn);

    await waitFor(() => {
      expect(capturedBody).toEqual({ clear_personality_prompt: true });
    });
  });

  it("shows Saved status after successful mutation", async () => {
    server.use(
      http.post("*/v1/buddy/settings", () =>
        HttpResponse.json(makeSnapshot().settings),
      ),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    const { user } = render(<BuddySettingsPanel />, { store });

    await user.click(
      screen.getByRole("switch", { name: /housekeeping enabled/i }),
    );

    await waitFor(() => {
      expect(
        screen.getByText("Saved to active Buddy settings"),
      ).toBeInTheDocument();
    });
  });

  it("renders active Buddy storage diagnostics when present", () => {
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot(undefined, {
          storage: {
            project_root: "/repo/root",
            buddy_dir: "/repo/root/.refact/buddy",
            settings_path: "/repo/root/.refact/buddy/settings.json",
          },
        }),
      ),
    );

    render(<BuddySettingsPanel />, { store });

    expect(screen.getByText("ADVANCED / DIAGNOSTICS")).toBeInTheDocument();
    expect(screen.getByText("Active Buddy folder")).toBeInTheDocument();
    expect(screen.getByText("/repo/root/.refact/buddy")).toBeInTheDocument();
    expect(
      screen.getByText("/repo/root/.refact/buddy/settings.json"),
    ).toBeInTheDocument();
  });

  it("tolerates missing Buddy storage diagnostics", () => {
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    render(<BuddySettingsPanel />, { store });

    expect(screen.getByTestId("buddy-storage-diagnostics")).toHaveTextContent(
      "Storage metadata is unavailable from this engine response.",
    );
  });

  it("shows Failed status on save error without unhandled rejections", async () => {
    server.use(
      http.post("*/v1/buddy/settings", () =>
        HttpResponse.json({ error: "nope" }, { status: 500 }),
      ),
    );

    const unhandled = vi.fn();
    window.addEventListener("unhandledrejection", unhandled);

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    try {
      const { user } = render(<BuddySettingsPanel />, { store });

      await user.click(
        screen.getByRole("switch", { name: /housekeeping enabled/i }),
      );

      expect(await screen.findByRole("alert")).toHaveTextContent("Save failed");

      await new Promise((resolve) => window.setTimeout(resolve, 0));
      expect(unhandled).not.toHaveBeenCalled();
    } finally {
      window.removeEventListener("unhandledrejection", unhandled);
    }
  });

  it("failed switch autosave rolls back the optimistic Redux and control state", async () => {
    const responseResolvers: ((response: Response) => void)[] = [];
    server.use(
      http.post(
        "*/v1/buddy/settings",
        () =>
          new Promise<Response>((resolve) => {
            responseResolvers.push(resolve);
          }),
      ),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    const { user } = render(<BuddySettingsPanel />, { store });

    const housekeepingSwitch = screen.getByRole("switch", {
      name: /housekeeping enabled/i,
    });
    await user.click(housekeepingSwitch);

    await waitFor(() => {
      expect(
        store.getState().buddy.snapshot?.settings.housekeeping_enabled,
      ).toBe(false);
    });
    expect(housekeepingSwitch).not.toBeChecked();

    responseResolvers[0]?.(
      HttpResponse.json({ error: "nope" }, { status: 500 }),
    );

    await waitFor(() => {
      expect(
        store.getState().buddy.snapshot?.settings.housekeeping_enabled,
      ).toBe(true);
    });
    expect(housekeepingSwitch).toBeChecked();
    expect(await screen.findByRole("alert")).toHaveTextContent("Save failed");
  });

  it("failed enabled autosave restores Buddy Home instead of trapping disabled UI", async () => {
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
      http.post("*/v1/buddy/settings", () =>
        HttpResponse.json({ error: "nope" }, { status: 500 }),
      ),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));

    const { user } = render(<BuddyHome />, { store });

    await screen.findByTestId("buddy-home-content");
    await user.click(screen.getByRole("button", { name: "Settings" }));
    await user.click(screen.getByRole("switch", { name: /buddy enabled/i }));

    await waitFor(() => {
      expect(store.getState().buddy.snapshot?.settings.enabled).toBe(true);
    });
    expect(store.getState().buddy.snapshot?.enabled).toBe(true);
    expect(screen.getByTestId("buddy-home-content")).toBeInTheDocument();
    expect(screen.queryByTestId("buddy-home-disabled")).not.toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /buddy enabled/i }),
    ).toBeChecked();
  });

  it("stale snapshot while a toggle is pending keeps the optimistic value visible", async () => {
    const responseResolvers: ((settings: BuddySnapshot["settings"]) => void)[] =
      [];
    server.use(
      http.post(
        "*/v1/buddy/settings",
        () =>
          new Promise<Response>((resolve) => {
            responseResolvers.push((settings) =>
              resolve(HttpResponse.json(settings)),
            );
          }),
      ),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    const { user } = render(<BuddySettingsPanel />, { store });

    const quietSwitch = await screen.findByRole("switch", {
      name: /quiet mode/i,
    });
    await user.click(quietSwitch);
    expect(quietSwitch).toBeChecked();

    store.dispatch(
      setBuddySnapshot(
        makeSnapshot(undefined, {
          settings: { ...makeSnapshot().settings, quiet_mode: false },
        }),
      ),
    );

    expect(store.getState().buddy.snapshot?.settings.quiet_mode).toBe(true);
    expect(quietSwitch).toBeChecked();

    responseResolvers[0]?.({ ...makeSnapshot().settings, quiet_mode: true });
    await waitFor(() => {
      expect(
        screen.getByText("Saved to active Buddy settings"),
      ).toBeInTheDocument();
    });
  });

  it("out-of-order different setting requests keep pending settings visible", async () => {
    const capturedBodies: unknown[] = [];
    const responseResolvers: ((settings: BuddySnapshot["settings"]) => void)[] =
      [];
    server.use(
      http.post("*/v1/buddy/settings", async ({ request }) => {
        capturedBodies.push(await request.json());
        const settings = await new Promise<BuddySnapshot["settings"]>(
          (resolve) => responseResolvers.push(resolve),
        );
        return HttpResponse.json(settings);
      }),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    const { user } = render(<BuddySettingsPanel />, { store });

    const quietSwitch = await screen.findByRole("switch", {
      name: /quiet mode/i,
    });
    const housekeepingSwitch = screen.getByRole("switch", {
      name: /housekeeping enabled/i,
    });
    await user.click(quietSwitch);
    await user.click(housekeepingSwitch);

    await waitFor(() => {
      expect(capturedBodies).toEqual([
        { quiet_mode: true },
        { housekeeping_enabled: false },
      ]);
    });
    expect(quietSwitch).toBeChecked();
    expect(housekeepingSwitch).not.toBeChecked();

    responseResolvers[0]?.({
      ...makeSnapshot().settings,
      quiet_mode: true,
      housekeeping_enabled: true,
    });
    await new Promise((resolve) => window.setTimeout(resolve, 0));

    expect(store.getState().buddy.snapshot?.settings.quiet_mode).toBe(true);
    expect(store.getState().buddy.snapshot?.settings.housekeeping_enabled).toBe(
      false,
    );
    expect(quietSwitch).toBeChecked();
    expect(housekeepingSwitch).not.toBeChecked();

    responseResolvers[1]?.({
      ...makeSnapshot().settings,
      quiet_mode: false,
      housekeeping_enabled: false,
    });
    await waitFor(() => {
      expect(
        screen.getByText("Saved to active Buddy settings"),
      ).toBeInTheDocument();
    });
    expect(store.getState().buddy.snapshot?.settings.quiet_mode).toBe(true);
    expect(store.getState().buddy.snapshot?.settings.housekeeping_enabled).toBe(
      false,
    );
  });

  it("slow failed request followed by fast success leaves status saved", async () => {
    const responseResolvers: ((response: Response) => void)[] = [];
    server.use(
      http.post(
        "*/v1/buddy/settings",
        () =>
          new Promise<Response>((resolve) => responseResolvers.push(resolve)),
      ),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    const { user } = render(<BuddySettingsPanel />, { store });

    await user.click(screen.getByRole("switch", { name: /quiet mode/i }));
    await user.click(
      screen.getByRole("switch", { name: /housekeeping enabled/i }),
    );
    await waitFor(() => {
      expect(responseResolvers).toHaveLength(2);
    });

    responseResolvers[1]?.(
      HttpResponse.json({
        ...makeSnapshot().settings,
        quiet_mode: false,
        housekeeping_enabled: false,
      }),
    );
    await waitFor(() => {
      expect(
        screen.getByText("Saved to active Buddy settings"),
      ).toBeInTheDocument();
    });

    responseResolvers[0]?.(
      HttpResponse.json({ error: "nope" }, { status: 500 }),
    );
    await new Promise((resolve) => window.setTimeout(resolve, 0));

    expect(
      screen.getByText("Saved to active Buddy settings"),
    ).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("prompt focus and blur without edits does not save", async () => {
    const capturedBodies: unknown[] = [];
    server.use(
      http.post("*/v1/buddy/settings", async ({ request }) => {
        capturedBodies.push(await request.json());
        return HttpResponse.json(makeSnapshot().settings);
      }),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot(undefined, {
          settings: {
            ...makeSnapshot().settings,
            personality_prompt: "Already saved prompt",
          },
        }),
      ),
    );

    render(<BuddySettingsPanel />, { store });

    const textarea = screen.getByRole("textbox", {
      name: /personality prompt/i,
    });
    fireEvent.focus(textarea);
    fireEvent.blur(textarea);

    await new Promise((resolve) => window.setTimeout(resolve, 0));
    expect(capturedBodies).toEqual([]);
    expect(screen.queryByText("Saving…")).not.toBeInTheDocument();
  });
});

describe("BuddySettingsPanel_renders_all_settings", () => {
  it("renders all required setting labels and controls", () => {
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    render(<BuddySettingsPanel />, { store });

    expect(
      screen.getByRole("switch", { name: /buddy enabled/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /quiet mode/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /auto diagnostics/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /auto issue creation/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /proactive suggestions/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /chat pattern observation/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /live chat reactions/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /autonomous buddy chats/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /housekeeping/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /humor enabled/i }),
    ).toBeInTheDocument();

    expect(screen.getByRole("radio", { name: /off/i })).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: /light/i })).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: /normal/i })).toBeInTheDocument();

    expect(
      screen.getByRole("radio", { name: /read only/i }),
    ).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: /suggest/i })).toBeInTheDocument();
    expect(
      screen.getByRole("radio", { name: /safe auto/i }),
    ).toBeInTheDocument();

    expect(
      screen.getByRole("textbox", { name: /personality prompt/i }),
    ).toBeInTheDocument();

    expect(
      screen.getByRole("spinbutton", { name: /daily digest hour/i }),
    ).toBeInTheDocument();
  });

  it("renders all observer toggle labels", () => {
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    render(<BuddySettingsPanel />, { store });

    expect(
      screen.getByRole("switch", { name: /task health/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /trajectory clutter/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: "Chat Pattern" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /customization drift/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /memory garden/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /mcp auth/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /git pressure/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: "Diagnostics" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /provider health/i }),
    ).toBeInTheDocument();
  });
});

describe("BuddySettingsPanel_segmented_controls_accessible_state", () => {
  it("active humor_level radio is checked and others are unchecked", () => {
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    render(<BuddySettingsPanel />, { store });

    const lightBtn = screen.getByRole("radio", { name: /light/i });
    const offBtn = screen.getByRole("radio", { name: /off/i });
    const normalBtn = screen.getByRole("radio", { name: /normal/i });

    expect(lightBtn).toBeChecked();
    expect(offBtn).not.toBeChecked();
    expect(normalBtn).not.toBeChecked();
  });

  it("active autonomy_level radio is checked and others are unchecked", () => {
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    render(<BuddySettingsPanel />, { store });

    const suggestBtn = screen.getByRole("radio", { name: /suggest/i });
    const readOnlyBtn = screen.getByRole("radio", { name: /read only/i });
    const safeAutoBtn = screen.getByRole("radio", { name: /safe auto/i });

    expect(suggestBtn).toBeChecked();
    expect(readOnlyBtn).not.toBeChecked();
    expect(safeAutoBtn).not.toBeChecked();
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

describe("BuddyPanel_opportunity_notifications", () => {
  it("renders unread opportunities as Buddy speech controls without a badge", async () => {
    server.use(
      http.get("*/v1/buddy/opportunities", () =>
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
      http.get("*/v1/buddy/opportunities", () =>
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

  it("renders disabled state instead of disappearing and sends enabled patch", async () => {
    let capturedBody: unknown;
    server.use(
      http.get("*/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities: [] }),
      ),
      http.post("*/v1/buddy/settings", async ({ request }) => {
        capturedBody = await request.json();
        return HttpResponse.json(defaultBuddySettings());
      }),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot(undefined, {
          enabled: false,
          settings: { ...makeSnapshot().settings, enabled: false },
        }),
      ),
    );

    const { user } = render(<BuddyPanel />, { store });

    expect(
      await screen.findByTestId("buddy-panel-disabled"),
    ).toBeInTheDocument();
    expect(screen.getByText("Pixel is disabled")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Enable" }));

    await waitFor(() => {
      expect(capturedBody).toEqual({ enabled: true });
    });
  });
});

describe("BuddyOpportunitiesFeed_suggestions", () => {
  it("shows active Buddy suggestions when detector opportunities are empty", async () => {
    server.use(
      http.get("*/v1/buddy/opportunities", () =>
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

  it("activity panel uses a standalone glass surface with an internal scroll list", async () => {
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));
    server.use(...commonHandlers);

    render(<BuddyHome />, { store });
    const panel = await screen.findByTestId("buddy-activity-panel");
    expect(panel.className).not.toContain("panelScroll");
    expect(panel.className).toContain("glass");
    expect(panel.querySelector('[class*="scrollList"]')).not.toBeNull();
  });

  it("recent errors panel uses a standalone glass surface with an internal scroll list", async () => {
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));
    server.use(...commonHandlers);

    render(<BuddyHome />, { store });
    const panel = await screen.findByTestId("buddy-recent-errors-panel");
    expect(panel.className).not.toContain("panelScroll");
    expect(panel.className).toContain("glass");
    expect(panel.querySelector('[class*="scrollList"]')).not.toBeNull();
  });

  it("recent chats panel uses a standalone glass surface with an internal scroll list", async () => {
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));
    server.use(...commonHandlers);

    render(<BuddyHome />, { store });
    const panel = await screen.findByTestId("buddy-recent-chats");
    expect(panel.className).not.toContain("panelScroll");
    expect(panel.className).toContain("glass");
  });

  it("main grid bounds redesigned panel rows", async () => {
    const css = await readGuiSource("features/Buddy/BuddyHome.module.css");
    const mainGridBlock = readCssBlock(css, ".mainGrid");
    const panelSlotBlock = readCssBlock(css, ".panelSlot");

    expect(mainGridBlock).toContain("repeat(3, minmax(0, 1fr))");
    expect(mainGridBlock).toContain("align-items: start");
    expect(panelSlotBlock).toContain("overflow: hidden");
    expect(panelSlotBlock).toContain("min-height: 0");
    expect(css).toContain("height: clamp(300px, 34vh, 360px)");
    expect(css).toContain("height: clamp(230px, 27vh, 290px)");
    expect(css).not.toContain("max-height: 860px");
  });

  it("recent chats panel clips overflow while entries scroll internally", async () => {
    const recentChatsCss = await readGuiSource(
      "features/Buddy/BuddyRecentChats.module.css",
    );
    const panelBlock = readCssBlock(recentChatsCss, ".panel");
    const entriesBlock = readCssBlock(recentChatsCss, ".scrollList");

    expect(panelBlock).toContain("height: 100%");
    expect(panelBlock).toContain("overflow: hidden");
    expect(entriesBlock).toContain("flex: 1");
    expect(entriesBlock).toContain("min-height: 0");
    expect(entriesBlock).toContain("overflow-y: auto");
    expect(entriesBlock).toContain("overflow-x: hidden");
  });

  it("activity panel CSS bounds the surface and keeps the list as the scroller", async () => {
    const css = await readGuiSource(
      "features/Buddy/BuddyActivityPanel.module.css",
    );
    const panelBlock = readCssBlock(css, ".panel");
    const scrollBlock = readCssBlock(css, ".scrollList");

    expect(panelBlock).toContain("height: 100%");
    expect(panelBlock).toContain("overflow: hidden");
    expect(scrollBlock).toContain("flex: 1 1 auto");
    expect(scrollBlock).toContain("min-height: 0");
    expect(scrollBlock).toContain("overflow-y: auto");
    expect(scrollBlock).toContain("overflow-x: hidden");
  });

  it("recent errors panel CSS bounds the surface and keeps the list as the scroller", async () => {
    const css = await readGuiSource(
      "features/Buddy/BuddyRecentErrorsPanel.module.css",
    );
    const panelBlock = readCssBlock(css, ".panel");
    const scrollBlock = readCssBlock(css, ".scrollList");

    expect(panelBlock).toContain("height: 100%");
    expect(panelBlock).toContain("overflow: hidden");
    expect(scrollBlock).toContain("flex: 1 1 auto");
    expect(scrollBlock).toContain("min-height: 0");
    expect(scrollBlock).toContain("overflow-y: auto");
    expect(scrollBlock).toContain("overflow-x: hidden");
  });

  it("stacked media keeps redesigned panels bounded", async () => {
    const homeCss = await readGuiSource("features/Buddy/BuddyHome.module.css");
    const homeWideStackBlock = readCssMediaBlock(
      homeCss,
      "(max-width: 1100px)",
    );
    const homeIdeStackBlock = readCssMediaBlock(homeCss, "(max-width: 720px)");
    const activityCss = await readGuiSource(
      "features/Buddy/BuddyActivityPanel.module.css",
    );
    const recentErrorsCss = await readGuiSource(
      "features/Buddy/BuddyRecentErrorsPanel.module.css",
    );

    expect(homeWideStackBlock).toContain("repeat(2, minmax(0, 1fr))");
    expect(homeIdeStackBlock).toContain(
      "grid-template-columns: minmax(0, 1fr)",
    );
    expect(homeIdeStackBlock).toContain("height: min(420px, 70vh)");
    expect(homeIdeStackBlock).not.toContain("overflow-y: visible");
    expect(activityCss).not.toContain("overflow-y: visible");
    expect(recentErrorsCss).not.toContain("overflow-y: visible");
  });
});
