import { render, screen, waitFor, within } from "../../../../utils/test-utils";
import { http, HttpResponse } from "msw";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { server } from "../../../../utils/mockServer";
import { setUpStore } from "../../../../app/store";
import {
  setBuddySnapshot,
  setPulse,
  addBuddySuggestion,
} from "../../buddySlice";
import { push } from "../../../Pages/pagesSlice";
import { BuddyPulseCard } from "../../BuddyPulseCard";
import { BuddyOpportunityCard } from "../../BuddyOpportunityCard";
import { BuddyOpportunitiesFeed } from "../../BuddyOpportunitiesFeed";
import { BuddyHome } from "../../BuddyHome";
import {
  buddyOperatorApi,
  buddyOperatorIdentity,
} from "../../../../services/refact/buddyOperator";
import type {
  BuddyOpportunity,
  BuddySuggestion,
  BuddyPulse,
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
  it("opens the durable proposal when its legacy editing draft has expired", async () => {
    const project = operatorStatus("enabled").project;
    const work = {
      schema_version: 1,
      id: "saved-proposal",
      project,
      subject: {
        resource_kind: "legacy_draft",
        resource_id: "expired",
        version: "1",
      },
      kind: "proposal",
      title: "Preserved proposal",
      state: "needs_attention",
      native_sources: [],
      current_artifact_id: "saved-report",
      current_artifact_revision: 1,
      last_material_change_at: "2026-01-01T00:00:00Z",
    };
    const record = (value: unknown) => ({ value, revision: 1, change_seq: 1 });
    const envelope = (data: unknown) => ({
      schema_version: 1,
      project,
      data,
      next_cursor: null,
    });
    let legacyRequests = 0;
    let resolved = 0;
    server.use(
      http.get("*/v1/buddy/operator/status", () =>
        HttpResponse.json(operatorStatus("enabled")),
      ),
      http.get("*/v1/buddy/operator/legacy-drafts/expired/work", () => {
        resolved++;
        return HttpResponse.json(envelope(record(work)));
      }),
      http.get("*/v1/buddy/work", () => HttpResponse.json(envelope([]))),
      http.get("*/v1/buddy/work/saved-proposal", () =>
        HttpResponse.json(envelope(record(work))),
      ),
      http.get("*/v1/buddy/artifacts/saved-report", () =>
        HttpResponse.json(
          envelope(
            record({
              schema_version: 1,
              artifact_id: "saved-report",
              revision: 1,
              project,
              source_run_ids: [],
              title: "Retained proposal report",
              conclusion: "The proposal still needs a decision.",
              delta: "",
              body: "Durable content survives the editing session.",
              uncertainty: null,
              proposed_action_ids: [],
              evidence: [],
              provenance: {
                source_run_ids: [],
                evidence_refs: [],
                contains_untrusted_source: false,
              },
            }),
          ),
        ),
      ),
      http.get("*/v1/buddy/work/saved-proposal/read", () =>
        HttpResponse.json(envelope(null)),
      ),
      http.get("*/v1/buddy/work/saved-proposal/snooze", () =>
        HttpResponse.json(envelope(null)),
      ),
      http.get("*/v1/buddy/work/saved-proposal/actions", () =>
        HttpResponse.json(envelope([])),
      ),
      http.get("*/v1/buddy/work/saved-proposal/outcomes", () =>
        HttpResponse.json(envelope([])),
      ),
      http.get("*/v1/buddy/artifacts/saved-report/revisions", () =>
        HttpResponse.json(envelope([])),
      ),
      http.all("*/v1/buddy/drafts/expired", () => {
        legacyRequests++;
        return new HttpResponse(null, { status: 404 });
      }),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));
    store.dispatch(push({ name: "buddy", draftId: "expired" }));
    await Promise.all([
      store
        .dispatch(
          buddyOperatorApi.endpoints.getBuddyLegacyDraftWork.initiate({
            id: "expired",
            project_id: "fixture",
            endpoint_identity: buddyOperatorIdentity(store.getState()),
          }),
        )
        .unwrap(),
      store
        .dispatch(
          buddyOperatorApi.endpoints.getBuddyOperatorArtifact.initiate({
            id: "saved-report",
            revision: 1,
            project_id: "fixture",
            endpoint_identity: buddyOperatorIdentity(store.getState()),
          }),
        )
        .unwrap(),
    ]);
    render(<BuddyHome />, { store });
    expect(
      await screen.findByText("Retained proposal report"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Durable content survives the editing session."),
    ).toBeInTheDocument();
    expect(
      screen.getByText("The proposal still needs a decision."),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Dismiss draft" }),
    ).not.toBeInTheDocument();
    expect(resolved).toBe(1);
    expect(legacyRequests).toBe(0);
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
  it("pulse and feed are visible when snapshot present", async () => {
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
    });
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
  it("shows the opportunity list exactly once", async () => {
    const opportunity = makeOpportunity({ id: "single-surface" });
    server.use(
      ...layoutHandlers,
      http.get("*/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities: [opportunity] }),
      ),
      http.get("*/v1/buddy/briefing", () =>
        HttpResponse.json({
          date: "2024-01-01",
          top_cards: [
            {
              id: opportunity.id,
              priority: "high",
              summary: opportunity.summary,
            },
          ],
          receipts: [],
          job_runs: [],
          spend: { tokens_in: 1, tokens_out: 1 },
          pulse: { diagnostics_last_hour: 0 },
        }),
      ),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot(makePulse(), { opportunities: [opportunity] }),
      ),
    );

    const { user } = render(<BuddyHome />, { store });

    await screen.findByTestId("buddy-home-content");
    await user.click(screen.getByRole("tab", { name: "Reports" }));

    const feeds = await screen.findAllByTestId("buddy-opportunities-feed");
    expect(feeds).toHaveLength(1);
    expect(
      within(feeds[0]).getByText("Model config is broken"),
    ).toBeInTheDocument();
    expect(screen.queryByTestId("buddy-briefing-card")).toBeNull();
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
