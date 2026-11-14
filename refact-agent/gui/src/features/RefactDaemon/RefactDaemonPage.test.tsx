import { http, HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";
import { act } from "react-dom/test-utils";

import { render, screen, waitFor } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { updateConfig } from "../Config/configSlice";
import { daemonApi } from "../../services/refact/daemon";
import { RefactDaemonPage } from "./RefactDaemonPage";

const status = {
  pid: 1234,
  version: "0.9.1",
  port: 8488,
  started_at_ms: 1_700_000_000_000,
  uptime_secs: 3_906,
  workers: 1,
  cron_pending: {
    hourly: 250,
    nightly: 1_000,
  },
};

const workers = [
  {
    project_id: "project-1",
    slug: "refact-main",
    root: "/workspace/refact",
    pinned: true,
    last_active_ms: 1_700_000_001_000,
    state: "running",
    pid: 2345,
    http_port: 8001,
    lsp_port: 8002,
    lsp_clients: 3,
    busy_chats: 1,
    exec_running: 2,
    live_proxy_streams: 4,
    cron_next_fire_ms: null,
    idle_deadline_ms: null,
    last_status_report_ms: 1_700_000_002_000,
    last_error: "",
  },
];

const PRELOADED_STATE = {
  config: {
    apiKey: "test-token",
    host: "vscode" as const,
    lspPort: 8488,
    lspUrl: "https://daemon.example.test/p/project-1/v1/ping",
    themeProps: {},
  },
};

describe("RefactDaemonPage", () => {
  it("clears unsaved settings and action confirmation when switching to another available daemon", async () => {
    const mutations: string[] = [];
    for (const port of [19481, 19482]) {
      const origin = `http://127.0.0.1:${port}`;
      server.use(
        http.get(`${origin}/daemon/v1/status`, () =>
          HttpResponse.json({ ...status, port, version: `daemon-${port}` }),
        ),
        http.get(`${origin}/daemon/v1/workers`, () => HttpResponse.json([])),
        http.get(`${origin}/daemon/v1/settings`, () =>
          HttpResponse.json({
            bind: `127.0.0.1:${port}`,
            lan_enabled: false,
            mdns_enabled: false,
            auth_enabled: true,
            username: `user-${port}`,
            has_password: true,
            hostname_local: "fixture.local",
            urls: { loopback: origin, mdns: "" },
          }),
        ),
        http.get(`${origin}/daemon/v1/update/status`, () =>
          HttpResponse.json({ phase: "idle" }),
        ),
        http.post(`${origin}/daemon/v1/*`, ({ request }) => {
          mutations.push(request.url);
          return HttpResponse.json({ success: true, restarting: true });
        }),
      );
    }
    const { store, user } = render(
      <RefactDaemonPage backFromDaemon={vi.fn()} />,
      {
        preloadedState: {
          config: {
            host: "vscode",
            themeProps: {},
            lspPort: 0,
            backendReady: false,
            connectionStatus: "failed",
            daemonPort: 19481,
          },
        },
      },
    );
    await screen.findByDisplayValue("user-19481");
    await user.clear(screen.getByLabelText("Username"));
    await user.type(screen.getByLabelText("Username"), "unsaved-user-A");
    await user.type(screen.getByLabelText("Password"), "unsaved-password-A");
    await user.click(screen.getByRole("button", { name: "Restart daemon" }));
    expect(
      screen.getByRole("button", { name: "Confirm restart" }),
    ).toBeVisible();

    act(() => {
      store.dispatch(updateConfig({ daemonPort: 19482 }));
    });

    await screen.findByDisplayValue("user-19482");
    expect(
      screen.queryByDisplayValue("unsaved-user-A"),
    ).not.toBeInTheDocument();
    expect(screen.getByLabelText("Password")).toHaveValue("");
    expect(
      screen.queryByRole("button", { name: "Confirm restart" }),
    ).not.toBeInTheDocument();
    expect(mutations).toEqual([]);
    await user.click(screen.getByRole("button", { name: "Restart daemon" }));
    expect(mutations).toEqual([]);
    await user.click(screen.getByRole("button", { name: "Confirm restart" }));
    await waitFor(() =>
      expect(mutations).toEqual(["http://127.0.0.1:19482/daemon/v1/restart"]),
    );
  });

  it("clears mounted daemon data and settings on a daemonPort-only switch and ignores delayed old responses", async () => {
    let statusHitsA = 0;
    let releaseOldStatus: () => void = () => undefined;
    const oldStatusGate = new Promise<void>((resolve) => {
      releaseOldStatus = resolve;
    });
    let releaseNewStatus: () => void = () => undefined;
    const newStatusGate = new Promise<void>((resolve) => {
      releaseNewStatus = resolve;
    });
    server.use(
      http.get("http://127.0.0.1:19481/daemon/v1/status", async () => {
        statusHitsA += 1;
        if (statusHitsA > 1) await oldStatusGate;
        return HttpResponse.json({ ...status, version: "daemon-A-version" });
      }),
      http.get("http://127.0.0.1:19481/daemon/v1/workers", () =>
        HttpResponse.json(workers),
      ),
      http.get("http://127.0.0.1:19481/daemon/v1/settings", () =>
        HttpResponse.json({
          bind: "127.0.0.1:19481",
          lan_enabled: false,
          mdns_enabled: false,
          auth_enabled: true,
          username: "daemon-A-user",
          has_password: true,
          hostname_local: "daemon-A.local",
          urls: {
            loopback: "http://127.0.0.1:19481",
            mdns: "http://daemon-A.local",
          },
        }),
      ),
      http.get("http://127.0.0.1:19481/daemon/v1/update/status", () =>
        HttpResponse.json({ phase: "idle" }),
      ),
      http.get("http://127.0.0.1:19482/daemon/v1/status", async () => {
        await newStatusGate;
        return HttpResponse.json({ ...status, version: "daemon-B-version" });
      }),
      http.get("http://127.0.0.1:19482/daemon/v1/workers", () =>
        HttpResponse.json({}, { status: 401 }),
      ),
      http.get("http://127.0.0.1:19482/daemon/v1/settings", () =>
        HttpResponse.json({}, { status: 401 }),
      ),
      http.get("http://127.0.0.1:19482/daemon/v1/update/status", () =>
        HttpResponse.json({ phase: "idle" }),
      ),
    );
    const { store } = render(<RefactDaemonPage backFromDaemon={vi.fn()} />, {
      preloadedState: {
        config: {
          host: "vscode",
          themeProps: {},
          lspPort: 0,
          backendReady: false,
          connectionStatus: "failed",
          daemonPort: 19481,
        },
      },
    });
    await screen.findAllByText("daemon-A-version");
    await waitFor(() =>
      expect(
        daemonApi.endpoints.getDaemonSettings.select(undefined)(
          store.getState(),
        ).data?.settings?.username,
      ).toBe("daemon-A-user"),
    );
    const oldRequest = store.dispatch(
      daemonApi.endpoints.getDaemonInfo.initiate(undefined, {
        forceRefetch: true,
        subscribe: false,
      }),
    );
    await waitFor(() => expect(statusHitsA).toBe(2));
    act(() => {
      store.dispatch(updateConfig({ daemonPort: 19482 }));
    });
    expect(
      daemonApi.endpoints.getDaemonInfo.select(undefined)(store.getState())
        .data,
    ).toBeUndefined();
    expect(
      daemonApi.endpoints.getDaemonSettings.select(undefined)(store.getState())
        .data?.settings,
    ).toBeFalsy();
    expect(screen.queryByText("daemon-A-version")).not.toBeInTheDocument();
    expect(screen.queryByDisplayValue("daemon-A-user")).not.toBeInTheDocument();
    act(() => {
      releaseNewStatus();
    });
    await screen.findAllByText("daemon-B-version");
    await screen.findAllByText("Workers hidden — daemon auth enabled");
    await screen.findAllByText("Settings hidden — daemon auth enabled");
    await act(async () => {
      releaseOldStatus();
      await oldRequest;
    });
    expect(screen.queryByText("daemon-A-version")).not.toBeInTheDocument();
    expect(
      daemonApi.endpoints.getDaemonInfo.select(undefined)(store.getState()).data
        ?.status.version,
    ).toBe("daemon-B-version");
    expect(
      daemonApi.endpoints.getDaemonSettings.select(undefined)(store.getState())
        .data?.access,
    ).toBe("auth_hidden");
  });

  it("renders daemon status, cron pending entries, and workers from the daemon API", async () => {
    server.use(
      http.get("https://daemon.example.test/daemon/v1/status", () =>
        HttpResponse.json(status),
      ),
      http.get("https://daemon.example.test/daemon/v1/workers", () =>
        HttpResponse.json(workers),
      ),
    );

    const onBack = vi.fn();
    const { user } = render(<RefactDaemonPage backFromDaemon={onBack} />, {
      preloadedState: PRELOADED_STATE,
    });

    expect(
      await screen.findByRole("heading", { name: "Refact Daemon" }),
    ).toBeInTheDocument();
    expect(screen.getAllByText("0.9.1").length).toBeGreaterThan(0);
    expect(screen.getByText("1h 5m")).toBeInTheDocument();
    expect(
      screen.getAllByText("https://daemon.example.test")[0],
    ).toBeInTheDocument();
    expect(screen.getByText("2 pending")).toBeInTheDocument();
    expect(screen.getByText("hourly")).toBeInTheDocument();
    expect(screen.getByText("250 ms")).toBeInTheDocument();
    expect(screen.getByText("nightly")).toBeInTheDocument();
    expect(screen.getByText("1000 ms")).toBeInTheDocument();
    expect(screen.getAllByText("refact-main")[0]).toBeInTheDocument();
    expect(screen.getAllByText("/workspace/refact")[0]).toBeInTheDocument();
    expect(screen.getAllByText("running")[0]).toBeInTheDocument();
    expect(screen.getByText("Daemon workers")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Back" }));
    expect(onBack).toHaveBeenCalledTimes(1);
  });

  it("shows auth-hidden workers separately from an empty worker list", async () => {
    server.use(
      http.get("https://daemon.example.test/daemon/v1/status", () =>
        HttpResponse.json({ ...status, cron_pending: {} }),
      ),
      http.get("https://daemon.example.test/daemon/v1/workers", () =>
        HttpResponse.json({ detail: "unauthorized" }, { status: 401 }),
      ),
    );

    render(<RefactDaemonPage backFromDaemon={vi.fn()} />, {
      preloadedState: PRELOADED_STATE,
    });

    expect(
      await screen.findAllByText("Workers hidden — daemon auth enabled"),
    ).not.toHaveLength(0);
    expect(screen.getByText("Hidden by auth")).toBeInTheDocument();
    expect(screen.getByText("0 pending")).toBeInTheDocument();
    expect(
      screen.queryByText("No workers reported by daemon."),
    ).not.toBeInTheDocument();
  });
});
