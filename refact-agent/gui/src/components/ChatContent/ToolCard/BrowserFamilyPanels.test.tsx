import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Theme } from "@radix-ui/themes";
import { Provider } from "react-redux";
import { configureStore } from "@reduxjs/toolkit";
import { describe, expect, test } from "vitest";

import { reducer as configReducer } from "../../../features/Config/configSlice";
import type {
  BrowserActionResponse,
  BrowserExecutionStep,
} from "../../../services/refact/browser";
import { collectBrowserFamilies } from "./browserFamilies";
import {
  CdpPanel,
  ClockTimeline,
  DevicePanel,
  HttpRequestPanel,
  NetworkSummaryPanel,
  ReadoutPanel,
  ResetPanel,
  RouteChainPanel,
  WebSocketPanel,
} from "./BrowserFamilyPanels";

function step(
  stepIndex: number,
  data: unknown,
  summary = "Step done",
): BrowserExecutionStep {
  return {
    step_index: stepIndex,
    ok: true,
    summary,
    retries: 0,
    data: data as BrowserExecutionStep["data"],
  };
}

function report(overrides: Partial<BrowserActionResponse>) {
  return { ok: true, steps: [], ...overrides } satisfies BrowserActionResponse;
}

function families(
  overrides: Partial<BrowserActionResponse>,
  requestSteps: Record<string, unknown>[] | null = null,
) {
  return collectBrowserFamilies(report(overrides), requestSteps);
}

function draw(ui: React.ReactNode) {
  const store = configureStore({ reducer: { config: configReducer } });
  return render(
    <Provider store={store}>
      <Theme>{ui}</Theme>
    </Provider>,
  );
}

