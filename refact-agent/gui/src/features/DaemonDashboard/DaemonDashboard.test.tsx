import { http, HttpResponse } from "msw";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { setUpStore } from "../../app/store";
import { server } from "../../utils/mockServer";
import { render, screen } from "../../utils/test-utils";
import { InnerApp } from "../App";
import {
  daemonEventsReceived,
  dashboardSlice,
  navigateDashboard,
  selectDashboardNavigation,
} from "./dashboardSlice";

class QuietEventSource {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSED = 2;
  readonly CONNECTING = 0;
  readonly OPEN = 1;
  readonly CLOSED = 2;
  onerror: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onopen: ((event: Event) => void) | null = null;
  readyState = QuietEventSource.CONNECTING;
  url: string;

  constructor(url: string | URL) {
    this.url = String(url);
  }

  addEventListener() {
    return undefined;
  }

  removeEventListener() {
    return undefined;
  }

  close() {
    this.readyState = QuietEventSource.CLOSED;
  }

  dispatchEvent() {
    return true;
  }
}

const dashboardConfig = {
  apiKey: "",
  host: "web" as const,
  lspPort: 8488,
  lspUrl: "http://127.0.0.1:8488",
  surface: "dashboard" as const,
  themeProps: {},
};

describe("dashboardSlice", () => {
  it("navigates with page parameters", () => {
    const store = setUpStore();

    store.dispatch(
      navigateDashboard({ page: "projects", params: { project: "p1" } }),
    );

    expect(selectDashboardNavigation(store.getState())).toEqual({
      page: "projects",
      params: { project: "p1" },
    });
  });

  it("deduplicates daemon events and caps the ring at 1000", () => {
    const events = Array.from({ length: 1_005 }, (_, index) => ({
      seq: index + 1,
      ts_ms: index,
      kind: "worker_status",
      project_id: null,
      payload: {},
    }));
    const state = dashboardSlice.reducer(
      undefined,
      daemonEventsReceived([...events, { ...events[1_004], kind: "latest" }]),
    );

    expect(state.events).toHaveLength(1_000);
    expect(state.events[0].seq).toBe(6);
    expect(state.events.at(-1)?.kind).toBe("latest");
  });
});

describe("App dashboard surface", () => {
  beforeEach(() => {
    vi.stubGlobal("EventSource", QuietEventSource);
    server.use(
      http.get("*/v1/ping", () => HttpResponse.text("pong")),
      http.get(
        "*/v1/sidebar/subscribe",
        () =>
          new HttpResponse(
            new ReadableStream<Uint8Array>({
              start() {
                return undefined;
              },
            }),
            { headers: { "Content-Type": "text/event-stream" } },
          ),
      ),
      http.get("*/daemon/v1/status", () =>
        HttpResponse.json({
          pid: 10,
          version: "9.1.0",
          port: 8488,
          started_at_ms: 1,
          uptime_secs: 3_700,
          workers: 2,
          cron_pending: {},
        }),
      ),
      http.get("*/daemon/v1/workers", () => HttpResponse.json([])),
      http.get(
        "*/daemon/v1/events",
        () =>
          new HttpResponse("", {
            headers: { "Content-Type": "text/event-stream" },
          }),
      ),
    );
  });

  it("renders the dashboard shell and all navigation destinations", async () => {
    const store = setUpStore({ config: dashboardConfig });
    const view = render(<InnerApp />, { store });

    expect(screen.getByTestId("daemon-dashboard-shell")).toBeInTheDocument();
    expect(await screen.findByText("Live")).toBeInTheDocument();
    expect(
      screen.getByText(/v9\.1\.0 · 1h 1m · 2 workers/),
    ).toBeInTheDocument();
    for (const label of [
      "Home",
      "Projects",
      "Activity",
      "Scheduler",
      "Usage",
      "Doctor",
      "Settings",
    ]) {
      expect(screen.getByRole("button", { name: label })).toBeInTheDocument();
    }

    await view.user.click(screen.getByRole("button", { name: "Doctor" }));
    expect(selectDashboardNavigation(store.getState()).page).toBe("doctor");
    expect(screen.getByRole("heading", { name: "Doctor" })).toBeInTheDocument();
  });

  it.each([
    ["workspace web", { ...dashboardConfig, surface: "workspace" as const }],
    [
      "IDE host",
      {
        ...dashboardConfig,
        host: "vscode" as const,
        surface: undefined,
      },
    ],
  ])("keeps dashboard chrome out of the %s surface", (_name, config) => {
    const store = setUpStore({ config });

    expect(store.getState().config.surface).not.toBe("dashboard");
    render(<InnerApp />, { store });

    expect(screen.queryByTestId("daemon-dashboard-shell")).toBeNull();
  });
});
