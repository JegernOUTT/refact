import { expect, test, type Page } from "@playwright/test";

const account = {
  state: "ok",
  fetched_at: "2026-01-01T00:00:00Z",
  stale: false,
  subjects: [
    {
      scope: "key",
      label: "ci-key-with-a-very-long-label-that-must-wrap-inside-the-card",
      state: "ok",
      spend: { value: 0.00001234, state: "known" },
      max_budget: { kind: "limited", value: 10 },
      remaining: { value: 9.99998766, derived: true, overage: false },
      currency: "USD",
      budget_reset_at: "2026-02-01T00:00:00Z",
      rpm_limit: null,
      model_limits: [{ model: "gpt-x", rpm_limit: 0 }],
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
  observed_headroom: {
    observed_at: "2026-01-01T00:00:00Z",
    scope: "unknown",
    model: "gpt-x",
    requests_remaining: 41,
    requests_limit: 60,
  },
};

const usage = {
  state: "ok",
  fetched_at: "2026-01-02T00:00:00Z",
  scope: "key",
  page: 1,
  page_size: 25,
  total_pages: 2,
  has_more: true,
  truncated: true,
  totals: { spend: 0.0042, requests: 10, total_tokens: 150 },
  daily: [
    { date: "2026-01-01", spend: 0.0042, requests: 10, total_tokens: 150 },
  ],
  models: [{ model: "gpt-x", spend: 0.0042, requests: 10, total_tokens: 150 }],
  api_keys: [{ label: "this", alias: "ci", current: true, spend: 0.0042 }],
};

const mcp = {
  state: "ok",
  fetched_at: "2026-01-01T00:00:00Z",
  stale: true,
  reconfirm_required: false,
  servers: [
    {
      server_id: "slack",
      display_name: "Slack",
      auth: { kind: "oauth", state: "auth_required" },
      health: "unknown",
      access: "allowed",
      enabled: false,
      connection: "disabled",
    },
    {
      server_id: "fibery",
      display_name: "Fibery",
      auth: { kind: "byok", state: "missing_setup" },
      health: "healthy",
      access: "denied",
      enabled: false,
      connection: "disabled",
    },
  ],
};

async function mockApi(page: Page, mutations: string[]) {
  await page.route("**/v1/**", async (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname;
    if (request.method() === "POST") mutations.push(path);
    let body: unknown = {};
    if (path === "/v1/providers") body = { providers: [] };
    if (path === "/v1/providers/litellm_team")
      body = {
        name: "litellm_team",
        base_provider: "litellm",
        display_name: "Team LiteLLM",
        enabled: true,
        readonly: false,
        status: "active",
        has_credentials: true,
        settings: { endpoint: "https://gateway.example.test/v1" },
        runtime: null,
      };
    if (path.endsWith("/schema"))
      body = {
        name: "litellm_team",
        schema: "description: LiteLLM gateway\nfields: {}",
      };
    if (path.endsWith("/available-models"))
      body = { models: [], source: "manual" };
    if (path.endsWith("/models")) body = { models: [] };
    if (path.endsWith("/litellm/account")) body = account;
    if (path.endsWith("/litellm/usage")) body = usage;
    if (path.endsWith("/litellm/mcp")) body = mcp;
    await route.fulfill({ json: body });
  });
}

for (const width of [360, 1280]) {
  test(`LiteLLM sections stay contained and keyboard operable at ${width}px`, async ({
    page,
  }) => {
    const mutations: string[] = [];
    await page.setViewportSize({ width, height: 900 });
    await mockApi(page, mutations);
    await page.goto("/tests/e2e/litellm-provider.html");

    const sections = page.getByRole("radiogroup", {
      name: "LiteLLM provider sections",
    });
    await expect(sections).toBeVisible();

    await sections.getByText("Quotas & Usage").click();
    await expect(page.getByTestId("litellm-subject-key")).toBeVisible();
    await expect(page.getByTestId("litellm-subject-team")).toContainText(
      "Permission denied",
    );
    await expect(page.getByTestId("litellm-observed-headroom")).toBeVisible();
    await expect(page.getByText(/Breakdowns may be incomplete/)).toBeVisible();

    await sections.getByText("MCP Servers").click();
    await expect(page.getByTestId("litellm-mcp-slack")).toContainText(
      "Auth required",
    );
    await expect(page.getByTestId("litellm-mcp-fibery")).toContainText(
      "Access denied",
    );
    await expect(page.getByText("Inventory stale")).toBeVisible();

    const overflow = await page.evaluate(
      () =>
        document.documentElement.scrollWidth -
        document.documentElement.clientWidth,
    );
    expect(overflow).toBeLessThanOrEqual(0);

    const enable = page
      .getByTestId("litellm-mcp-slack")
      .getByRole("button", { name: "Enable" });
    await enable.focus();
    await expect(enable).toBeFocused();
    expect(mutations).toEqual([]);
  });
}
