import { afterEach, describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { render, screen } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { MetricsPanel } from "./MetricsPanel";
import type { MetricsPage } from "../../../services/refact/metricsTypes";

afterEach(() => vi.restoreAllMocks());

const renderPanel = () =>
  render(<MetricsPanel />, {
    preloadedState: {
      config: {
        host: "ide",
        lspPort: 8001,
        themeProps: { appearance: "dark" },
      },
    },
  });

function metricsHandler(page: Record<string, unknown> | MetricsPage) {
  server.use(http.get("*/v1/metrics", () => HttpResponse.json(page)));
}

const page = (overrides: Partial<MetricsPage> = {}): MetricsPage => ({
  snapshot_id: "metrics-1",
  seq: 1,
  project: "project",
  entries: [],
  total: 0,
  series_total: 0,
  explanations_total: 0,
  returned: 0,
  truncated: false,
  generated_at_ms: 2_000,
  ...overrides,
});

const series = {
  kind: "series" as const,
  series: {
    name: "evidence_verdicts",
    labels: [{ name: "verdict", value: "failed" }],
    sample: { kind: "counter" as const, value: 2 },
  },
};

const explanation = {
  kind: "explanation" as const,
  explanation: {
    decision: "denial" as const,
    outcome: "denied",
    rule: "policy.no_secret",
    inputs: [["reason", "redacted:123abc"]] as [string, string][],
    seq: 1,
    at_ms: 1_000,
  },
};

describe("Metrics panel", () => {
  it("renders an explicit unavailable state rather than an empty success", async () => {
    server.use(
      http.get("*/v1/metrics", () =>
        HttpResponse.json({ code: "unauthorized" }, { status: 401 }),
      ),
    );
    renderPanel();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Metrics unavailable",
    );
    expect(screen.queryByLabelText("Metrics entries")).toBeNull();
  });

  it("renders the zero state when nothing has been recorded", async () => {
    metricsHandler(page());
    renderPanel();
    expect(
      await screen.findByText("No metrics recorded yet"),
    ).toBeInTheDocument();
  });

  it("renders one distinct verdict series", async () => {
    metricsHandler(
      page({
        entries: [series],
        total: 1,
        series_total: 1,
        returned: 1,
      }),
    );
    renderPanel();
    expect(
      await screen.findByText("evidence_verdicts{verdict=failed}"),
    ).toBeInTheDocument();
    expect(screen.getByText("2")).toBeInTheDocument();
  });

  it("renders many series without collapsing verdict names", async () => {
    const blocked = {
      ...series,
      series: {
        ...series.series,
        labels: [{ name: "verdict", value: "blocked" }],
        sample: { kind: "counter" as const, value: 1 },
      },
    };
    metricsHandler(
      page({
        entries: [series, blocked, explanation],
        total: 3,
        series_total: 2,
        explanations_total: 1,
        returned: 3,
      }),
    );
    renderPanel();
    expect(
      await screen.findByText("evidence_verdicts{verdict=failed}"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("evidence_verdicts{verdict=blocked}"),
    ).toBeInTheDocument();
    expect(screen.getByText("Rule: policy.no_secret")).toBeInTheDocument();
    expect(
      screen.getByLabelText("Inputs for policy.no_secret"),
    ).toHaveTextContent("reasonredacted:123abc");
  });

  it("treats an unknown snapshot shape as unavailable", async () => {
    metricsHandler({ ...page(), entries: [{ kind: "mystery" }] });
    renderPanel();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Metrics unavailable",
    );
  });
});
