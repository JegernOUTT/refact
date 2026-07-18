import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";

import { setUpStore } from "../../../app/store";
import type { DaemonWorker } from "../../../services/refact/daemon";
import type { CronTask } from "../../../services/refact/schedulerApi";
import { server } from "../../../utils/mockServer";
import { render, screen, waitFor, within } from "../../../utils/test-utils";
import { SchedulerPage } from "./SchedulerPage";

HTMLElement.prototype.hasPointerCapture = () => false;

const BASE = "https://daemon.example.test";

const config = {
  apiKey: "",
  host: "web" as const,
  lspPort: 8488,
  lspUrl: BASE,
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

function cron(id: string, description: string): CronTask {
  return {
    id,
    cron: "7 * * * *",
    human_schedule: "Hourly at :07",
    description,
    prompt: "Run checks",
    recurring: true,
    durable: true,
    next_fire_at_ms: Date.now() + 60_000,
    fire_count: 3,
    created_at_ms: 1,
    enabled: true,
    paused: false,
    trigger_kind: "cron",
    tz: null,
    every_ms: null,
    at_ms: null,
    last_status: "fired",
    last_error: null,
    recent_runs: [],
    action_kind: "agent_turn",
    delivery_kind: "chat",
    chat_id: null,
    target: "isolated",
    isolated: true,
  };
}

function daemonHandlers(
  workers: DaemonWorker[],
  cronPending: Record<string, number> = {},
) {
  return [
    http.get(`${BASE}/daemon/v1/status`, () =>
      HttpResponse.json({
        pid: 1,
        version: "1.0.0",
        port: 8488,
        started_at_ms: 1,
        uptime_secs: 10,
        workers: workers.length,
        cron_pending: cronPending,
      }),
    ),
    http.get(`${BASE}/daemon/v1/workers`, () => HttpResponse.json(workers)),
    http.get(`${BASE}/cron/status`, () =>
      HttpResponse.json({
        enabled: true,
        jobs: 2,
        next_wake_ms: Date.now() + 42_000,
      }),
    ),
  ];
}

function renderScheduler() {
  return render(<SchedulerPage />, { store: setUpStore({ config }) });
}

describe("Dashboard Scheduler", () => {
  it("fans out to ready workers and groups jobs by project", async () => {
    server.use(
      ...daemonHandlers([
        worker("alpha", "ready"),
        worker("beta", "ready"),
        worker("stopped-project", "stopped"),
      ]),
      http.get(`${BASE}/p/alpha/v1/scheduler/cron`, () =>
        HttpResponse.json([cron("cron-a", "Alpha nightly")]),
      ),
      http.get(`${BASE}/p/beta/v1/scheduler/cron`, () =>
        HttpResponse.json([cron("cron-b", "Beta cleanup")]),
      ),
    );

    renderScheduler();

    expect(await screen.findByText("Alpha nightly")).toBeInTheDocument();
    expect(screen.getByText("Beta cleanup")).toBeInTheDocument();
    expect(screen.getByText("alpha")).toBeInTheDocument();
    expect(screen.getByText("beta")).toBeInTheDocument();
    expect(screen.getByText("stopped-project")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Wake to view" }),
    ).toBeInTheDocument();
  });

  it("renders the cron clock header from /cron/status", async () => {
    server.use(
      ...daemonHandlers([worker("alpha", "ready")]),
      http.get(`${BASE}/p/alpha/v1/scheduler/cron`, () =>
        HttpResponse.json([]),
      ),
    );

    renderScheduler();

    const clock = await screen.findByTestId("scheduler-clock");
    expect(clock).toHaveTextContent(/Clock on · 2 jobs · next wake in \d+s/);
  });

  it("targets the owning project prefix for run-now, pause, and delete", async () => {
    const mutationUrls: string[] = [];
    server.use(
      ...daemonHandlers([worker("alpha", "ready"), worker("beta", "ready")]),
      http.get(`${BASE}/p/alpha/v1/scheduler/cron`, () =>
        HttpResponse.json([cron("cron-a", "Alpha nightly")]),
      ),
      http.get(`${BASE}/p/beta/v1/scheduler/cron`, () =>
        HttpResponse.json([cron("cron-b", "Beta cleanup")]),
      ),
      http.post(
        `${BASE}/p/:projectId/v1/scheduler/cron/:id/run`,
        ({ request }) => {
          mutationUrls.push(`POST ${new URL(request.url).pathname}`);
          return HttpResponse.json({ id: "cron-b", triggered: true });
        },
      ),
      http.patch(
        `${BASE}/p/:projectId/v1/scheduler/cron/:id`,
        ({ request }) => {
          mutationUrls.push(`PATCH ${new URL(request.url).pathname}`);
          return HttpResponse.json({
            id: "cron-a",
            updated: true,
            human_schedule: "Hourly at :07",
          });
        },
      ),
      http.delete(
        `${BASE}/p/:projectId/v1/scheduler/cron/:id`,
        ({ request }) => {
          mutationUrls.push(`DELETE ${new URL(request.url).pathname}`);
          return HttpResponse.json({ removed: true });
        },
      ),
    );

    const view = renderScheduler();

    const betaRow = (await screen.findByText("Beta cleanup")).closest("li");
    expect(betaRow).not.toBeNull();
    await view.user.click(
      within(betaRow as HTMLElement).getByRole("button", { name: "Run now" }),
    );
    await waitFor(() =>
      expect(mutationUrls).toContain(
        "POST /p/beta/v1/scheduler/cron/cron-b/run",
      ),
    );

    const alphaRow = screen.getByText("Alpha nightly").closest("li");
    await view.user.click(
      within(alphaRow as HTMLElement).getByRole("button", { name: "Pause" }),
    );
    await waitFor(() =>
      expect(mutationUrls).toContain("PATCH /p/alpha/v1/scheduler/cron/cron-a"),
    );

    await view.user.click(
      within(alphaRow as HTMLElement).getByRole("button", { name: "Delete" }),
    );
    await waitFor(() =>
      expect(mutationUrls).toContain(
        "DELETE /p/alpha/v1/scheduler/cron/cron-a",
      ),
    );
  });

  it("creates a job against the selected project and refreshes its list", async () => {
    const createdBodies: { projectId: string; prompt?: string }[] = [];
    let betaListRequests = 0;
    server.use(
      ...daemonHandlers([worker("alpha", "stopped"), worker("beta", "ready")]),
      http.get(`${BASE}/p/beta/v1/scheduler/cron`, () => {
        betaListRequests += 1;
        return HttpResponse.json([]);
      }),
      http.post(
        `${BASE}/p/:projectId/v1/scheduler/cron`,
        async ({ params, request }) => {
          const body = (await request.json()) as { prompt?: string };
          createdBodies.push({
            projectId: String(params.projectId),
            prompt: body.prompt,
          });
          return HttpResponse.json({
            id: "new-cron",
            human_schedule: "Hourly at :07",
            recurring: true,
            durable: false,
            action_kind: "agent_turn",
            delivery_kind: "chat",
          });
        },
      ),
    );

    const view = renderScheduler();

    await view.user.click(
      await screen.findByRole("button", { name: "New job" }),
    );
    const dialog = await screen.findByRole("dialog");

    expect(
      within(dialog).getByRole("combobox", { name: "Project" }),
    ).toHaveTextContent("beta");

    const listRequestsBeforeCreate = betaListRequests;
    await view.user.type(
      within(dialog).getByRole("textbox", { name: "Prompt" }),
      "Nightly summary",
    );
    await view.user.click(
      within(dialog).getByRole("switch", { name: "Isolated session" }),
    );
    await view.user.type(
      within(dialog).getByRole("textbox", { name: "Description" }),
      "Nightly summary job",
    );
    await view.user.click(
      within(dialog).getByRole("button", { name: "Create" }),
    );

    await waitFor(() =>
      expect(createdBodies).toEqual([
        { projectId: "beta", prompt: "Nightly summary" },
      ]),
    );
    await waitFor(() =>
      expect(betaListRequests).toBeGreaterThan(listRequestsBeforeCreate),
    );
  });
});