describe("BrowserFamilyPanels", () => {
  test("renders the V-31 network summary as aligned request rows", () => {
    const collected = families({
      network_summary: [
        "GET https://example.test/api/items 200 1536b 125ms",
        "POST https://example.test/api/save 500 0b 20ms net::ERR_FAILED",
      ],
    });

    draw(<NetworkSummaryPanel rows={collected.networkSummary} />);

    expect(screen.getByText("Network summary (2)")).toBeInTheDocument();
    expect(screen.getByText("GET")).toBeInTheDocument();
    expect(screen.getByText("POST")).toBeInTheDocument();
    expect(screen.getByText("200")).toBeInTheDocument();
    expect(screen.getByText("500")).toBeInTheDocument();
    expect(screen.getByText("1.5 KB")).toBeInTheDocument();
    expect(screen.getByText("125 ms")).toBeInTheDocument();
    expect(screen.getByText("net::ERR_FAILED")).toBeInTheDocument();
    expect(screen.getByTestId("network-summary-row-1")).toHaveAttribute(
      "data-error",
      "true",
    );
  });

  test("keeps an unparseable summary line readable", () => {
    const collected = families({ network_summary: ["totally unexpected"] });

    draw(<NetworkSummaryPanel rows={collected.networkSummary} />);

    expect(screen.getByTitle("totally unexpected")).toBeInTheDocument();
  });

  test("renders the G-3 route chain newest-first with a HAR tail", () => {
    const collected = families({
      steps: [
        step(0, {
          routes: [
            {
              order: 0,
              pattern: "**/api/**",
              handler: { type: "fulfill", status: 201 },
              times_remaining: 2,
            },
            {
              order: 1,
              pattern: { source: "cdn\\.", flags: "i" },
              handler: { type: "abort", reason: "blockedbyclient" },
            },
            {
              order: 2,
              pattern: "**/*",
              handler: { type: "continue" },
              har: { entry_count: 12, not_found: "abort" },
            },
          ],
        }),
      ],
    });

    draw(<RouteChainPanel chains={collected.routeChains} />);

    expect(screen.getAllByTestId("route-chain-row")).toHaveLength(3);
    expect(screen.getByText("**/api/**")).toBeInTheDocument();
    expect(screen.getByText("/cdn\\./i")).toBeInTheDocument();
    expect(screen.getByText("status 201")).toBeInTheDocument();
    expect(screen.getByText("2 left")).toBeInTheDocument();
    expect(screen.getByText("HAR · 12 entries")).toBeInTheDocument();
  });

  test("renders a G-4 http_request with status, cookies and a body preview", async () => {
    const user = userEvent.setup();
    const collected = families({
      steps: [
        step(0, {
          http_request: {
            method: "POST",
            url: "https://api.example.test/v1/login",
            status: 401,
            status_text: "Unauthorized",
            redirects: 1,
            body_bytes: 42,
            set_cookies: { count: 1, names: ["session"] },
            body: '{"error":"denied"}',
          },
        }),
      ],
    });

    draw(<HttpRequestPanel entries={collected.httpRequests} />);

    expect(screen.getByText("POST")).toBeInTheDocument();
    expect(screen.getByText("401 Unauthorized")).toBeInTheDocument();
    expect(screen.getByText("42 B")).toBeInTheDocument();
    expect(screen.getByText("1 redirects")).toBeInTheDocument();
    expect(screen.getByText("Set-Cookie: session")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Response body" }));
    expect(
      screen.getByText((text) => text.includes('"error": "denied"')),
    ).toBeInTheDocument();
  });

  test("renders a spilled http body as an artifact pointer chip", () => {
    const collected = families({
      steps: [
        step(0, {
          http_request: {
            method: "GET",
            url: "https://api.example.test/dump",
            status: 200,
            body_bytes: 2_097_152,
            artifact: {
              path: "/tmp/refact-browser/http-1.json",
              bytes: 2_097_152,
            },
          },
        }),
      ],
    });

    draw(<HttpRequestPanel entries={collected.httpRequests} />);

    expect(
      screen.getByText("/tmp/refact-browser/http-1.json · 2.0 MB"),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Response body" })).toBeNull();
  });

  test("renders G-2 clock steps as compact timeline chips", () => {
    const collected = families(
      {
        steps: [
          step(0, { clock: { installed: true, paused: false } }),
          step(1, { clock: { installed: true, paused: true } }),
        ],
      },
      [
        { action: "fast_forward", ticks_ms: 3_600_000 },
        { action: "pause_at", time_ms: 0 },
      ],
    );

    draw(<ClockTimeline entries={collected.clock} />);

    expect(screen.getAllByTestId("clock-chip")).toHaveLength(2);
    expect(screen.getByText("fast_forward +01:00")).toBeInTheDocument();
    expect(screen.getByText("pause_at 00:00:00 · paused")).toBeInTheDocument();
  });

  test("renders G-5 readouts as bbox chips, counts, values and state dots", () => {
    const collected = families({
      steps: [
        step(
          0,
          { bounding_box: { x: 12, y: 24, width: 100, height: 40 } },
          "Got bounding box of <button>",
        ),
        step(1, { count: 7 }, "Matched 7 element(s)"),
        step(2, { value: "hello" }, "Got input value of <input>"),
        step(
          3,
          { attribute: "href", value: "/docs" },
          "Got attribute 'href' from <a>",
        ),
        step(
          4,
          {
            state: {
              visible: true,
              enabled: false,
              editable: true,
              stable: true,
              checked: "mixed",
            },
          },
          "Read element state of <input>",
        ),
      ],
    });

    draw(<ReadoutPanel entries={collected.readouts} />);

    expect(screen.getByText("x 12")).toBeInTheDocument();
    expect(screen.getByText("h 40")).toBeInTheDocument();
    expect(screen.getByText("7 matched")).toBeInTheDocument();
    expect(screen.getByText("hello")).toBeInTheDocument();
    expect(screen.getByText("href")).toBeInTheDocument();
    expect(screen.getByText("/docs")).toBeInTheDocument();
    expect(screen.getByText("checked: mixed")).toBeInTheDocument();
    expect(screen.getByTestId("readout-state-visible")).toBeInTheDocument();
    expect(screen.getByTestId("readout-state-enabled")).toBeInTheDocument();
    expect(screen.getByTestId("readout-bounding_box")).toBeInTheDocument();
  });

  test("marks a missing bounding box instead of rendering empty chips", () => {
    const collected = families({
      steps: [step(0, { bounding_box: null }, "<span> is not visible")],
    });

    draw(<ReadoutPanel entries={collected.readouts} />);

    expect(screen.getByText("no bounding box")).toBeInTheDocument();
  });

  test("ignores storage state payloads that are not element readouts", () => {
    const collected = families({
      steps: [step(0, { state: { cookies: [], origins: [] } })],
    });

    expect(collected.readouts).toHaveLength(0);
  });

  test("renders the G-9 device catalog as a filterable chip cloud", async () => {
    const user = userEvent.setup();
    const collected = families({
      steps: [
        step(0, {
          devices: ["iPhone 14", "Pixel 7", "iPad Pro 11"],
          aliases: ["mobile", "tablet", "desktop"],
        }),
        step(1, {
          network_conditions: {
            latency_ms: 120,
            download_kbps: 1_500,
            upload_kbps: 750,
          },
          offline: false,
        }),
        step(2, { cpu_throttling_rate: 4 }),
      ],
    });

    draw(
      <DevicePanel
        catalogs={collected.devices}
        emulation={collected.emulation}
      />,
    );

    expect(screen.getAllByTestId("device-chip")).toHaveLength(3);
    expect(
      screen.getByText("120ms · ↓1500 kbps · ↑750 kbps"),
    ).toBeInTheDocument();
    expect(screen.getByText("CPU 4× slower")).toBeInTheDocument();

    await user.type(screen.getByPlaceholderText("Filter devices"), "iP");

    expect(screen.getAllByTestId("device-chip")).toHaveLength(2);
    expect(screen.queryByText("Pixel 7")).toBeNull();

    await user.clear(screen.getByPlaceholderText("Filter devices"));
    await user.type(screen.getByPlaceholderText("Filter devices"), "nothing");

    expect(
      screen.getByText("No devices match this filter."),
    ).toBeInTheDocument();
  });

  test("renders G-11 websocket routes, directions and close reasons", () => {
    const collected = families(
      {
        steps: [
          step(0, { route_count: 1 }, "Added WebSocket route"),
          step(
            1,
            { closed: 2, code: 1001, reason: "going away" },
            "Closed 2 WebSocket(s)",
          ),
        ],
        websockets: [
          {
            sequence: 1,
            socket_id: "ws-1",
            url: "wss://example.test/socket",
            kind: "frame_sent",
            data: "ping",
            routed: true,
          },
          {
            sequence: 2,
            socket_id: "ws-1",
            url: "wss://example.test/socket",
            kind: "frame_received",
            data: "pong",
            routed: true,
          },
          {
            sequence: 3,
            socket_id: "ws-1",
            url: "wss://example.test/socket",
            kind: "error",
            error: "handshake failed",
            routed: false,
          },
        ],
      },
      [
        {
          action: "route_web_socket",
          pattern: "wss://example.test/**",
          mode: "intercept",
        },
        { action: "close_web_socket" },
      ],
    );

    draw(
      <WebSocketPanel
        closes={collected.webSocketCloses}
        events={collected.webSocketEvents}
        routes={collected.webSocketRoutes}
      />,
    );

    expect(screen.getByText("intercept")).toBeInTheDocument();
    expect(screen.getByText("wss://example.test/**")).toBeInTheDocument();
    expect(screen.getByText("sent")).toBeInTheDocument();
    expect(screen.getByText("received")).toBeInTheDocument();
    expect(screen.getByText("ping")).toBeInTheDocument();
    expect(screen.getByText("closed 2")).toBeInTheDocument();
    expect(screen.getByText("code 1001")).toBeInTheDocument();
    expect(screen.getByText("going away")).toBeInTheDocument();
    expect(screen.getAllByTestId("websocket-event")[2]).toHaveAttribute(
      "data-status",
      "error",
    );
  });

  test("renders a G-12 cdp_send call with a collapsible result", async () => {
    const user = userEvent.setup();
    const collected = families({
      steps: [
        step(0, {
          cdp_send: {
            method: "Page.navigate",
            target: "page",
            warnings: ["target is not attached"],
            bytes: 32,
            result: { frameId: "F1" },
          },
        }),
      ],
    });

    draw(<CdpPanel entries={collected.cdp} />);

    expect(screen.getByText("Page.navigate")).toBeInTheDocument();
    expect(screen.getByText("page")).toBeInTheDocument();
    expect(screen.getByText("32 B")).toBeInTheDocument();
    expect(screen.getByText("target is not attached")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Result" }));
    expect(
      screen.getByText((text) => text.includes('"frameId": "F1"')),
    ).toBeInTheDocument();
  });

  test("renders a spilled cdp result as an artifact chip", () => {
    const collected = families({
      steps: [
        step(0, {
          cdp_send: {
            method: "Network.getCookies",
            target: "browser",
            warnings: [],
            bytes: 65_536,
            artifact: {
              kind: "cdp_result",
              mime: "application/json",
              path: "/tmp/refact-browser/cdp-1.json",
              bytes: 65_536,
            },
          },
        }),
      ],
    });

    draw(<CdpPanel entries={collected.cdp} />);

    expect(
      screen.getByText("/tmp/refact-browser/cdp-1.json"),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Result" })).toBeNull();
  });

  test("renders V-34 reset counts and cleared flags", () => {
    const collected = families({
      steps: [
        step(0, {
          reset: {
            routes: 2,
            har_replays: 0,
            websocket_routes: 1,
            locator_handlers: 0,
            authenticators: 0,
            init_scripts: 3,
            offline: false,
            throttling_cleared: true,
            emulation_cleared: true,
            clock_cleared: false,
            service_worker_block_cleared: false,
          },
        }),
      ],
    });

    draw(<ResetPanel entries={collected.resets} />);

    expect(screen.getAllByTestId("reset-chip")).toHaveLength(6);
    expect(screen.getByText("2 routes")).toBeInTheDocument();
    expect(screen.getByText("3 init scripts")).toBeInTheDocument();
    expect(screen.getByText("throttling cleared")).toBeInTheDocument();
    expect(screen.getByText("emulation cleared")).toBeInTheDocument();
    expect(screen.queryByText("clock cleared")).toBeNull();
  });

  test("renders nothing for reports without any family payload", () => {
    const collected = families({
      steps: [step(0, { unknown_family: { nested: true } }, "Did something")],
    });

    const { container } = draw(
      <>
        <NetworkSummaryPanel rows={collected.networkSummary} />
        <RouteChainPanel chains={collected.routeChains} />
        <HttpRequestPanel entries={collected.httpRequests} />
        <ClockTimeline entries={collected.clock} />
        <ReadoutPanel entries={collected.readouts} />
        <DevicePanel
          catalogs={collected.devices}
          emulation={collected.emulation}
        />
        <WebSocketPanel
          closes={collected.webSocketCloses}
          events={collected.webSocketEvents}
          routes={collected.webSocketRoutes}
        />
        <CdpPanel entries={collected.cdp} />
        <ResetPanel entries={collected.resets} />
      </>,
    );

    expect(container.querySelectorAll("[data-testid]")).toHaveLength(0);
  });
});
