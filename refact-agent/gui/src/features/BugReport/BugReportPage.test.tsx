import { http, HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";

import { render, screen } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { BugReportPage } from "./BugReportPage";

const context = {
  engine_version: "0.10.14",
  os: "linux x86_64",
  http_port: 8001,
  cache_dir: "/home/user/.cache/refact",
  config_dir: "/home/user/.config/refact",
  workspace_roots: ["/workspace/refact"],
  log_paths: {
    engine_log_target: "/home/user/.cache/refact/daemon/logs/worker-refact.log",
    engine_log_exists: true,
    daemon_log_file: "/home/user/.cache/refact/daemon/logs/daemon.log",
    daemon_log_exists: true,
    daemon_logs_dir: "/home/user/.cache/refact/daemon/logs",
  },
  bundle_default_dir: "/home/user/.cache/refact/bug-reports",
};

const engineLogs = {
  source: "engine",
  path: "/home/user/.cache/refact/daemon/logs/worker-refact.log",
  exists: true,
  lines: [
    "12:00:00 INFO chat/session.rs chat started",
    "12:00:01 ERROR chat/generation.rs Context too large",
  ],
};

const daemonLogs = {
  source: "daemon",
  path: "/home/user/.cache/refact/daemon/logs/daemon.log",
  exists: true,
  lines: ["12:00:00 INFO daemon/supervisor.rs heartbeat ok"],
};

const errors = {
  errors: [
    {
      source: "engine",
      level: "error",
      message: "Context too large and automatic compaction failed",
    },
    {
      source: "daemon",
      level: "error",
      message: "daemon worker crashed unexpectedly",
    },
  ],
};

const PRELOADED_STATE = {
  config: {
    apiKey: "test-token",
    host: "vscode" as const,
    lspPort: 8001,
    lspUrl: "https://engine.example.test/v1/ping",
    themeProps: {},
  },
};

function useBugReportHandlers() {
  server.use(
    http.get("*/v1/bug-report/context", () => HttpResponse.json(context)),
    http.get("*/v1/bug-report/logs", ({ request }) => {
      const url = new URL(request.url);
      return HttpResponse.json(
        url.searchParams.get("source") === "daemon" ? daemonLogs : engineLogs,
      );
    }),
    http.get("*/v1/bug-report/errors", () => HttpResponse.json(errors)),
  );
}

describe("BugReportPage", () => {
  it("renders log tabs, streamed lines, aggregated errors and log paths", async () => {
    useBugReportHandlers();
    const onBack = vi.fn();
    const { user } = render(<BugReportPage onBack={onBack} />, {
      preloadedState: PRELOADED_STATE,
    });

    expect(screen.getByText("Report a Bug")).toBeInTheDocument();
    expect(screen.getByText("Daemon")).toBeInTheDocument();
    expect(screen.getByText("Engine")).toBeInTheDocument();
    expect(screen.getByText("Web UI")).toBeInTheDocument();
    expect(screen.getByText("IDE")).toBeInTheDocument();

    expect(
      await screen.findByText(
        "12:00:01 ERROR chat/generation.rs Context too large",
      ),
    ).toBeInTheDocument();

    expect(
      await screen.findByText(
        "Context too large and automatic compaction failed",
      ),
    ).toBeInTheDocument();

    expect(screen.getByText("Latest errors")).toBeInTheDocument();
    expect(screen.getByText("Describe the bug")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Back" }));
    expect(onBack).toHaveBeenCalledTimes(1);
  });

  it("jumps to the source tab when an aggregated error is clicked", async () => {
    useBugReportHandlers();
    const { user } = render(<BugReportPage onBack={vi.fn()} />, {
      preloadedState: PRELOADED_STATE,
    });

    const daemonLine = "12:00:00 INFO daemon/supervisor.rs heartbeat ok";
    expect(screen.queryByText(daemonLine)).not.toBeInTheDocument();

    const errorItem = await screen.findByText(
      "daemon worker crashed unexpectedly",
    );
    await user.click(errorItem);

    expect(await screen.findByText(daemonLine)).toBeInTheDocument();
  });
});
