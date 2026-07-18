import { http, HttpResponse } from "msw";
import { beforeEach, describe, expect, it } from "vitest";

import { server } from "../../../utils/mockServer";
import { render, screen, waitFor } from "../../../utils/test-utils";
import { SettingsPage } from "./SettingsPage";

const CONFIG_STATE = {
  config: {
    apiKey: "",
    host: "web" as const,
    lspPort: 8488,
    lspUrl: "http://127.0.0.1:8488",
    surface: "dashboard" as const,
    themeProps: {},
  },
};

const workers = [
  {
    project_id: "project-first",
    slug: "first",
    root: "/work/first",
    pinned: false,
    last_active_ms: null,
    state: "running",
    pid: 1,
    http_port: 8001,
    lsp_port: 8002,
    lsp_clients: 0,
    busy_chats: 0,
    exec_running: 0,
    live_proxy_streams: 0,
    cron_next_fire_ms: null,
    idle_deadline_ms: null,
    last_status_report_ms: null,
    last_error: null,
  },
  {
    project_id: "project-pinned",
    slug: "pinned-project",
    root: "/work/pinned",
    pinned: true,
    last_active_ms: null,
    state: "running",
    pid: 2,
    http_port: 8011,
    lsp_port: 8012,
    lsp_clients: 0,
    busy_chats: 0,
    exec_running: 0,
    live_proxy_streams: 0,
    cron_next_fire_ms: null,
    idle_deadline_ms: null,
    last_status_report_ms: null,
    last_error: null,
  },
];

function daemonSettings(overrides: Record<string, unknown> = {}) {
  return {
    bind: "127.0.0.1",
    lan_enabled: false,
    mdns_enabled: false,
    auth_enabled: false,
    username: null,
    has_password: false,
    hostname_local: "refact.local",
    urls: {
      loopback: "http://127.0.0.1:8488/",
      mdns: "http://refact.local:8488/",
    },
    ...overrides,
  };
}

function daemonStatus(startedAt = 100) {
  return {
    pid: 123,
    version: "1.0.0",
    port: 8488,
    started_at_ms: startedAt,
    uptime_secs: 10,
    workers: workers.length,
    cron_pending: {},
  };
}

function idleUpdateStatus() {
  return {
    phase: "idle",
    detail: null,
    target_version: null,
    started_at_ms: null,
    finished_at_ms: null,
  };
}

function installBaseHandlers(settings = daemonSettings()) {
  server.use(
    http.get("*/daemon/v1/settings", () => HttpResponse.json(settings)),
    http.get("*/daemon/v1/workers", () => HttpResponse.json(workers)),
    http.get("*/daemon/v1/status", () => HttpResponse.json(daemonStatus())),
    http.get("*/daemon/v1/update/status", () =>
      HttpResponse.json(idleUpdateStatus()),
    ),
  );
}

function renderPage() {
  return render(<SettingsPage />, { preloadedState: CONFIG_STATE });
}

