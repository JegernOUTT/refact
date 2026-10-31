import { describe, expect, it } from "vitest";
import { render, screen, waitFor } from "../../../utils/test-utils";
import { http, HttpResponse } from "msw";

import { server } from "../../../utils/mockServer";
import { ProviderForm, type ProviderListItem } from "./ProviderForm";

HTMLElement.prototype.hasPointerCapture = () => false;
HTMLElement.prototype.releasePointerCapture = () => undefined;
HTMLElement.prototype.scrollIntoView = () => undefined;

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
    http.get("*/v1/providers/claude_code/bedrock/profiles", () =>
      HttpResponse.json({
        profiles: [],
        suggested_profile: "work",
        suggested_region: "eu-west-1",
      }),
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
        await screen.findByText(
          backend === "bedrock"
            ? /Use AWS profiles on the engine host/
            : /Credentials are resolved by the engine/,
        ),
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

const backendSchema = `
description: Claude Code
fields:
  backend:
    f_type: select
    f_label: Backend
    f_default: oauth
    f_options:
      - {value: oauth, label: "Claude subscription"}
      - {value: anthropic, label: "Anthropic API"}
      - {value: bedrock, label: "Amazon Bedrock"}
      - {value: vertex, label: "Google Vertex AI"}
      - {value: foundry, label: "Microsoft Foundry"}
      - {value: custom, label: "Custom endpoint"}
  auth_type:
    f_type: select
    f_label: Authentication
    f_default: api_key
    f_options:
      - {value: api_key, label: "API key option"}
      - {value: bearer, label: "Bearer option", f_visible_when: {backend: [custom]}}
      - {value: none, label: "None option", f_visible_when: {backend: [custom]}}
      - {value: oauth, label: "Console sign-in option"}
    f_visible_when: {backend: [anthropic, custom]}
oauth:
  supported: true
  methods:
    - id: max
      label: "Claude Pro/Max"
`;

function useBackendHandlers(
  settings: Record<string, unknown>,
  readonly = false,
  oauthSchema = backendSchema,
) {
  let quotaRequests = 0;
  useProviderHandlers(
    http.get("*/v1/providers/claude_code", () =>
      HttpResponse.json({
        ...provider,
        readonly,
        settings,
        runtime: null,
      }),
    ),
    http.get("*/v1/providers/claude_code/schema", () =>
      HttpResponse.json({ name: "claude_code", schema: oauthSchema }),
    ),
    http.get("*/v1/providers/claude_code/quota", () => {
      quotaRequests++;
      return HttpResponse.json(quota);
    }),
    http.get("*/v1/providers/claude_code/usage/resets", () => {
      quotaRequests++;
      return HttpResponse.json({ data: null });
    }),
  );
  return () => quotaRequests;
}

describe("Claude Code sign-in actions", () => {
  it.each([
    [
      { backend: "anthropic", auth_type: "oauth" },
      "Login with Anthropic Console",
    ],
    [{ backend: "vertex" }, "Login with Google Cloud"],
    [{ backend: "foundry" }, "Login with Microsoft Azure"],
  ])(
    "offers login for %o without subscription quota",
    async (settings, label) => {
      const quotaRequests = useBackendHandlers(settings);
      render(<ProviderForm currentProvider={provider} />, { preloadedState });
      expect(await screen.findByText(label)).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Login" })).toBeEnabled();
      expect(screen.queryByText("Quota")).not.toBeInTheDocument();
      expect(
        screen.queryByTestId("claude-code-resets"),
      ).not.toBeInTheDocument();
      expect(quotaRequests()).toBe(0);
    },
  );

  it("keeps subscription login together with quota", async () => {
    useBackendHandlers({ backend: "oauth" });
    render(<ProviderForm currentProvider={provider} />, { preloadedState });
    expect(await screen.findByText("Login with Anthropic")).toBeInTheDocument();
    expect(await screen.findByText("Quota")).toBeInTheDocument();
  });

  it.each([
    [{ backend: "anthropic" }],
    [{ backend: "anthropic", auth_type: "api_key" }],
    [{ backend: "custom", auth_type: "oauth" }],
    [{ backend: "custom", auth_type: "bearer" }],
  ])("does not offer login for %o", async (settings) => {
    useBackendHandlers(settings);
    render(<ProviderForm currentProvider={provider} />, { preloadedState });
    expect(
      await screen.findByText(/Credentials are resolved by the engine/),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Login" }),
    ).not.toBeInTheDocument();
  });

  it("offers Bedrock SSO only as explicit optional recovery", async () => {
    useBackendHandlers({ backend: "bedrock" });
    render(<ProviderForm currentProvider={provider} />, { preloadedState });
    expect(
      await screen.findByText(/Use AWS profiles on the engine host/),
    ).toBeInTheDocument();
    expect(screen.getByText(/Optional AWS SSO recovery/)).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Login" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByText(/aws configure sso on the engine host/),
    ).toBeInTheDocument();
  });

  it("disables login for readonly providers", async () => {
    useBackendHandlers({ backend: "vertex" }, true);
    render(<ProviderForm currentProvider={provider} />, { preloadedState });
    await screen.findByText("Login with Google Cloud");
    expect(screen.getByRole("button", { name: "Login" })).toBeDisabled();
  });

  it("respects engine login method conditions", async () => {
    const conditioned = backendSchema.replace(
      `    - id: max
      label: "Claude Pro/Max"`,
      `    - id: max
      label: "Claude Pro/Max"
      f_visible_when: {backend: [oauth]}
    - id: native
      label: "Cloud sign-in"
      f_visible_when: {backend: [bedrock, vertex]}`,
    );
    useBackendHandlers({ backend: "foundry" }, false, conditioned);
    render(<ProviderForm currentProvider={provider} />, { preloadedState });
    expect(
      await screen.findByText(/native credential store/),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Login" }),
    ).not.toBeInTheDocument();
  });

  it("hides the Console auth option for custom endpoints", async () => {
    useBackendHandlers({ backend: "custom" });
    const { user } = render(<ProviderForm currentProvider={provider} />, {
      preloadedState,
    });
    const authSelect = await screen.findByRole("combobox", {
      name: /Authentication/,
    });
    await user.click(authSelect);
    expect(await screen.findByText("Bearer option")).toBeInTheDocument();
    expect(
      screen.queryByText("Console sign-in option"),
    ).not.toBeInTheDocument();
  });

  it("offers the Console auth option for the Anthropic backend", async () => {
    useBackendHandlers({ backend: "anthropic" });
    const { user } = render(<ProviderForm currentProvider={provider} />, {
      preloadedState,
    });
    const authSelect = await screen.findByRole("combobox", {
      name: /Authentication/,
    });
    await user.click(authSelect);
    expect(
      await screen.findByText("Console sign-in option"),
    ).toBeInTheDocument();
    expect(screen.queryByText("Bearer option")).not.toBeInTheDocument();
  });

  it("resets an in-flight native session when the saved backend changes", async () => {
    const settings: Record<string, unknown> = { backend: "bedrock" };
    const logouts: unknown[] = [];
    const updates: unknown[] = [];
    useBackendHandlers(settings);
    server.use(
      http.get("*/v1/providers/claude_code", () =>
        HttpResponse.json({ ...provider, settings, runtime: null }),
      ),
      http.post("*/v1/providers/claude_code/oauth/start", () =>
        HttpResponse.json({
          session_id: "native-bedrock",
          authorize_url: "",
          mode: "native",
          instructions: "Run the AWS sign-in on the engine host.",
          poll_interval: 30,
        }),
      ),
      http.post(
        "*/v1/providers/claude_code/oauth/logout",
        async ({ request }) => {
          logouts.push(await request.json());
          return HttpResponse.json({ success: true });
        },
      ),
      http.post("*/v1/providers/claude_code", async ({ request }) => {
        const update: unknown = await request.json();
        updates.push(update);
        Object.assign(settings, update);
        return HttpResponse.json({ success: true });
      }),
    );
    const { user } = render(<ProviderForm currentProvider={provider} />, {
      preloadedState,
    });
    await user.click(await screen.findByText("Optional AWS SSO recovery"));
    await user.click(
      screen.getByRole("button", {
        name: "Save selected profile for SSO recovery",
      }),
    );
    await user.click(await screen.findByRole("button", { name: "Login" }));
    expect(updates).toContainEqual({
      base_provider: "claude_code",
      display_name: "Claude Code",
      aws_profile: "work",
      aws_region: "eu-west-1",
    });
    expect(
      await screen.findByText("Run the AWS sign-in on the engine host."),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("combobox", { name: /Backend/ }));
    await user.click(
      await screen.findByRole("option", { name: "Google Vertex AI" }),
    );

    expect(
      await screen.findByText("Login with Google Cloud"),
    ).toBeInTheDocument();
    expect(
      screen.queryByText("Run the AWS sign-in on the engine host."),
    ).not.toBeInTheDocument();
    await waitFor(() => {
      expect(logouts).toEqual([{ session_id: "native-bedrock" }]);
    });
  });
});
