import { http, HttpResponse } from "msw";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { setUpStore } from "../../../app/store";
import type { DaemonWorker } from "../../../services/refact/daemon";
import { server } from "../../../utils/mockServer";
import { render, screen, waitFor, within } from "../../../utils/test-utils";
import { fetchHomeFanout } from "./homeFanout";
import { HomePage, WIZARD_DONE_KEY } from "./HomePage";

const config = {
  apiKey: "",
  host: "web" as const,
  lspPort: 8488,
  lspUrl: "https://daemon.example.test",
  surface: "dashboard" as const,
  themeProps: {},
};

function worker(
  projectId: string,
  state: string,
  extra: Partial<DaemonWorker> = {},
): DaemonWorker {
  return {
    project_id: projectId,
    slug: projectId,
    root: `/work/${projectId}`,
    pinned: false,
    last_active_ms: 1,
    state,
    pid: state === "ready" ? 10 : null,
    http_port: state === "ready" ? 8001 : null,
    lsp_port: state === "ready" ? 9001 : null,
    lsp_clients: 0,
    busy_chats: 0,
    exec_running: 0,
    live_proxy_streams: 0,
    cron_next_fire_ms: null,
    idle_deadline_ms: null,
    last_status_report_ms: 1,
    last_error: null,
    ...extra,
  };
}

function renderHome() {
  return render(<HomePage />, { store: setUpStore({ config }) });
}

function updateCheck(updateAvailable = false) {
  return http.get("https://daemon.example.test/daemon/v1/update/check", () =>
    HttpResponse.json({
      current_version: "1.0.0",
      latest_version: updateAvailable ? "1.1.0" : "1.0.0",
      update_available: updateAvailable,
      releases: [],
      checked_at_ms: 1,
    }),
  );
}

function trajectory(id: string, title: string, updatedAt: string) {
  return {
    id,
    title,
    created_at: updatedAt,
    updated_at: updatedAt,
    model: "test",
    mode: "agent",
    message_count: 2,
    total_lines_added: 0,
    total_lines_removed: 0,
    tasks_total: 0,
    tasks_done: 0,
    tasks_failed: 0,
  };
}

function cron(id: string, failed: boolean) {
  return {
    id,
    cron: "* * * * *",
    human_schedule: "Every minute",
    description: "Nightly checks",
    prompt: "Run checks",
    recurring: true,
    durable: true,
    next_fire_at_ms: 1,
    fire_count: 1,
    created_at_ms: 1,
    enabled: true,
    paused: false,
    trigger_kind: "cron",
    tz: null,
    every_ms: null,
    at_ms: null,
    last_status: failed ? "failed" : "completed",
    last_error: failed ? "tests failed" : null,
    recent_runs: [],
    action_kind: "agent_turn",
    delivery_kind: "chat",
    chat_id: null,
    target: "isolated",
    isolated: true,
  };
}

