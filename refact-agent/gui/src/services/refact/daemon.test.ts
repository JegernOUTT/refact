import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";

import { setUpStore } from "../../app/store";
import { server } from "../../utils/mockServer";
import { daemonApi, resolveDaemonBaseUrl } from "./daemon";

const status = {
  pid: 123,
  version: "1.0.0",
  port: 8488,
  started_at_ms: 1_700_000_000_000,
  uptime_secs: 120,
  workers: 1,
  cron_pending: 0,
};

const workers = [
  {
    project_id: "project-1",
    slug: "refact",
    root: "/work/refact",
    pinned: true,
    last_active_ms: 1_700_000_001_000,
    state: "running",
    pid: 456,
    http_port: 8001,
    lsp_port: 8002,
    lsp_clients: 2,
    busy_chats: 1,
    exec_running: 3,
    live_proxy_streams: 4,
    cron_next_fire_ms: null,
    idle_deadline_ms: null,
    last_status_report_ms: 1_700_000_002_000,
    last_error: null,
  },
];

describe("daemonApi", () => {
  it("derives daemon root from lspUrl origin and sends apiKey", async () => {
    const requested: { url: string; authorization: string | null }[] = [];
    server.use(
      http.get(
        "https://daemon.example.test/daemon/v1/status",
        ({ request }) => {
          requested.push({
            url: request.url,
            authorization: request.headers.get("Authorization"),
          });
          return HttpResponse.json(status);
        },
      ),
      http.get(
        "https://daemon.example.test/daemon/v1/workers",
        ({ request }) => {
          requested.push({
            url: request.url,
            authorization: request.headers.get("Authorization"),
          });
          return HttpResponse.json(workers);
        },
      ),
    );

    const store = setUpStore({
      config: {
        apiKey: "secret-token",
        host: "vscode",
        lspPort: 8488,
        lspUrl: "https://daemon.example.test/p/project-1/v1/ping",
        themeProps: {},
      },
    });

    const result = await store.dispatch(
      daemonApi.endpoints.getDaemonInfo.initiate(undefined),
    );

    expect(result.data?.workers).toHaveLength(1);
    expect(requested.map((entry) => entry.url)).toEqual([
      "https://daemon.example.test/daemon/v1/status",
      "https://daemon.example.test/daemon/v1/workers",
    ]);
    expect(requested.map((entry) => entry.authorization)).toEqual([
      "Bearer secret-token",
      "Bearer secret-token",
    ]);
  });

  it("falls back to lspPort and tolerates workers authorization failure", async () => {
    const requested: string[] = [];
    server.use(
      http.get("http://127.0.0.1:9494/daemon/v1/status", ({ request }) => {
        requested.push(request.url);
        return HttpResponse.json(status);
      }),
      http.get("http://127.0.0.1:9494/daemon/v1/workers", ({ request }) => {
        requested.push(request.url);
        return HttpResponse.json({ detail: "unauthorized" }, { status: 401 });
      }),
    );

    const store = setUpStore({
      config: {
        apiKey: null,
        host: "vscode",
        lspPort: 9494,
        themeProps: {},
      },
    });

    const result = await store.dispatch(
      daemonApi.endpoints.getDaemonInfo.initiate(undefined),
    );

    expect(result.data).toEqual({ status, workers: [] });
    expect(requested).toEqual([
      "http://127.0.0.1:9494/daemon/v1/status",
      "http://127.0.0.1:9494/daemon/v1/workers",
    ]);
  });
});

describe("resolveDaemonBaseUrl", () => {
  it("uses the lspUrl origin before loopback fallback", () => {
    expect(
      resolveDaemonBaseUrl({
        host: "vscode",
        lspPort: 8488,
        lspUrl: "http://127.0.0.1:8488/p/proj/v1/ping",
      }),
    ).toBe("http://127.0.0.1:8488");
  });

  it("uses same-origin relative lspUrl before loopback fallback", () => {
    expect(
      resolveDaemonBaseUrl({
        host: "web",
        lspPort: 8488,
        lspUrl: "/p/proj/v1/ping",
      }),
    ).toBe(window.location.origin);
  });

  it("uses daemon default port when lspPort is not usable", () => {
    expect(resolveDaemonBaseUrl({ host: "vscode", lspPort: 0 })).toBe(
      "http://127.0.0.1:8488",
    );
  });
});
