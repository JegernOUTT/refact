import { http, HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";

import { render, screen } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { RefactDaemonPage } from "./RefactDaemonPage";

const status = {
  pid: 1234,
  version: "0.9.1",
  port: 8488,
  started_at_ms: 1_700_000_000_000,
  uptime_secs: 3_906,
  workers: 1,
  cron_pending: 2,
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
  it("renders daemon status and workers from the daemon API", async () => {
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
    expect(screen.getByText("0.9.1")).toBeInTheDocument();
    expect(screen.getByText("1h 5m")).toBeInTheDocument();
    expect(
      screen.getAllByText("https://daemon.example.test")[0],
    ).toBeInTheDocument();
    expect(screen.getAllByText("refact-main")[0]).toBeInTheDocument();
    expect(screen.getAllByText("/workspace/refact")[0]).toBeInTheDocument();
    expect(screen.getAllByText("running")[0]).toBeInTheDocument();
    expect(screen.getByText("Daemon workers")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Back" }));
    expect(onBack).toHaveBeenCalledTimes(1);
  });
});
