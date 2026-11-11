import { describe, expect, it, vi } from "vitest";
import { screen, waitFor, within } from "@testing-library/react";
import { http, HttpResponse } from "msw";

import { render } from "../../../../utils/test-utils";
import { server } from "../../../../utils/mockServer";
import { LiteLLMSections } from "./LiteLLMSections";
import { formatUsd, validateDateRange } from "./litellmFormat";

const preloadedState = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

const baseAccount = {
  state: "ok",
  fetched_at: "2026-01-01T00:00:00Z",
  stale: false,
  subjects: [
    {
      scope: "key",
      label: "ci-key",
      state: "ok",
      spend: { value: 0.00001234, state: "known" },
      max_budget: { kind: "unlimited" },
      remaining: { derived: false, overage: false },
      currency: "USD",
      rpm_limit: null,
      model_limits: [{ model: "gpt-x", max_budget: 0 }],
    },
    {
      scope: "team",
      id: "team-1",
      label: "platform",
      state: "permission_denied",
      spend: { value: null, state: "unknown" },
      max_budget: { kind: "denied" },
      remaining: { derived: false, overage: false },
    },
  ],
};

function renderSections(providerName = "litellm_a", hasCredentials = true) {
  return render(
    <LiteLLMSections
      providerName={providerName}
      hasCredentials={hasCredentials}
      models={<div>models-list</div>}
    />,
    { preloadedState },
  );
}

describe("LiteLLM section tabs", () => {
  it("is keyboard operable as a radiogroup and defaults to Models", async () => {
    server.use(
      http.get("*/v1/providers/litellm_a/litellm/account", () =>
        HttpResponse.json({ state: "unsupported", subjects: [] }),
      ),
      http.get("*/v1/providers/litellm_a/litellm/usage", () =>
        HttpResponse.json({ state: "unsupported" }),
      ),
    );
    const { user } = renderSections();
    const group = screen.getByRole("radiogroup", {
      name: "LiteLLM provider sections",
    });
    const models = within(group).getByRole("radio", { name: "Models" });
    expect(models).toBeChecked();
    const quotas = within(group).getByRole("radio", { name: "Quotas & Usage" });
    quotas.focus();
    await user.keyboard(" ");
    expect(quotas).toBeChecked();
    expect(
      await screen.findByText(/Budgets are read with the configured provider/),
    ).toBeVisible();
  });
});

describe("LiteLLM formatting", () => {
  it("never rounds a small non-zero spend to zero", () => {
    expect(formatUsd(0.00001234)).toContain("0.00001234");
    expect(formatUsd(0)).toContain("0.00");
    expect(formatUsd(-3.5)).toContain("3.50");
  });

  it("validates date ranges", () => {
    expect(validateDateRange("2026-01-01", "2026-01-07")).toBeNull();
    expect(validateDateRange("2026-01-08", "2026-01-07")).not.toBeNull();
    expect(validateDateRange("2026-01-01", "2026-06-01")).not.toBeNull();
    expect(validateDateRange("", "2026-01-07")).not.toBeNull();
  });
});

