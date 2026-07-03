import { http, HttpResponse, delay } from "msw";
import { describe, expect, it } from "vitest";
import { render, screen, waitFor } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { BuddyOpportunityCard } from "../BuddyOpportunityCard";
import type { BuddyAction, BuddyOpportunity, BuddySnapshot } from "../types";

const CONFIG_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

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

function makeSnapshot(): BuddySnapshot {
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
      message_observation_enabled: true,
      chat_reactions_enabled: true,
      housekeeping_enabled: true,
      humor_enabled: true,
      humor_level: "light",
      autonomy_level: "suggest",
      quiet_mode: false,
      daily_digest_hour: 18,
      observers: {
        task_health: true,
        trajectory_clutter: true,
        chat_pattern: true,
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

describe("BuddyOpportunityCard", () => {
  it("invokes the action endpoint with the selected opportunity and index", async () => {
    let requestBody: unknown;
    const action: BuddyAction = { kind: "open_page", page: { type: "stats" } };
    const opportunity = makeOpportunity({ proposed_actions: [action] });
    server.use(
      http.post("*/v1/buddy/opportunities/:id/accept", async ({ request }) => {
        requestBody = await request.json();
        return HttpResponse.json({
          snapshot: makeSnapshot(),
          action_result: { kind: "dismiss" },
        });
      }),
    );

    const { user } = render(
      <BuddyOpportunityCard opportunity={opportunity} />,
      {
        preloadedState: CONFIG_STATE,
      },
    );

    await user.click(screen.getByRole("button", { name: "Open Stats" }));

    await waitFor(() => {
      expect(requestBody).toEqual({ action_index: 0 });
    });
  });

  it("shows pending state while an action is running", async () => {
    server.use(
      http.post("*/v1/buddy/opportunities/:id/dismiss", async () => {
        await delay(100);
        return HttpResponse.json({ snapshot: makeSnapshot() });
      }),
    );
    const opportunity = makeOpportunity({
      proposed_actions: [{ kind: "dismiss" }],
    });
    const { user } = render(
      <BuddyOpportunityCard opportunity={opportunity} />,
      {
        preloadedState: CONFIG_STATE,
      },
    );

    await user.click(screen.getByRole("button", { name: "Dismiss" }));

    const pendingButton = screen.getByRole("button", { name: "Dismiss" });
    expect(pendingButton).toHaveTextContent("Working…");
    expect(pendingButton).toHaveAttribute("aria-busy", "true");
    expect(pendingButton).toBeDisabled();

    await waitFor(() => {
      expect(pendingButton).toHaveTextContent("Dismiss");
    });
  });

  it("shows an alert when an action fails", async () => {
    server.use(
      http.post("*/v1/buddy/opportunities/:id/accept", () =>
        HttpResponse.json(
          { message: "Gremlin jammed the gears" },
          { status: 500 },
        ),
      ),
    );
    const opportunity = makeOpportunity({
      proposed_actions: [{ kind: "create_pulse_report", scope: "all" }],
    });
    const { user } = render(
      <BuddyOpportunityCard opportunity={opportunity} />,
      {
        preloadedState: CONFIG_STATE,
      },
    );

    await user.click(
      screen.getByRole("button", { name: "Create system report" }),
    );

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Gremlin jammed the gears",
    );
  });

  it("never button dismisses with never flag and quest badge shows", async () => {
    let dismissBody: unknown;
    server.use(
      http.post("*/v1/buddy/opportunities/:id/dismiss", async ({ request }) => {
        dismissBody = await request.json();
        return HttpResponse.json({ snapshot: makeSnapshot() });
      }),
    );
    const opportunity = makeOpportunity({
      kind: "quest",
      summary: "Make one productive move (+12 XP)",
      proposed_actions: [
        { kind: "accept_quest", suggestion_id: "sugg-1" },
        { kind: "dismiss" },
      ],
    });
    const { user } = render(
      <BuddyOpportunityCard opportunity={opportunity} />,
      {
        preloadedState: CONFIG_STATE,
      },
    );

    expect(screen.getByLabelText("Quest")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Take quest" }),
    ).toBeInTheDocument();

    await user.click(
      screen.getByRole("button", { name: "Never show this again" }),
    );

    await waitFor(() => {
      expect(dismissBody).toEqual({ never: true });
    });
  });
});
