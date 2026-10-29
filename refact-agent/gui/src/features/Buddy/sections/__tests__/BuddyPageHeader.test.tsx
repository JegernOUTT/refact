import { render, screen, waitFor, within } from "../../../../utils/test-utils";
import { http, HttpResponse } from "msw";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { server } from "../../../../utils/mockServer";
import { setUpStore } from "../../../../app/store";
import { setBuddySnapshot } from "../../buddySlice";
import { BuddyHome } from "../../BuddyHome";
import type { BuddyPulse, BuddySnapshot } from "../../types";

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
  it.each([
    ["enabled", "Enabled"],
    ["disabled", "Local only"],
    ["unavailable", "Not responding"],
  ] as const)(
    "renders the %s work authority branch with its own status pill",
    async (mode, label) => {
      server.use(
        ...layoutHandlers,
        http.get("*/v1/buddy/operator/status", () =>
          HttpResponse.json(operatorStatus(mode)),
        ),
        http.get("*/v1/buddy/opportunities", () =>
          HttpResponse.json({ opportunities: [] }),
        ),
        http.get("*/v1/buddy/work", () =>
          HttpResponse.json({
            schema_version: 1,
            project: operatorStatus("enabled").project,
            data: [],
            next_cursor: null,
          }),
        ),
      );
      const store = setUpStore({ ...CONFIG_STATE });
      store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));

      const { user } = render(<BuddyHome />, { store });

      const pill = await screen.findByTestId("buddy-operator-status");
      await waitFor(() => expect(pill).toHaveAttribute("data-state", mode));
      expect(pill).toHaveTextContent(label);

      if (mode === "enabled") {
        await user.click(screen.getByRole("tab", { name: "Reports" }));
        expect(
          await screen.findByText("Nothing needs you right now."),
        ).toBeInTheDocument();
        const desk = screen.getByTestId("buddy-desk");
        expect(
          within(desk).getByRole("heading", { name: "Reports" }),
        ).toBeInTheDocument();
        expect(within(desk).queryByRole("searchbox")).toBeNull();
      }
      const retry = screen.queryByRole("button", {
        name: "Retry connection",
      });
      const check = screen.queryByRole("button", {
        name: "Check connection",
      });
      if (mode === "unavailable") {
        expect(retry).toBeNull();
        expect(check).toBeInTheDocument();
      } else {
        expect(retry).toBeNull();
        expect(check).toBeNull();
      }
    },
  );

  it("renders a broken status pill and an honest check control when status fails", async () => {
    server.use(
      ...layoutHandlers,
      http.get(
        "*/v1/buddy/operator/status",
        () => new HttpResponse(null, { status: 500 }),
      ),
      http.get("*/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities: [] }),
      ),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));

    render(<BuddyHome />, { store });

    const pill = await screen.findByTestId("buddy-operator-status");
    await waitFor(() => expect(pill).toHaveAttribute("data-state", "error"));
    expect(pill).toHaveTextContent("Broken");
    expect(
      await screen.findByRole("button", { name: "Check connection" }),
    ).toBeInTheDocument();
    expect(screen.queryByTestId("buddy-opportunities-feed")).toBeNull();
  });
});

describe("BuddyHome_operator_surface", () => {
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
  it("renders the lock holder and invokes the operator reopen endpoint", async () => {
    let reopenCalls = 0;
    const unavailableReason = {
      kind: "already_owned",
      detail:
        "operator store is already owned by another writer (pid 2810136 (refact-lsp 0.1.0) on svakhreev-work, started 2026-09-15 09:36:52 UTC, cmd: refact-lsp --http-port 8001)",
      retry_in_seconds: 900,
    };
    server.use(
      ...surfaceHandlers,
      http.get("*/v1/buddy/operator/status", () =>
        HttpResponse.json(operatorStatus("unavailable", unavailableReason)),
      ),
      http.post("*/v1/buddy/operator/reopen", () => {
        reopenCalls += 1;
        return HttpResponse.json(
          operatorStatus("unavailable", unavailableReason),
        );
      }),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));

    const { user } = render(<BuddyHome />, { store });

    const banner = await screen.findByTestId("buddy-operator-degraded");
    expect(banner).toHaveTextContent(
      "Another Refact process has the reports open",
    );
    expect(banner).not.toHaveTextContent("pid 2810136");
    await user.click(
      within(banner).getByRole("button", { name: "Technical details" }),
    );
    expect(banner).toHaveTextContent("pid 2810136");
    expect(banner).toHaveTextContent("refact-lsp --http-port 8001");
    expect(banner).toHaveTextContent(
      "Buddy is already trying again in about 15 minutes.",
    );
    await user.click(
      within(banner).getByRole("button", { name: "Retry connection" }),
    );
    await waitFor(() => expect(reopenCalls).toBe(1));
    expect(await within(banner).findByRole("status")).toHaveTextContent(
      "Reports are still unavailable.",
    );
  });

  it("falls back safely for an unfamiliar unavailable reason", async () => {
    server.use(
      ...surfaceHandlers,
      http.get("*/v1/buddy/operator/status", () =>
        HttpResponse.json(
          operatorStatus("unavailable", {
            kind: "a-new-engine-kind",
            detail: "The engine reported a new condition.",
          }),
        ),
      ),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));

    render(<BuddyHome />, { store });

    const banner = await screen.findByTestId("buddy-operator-degraded");
    expect(banner).toHaveTextContent("Buddy couldn't open the reports");
    expect(banner).not.toHaveTextContent("a-new-engine-kind");
  });
});