describe("LiteLLM sections navigation and quotas", () => {
  it("starts on Models and does not fetch account data until opened", async () => {
    let accountCalls = 0;
    server.use(
      http.get("*/v1/providers/litellm_a/litellm/account", () => {
        accountCalls += 1;
        return HttpResponse.json(baseAccount);
      }),
      http.get("*/v1/providers/litellm_a/litellm/usage", () =>
        HttpResponse.json({ state: "unsupported" }),
      ),
    );
    const { user } = renderSections();
    expect(screen.getByText("models-list")).toBeInTheDocument();
    expect(accountCalls).toBe(0);

    await user.click(screen.getByRole("radio", { name: "Quotas & Usage" }));
    expect(await screen.findByTestId("litellm-subject-key")).toBeVisible();
    expect(accountCalls).toBe(1);
  });

  it("keeps independent scopes, unlimited vs denied, and tiny spend", async () => {
    server.use(
      http.get("*/v1/providers/litellm_a/litellm/account", () =>
        HttpResponse.json(baseAccount),
      ),
      http.get("*/v1/providers/litellm_a/litellm/usage", () =>
        HttpResponse.json({ state: "unsupported" }),
      ),
    );
    const { user } = renderSections();
    await user.click(screen.getByRole("radio", { name: "Quotas & Usage" }));

    const key = await screen.findByTestId("litellm-subject-key");
    expect(within(key).getAllByText(/0\.00001234/).length).toBeGreaterThan(0);
    expect(within(key).getAllByText("Unlimited").length).toBeGreaterThan(0);
    expect(within(key).getByText("No limit")).toBeInTheDocument();
    expect(within(key).getAllByText("Not reported").length).toBeGreaterThan(0);

    const team = screen.getByTestId("litellm-subject-team");
    expect(within(team).getByText("Permission denied")).toBeInTheDocument();
    expect(within(team).queryByText(/\$0\.00/)).not.toBeInTheDocument();
    expect(
      await screen.findByText(
        "This LiteLLM deployment does not expose usage history.",
      ),
    ).toBeInTheDocument();
  });

  it("shows over-budget amounts exactly and labels derived remaining", async () => {
    server.use(
      http.get("*/v1/providers/litellm_a/litellm/account", () =>
        HttpResponse.json({
          ...baseAccount,
          subjects: [
            {
              scope: "key",
              label: "k",
              state: "ok",
              spend: { value: 12.5, state: "known" },
              max_budget: { kind: "limited", value: 10 },
              remaining: { value: -2.5, derived: true, overage: true },
              currency: "USD",
            },
          ],
        }),
      ),
      http.get("*/v1/providers/litellm_a/litellm/usage", () =>
        HttpResponse.json({ state: "unsupported" }),
      ),
    );
    const { user } = renderSections();
    await user.click(screen.getByRole("radio", { name: "Quotas & Usage" }));
    const key = await screen.findByTestId("litellm-subject-key");
    expect(within(key).getByText(/derived, over budget/)).toBeInTheDocument();
    expect(within(key).getAllByText(/12\.50/).length).toBeGreaterThan(0);
  });

  it("does not claim a key never expires when expiry is not reported", async () => {
    server.use(
      http.get("*/v1/providers/litellm_a/litellm/account", () =>
        HttpResponse.json(baseAccount),
      ),
      http.get("*/v1/providers/litellm_a/litellm/usage", () =>
        HttpResponse.json({ state: "unsupported" }),
      ),
    );
    const { user } = renderSections();
    await user.click(screen.getByRole("radio", { name: "Quotas & Usage" }));
    const key = await screen.findByTestId("litellm-subject-key");
    expect(within(key).queryByText("No expiry")).not.toBeInTheDocument();
  });

  it("distinguishes unlimited (null), unknown (absent) and zero limits", async () => {
    server.use(
      http.get("*/v1/providers/litellm_a/litellm/account", () =>
        HttpResponse.json({
          ...baseAccount,
          subjects: [
            {
              scope: "key",
              label: "k",
              state: "ok",
              spend: { value: 1, state: "known" },
              max_budget: { kind: "limited", value: 5 },
              remaining: { value: 4, derived: false, overage: false },
              rpm_limit: 0,
              tpm_limit: null,
              model_limits: [
                { model: "absent-model", max_budget: null },
                { model: "zero-model", rpm_limit: 0, tpm_limit: 100 },
              ],
            },
          ],
        }),
      ),
      http.get("*/v1/providers/litellm_a/litellm/usage", () =>
        HttpResponse.json({ state: "unsupported" }),
      ),
    );
    const { user } = renderSections();
    await user.click(screen.getByRole("radio", { name: "Quotas & Usage" }));
    const key = await screen.findByTestId("litellm-subject-key");
    const dt = (label: string): HTMLElement => {
      const holder = within(key).getByText(label).parentElement;
      if (!holder) throw new Error(`no fact for ${label}`);
      return holder;
    };
    expect(
      within(dt("Configured RPM limit")).getByText("0"),
    ).toBeInTheDocument();
    expect(
      within(dt("Configured TPM limit")).getByText("No limit"),
    ).toBeInTheDocument();
    expect(
      within(dt("Max parallel requests")).getByText("Not reported"),
    ).toBeInTheDocument();
    const absentRow = within(key).getByText("absent-model").closest("tr");
    expect(absentRow).not.toBeNull();
    expect(
      within(absentRow as HTMLElement).queryByText("No limit"),
    ).not.toBeInTheDocument();
    expect(
      within(absentRow as HTMLElement).getAllByText("Not reported"),
    ).toHaveLength(3);
    const zeroRow = within(key).getByText("zero-model").closest("tr");
    expect(within(zeroRow as HTMLElement).getByText("100")).toBeInTheDocument();
  });

  it("shows a negative remaining exactly", async () => {
    server.use(
      http.get("*/v1/providers/litellm_a/litellm/account", () =>
        HttpResponse.json({
          ...baseAccount,
          subjects: [
            {
              scope: "key",
              label: "k",
              state: "ok",
              spend: { value: 12.5, state: "known" },
              max_budget: { kind: "limited", value: 10 },
              remaining: { value: -2.5, derived: false, overage: true },
            },
          ],
        }),
      ),
      http.get("*/v1/providers/litellm_a/litellm/usage", () =>
        HttpResponse.json({ state: "unsupported" }),
      ),
    );
    const { user } = renderSections();
    await user.click(screen.getByRole("radio", { name: "Quotas & Usage" }));
    const key = await screen.findByTestId("litellm-subject-key");
    expect(within(key).getByText(/-\$2\.50 \(over budget\)/)).toBeVisible();
  });

  it("renders observed headroom separately from account budget", async () => {
    server.use(
      http.get("*/v1/providers/litellm_a/litellm/account", () =>
        HttpResponse.json({
          ...baseAccount,
          observed_headroom: {
            observed_at: "2026-01-01T00:00:00Z",
            scope: "unknown",
            model: "gpt-x",
            requests_remaining: 41,
            requests_limit: 60,
          },
        }),
      ),
      http.get("*/v1/providers/litellm_a/litellm/usage", () =>
        HttpResponse.json({ state: "unsupported" }),
      ),
    );
    const { user } = renderSections();
    await user.click(screen.getByRole("radio", { name: "Quotas & Usage" }));
    const headroom = await screen.findByTestId("litellm-observed-headroom");
    expect(within(headroom).getByText("41")).toBeInTheDocument();
    expect(
      within(headroom).getByText(/not an account balance for the key/),
    ).toBeInTheDocument();
  });

  it("shows an unavailable key as unavailable and the user budget as User, never as key zero/unlimited", async () => {
    server.use(
      http.get("*/v1/providers/litellm_a/litellm/account", () =>
        HttpResponse.json({
          state: "ok",
          fetched_at: "2026-01-01T00:00:00Z",
          stale: false,
          subjects: [
            {
              scope: "key",
              label: "login",
              state: "unavailable",
              message: "Key unavailable: this login has no virtual key.",
              spend: { value: null, state: "unknown" },
              max_budget: { kind: "unlimited" },
              remaining: { derived: false, overage: false },
              expires_at: "2030-01-01T00:00:00Z",
            },
            {
              scope: "user",
              id: "u-1",
              label: "alice",
              state: "ok",
              spend: { value: 5, state: "known" },
              max_budget: { kind: "limited", value: 20 },
              remaining: { value: 15, derived: true, overage: false },
              currency: "USD",
              expires_at: "2031-01-01T00:00:00Z",
            },
          ],
        }),
      ),
      http.get("*/v1/providers/litellm_a/litellm/usage", () =>
        HttpResponse.json({ state: "unsupported" }),
      ),
    );
    const { user } = renderSections();
    await user.click(screen.getByRole("radio", { name: "Quotas & Usage" }));

    const key = await screen.findByTestId("litellm-subject-key");
    expect(within(key).getByText("Unavailable")).toBeInTheDocument();
    expect(
      within(key).getByText(/Key unavailable: this login has no virtual key/),
    ).toBeVisible();
    expect(within(key).queryByText(/\$0\.00/)).not.toBeInTheDocument();
    expect(within(key).queryByText("Unlimited")).not.toBeInTheDocument();
    expect(within(key).queryByText("Key expires")).not.toBeInTheDocument();
    expect(within(key).queryByText(/\$20\.00/)).not.toBeInTheDocument();

    const userCard = screen.getByTestId("litellm-subject-user");
    expect(within(userCard).getByText(/User · alice/)).toBeInTheDocument();
    expect(within(userCard).getAllByText(/\$20\.00/).length).toBeGreaterThan(0);
    expect(within(userCard).getAllByText(/\$5\.00/).length).toBeGreaterThan(0);
    expect(within(userCard).getByText(/\$15\.00 \(derived\)/)).toBeVisible();
    // Key expiry is a key-scope fact and must not be attributed to the user.
    expect(within(userCard).queryByText("Key expires")).not.toBeInTheDocument();
    expect(
      screen.getAllByText(/configured provider credential or login/).length,
    ).toBeGreaterThan(0);
  });

  it.each([
    ["permission_denied", /not permitted/],
    ["unsupported", /does not expose/],
    ["not_configured", /not configured/],
  ])("shows %s as a state, never as zero", async (state, text) => {
    server.use(
      http.get("*/v1/providers/litellm_a/litellm/account", () =>
        HttpResponse.json({ state, subjects: [] }),
      ),
      http.get("*/v1/providers/litellm_a/litellm/usage", () =>
        HttpResponse.json({ state: "unsupported" }),
      ),
    );
    const { user } = renderSections();
    await user.click(screen.getByRole("radio", { name: "Quotas & Usage" }));
    expect((await screen.findAllByText(text)).length).toBeGreaterThan(0);
    expect(screen.queryByTestId("litellm-subject-key")).not.toBeInTheDocument();
  });

  it("shows an error, not zero values, when the request fails", async () => {
    server.use(
      http.get("*/v1/providers/litellm_a/litellm/account", () =>
        HttpResponse.json({ detail: "boom" }, { status: 500 }),
      ),
      http.get("*/v1/providers/litellm_a/litellm/usage", () =>
        HttpResponse.json({ state: "unsupported" }),
      ),
    );
    const { user } = renderSections();
    await user.click(screen.getByRole("radio", { name: "Quotas & Usage" }));
    expect(await screen.findByText("Failed to load quotas.")).toBeVisible();
  });

  it("asks for credentials instead of querying when none are saved", async () => {
    const { user } = renderSections("litellm_a", false);
    await user.click(screen.getByRole("radio", { name: "Quotas & Usage" }));
    expect(
      screen.getByText("Configure this provider to see quotas and usage."),
    ).toBeInTheDocument();
  });

  it("does not show a previous provider's account after switching", async () => {
    server.use(
      http.get("*/v1/providers/litellm_a/litellm/account", () =>
        HttpResponse.json(baseAccount),
      ),
      http.get("*/v1/providers/litellm_b/litellm/account", async () => {
        await new Promise((resolve) => setTimeout(resolve, 150));
        return HttpResponse.json({ ...baseAccount, subjects: [] });
      }),
      http.get("*/v1/providers/:name/litellm/usage", () =>
        HttpResponse.json({ state: "unsupported" }),
      ),
    );
    const { user, rerender } = renderSections("litellm_a");
    await user.click(screen.getByRole("radio", { name: "Quotas & Usage" }));
    expect(await screen.findByTestId("litellm-subject-key")).toBeVisible();

    rerender(
      <LiteLLMSections
        providerName="litellm_b"
        hasCredentials
        models={<div>models-list</div>}
      />,
    );
    expect(screen.queryByTestId("litellm-subject-key")).not.toBeInTheDocument();
  });
});

