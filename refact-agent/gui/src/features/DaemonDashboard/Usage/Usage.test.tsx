import { http, HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";

import { setUpStore } from "../../../app/store";
import type { DaemonWorker } from "../../../services/refact/daemon";
import type { ProviderListItem } from "../../../services/refact/providers";
import { server } from "../../../utils/mockServer";
import { render, screen, waitFor, within } from "../../../utils/test-utils";
import type { StatsSummary } from "../../StatsDashboard/types";
import { formatCostTick } from "./costTicks";
import { UsagePage } from "./UsagePage";

vi.mock("echarts-for-react/lib/core", () => ({
  default: ({ className }: { className?: string }) => (
    <div className={className} data-testid="echarts-mock" />
  ),
}));

const config = {
  apiKey: "",
  host: "web" as const,
  lspPort: 8488,
  lspUrl: "https://daemon.example.test",
  surface: "dashboard" as const,
  themeProps: {},
};

const BASE = "https://daemon.example.test";

function worker(
  projectId: string,
  state: string,
  extra: Partial<DaemonWorker> = {},
): DaemonWorker {
  return {
    project_id: projectId,
    slug: projectId,
    root: `/work/${projectId}`,
    pinned: false,
    last_active_ms: 1,
    state,
    pid: state === "stopped" ? null : 10,
    http_port: state === "stopped" ? null : 8001,
    lsp_port: state === "stopped" ? null : 9001,
    lsp_clients: 0,
    busy_chats: 0,
    exec_running: 0,
    live_proxy_streams: 0,
    cron_next_fire_ms: null,
    idle_deadline_ms: null,
    last_status_report_ms: 1,
    last_error: null,
    ...extra,
  };
}

function projectSummary(overrides: Partial<StatsSummary> = {}): StatsSummary {
  return {
    date_range: { from: "2026-07-01", to: "2026-07-18" },
    totals: {
      total_calls: 0,
      successful_calls: 0,
      failed_calls: 0,
      total_prompt_tokens: 0,
      total_completion_tokens: 0,
      total_tokens: 0,
      total_cache_read_tokens: 0,
      total_cache_creation_tokens: 0,
      total_cost_usd: null,
      total_duration_ms: 0,
      avg_duration_ms: 0,
      total_conversations: 0,
      total_messages_sent: 0,
    },
    by_model: [],
    by_provider: [],
    by_day: [],
    by_mode: [],
    top_conversations: [],
    ...overrides,
  };
}

function modelRow(calls: number, tokens: number, cost: number) {
  return {
    model_id: "anthropic/claude",
    model: "claude",
    provider: "anthropic",
    total_calls: calls,
    successful_calls: calls,
    failed_calls: 0,
    total_prompt_tokens: tokens,
    total_completion_tokens: 0,
    total_tokens: tokens,
    total_cache_read_tokens: 0,
    total_cache_creation_tokens: 0,
    total_cost_usd: cost,
    total_duration_ms: 1_000 * calls,
    avg_duration_ms: 1_000,
  };
}

function providerRow(calls: number, provider = "anthropic") {
  return {
    provider,
    total_calls: calls,
    successful_calls: calls,
    failed_calls: 0,
    total_prompt_tokens: 0,
    total_completion_tokens: 0,
    total_tokens: 0,
    total_cache_read_tokens: 0,
    total_cache_creation_tokens: 0,
    total_cost_usd: 0,
    total_duration_ms: 0,
  };
}

function providerListItem(name: string, baseProvider = name): ProviderListItem {
  return {
    name,
    base_provider: baseProvider,
    display_name: name,
    enabled: true,
    readonly: false,
    has_credentials: true,
    status: "configured",
    model_count: 1,
  };
}

function dayRow(date: string, tokens: number, cost: number) {
  return {
    date,
    total_calls: 1,
    successful_calls: 1,
    total_prompt_tokens: tokens,
    total_completion_tokens: 0,
    total_tokens: tokens,
    total_cache_read_tokens: 0,
    total_cache_creation_tokens: 0,
    total_cost_usd: cost,
    total_duration_ms: 1_000,
  };
}

const alphaSummary = projectSummary({
  totals: {
    total_calls: 10,
    successful_calls: 9,
    failed_calls: 1,
    total_prompt_tokens: 700,
    total_completion_tokens: 300,
    total_tokens: 1_000,
    total_cache_read_tokens: 0,
    total_cache_creation_tokens: 0,
    total_cost_usd: 1.25,
    total_duration_ms: 10_000,
    avg_duration_ms: 1_000,
    total_conversations: 2,
    total_messages_sent: 5,
  },
  by_model: [modelRow(10, 1_000, 1.25)],
  by_provider: [providerRow(10, "openrouter")],
  by_day: [dayRow("2026-07-01", 1_000, 1.25)],
});

const betaSummary = projectSummary({
  totals: {
    total_calls: 5,
    successful_calls: 5,
    failed_calls: 0,
    total_prompt_tokens: 300,
    total_completion_tokens: 200,
    total_tokens: 500,
    total_cache_read_tokens: 0,
    total_cache_creation_tokens: 0,
    total_cost_usd: 0.75,
    total_duration_ms: 5_000,
    avg_duration_ms: 1_000,
    total_conversations: 1,
    total_messages_sent: 3,
  },
  by_model: [modelRow(5, 500, 0.75)],
  by_provider: [providerRow(5, "openrouter")],
  by_day: [dayRow("2026-07-02", 500, 0.75)],
});

function renderUsage() {
  const store = setUpStore({ config });
  const view = render(<UsagePage />, { store });
  return { store, view };
}

describe("UsagePage", () => {
  it("aggregates two projects so totals match the per-project sum", async () => {
    server.use(
      http.get(`${BASE}/daemon/v1/workers`, () =>
        HttpResponse.json([worker("alpha", "ready"), worker("beta", "ready")]),
      ),
      http.get(`${BASE}/p/alpha/v1/stats/llm/summary`, () =>
        HttpResponse.json(alphaSummary),
      ),
      http.get(`${BASE}/p/beta/v1/stats/llm/summary`, () =>
        HttpResponse.json(betaSummary),
      ),
      http.get(`${BASE}/p/alpha/v1/providers`, () =>
        HttpResponse.json({ providers: [providerListItem("openrouter")] }),
      ),
      http.get(`${BASE}/p/alpha/v1/providers/openrouter/account-info`, () =>
        HttpResponse.json({ data: { limit: 100, usage: 95, remaining: 5 } }),
      ),
    );

    renderUsage();

    expect((await screen.findAllByText("1.5K")).length).toBeGreaterThan(0);
    expect(screen.getAllByText("$2.00").length).toBeGreaterThan(0);
    expect(screen.getByText("93%")).toBeInTheDocument();

    const projectTable = screen.getByLabelText("Usage by project");
    expect(within(projectTable).getByText("alpha")).toBeInTheDocument();
    expect(within(projectTable).getByText("beta")).toBeInTheDocument();
    expect(within(projectTable).getByText("$1.25")).toBeInTheDocument();
    expect(within(projectTable).getByText("$0.750")).toBeInTheDocument();

    const modelTable = screen.getByLabelText("Usage by model");
    expect(within(modelTable).getAllByText("claude")).toHaveLength(1);
    expect(within(modelTable).getByText("15")).toBeInTheDocument();

    expect(
      await screen.findByText("openrouter: $5.00 of $100.00 plan remaining"),
    ).toBeInTheDocument();
    expect(screen.getAllByTestId("echarts-mock").length).toBeGreaterThan(0);
  });

  it("skips account-info probes for providers without advertised support", async () => {
    let providersListRequested = false;
    let accountInfoProbed = false;
    server.use(
      http.get(`${BASE}/daemon/v1/workers`, () =>
        HttpResponse.json([worker("alpha", "ready")]),
      ),
      http.get(`${BASE}/p/alpha/v1/stats/llm/summary`, () =>
        HttpResponse.json(alphaSummary),
      ),
      http.get(`${BASE}/p/alpha/v1/providers`, () => {
        providersListRequested = true;
        return HttpResponse.json({
          providers: [
            providerListItem("anthropic"),
            providerListItem("openrouter"),
          ],
        });
      }),
      http.get(`${BASE}/p/alpha/v1/providers/openrouter/account-info`, () =>
        HttpResponse.json({ data: { remaining: 50, limit: 100 } }),
      ),
      http.get(`${BASE}/p/alpha/v1/providers/anthropic/account-info`, () => {
        accountInfoProbed = true;
        return new HttpResponse(null, { status: 400 });
      }),
    );

    renderUsage();

    expect((await screen.findAllByText("1.0K")).length).toBeGreaterThan(0);
    await waitFor(() => expect(providersListRequested).toBe(true));
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(accountInfoProbed).toBe(false);
    expect(screen.queryByText(/token plan/)).toBeNull();
  });

  it("lists stopped workers as not counted and wakes them on demand", async () => {
    let restarted = false;
    server.use(
      http.get(`${BASE}/daemon/v1/workers`, () =>
        HttpResponse.json([
          worker("alpha", "ready"),
          worker("gamma", restarted ? "ready" : "stopped"),
        ]),
      ),
      http.get(`${BASE}/p/alpha/v1/stats/llm/summary`, () =>
        HttpResponse.json(projectSummary()),
      ),
      http.get(`${BASE}/p/gamma/v1/stats/llm/summary`, () =>
        HttpResponse.json(projectSummary()),
      ),
      http.post(`${BASE}/daemon/v1/projects/gamma/restart`, () => {
        restarted = true;
        return HttpResponse.json({
          project_id: "gamma",
          pid: 42,
          http_port: 8002,
          lsp_port: 9002,
          state: "ready",
        });
      }),
    );

    const { view } = renderUsage();

    expect(
      await screen.findByText("not counted (worker stopped)"),
    ).toBeInTheDocument();
    expect(screen.getByText("gamma")).toBeInTheDocument();

    await view.user.click(screen.getByRole("button", { name: "Wake" }));

    await waitFor(() => expect(restarted).toBe(true));
    await waitFor(() =>
      expect(
        screen.queryByText("not counted (worker stopped)"),
      ).not.toBeInTheDocument(),
    );
  });

  it("shows an empty state when no project recorded LLM calls", async () => {
    server.use(
      http.get(`${BASE}/daemon/v1/workers`, () =>
        HttpResponse.json([worker("alpha", "ready")]),
      ),
      http.get(`${BASE}/p/alpha/v1/stats/llm/summary`, () =>
        HttpResponse.json(projectSummary()),
      ),
    );

    renderUsage();

    expect(
      await screen.findByText("No LLM calls recorded yet"),
    ).toBeInTheDocument();
  });
});

describe("formatCostTick", () => {
  it("uses 2-3 decimals for sub-dollar ranges", () => {
    expect(formatCostTick(0.05, 0.195)).toBe("$0.05");
    expect(formatCostTick(0.1, 0.195)).toBe("$0.10");
    expect(formatCostTick(0.15, 0.195)).toBe("$0.15");
    expect(formatCostTick(0.125, 0.5)).toBe("$0.125");
    expect(formatCostTick(0, 0.195)).toBe("$0.00");
  });

  it("uses 2 decimals below ten dollars", () => {
    expect(formatCostTick(2.5, 9)).toBe("$2.50");
    expect(formatCostTick(1, 9.99)).toBe("$1.00");
  });

  it("uses integers for ten dollars and above", () => {
    expect(formatCostTick(4, 12)).toBe("$4");
    expect(formatCostTick(150, 600)).toBe("$150");
  });
});
