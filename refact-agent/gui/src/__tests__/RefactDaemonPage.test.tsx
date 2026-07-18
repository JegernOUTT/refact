import { http, HttpResponse } from "msw";
import { beforeEach, describe, expect, it } from "vitest";

import { RefactDaemonPage } from "../features/RefactDaemon/RefactDaemonPage";
import { server } from "../utils/mockServer";
import { render, screen, waitFor } from "../utils/test-utils";

const CONFIG_STATE = {
  config: {
    host: "web" as const,
    lspPort: 8488,
    apiKey: null,
    features: {
      statistics: true,
      vecdb: true,
      ast: true,
    },
    themeProps: {
      appearance: "dark" as const,
    },
  },
};

function daemonStatus() {
  return {
    pid: 123,
    version: "1.0.0",
    executable_sha256: "abcdef1234567890",
    port: 8488,
    started_at_ms: 1_700_000_000_000,
    uptime_secs: 42,
    workers: 0,
    cron_pending: {},
  };
}

function daemonSettings() {
  return {
    bind: "127.0.0.1:8488",
    lan_enabled: false,
    mdns_enabled: true,
    auth_enabled: true,
    username: "refact",
    has_password: true,
    hostname_local: "refact.local",
    urls: {
      loopback: "http://127.0.0.1:8488",
      mdns: "http://refact.local:8488",
    },
  };
}

function installBaseHandlers() {
  server.use(
    http.get("*/daemon/v1/status", () => HttpResponse.json(daemonStatus())),
    http.get("*/daemon/v1/workers", () => HttpResponse.json([])),
    http.get("*/daemon/v1/settings", () => HttpResponse.json(daemonSettings())),
    http.get("*/daemon/v1/update/status", () =>
      HttpResponse.json({
        phase: "idle",
        detail: null,
        target_version: null,
        started_at_ms: null,
        finished_at_ms: null,
      }),
    ),
  );
}

function renderPage() {
  return render(<RefactDaemonPage backFromDaemon={() => undefined} />, {
    preloadedState: CONFIG_STATE,
  });
}

describe("RefactDaemonPage", () => {
  beforeEach(() => {
    installBaseHandlers();
  });

  it("renders daemon settings from the API", async () => {
    renderPage();

    expect(await screen.findByText("Network & Access")).toBeInTheDocument();
    expect(await screen.findByDisplayValue("refact")).toBeInTheDocument();
    expect(screen.getAllByText("http://127.0.0.1:8488").length).toBeGreaterThan(
      0,
    );
    expect(screen.getByText("http://refact.local:8488")).toBeInTheDocument();
  });

  it("saves settings payload and shows restarting notice", async () => {
    let postedBody: unknown = null;
    server.use(
      http.post("*/daemon/v1/settings", async ({ request }) => {
        postedBody = await request.json();
        return HttpResponse.json({ success: true, restarting: true });
      }),
    );
    const { user } = renderPage();

    await user.click(await screen.findByLabelText("Listen on 0.0.0.0"));
    await user.clear(screen.getByDisplayValue("refact"));
    await user.type(screen.getByLabelText("Username"), "daemon-user");
    await user.type(screen.getByLabelText("Password"), "secret");
    await user.click(screen.getByRole("button", { name: /save settings/i }));

    await waitFor(() => {
      expect(postedBody).toEqual({
        lan_enabled: true,
        mdns_enabled: true,
        auth_enabled: true,
        username: "daemon-user",
        password: "secret",
      });
    });
    expect(
      await screen.findByText("Daemon is restarting…"),
    ).toBeInTheDocument();
  });

  it("shows settings validation errors", async () => {
    server.use(
      http.post("*/daemon/v1/settings", () =>
        HttpResponse.json(
          { error: "LAN requires auth credentials" },
          { status: 400 },
        ),
      ),
    );
    const { user } = renderPage();

    await user.click(await screen.findByLabelText("Authentication"));
    await user.click(screen.getByRole("button", { name: /save settings/i }));

    expect(
      await screen.findByText("LAN requires auth credentials"),
    ).toBeInTheDocument();
  });

  it("confirms restart before firing restart mutation", async () => {
    let restartCount = 0;
    server.use(
      http.post("*/daemon/v1/restart", () => {
        restartCount += 1;
        return HttpResponse.json({ success: true, restarting: true });
      }),
    );
    const { user } = renderPage();

    await user.click(
      await screen.findByRole("button", { name: /restart daemon/i }),
    );
    expect(restartCount).toBe(0);
    await user.click(screen.getByRole("button", { name: /confirm restart/i }));

    await waitFor(() => expect(restartCount).toBe(1));
    expect(
      await screen.findByText("Daemon is restarting…"),
    ).toBeInTheDocument();
  });

  it("checks updates and installs a selected release", async () => {
    let installBody: unknown = null;
    server.use(
      http.get("*/daemon/v1/update/check", () =>
        HttpResponse.json({
          current_version: "1.0.0",
          latest_version: "1.1.0",
          update_available: true,
          releases: [
            {
              version: "1.1.0",
              published_at: "2024-01-02T00:00:00Z",
              prerelease: false,
              url: "https://example.test/1.1.0",
            },
            {
              version: "1.2.0-beta",
              published_at: "2024-01-03T00:00:00Z",
              prerelease: true,
              url: null,
            },
          ],
          checked_at_ms: 1_700_000_000_100,
        }),
      ),
      http.post("*/daemon/v1/update/install", async ({ request }) => {
        installBody = await request.json();
        return HttpResponse.json(
          { started: true, target_version: "1.2.0-beta" },
          { status: 202 },
        );
      }),
      http.get("*/daemon/v1/update/status", () =>
        HttpResponse.json({
          phase: "downloading",
          detail: "Downloading update",
          target_version: "1.2.0-beta",
          started_at_ms: 1_700_000_000_200,
          finished_at_ms: null,
        }),
      ),
    );
    const { user } = renderPage();

    await user.click(
      await screen.findByRole("button", { name: /check for updates/i }),
    );

    expect(await screen.findByText("1.1.0")).toBeInTheDocument();
    expect(await screen.findByText("1.2.0-beta")).toBeInTheDocument();
    expect(screen.getByText("Prerelease")).toBeInTheDocument();

    const installButtons = screen.getAllByRole("button", {
      name: /^install$/i,
    });
    await user.click(installButtons[0]);

    await waitFor(() => {
      expect(installBody).toEqual({ version: "1.2.0-beta" });
    });
    expect(await screen.findByText("downloading")).toBeInTheDocument();
    expect(screen.getByText("Downloading update")).toBeInTheDocument();
  });
});