describe("LiteLLM usage", () => {
  const usage = {
    state: "ok",
    fetched_at: "2026-01-02T00:00:00Z",
    scope: "key",
    start: "2026-01-01",
    end: "2026-01-02",
    page: 1,
    page_size: 25,
    total_pages: 2,
    has_more: true,
    truncated: true,
    totals_source: "page",
    partial: true,
    totals: {
      spend: 0.0042,
      requests: 10,
      prompt_tokens: 100,
      completion_tokens: 50,
      total_tokens: 150,
    },
    daily: [
      {
        date: "2026-01-01",
        spend: 0.0042,
        requests: 10,
        total_tokens: 150,
      },
    ],
    models: [
      { model: "gpt-x", spend: 0.0042, requests: 10, total_tokens: 150 },
    ],
    api_keys: [
      {
        label: "this",
        alias: "ci",
        current: true,
        spend: 0.0042,
        requests: 10,
      },
    ],
  };

  it("sends scope/date/page params, shows truncation, and paginates", async () => {
    const seen: URLSearchParams[] = [];
    server.use(
      http.get("*/v1/providers/litellm_a/litellm/account", () =>
        HttpResponse.json({ state: "unsupported", subjects: [] }),
      ),
      http.get("*/v1/providers/litellm_a/litellm/usage", ({ request }) => {
        seen.push(new URL(request.url).searchParams);
        return HttpResponse.json(usage);
      }),
    );
    const { user } = renderSections();
    await user.click(screen.getByRole("radio", { name: "Quotas & Usage" }));

    expect(
      await screen.findByText(/Breakdowns may be incomplete/),
    ).toBeVisible();
    expect(
      screen.getByText(/Totals cover the current page only/),
    ).toBeVisible();
    expect(screen.getByText("By model (this page)")).toBeInTheDocument();
    expect(screen.getAllByText(/0\.0042/).length).toBeGreaterThan(0);
    expect(screen.getByText("ci (current credential)")).toBeInTheDocument();
    expect(seen[0].get("scope")).toBe("key");
    expect(seen[0].get("page")).toBe("1");
    expect(seen[0].get("page_size")).toBe("25");
    expect(seen[0].has("scope_id")).toBe(false);

    await user.click(screen.getByRole("button", { name: "Next page" }));
    await waitFor(() =>
      expect(seen.some((p) => p.get("page") === "2")).toBe(true),
    );

    await user.click(screen.getByRole("radio", { name: "Team" }));
    await waitFor(() =>
      expect(
        seen.some((p) => p.get("scope") === "team" && p.get("page") === "1"),
      ).toBe(true),
    );
  });

  it("keeps Key scope unavailable by default and loads User only on explicit selection", async () => {
    const scopes: string[] = [];
    server.use(
      http.get("*/v1/providers/litellm_a/litellm/account", () =>
        HttpResponse.json({ state: "unsupported", subjects: [] }),
      ),
      http.get("*/v1/providers/litellm_a/litellm/usage", ({ request }) => {
        const scope = new URL(request.url).searchParams.get("scope") ?? "";
        scopes.push(scope);
        if (scope === "user") {
          return HttpResponse.json({
            ...usage,
            scope: "user",
            truncated: false,
            partial: false,
            has_more: false,
            total_pages: 1,
            totals_source: "gateway_summary",
            totals: { ...usage.totals, spend: 7.25 },
            api_keys: [],
          });
        }
        return HttpResponse.json({
          state: "unavailable",
          scope: "key",
          message: "Key unavailable: this login has no virtual key.",
        });
      }),
    );
    const { user } = renderSections();
    await user.click(screen.getByRole("radio", { name: "Quotas & Usage" }));

    expect(
      await screen.findByText(/Key unavailable: this login has no virtual key/),
    ).toBeVisible();
    expect(screen.getByRole("radio", { name: "Key" })).toBeChecked();
    expect(screen.queryByText("Daily activity")).not.toBeInTheDocument();
    expect(screen.queryByRole("table")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Usage totals")).not.toBeInTheDocument();
    // No silent broadening: only the key scope was requested.
    expect(scopes).toEqual(["key"]);

    await user.click(screen.getByRole("radio", { name: "User" }));
    await waitFor(() => expect(scopes).toContain("user"));
    expect(await screen.findByText("Daily activity")).toBeVisible();
    expect(
      within(screen.getByLabelText("Usage totals")).getByText(/7\.25/),
    ).toBeVisible();
    expect(
      screen.getByText(/Aggregate activity for the User scope/),
    ).toBeVisible();
    expect(
      screen.queryByText(/Key unavailable: this login has no virtual key/),
    ).not.toBeInTheDocument();
  });

  it("shows usage permission denial as such, not as zero", async () => {
    let usageCalls = 0;
    server.use(
      http.get("*/v1/providers/litellm_a/litellm/account", () =>
        HttpResponse.json({ state: "unsupported", subjects: [] }),
      ),
      http.get("*/v1/providers/litellm_a/litellm/usage", () => {
        usageCalls += 1;
        return HttpResponse.json({ state: "permission_denied" });
      }),
    );
    const { user } = renderSections();
    await user.click(screen.getByRole("radio", { name: "Quotas & Usage" }));
    expect(
      (await screen.findAllByText(/not permitted to read usage history/))
        .length,
    ).toBeGreaterThan(0);
    expect(screen.queryByText("Daily activity")).not.toBeInTheDocument();
    expect(usageCalls).toBe(1);
  });
});

describe("LiteLLM MCP servers", () => {
  const mcp = {
    state: "ok",
    fetched_at: "2026-01-01T00:00:00Z",
    stale: false,
    servers: [
      {
        server_id: "slack",
        display_name: "Slack",
        auth: {
          kind: "oauth",
          state: "auth_required",
          setup_url: "https://gateway.example.com/setup",
        },
        health: "unknown",
        access: "allowed",
        enabled: false,
        connected: false,
      },
      {
        server_id: "fibery",
        display_name: "Fibery",
        auth: { kind: "byok", state: "missing_setup" },
        health: "healthy",
        access: "denied",
        enabled: false,
      },
    ],
  };

  it("lists servers disabled by default with honest status and no auto-enable", async () => {
    server.use(
      http.get("*/v1/providers/litellm_a/litellm/mcp", () =>
        HttpResponse.json(mcp),
      ),
    );
    const { user } = renderSections();
    await user.click(screen.getByRole("radio", { name: "MCP Servers" }));
    const slack = await screen.findByTestId("litellm-mcp-slack");
    expect(within(slack).getByText("Disabled")).toBeInTheDocument();
    expect(within(slack).getByText("Auth required")).toBeInTheDocument();
    expect(within(slack).getByText("Health unknown")).toBeInTheDocument();
    expect(
      within(slack).getByRole("button", { name: "Open gateway setup" }),
    ).toBeInTheDocument();
    const fibery = screen.getByTestId("litellm-mcp-fibery");
    expect(within(fibery).getByText("Access denied")).toBeInTheDocument();
    expect(within(fibery).getByText("Missing setup")).toBeInTheDocument();
    expect(
      within(fibery).getByRole("button", { name: "Enable" }),
    ).toBeDisabled();
  });

  it("enables only on explicit click and keeps state when the request fails", async () => {
    const posts: string[] = [];
    server.use(
      http.get("*/v1/providers/litellm_a/litellm/mcp", () =>
        HttpResponse.json(mcp),
      ),
      http.post(
        "*/v1/providers/litellm_a/litellm/mcp/:serverId/enable",
        ({ params }) => {
          posts.push(String(params.serverId));
          return HttpResponse.json({ detail: "nope" }, { status: 409 });
        },
      ),
    );
    const { user } = renderSections();
    await user.click(screen.getByRole("radio", { name: "MCP Servers" }));
    const slack = await screen.findByTestId("litellm-mcp-slack");
    expect(posts).toEqual([]);
    await user.click(within(slack).getByRole("button", { name: "Enable" }));
    expect(
      await within(slack).findByText(/previous setting was kept/),
    ).toBeVisible();
    expect(posts).toEqual(["slack"]);
    expect(within(slack).getByText("Disabled")).toBeInTheDocument();
  });

  it("distinguishes a denied credential-status read from runtime tool access", async () => {
    server.use(
      http.get("*/v1/providers/litellm_a/litellm/mcp", () =>
        HttpResponse.json({
          ...mcp,
          servers: [
            {
              ...mcp.servers[0],
              access: "unknown",
              auth: {
                kind: "oauth",
                state: "unknown",
                status_check_state: "permission_denied",
              },
            },
          ],
        }),
      ),
    );
    const { user } = renderSections();
    await user.click(screen.getByRole("radio", { name: "MCP Servers" }));
    const slack = await screen.findByTestId("litellm-mcp-slack");
    expect(
      within(slack).getByText(/Gateway credential status: Permission denied/),
    ).toBeVisible();
    expect(
      within(slack).getByText(
        /does not determine whether tool access is permitted/,
      ),
    ).toBeVisible();
    expect(within(slack).queryByText("Access denied")).not.toBeInTheDocument();
    expect(within(slack).getByRole("button", { name: "Enable" })).toBeEnabled();
    expect(
      within(slack).getByText(/cannot verify that the browser user matches/),
    ).toBeVisible();
    expect(screen.getByText(/externally reachable MCP OAuth/)).toBeVisible();
  });

  it("shows stale inventory, permission denial and unsupported states", async () => {
    server.use(
      http.get("*/v1/providers/litellm_a/litellm/mcp", () =>
        HttpResponse.json({ ...mcp, stale: true }),
      ),
    );
    const { user } = renderSections();
    await user.click(screen.getByRole("radio", { name: "MCP Servers" }));
    expect(await screen.findByText("Inventory stale")).toBeVisible();
  });

  it("explains a forbidden inventory instead of showing an empty list", async () => {
    server.use(
      http.get("*/v1/providers/litellm_a/litellm/mcp", () =>
        HttpResponse.json({ state: "permission_denied", servers: [] }),
      ),
    );
    const { user } = renderSections();
    await user.click(screen.getByRole("radio", { name: "MCP Servers" }));
    expect(
      await screen.findByText(/not permitted to read MCP servers/),
    ).toBeVisible();
    expect(
      screen.queryByText(/No MCP servers are available/),
    ).not.toBeInTheDocument();
  });

  it("triggers rescan explicitly", async () => {
    let rescans = 0;
    server.use(
      http.get("*/v1/providers/litellm_a/litellm/mcp", () =>
        HttpResponse.json(mcp),
      ),
      http.post("*/v1/providers/litellm_a/litellm/mcp/rescan", () => {
        rescans += 1;
        return HttpResponse.json(mcp);
      }),
    );
    const { user } = renderSections();
    await user.click(screen.getByRole("radio", { name: "MCP Servers" }));
    await screen.findByTestId("litellm-mcp-slack");
    await user.click(screen.getByRole("button", { name: "Rescan" }));
    await waitFor(() => expect(rescans).toBe(1));
  });

  it("applies the full inventory returned by enable and offers OAuth sign-in via the existing MCP service", async () => {
    let enabled = false;
    const inventory = () => ({
      ...mcp,
      servers: mcp.servers.map((srv) =>
        srv.server_id === "slack"
          ? {
              ...srv,
              enabled,
              connection: enabled ? "needs_auth" : "disabled",
              integration_path: "/cfg/mcp_slack.yaml",
            }
          : srv,
      ),
    });
    const oauthStarts: unknown[] = [];
    server.use(
      http.get("*/v1/providers/litellm_a/litellm/mcp", () =>
        HttpResponse.json(inventory()),
      ),
      http.post("*/v1/providers/litellm_a/litellm/mcp/slack/enable", () => {
        enabled = true;
        return HttpResponse.json(inventory());
      }),
      http.get("*/v1/mcp/oauth/status", ({ request }) => {
        expect(new URL(request.url).searchParams.get("config_path")).toBe(
          "/cfg/mcp_slack.yaml",
        );
        return HttpResponse.json({
          authenticated: false,
          needs_login: true,
          oauth_available: true,
          auth_type: "oauth2_pkce",
        });
      }),
      http.post("*/v1/mcp/oauth/start", async ({ request }) => {
        oauthStarts.push(await request.json());
        return HttpResponse.json({
          session_id: "s1",
          authorize_url: "https://gateway.example.com/authorize",
        });
      }),
    );
    const openSpy = vi.spyOn(window, "open").mockReturnValue(null);
    const { user } = renderSections();
    await user.click(screen.getByRole("radio", { name: "MCP Servers" }));
    const slack = await screen.findByTestId("litellm-mcp-slack");
    expect(oauthStarts).toEqual([]);
    expect(openSpy).not.toHaveBeenCalled();
    await user.click(within(slack).getByRole("button", { name: "Enable" }));
    expect(await within(slack).findByText("Needs sign-in")).toBeVisible();
    expect(
      within(slack).getByRole("button", { name: "Disable" }),
    ).toBeInTheDocument();
    expect(
      within(slack).getByRole("button", { name: "Approvals & logs" }),
    ).toBeInTheDocument();
    expect(oauthStarts).toEqual([]);
    const login = await within(slack).findByRole("button", {
      name: "Login with OAuth",
    });
    await user.click(login);
    await waitFor(() =>
      expect(oauthStarts).toEqual([{ config_path: "/cfg/mcp_slack.yaml" }]),
    );
    expect(openSpy).toHaveBeenCalledWith(
      "https://gateway.example.com/authorize",
      "_blank",
      "noopener,noreferrer",
    );
    openSpy.mockRestore();
  });

  it("flags identity changes that need reconfirmation", async () => {
    server.use(
      http.get("*/v1/providers/litellm_a/litellm/mcp", () =>
        HttpResponse.json({
          ...mcp,
          reconfirm_required: true,
          servers: [
            {
              ...mcp.servers[0],
              enabled: true,
              connection: "reconfirm_required",
            },
          ],
        }),
      ),
    );
    const { user } = renderSections();
    await user.click(screen.getByRole("radio", { name: "MCP Servers" }));
    expect(await screen.findByText("Reconfirm required")).toBeVisible();
    expect(screen.getByText(/identity changed since/)).toBeVisible();
  });

  it("offers logout for an authenticated server via the existing MCP OAuth service", async () => {
    const logouts: unknown[] = [];
    server.use(
      http.get("*/v1/providers/litellm_a/litellm/mcp", () =>
        HttpResponse.json({
          ...mcp,
          servers: [
            {
              ...mcp.servers[0],
              enabled: true,
              connected: true,
              connection: "connected",
              auth: { kind: "oauth", state: "ok" },
              integration_path: "/cfg/mcp_slack.yaml",
            },
          ],
        }),
      ),
      http.get("*/v1/mcp/oauth/status", () =>
        HttpResponse.json({
          authenticated: true,
          needs_login: false,
          oauth_available: true,
          auth_type: "oauth2_pkce",
        }),
      ),
      http.post("*/v1/mcp/oauth/logout", async ({ request }) => {
        logouts.push(await request.json());
        return HttpResponse.json({ success: true });
      }),
    );
    const { user } = renderSections();
    await user.click(screen.getByRole("radio", { name: "MCP Servers" }));
    const slack = await screen.findByTestId("litellm-mcp-slack");
    expect(await within(slack).findByText("Authenticated")).toBeVisible();
    expect(logouts).toEqual([]);
    await user.click(within(slack).getByRole("button", { name: "Logout" }));
    await waitFor(() =>
      expect(logouts).toEqual([{ config_path: "/cfg/mcp_slack.yaml" }]),
    );
  });
});
