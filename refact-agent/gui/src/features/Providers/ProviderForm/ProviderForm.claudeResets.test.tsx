import { describe, expect, it } from "vitest";
import { render, screen, waitFor } from "../../../utils/test-utils";
import { http, HttpResponse } from "msw";

import { server } from "../../../utils/mockServer";
import { ProviderForm, type ProviderListItem } from "./ProviderForm";

const provider: ProviderListItem = {
  name: "claude_code",
  base_provider: "claude_code",
  display_name: "Claude Code",
  enabled: true,
  readonly: false,
  has_credentials: true,
  status: "active",
  model_count: 2,
};

const schema = `
description: Claude subscription via OAuth
fields:
  enabled:
    f_type: boolean
    f_label: Enable Provider
`;

const preloadedState = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

const quota = {
  quota: {
    provider_name: "claude_code",
    base_provider: "claude_code",
    plan: null,
    source: "claude_code",
    available: true,
    fetched_at: "2026-09-24T02:00:00Z",
    stale: false,
    windows: [
      {
        id: "five_hour",
        label: "five hour",
        used_percent: 100,
        reset_at: "2026-09-24T07:00:00Z",
      },
    ],
    facts: [],
    error: null,
  },
};

const savedResets = {
  eligible: true,
  ineligible_reason: null,
  at_limit: true,
  exhausted: ["five_hour"],
  grants: [
    {
      id: "launch_2026-09",
      label: "Launch bonus",
      resets_total: 3,
      resets_left: 2,
      starts_at: "2026-09-01T00:00:00Z",
      ends_at: "2026-10-01T00:00:00Z",
      clears: ["five_hour", "seven_day"],
      paused: false,
      usable_now: true,
      use_requires_limit: true,
      percent_used: { five_hour: 100 },
      blocking: ["five_hour"],
    },
  ],
  next_grant_id: "launch_2026-09",
  weekly_resets_at: "2026-09-29T23:00:00Z",
  cooldown_until: null,
};

function useProviderHandlers(...extra: Parameters<typeof server.use>) {
  server.use(
    http.get("*/v1/providers", () =>
      HttpResponse.json({ providers: [provider] }),
    ),
    http.get("*/v1/providers/claude_code", () =>
      HttpResponse.json({
        ...provider,
        selected_models_count: 2,
        settings: { enabled: true },
        runtime: null,
      }),
    ),
    http.get("*/v1/providers/claude_code/schema", () =>
      HttpResponse.json({ name: "claude_code", schema }),
    ),
    http.get("*/v1/providers/claude_code/quota", () =>
      HttpResponse.json(quota),
    ),
    http.get("*/v1/providers/claude_code/available-models", () =>
      HttpResponse.json({ models: [], source: "model_caps" }),
    ),
  );
  server.use(...extra);
}

describe("Claude Code usage resets", () => {
  it.each(["anthropic", "bedrock", "vertex", "foundry", "custom"])(
    "does not request subscription quota or resets for %s",
    async (backend) => {
      let requests = 0;
      useProviderHandlers(
        http.get("*/v1/providers/claude_code", () =>
          HttpResponse.json({
            ...provider,
            settings: { backend },
            runtime: null,
          }),
        ),
        http.get("*/v1/providers/claude_code/quota", () => {
          requests++;
          return HttpResponse.json(quota);
        }),
        http.get("*/v1/providers/claude_code/usage/resets", () => {
          requests++;
          return HttpResponse.json({ data: null });
        }),
      );
      render(<ProviderForm currentProvider={provider} />, { preloadedState });
      expect(
        await screen.findByText(/Credentials are resolved by the engine/),
      ).toBeInTheDocument();
      expect(
        screen.queryByTestId("claude-code-resets"),
      ).not.toBeInTheDocument();
      expect(requests).toBe(0);
    },
  );

  it("offers a usable saved reset and a session reset, then claims the saved one", async () => {
    const claims: unknown[] = [];
    useProviderHandlers(
      http.get("*/v1/providers/claude_code/usage/resets", () =>
        HttpResponse.json({
          data: {
            cedar_ember: savedResets,
            juniper_tide: {
              eligible: true,
              ineligible_reason: null,
              in_experiment: true,
              arm: "reset",
              available: false,
              next_available_at: "2026-09-25T02:00:00Z",
              weekly_resets_at: null,
              resets_per_week: 1,
            },
          },
        }),
      ),
      http.post(
        "*/v1/providers/claude_code/usage/redeem",
        async ({ request }) => {
          claims.push(await request.json());
          return HttpResponse.json({
            data: {
              program: "cedar_ember",
              result: "reset",
              reason: null,
              resets_left: 1,
              cleared: ["five_hour"],
              weekly_resets_at: "2026-09-29T23:00:00Z",
              cooldown_until: null,
              next_available_at: null,
            },
          });
        },
      ),
    );

    const { user } = render(<ProviderForm currentProvider={provider} />, {
      preloadedState,
    });

    const useSaved = await screen.findByRole("button", {
      name: "Use saved reset",
    });
    expect(useSaved).toBeEnabled();
    expect(
      screen.getByText(/2 of 3 resets left · Launch bonus/),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/Clears Current session, Current week — all models/),
    ).toBeInTheDocument();

    const sessionReset = screen.getByRole("button", {
      name: "Reset session limit",
    });
    expect(sessionReset).toBeDisabled();
    expect(
      screen.getByText(/Next session reset available/),
    ).toBeInTheDocument();

    await user.click(useSaved);

    expect(
      await screen.findByText(
        "Usage limit reset. Cleared: Current session. 1 saved reset left.",
      ),
    ).toBeInTheDocument();
    await waitFor(() => {
      expect(claims).toHaveLength(1);
    });
    const claim = claims[0] as Record<string, unknown>;
    expect(claim.program).toBe("cedar_ember");
    expect(claim.grant_id).toBe("launch_2026-09");
    expect(typeof claim.request_id).toBe("string");
  });

  it("explains when the account has no resets and surfaces engine errors", async () => {
    useProviderHandlers(
      http.get("*/v1/providers/claude_code/usage/resets", () =>
        HttpResponse.json({
          data: {
            cedar_ember: {
              eligible: false,
              ineligible_reason: "no_grant",
              at_limit: false,
              exhausted: [],
              grants: [],
              next_grant_id: null,
            },
            juniper_tide: null,
          },
        }),
      ),
    );

    render(<ProviderForm currentProvider={provider} />, { preloadedState });

    expect(
      await screen.findByText(
        "No usage resets are available for this account (no grant).",
      ),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Use saved reset" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Reset session limit" }),
    ).not.toBeInTheDocument();
  });

  it("surfaces engine errors from the reset status endpoint", async () => {
    useProviderHandlers(
      http.get("*/v1/providers/claude_code/usage/resets", () =>
        HttpResponse.json({
          error:
            "Anthropic is rate limiting reset requests right now. Try again in a minute.",
        }),
      ),
    );

    render(<ProviderForm currentProvider={provider} />, { preloadedState });

    expect(
      await screen.findByText(/rate limiting reset requests/),
    ).toHaveAttribute("role", "alert");
    expect(screen.getByRole("button", { name: "Refresh" })).toBeInTheDocument();
    expect(
      screen.queryByText(/No usage resets are available/),
    ).not.toBeInTheDocument();
  });
});
