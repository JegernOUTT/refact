import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";

import { setUpStore } from "../../app/store";
import { server } from "../../utils/mockServer";
import { daemonApi, projectApiUrl, resolveDaemonBaseUrl } from "./daemon";

const status = {
  pid: 123,
  version: "1.0.0",
  port: 8488,
  started_at_ms: 1_700_000_000_000,
  uptime_secs: 120,
  workers: 1,
  cron_pending: {
    nightly: 1_000,
  },
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
  it("builds encoded project proxy API URLs", () => {
    expect(
      projectApiUrl(
        "https://daemon.example.test/",
        "project / one",
        "rag-status",
      ),
    ).toBe("https://daemon.example.test/p/project%20%2F%20one/v1/rag-status");
  });

  it("derives daemon root from lspUrl origin and omits the model apiKey", async () => {
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
    expect(result.data?.workersAccess).toBe("visible");
    expect(result.data?.status.cron_pending).toEqual({ nightly: 1_000 });
    expect(requested.map((entry) => entry.url)).toEqual([
      "https://daemon.example.test/daemon/v1/status",
      "https://daemon.example.test/daemon/v1/workers",
    ]);
    expect(requested.map((entry) => entry.authorization)).toEqual([null, null]);
  });

  it("falls back to lspPort and reports auth-hidden workers", async () => {
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

    expect(result.data).toEqual({
      status,
      workers: [],
      workersAccess: "auth_hidden",
    });
    expect(requested).toEqual([
      "http://127.0.0.1:9494/daemon/v1/status",
      "http://127.0.0.1:9494/daemon/v1/workers",
    ]);
  });

  it("distinguishes a genuine empty worker list from auth-hidden workers", async () => {
    server.use(
      http.get("http://127.0.0.1:9494/daemon/v1/status", () =>
        HttpResponse.json({ ...status, workers: 0 }),
      ),
      http.get("http://127.0.0.1:9494/daemon/v1/workers", () =>
        HttpResponse.json([]),
      ),
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

    expect(result.data).toEqual({
      status: { ...status, workers: 0 },
      workers: [],
      workersAccess: "visible",
    });
  });

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

  it("uses same-origin when the daemon serves the GUI", () => {
    expect(
      resolveDaemonBaseUrl({
        host: "web",
        engineServed: true,
        lspPort: 8001,
      }),
    ).toBe(window.location.origin);
  });

  it("uses daemon default port when lspPort is not usable", () => {
    expect(resolveDaemonBaseUrl({ host: "vscode", lspPort: 0 })).toBe(
      "http://127.0.0.1:8488",
    );
  });

  it("uses the daemon control-plane endpoint shapes", async () => {
    const requested: { url: string; method: string; body: unknown }[] = [];
    const record = async (request: Request) => {
      const bodyText =
        request.method === "POST" ? await request.clone().text() : "";
      requested.push({
        url: request.url,
        method: request.method,
        body: bodyText ? (JSON.parse(bodyText) as unknown) : undefined,
      });
      return HttpResponse.json({ success: true });
    };
    server.use(
      http.get("https://daemon.example.test/daemon/v1/workers", () =>
        HttpResponse.json(workers),
      ),
      http.post(
        "https://daemon.example.test/daemon/v1/projects/open",
        ({ request }) => record(request),
      ),
      http.delete(
        "https://daemon.example.test/daemon/v1/projects/:id",
        ({ request }) => record(request),
      ),
      http.post(
        "https://daemon.example.test/daemon/v1/projects/:id/pin",
        ({ request }) => record(request),
      ),
      http.post(
        "https://daemon.example.test/daemon/v1/projects/:id/restart",
        ({ request }) => record(request),
      ),
      http.post(
        "https://daemon.example.test/daemon/v1/projects/:id/stop",
        ({ request }) => record(request),
      ),
      http.get("https://daemon.example.test/cron/status", ({ request }) => {
        requested.push({
          url: request.url,
          method: request.method,
          body: undefined,
        });
        return HttpResponse.json({ enabled: true, jobs: 2, next_wake_ms: 3 });
      }),
      http.post(
        "https://daemon.example.test/daemon/v1/fs/browse",
        ({ request }) => record(request),
      ),
    );
    const store = setUpStore({
      config: {
        host: "web",
        lspPort: 8488,
        lspUrl: "https://daemon.example.test",
        themeProps: {},
      },
    });

    const listed = await store.dispatch(
      daemonApi.endpoints.listProjects.initiate(undefined),
    );
    expect(listed.data).toEqual(workers);
    await store.dispatch(
      daemonApi.endpoints.openProject.initiate({ root: "/work/refact" }),
    );
    await store.dispatch(
      daemonApi.endpoints.forgetProject.initiate("project / one"),
    );
    await store.dispatch(
      daemonApi.endpoints.pinProject.initiate({
        projectId: "project / one",
        pinned: true,
      }),
    );
    await store.dispatch(
      daemonApi.endpoints.restartProject.initiate("project / one"),
    );
    await store.dispatch(
      daemonApi.endpoints.stopProject.initiate("project / one"),
    );
    await store.dispatch(daemonApi.endpoints.getCronStatus.initiate(undefined));
    await store.dispatch(
      daemonApi.endpoints.browseFolders.initiate({ path: "/work" }),
    );

    expect(requested).toEqual([
      {
        url: "https://daemon.example.test/daemon/v1/projects/open",
        method: "POST",
        body: { root: "/work/refact" },
      },
      {
        url: "https://daemon.example.test/daemon/v1/projects/project%20%2F%20one",
        method: "DELETE",
        body: undefined,
      },
      {
        url: "https://daemon.example.test/daemon/v1/projects/project%20%2F%20one/pin",
        method: "POST",
        body: { pinned: true },
      },
      {
        url: "https://daemon.example.test/daemon/v1/projects/project%20%2F%20one/restart",
        method: "POST",
        body: undefined,
      },
      {
        url: "https://daemon.example.test/daemon/v1/projects/project%20%2F%20one/stop",
        method: "POST",
        body: undefined,
      },
      {
        url: "https://daemon.example.test/cron/status",
        method: "GET",
        body: undefined,
      },
      {
        url: "https://daemon.example.test/daemon/v1/fs/browse",
        method: "POST",
        body: { path: "/work" },
      },
    ]);
  });

  it("requests and parses daemon event backfill", async () => {
    let requestedUrl = "";
    server.use(
      http.get(
        "https://daemon.example.test/daemon/v1/events",
        ({ request }) => {
          requestedUrl = request.url;
          return new HttpResponse(
            'id: 8\nevent: daemon\ndata: {"seq":8,"ts_ms":10,"kind":"ready","project_id":null,"payload":{}}\n\n',
            { headers: { "Content-Type": "text/event-stream" } },
          );
        },
      ),
    );
    const store = setUpStore({
      config: {
        host: "web",
        lspPort: 8488,
        lspUrl: "https://daemon.example.test",
        themeProps: {},
      },
    });

    const result = await store.dispatch(
      daemonApi.endpoints.getDaemonEvents.initiate(7),
    );

    expect(requestedUrl).toBe(
      "https://daemon.example.test/daemon/v1/events?after_seq=7&follow=false",
    );
    expect(result.data).toEqual([
      {
        seq: 8,
        ts_ms: 10,
        kind: "ready",
        project_id: null,
        payload: {},
      },
    ]);
  });
});