describe("Dashboard Home", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("shows the first wizard step with widgets below on a fresh daemon", async () => {
    server.use(
      http.get("https://daemon.example.test/daemon/v1/workers", () =>
        HttpResponse.json([]),
      ),
      updateCheck(),
    );

    renderHome();

    expect(
      await screen.findByRole("heading", {
        name: "Bring your first project into Refact",
      }),
    ).toBeInTheDocument();
    expect(screen.getByText("Step 1 of 3")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Dismiss setup" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Skip setup" }),
    ).toBeInTheDocument();
    expect(await screen.findByText("No recent chats yet")).toBeInTheDocument();
    expect(screen.getByText("All clear")).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "Quick actions" }),
    ).toBeInTheDocument();
  });

  it("shows the provider pointer for a ready worker with no configured providers", async () => {
    server.use(
      http.get("https://daemon.example.test/daemon/v1/workers", () =>
        HttpResponse.json([worker("refact", "ready")]),
      ),
      http.get("https://daemon.example.test/p/refact/v1/providers", () =>
        HttpResponse.json({
          providers: [
            {
              name: "openai",
              base_provider: "openai",
              display_name: "OpenAI",
              enabled: false,
              readonly: false,
              has_credentials: false,
              status: "not_configured",
              model_count: 0,
            },
          ],
        }),
      ),
      http.get("https://daemon.example.test/p/refact/v1/trajectories", () =>
        HttpResponse.json({ items: [] }),
      ),
      http.get("https://daemon.example.test/p/refact/v1/scheduler/cron", () =>
        HttpResponse.json([]),
      ),
      updateCheck(),
    );

    renderHome();

    expect(
      await screen.findByRole("heading", { name: "Set up a provider" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Step 2 of 3")).toBeInTheDocument();
    expect(
      screen.getByRole("link", { name: "Open provider setup" }),
    ).toHaveAttribute("href", "/p/refact/?page=providers");
  });

  it("auto-completes setup for an established install with providers", async () => {
    server.use(
      http.get("https://daemon.example.test/daemon/v1/workers", () =>
        HttpResponse.json([worker("refact", "ready")]),
      ),
      http.get("https://daemon.example.test/p/refact/v1/providers", () =>
        HttpResponse.json({
          providers: [
            {
              name: "openai",
              base_provider: "openai",
              display_name: "OpenAI",
              enabled: true,
              readonly: false,
              has_credentials: true,
              status: "active",
              model_count: 1,
            },
          ],
        }),
      ),
      http.get("https://daemon.example.test/p/refact/v1/trajectories", () =>
        HttpResponse.json({ items: [] }),
      ),
      http.get("https://daemon.example.test/p/refact/v1/scheduler/cron", () =>
        HttpResponse.json([]),
      ),
      updateCheck(),
    );

    renderHome();

    await waitFor(() =>
      expect(window.localStorage.getItem(WIZARD_DONE_KEY)).toBe("true"),
    );
    expect(screen.queryByText("First-run setup")).not.toBeInTheDocument();
    expect(await screen.findByText("No recent chats yet")).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "Needs attention" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "Quick actions" }),
    ).toBeInTheDocument();
  });

  it("auto-completes setup when chats exist without configured providers", async () => {
    server.use(
      http.get("https://daemon.example.test/daemon/v1/workers", () =>
        HttpResponse.json([worker("refact", "ready")]),
      ),
      http.get("https://daemon.example.test/p/refact/v1/providers", () =>
        HttpResponse.json({
          providers: [
            {
              name: "openai",
              base_provider: "openai",
              display_name: "OpenAI",
              enabled: false,
              readonly: false,
              has_credentials: false,
              status: "not_configured",
              model_count: 0,
            },
          ],
        }),
      ),
      http.get("https://daemon.example.test/p/refact/v1/trajectories", () =>
        HttpResponse.json({
          items: [
            trajectory("chat-1", "Ship the release", new Date().toISOString()),
          ],
          next_cursor: null,
          has_more: false,
          total_count: 1,
        }),
      ),
      http.get("https://daemon.example.test/p/refact/v1/scheduler/cron", () =>
        HttpResponse.json([]),
      ),
      updateCheck(),
    );

    renderHome();

    await waitFor(() =>
      expect(window.localStorage.getItem(WIZARD_DONE_KEY)).toBe("true"),
    );
    expect(screen.queryByText("First-run setup")).not.toBeInTheDocument();
    expect(await screen.findByText("Ship the release")).toBeInTheDocument();
  });

  it("renders populated steady-state widgets and navigation actions", async () => {
    window.localStorage.setItem(WIZARD_DONE_KEY, "true");
    server.use(
      http.get("https://daemon.example.test/daemon/v1/workers", () =>
        HttpResponse.json([
          worker("ready", "ready", { last_active_ms: 100 }),
          worker("crashed", "crashed", { last_error: "worker exited" }),
        ]),
      ),
      http.get("https://daemon.example.test/p/ready/v1/trajectories", () =>
        HttpResponse.json({
          items: [
            trajectory("chat-1", "Fix dashboard", new Date().toISOString()),
          ],
          next_cursor: null,
          has_more: false,
          total_count: 1,
        }),
      ),
      http.get("https://daemon.example.test/p/ready/v1/scheduler/cron", () =>
        HttpResponse.json([cron("cron-1", true)]),
      ),
      updateCheck(true),
      http.get("https://daemon.example.test/p/ready/v1/providers", () =>
        HttpResponse.json({
          providers: [
            {
              name: "openai",
              base_provider: "openai",
              display_name: "OpenAI",
              enabled: true,
              readonly: false,
              has_credentials: true,
              status: "active",
              model_count: 1,
            },
          ],
        }),
      ),
    );

    const view = renderHome();

    expect(await screen.findByText("Fix dashboard")).toBeInTheDocument();
    expect(screen.getByText("Daemon update available")).toBeInTheDocument();
    expect(screen.getByText("crashed worker crashed")).toBeInTheDocument();
    expect(screen.getByText("Nightly checks failed")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Add project" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Setup" })).toBeInTheDocument();

    await view.user.click(screen.getByRole("button", { name: "Setup" }));
    expect(
      await screen.findByRole("heading", {
        name: "Your workspace is ready",
      }),
    ).toBeInTheDocument();
    expect(window.localStorage.getItem(WIZARD_DONE_KEY)).toBeNull();
  });

  it("renders friendly widget empty states", async () => {
    window.localStorage.setItem(WIZARD_DONE_KEY, "true");
    server.use(
      http.get("https://daemon.example.test/daemon/v1/workers", () =>
        HttpResponse.json([]),
      ),
      updateCheck(),
    );

    renderHome();

    expect(await screen.findByText("No recent chats yet")).toBeInTheDocument();
    expect(screen.getByText("All clear")).toBeInTheDocument();
  });

  it("fans out only to ready workers", async () => {
    window.localStorage.setItem(WIZARD_DONE_KEY, "true");
    const requestedProjects: string[] = [];
    server.use(
      http.get("https://daemon.example.test/daemon/v1/workers", () =>
        HttpResponse.json([
          worker("ready", "ready"),
          worker("starting", "starting"),
          worker("crashed", "crashed"),
        ]),
      ),
      http.get(
        "https://daemon.example.test/p/:projectId/v1/trajectories",
        ({ params }) => {
          requestedProjects.push(`chat:${String(params.projectId)}`);
          return HttpResponse.json({ items: [] });
        },
      ),
      http.get(
        "https://daemon.example.test/p/:projectId/v1/scheduler/cron",
        ({ params }) => {
          requestedProjects.push(`cron:${String(params.projectId)}`);
          return HttpResponse.json([]);
        },
      ),
      updateCheck(),
    );

    renderHome();

    await waitFor(() => expect(requestedProjects).toHaveLength(2));
    expect(requestedProjects.sort()).toEqual(["chat:ready", "cron:ready"]);
  });

  it("limits combined home fan-out to three requests", async () => {
    let activeRequests = 0;
    let maxActiveRequests = 0;
    let releaseRequests: (() => void) | undefined;
    const blocked = new Promise<void>((resolve) => {
      releaseRequests = resolve;
    });
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL) => {
        activeRequests += 1;
        maxActiveRequests = Math.max(maxActiveRequests, activeRequests);
        await blocked;
        activeRequests -= 1;
        return new Response(
          String(input).includes("trajectories")
            ? JSON.stringify({ items: [] })
            : JSON.stringify([]),
          { headers: { "Content-Type": "application/json" } },
        );
      }),
    );

    const request = fetchHomeFanout(
      "https://daemon.example.test",
      Array.from({ length: 5 }, (_, index) =>
        worker(`ready-${String(index)}`, "ready", {
          last_active_ms: 10 - index,
        }),
      ),
    );
    await waitFor(() => expect(maxActiveRequests).toBe(3));
    releaseRequests?.();
    await request;

    expect(maxActiveRequests).toBe(3);
    vi.unstubAllGlobals();
  });

  it("reuses the add project dialog from Quick actions", async () => {
    window.localStorage.setItem(WIZARD_DONE_KEY, "true");
    server.use(
      http.get("https://daemon.example.test/daemon/v1/workers", () =>
        HttpResponse.json([]),
      ),
      updateCheck(),
      http.post("https://daemon.example.test/daemon/v1/fs/browse", () =>
        HttpResponse.json({
          path: "/home",
          parent: "/",
          dirs: [],
          can_open: true,
          truncated: false,
        }),
      ),
    );

    const view = renderHome();
    await view.user.click(
      await screen.findByRole("button", { name: "Add project" }),
    );

    expect(await screen.findByRole("dialog")).toBeInTheDocument();
    expect(
      within(screen.getByRole("dialog")).getByLabelText("Project path"),
    ).toBeInTheDocument();
  });
});
