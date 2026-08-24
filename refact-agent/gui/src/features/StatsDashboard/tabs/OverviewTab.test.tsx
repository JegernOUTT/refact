import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";

import type { ProviderQuotaSnapshot } from "../../../services/refact/providers";
import { server } from "../../../utils/mockServer";
import { render, screen } from "../../../utils/test-utils";
import type { StatsSummary } from "../types";
import { OverviewTab } from "./OverviewTab";

const config = {
  apiKey: "test",
  host: "vscode" as const,
  lspPort: 8001,
  themeProps: {},
};

const emptySummary: StatsSummary = {
  date_range: { from: "2026-07-01", to: "2026-07-07" },
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
};

function quota(
  providerName: string,
  overrides: Partial<ProviderQuotaSnapshot> = {},
): ProviderQuotaSnapshot {
  return {
    provider_name: providerName,
    base_provider: providerName,
    source: "account",
    available: true,
    fetched_at: "2026-07-07T10:00:00Z",
    stale: false,
    windows: [],
    facts: [],
    ...overrides,
  };
}

function renderOverview(quotas: ProviderQuotaSnapshot[]) {
  server.use(
    http.get("*/v1/stats/llm/summary", () => HttpResponse.json(emptySummary)),
    http.get("*/v1/providers/quotas", () => HttpResponse.json({ quotas })),
  );

  return render(<OverviewTab dateRange={{ preset: "7d" }} />, {
    preloadedState: { config },
  });
}

describe("OverviewTab provider quotas", () => {
  it("omits unavailable snapshots while preserving available quota details", async () => {
    renderOverview([
      quota("unavailable-provider", {
        available: false,
        error: "Credentials missing",
      }),
      quota("available-provider", {
        base_provider: "available-base-provider",
        error: "Temporary quota warning",
        windows: [
          {
            id: "daily",
            label: "Daily limit",
            used_percent: 25,
            used: 25,
            limit: 100,
          },
        ],
        facts: [{ id: "tier", label: "Tier", value: "Pro" }],
      }),
    ]);

    expect(await screen.findByText("available-provider")).toBeInTheDocument();
    expect(screen.getByText("available-base-provider")).toBeInTheDocument();
    expect(screen.getByText("Temporary quota warning")).toBeInTheDocument();
    expect(screen.getByText("Daily limit")).toBeInTheDocument();
    expect(screen.getByText("Tier")).toBeInTheDocument();
    expect(screen.queryByText("unavailable-provider")).not.toBeInTheDocument();
    expect(screen.queryByText("Credentials missing")).not.toBeInTheDocument();
  });

  it("shows the existing empty state when every snapshot is unavailable", async () => {
    renderOverview([
      quota("unavailable-provider", {
        available: false,
        error: "Credentials missing",
      }),
    ]);

    expect(
      await screen.findByText("No provider quota snapshots reported."),
    ).toBeInTheDocument();
    expect(screen.queryByText("unavailable-provider")).not.toBeInTheDocument();
    expect(screen.queryByText("Unavailable")).not.toBeInTheDocument();
  });
});
