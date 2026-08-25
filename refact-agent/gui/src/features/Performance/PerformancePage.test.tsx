import { afterEach, describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw";

import type {
  PerformanceAggregate,
  PerformanceTelemetryResponse,
} from "../../services/refact/performance";
import { server } from "../../utils/mockServer";
import { render, screen, waitFor } from "../../utils/test-utils";
import { formatBytes, formatDurationUs } from "./performanceFormatters";
import {
  PerformancePage,
  PERFORMANCE_POLLING_INTERVAL_MS,
} from "./PerformancePage";

const configState = {
  config: {
    apiKey: null,
    host: "web" as const,
    lspPort: 8001,
    themeProps: { appearance: "dark" as const },
  },
};

function aggregate(overrides: Partial<PerformanceAggregate> = {}) {
  return {
    component: "tool.runtime",
    sample_count: 7,
    success_count: 6,
    failure_count: 1,
    skipped_count: 0,
    min_us: 200,
    max_us: 2_000_000,
    p50_us: 1_500,
    p95_us: 2_000_000,
    p99_us: 900,
    last_sample_at_ms: 1_700_000_000_000,
    size_bytes_sum: 1_572_864,
    item_count_sum: 4,
    batch_size_sum: 2,
    ...overrides,
  };
}

function telemetry(
  enabled: boolean,
  components: PerformanceAggregate[] = [aggregate()],
): PerformanceTelemetryResponse {
  const rollup = aggregate({ component: null });
  return {
    schema_version: 1,
    enabled,
    collection_started_at_ms: 1_700_000_000_000,
    uptime_ms: 3_900_000,
    components,
    rollups: {
      advancement: rollup,
      tool_stages: rollup,
      enrichment_stages: rollup,
      index_watcher_vecdb_amplification: {
        aggregate: rollup,
        trajectory_index_operations_per_commit: 1.25,
        watcher_rebuilds_per_commit: 0.5,
        vecdb_searches_per_enrichment_attempt: 0.75,
      },
    },
    rollout_switches: {
      trajectory_writer_enabled: true,
      vecdb_path_coalescing_enabled: false,
    },
  };
}

function renderPage() {
  return render(<PerformancePage onBack={() => undefined} />, {
    preloadedState: configState,
  });
}

afterEach(() => {
  vi.useRealTimers();
  delete (document as { visibilityState?: DocumentVisibilityState })
    .visibilityState;
});

describe("PerformancePage", () => {
  it("renders aggregate groups and readable percentile and unit values", async () => {
    server.use(
      http.get("*/v1/performance/telemetry", () =>
        HttpResponse.json(telemetry(true)),
      ),
    );

    renderPage();

    expect(
      (await screen.findAllByText("Tool · Runtime")).length,
    ).toBeGreaterThan(0);
    expect(screen.getAllByText("1.5 ms").length).toBeGreaterThan(0);
    expect(screen.getAllByText("2 s").length).toBeGreaterThan(0);
    expect(
      screen.getAllByText("1.5 MB · 4 items · 2 batch items").length,
    ).toBeGreaterThan(0);
    expect(screen.getByText("Chat advancement")).toBeInTheDocument();
    expect(
      screen.getByText("Trajectory persistence / index"),
    ).toBeInTheDocument();
    expect(screen.getByText("Watcher / VecDB")).toBeInTheDocument();
    expect(screen.getByText("Automatic enrichment")).toBeInTheDocument();
    expect(screen.getByText("Trajectory writer enabled")).toBeInTheDocument();
  });

  it("enables disabled collection and refetches telemetry", async () => {
    let enabled = false;
    let updatePayload: unknown = null;
    server.use(
      http.get("*/v1/performance/telemetry", () =>
        HttpResponse.json(telemetry(enabled, [])),
      ),
      http.post("*/v1/performance/telemetry", async ({ request }) => {
        updatePayload = await request.json();
        enabled = true;
        return HttpResponse.json({ schema_version: 1, enabled });
      }),
    );
    const { user } = renderPage();

    expect(
      await screen.findByText("Telemetry collection is disabled"),
    ).toBeInTheDocument();
    await user.click(
      screen.getAllByRole("button", { name: "Enable collection" })[0],
    );

    await waitFor(() => expect(updatePayload).toEqual({ enabled: true }));
    expect(
      await screen.findByText("No telemetry samples yet"),
    ).toBeInTheDocument();
    expect(screen.getByText("Enabled")).toBeInTheDocument();
  });

  it("resets telemetry and clears the aggregate view", async () => {
    let components: PerformanceAggregate[] = [aggregate()];
    let resetRequests = 0;
    server.use(
      http.get("*/v1/performance/telemetry", () =>
        HttpResponse.json(telemetry(true, components)),
      ),
      http.post("*/v1/performance/telemetry/reset", () => {
        resetRequests += 1;
        components = [];
        return HttpResponse.json({
          schema_version: 1,
          reset: true,
          enabled: true,
        });
      }),
    );
    const { user } = renderPage();

    expect(
      (await screen.findAllByText("Tool · Runtime")).length,
    ).toBeGreaterThan(0);
    await user.click(screen.getByRole("button", { name: "Reset" }));

    await waitFor(() => expect(resetRequests).toBe(1));
    expect(
      await screen.findByText("No telemetry samples yet"),
    ).toBeInTheDocument();
  });

  it("renders explicit empty and error states", async () => {
    server.use(
      http.get("*/v1/performance/telemetry", () =>
        HttpResponse.json(telemetry(true, [])),
      ),
    );
    const view = renderPage();

    expect(
      await screen.findByText("No telemetry samples yet"),
    ).toBeInTheDocument();
    view.unmount();

    server.use(
      http.get("*/v1/performance/telemetry", () =>
        HttpResponse.json({ detail: "Unavailable" }, { status: 503 }),
      ),
    );
    renderPage();

    expect(
      await screen.findByText("Performance telemetry unavailable"),
    ).toBeInTheDocument();
  });

  it("does not poll while the tab is hidden", async () => {
    let requests = 0;
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      value: "hidden",
    });
    server.use(
      http.get("*/v1/performance/telemetry", () => {
        requests += 1;
        return HttpResponse.json(telemetry(false, []));
      }),
    );
    renderPage();

    expect(
      await screen.findByText("Telemetry collection is disabled"),
    ).toBeInTheDocument();
    const initialRequests = requests;
    vi.useFakeTimers();
    vi.advanceTimersByTime(PERFORMANCE_POLLING_INTERVAL_MS * 3);

    expect(requests).toBe(initialRequests);
  });
});

describe("performance formatters", () => {
  it("formats duration and byte values with readable units", () => {
    expect(formatDurationUs(900)).toBe("900 µs");
    expect(formatDurationUs(1_500)).toBe("1.5 ms");
    expect(formatDurationUs(2_000_000)).toBe("2 s");
    expect(formatBytes(1_572_864)).toBe("1.5 MB");
  });
});