describe("DaemonDashboard Settings", () => {
  beforeEach(() => {
    installBaseHandlers();
  });

  it("refuses LAN without credentials and does not call the mutation", async () => {
    let settingsPosts = 0;
    server.use(
      http.post("*/daemon/v1/settings", () => {
        settingsPosts += 1;
        return HttpResponse.json({ success: true, restarting: true });
      }),
    );
    const { user } = renderPage();

    await user.click(
      await screen.findByRole("switch", { name: "Local network access" }),
    );

    expect(
      await screen.findByText(
        "LAN access requires authentication with a username and password.",
      ),
    ).toBeInTheDocument();
    expect(settingsPosts).toBe(0);
  });

  it("blur-saves daemon settings with the edited credentials", async () => {
    installBaseHandlers(
      daemonSettings({
        auth_enabled: true,
        username: "old-user",
        has_password: true,
      }),
    );
    let postedBody: unknown = null;
    server.use(
      http.post("*/daemon/v1/settings", async ({ request }) => {
        postedBody = await request.json();
        return HttpResponse.json({ success: true, restarting: true });
      }),
    );
    const { user } = renderPage();
    const username = await screen.findByRole("textbox", {
      name: "Basic-auth username",
    });

    await user.clear(username);
    await user.type(username, "new-user");
    await user.tab();

    await waitFor(() => {
      expect(postedBody).toEqual({
        lan_enabled: false,
        mdns_enabled: false,
        auth_enabled: true,
        username: "new-user",
      });
    });
    expect(await screen.findByText("Restarting…")).toBeInTheDocument();
  });

  it("shows backend validation errors from settings saves", async () => {
    server.use(
      http.post("*/daemon/v1/settings", () =>
        HttpResponse.json(
          { error: "The daemon rejected these settings." },
          { status: 400 },
        ),
      ),
    );
    const { user } = renderPage();

    await user.click(
      await screen.findByRole("switch", { name: "mDNS discovery" }),
    );

    expect(
      await screen.findByText("The daemon rejected these settings."),
    ).toBeInTheDocument();
  });

  it("renders a QR code and defaults provider links to the pinned project", async () => {
    installBaseHandlers(
      daemonSettings({
        bind: "0.0.0.0",
        lan_enabled: true,
        mdns_enabled: true,
        auth_enabled: true,
        username: "refact",
        has_password: true,
      }),
    );
    renderPage();

    const qr = await screen.findByRole("img", {
      name: "QR code for http://refact.local:8488/",
    });
    expect(qr.querySelector("svg")).not.toBeNull();
    expect(screen.getByText("http://refact.local:8488/")).toBeInTheDocument();

    const providerLink = await screen.findByRole("link", {
      name: "Open provider settings for pinned-project",
    });
    expect(providerLink).toHaveAttribute("href", "/p/project-pinned/");
    expect(
      screen.getByRole("combobox", { name: "Provider settings project" }),
    ).toHaveTextContent("pinned-project · pinned");
    expect(
      screen.getByText(
        "Providers, models and API keys are configured per project.",
      ),
    ).toBeInTheDocument();
    for (const section of [
      "Daemon",
      "Providers & Models",
      "Updates",
      "Danger zone",
    ]) {
      expect(
        screen.getByRole("heading", { name: section }),
      ).toBeInTheDocument();
    }
  });

  it("moves through check, install, progress, restart, and reconnect states", async () => {
    let installStarted = false;
    let installationFinished = false;
    let restartRequested = false;
    let reconnectReady = false;

    server.use(
      http.get("*/daemon/v1/update/check", () =>
        HttpResponse.json({
          current_version: "1.0.0",
          latest_version: "1.1.0",
          update_available: true,
          releases: [],
          checked_at_ms: 200,
        }),
      ),
      http.post("*/daemon/v1/update/install", () => {
        installStarted = true;
        return HttpResponse.json(
          { started: true, target_version: "1.1.0" },
          { status: 202 },
        );
      }),
      http.get("*/daemon/v1/update/status", () => {
        if (!installStarted) return HttpResponse.json(idleUpdateStatus());
        if (!installationFinished) {
          return HttpResponse.json({
            phase: "downloading",
            detail: "Downloading update",
            target_version: "1.1.0",
            started_at_ms: 201,
            finished_at_ms: null,
          });
        }
        return HttpResponse.json({
          phase: "restarting",
          detail: "installed 1.1.0",
          target_version: "1.1.0",
          started_at_ms: 201,
          finished_at_ms: 202,
        });
      }),
      http.post("*/daemon/v1/restart", () => {
        restartRequested = true;
        return HttpResponse.json(
          { error: "connection closed during restart" },
          { status: 503 },
        );
      }),
      http.get("*/daemon/v1/status", () => {
        if (restartRequested && !reconnectReady) {
          return HttpResponse.json({ error: "offline" }, { status: 503 });
        }
        return HttpResponse.json(daemonStatus(restartRequested ? 300 : 100));
      }),
    );
    const { user } = renderPage();

    await user.click(
      await screen.findByRole("button", { name: "Check for updates" }),
    );
    expect(await screen.findByText("Version 1.1.0")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Install update" }));
    expect(await screen.findByText("downloading")).toBeInTheDocument();
    expect(screen.getByText("Downloading update")).toBeInTheDocument();

    installationFinished = true;
    expect(await screen.findByText("installed")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Restart daemon" }));
    expect(
      await screen.findByText("Reconnecting to the daemon…"),
    ).toBeInTheDocument();

    reconnectReady = true;
    expect(await screen.findByText("Daemon reconnected.")).toBeInTheDocument();
  });

  it("requires typed confirmation before shutting down", async () => {
    let shutdownBody: unknown = null;
    server.use(
      http.post("*/daemon/v1/shutdown", async ({ request }) => {
        shutdownBody = await request.json();
        return HttpResponse.json({ success: true });
      }),
    );
    const { user } = renderPage();
    const shutdownButton = await screen.findByRole("button", {
      name: "Shutdown daemon",
    });

    expect(shutdownButton).toBeDisabled();
    await user.type(
      screen.getByRole("textbox", { name: "Shutdown confirmation" }),
      "shutdown",
    );
    expect(shutdownButton).toBeEnabled();
    await user.click(shutdownButton);

    await waitFor(() => {
      expect(shutdownBody).toEqual({ reason: "dashboard_settings_shutdown" });
    });
    expect(
      screen.getByRole("link", { name: "Open legacy picker" }),
    ).toHaveAttribute("href", "/picker");
  });
});
