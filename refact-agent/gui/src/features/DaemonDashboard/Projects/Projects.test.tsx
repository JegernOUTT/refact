import { http, HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";

import { setUpStore } from "../../../app/store";
import type { DaemonWorker } from "../../../services/refact/daemon";
import { server } from "../../../utils/mockServer";
import { render, screen, waitFor, within } from "../../../utils/test-utils";
import { ProjectsPage } from "./ProjectsPage";
import { splitProjectWorkers } from "./projectOrdering";
import { fetchReadyProjectStatuses } from "./projectRagStatus";

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
    pid: state === "stopped" ? null : 10,
    http_port: state === "stopped" ? null : 8001,
    lsp_port: state === "stopped" ? null : 9001,
    lsp_clients: 2,
    busy_chats: 1,
    exec_running: 3,
    live_proxy_streams: 0,
    cron_next_fire_ms: null,
    idle_deadline_ms: null,
    last_status_report_ms: 1,
    last_error: null,
    ...extra,
  };
}

const ragStatus = {
  ast: null,
  ast_alive: "",
  vecdb: {
    files_unprocessed: 0,
    files_total: 12,
    requests_made_since_start: 1,
    vectors_made_since_start: 12,
    db_size: 1,
    db_cache_size: 1,
    state: "done",
    queue_additions: false,
    vecdb_max_files_hit: false,
    vecdb_errors: {},
  },
  vecdb_alive: "true",
  vec_db_error: "",
  codegraph: {
    counts: { nodes: 10, edges: 9, files: 3, fts_docs: 2 },
    queued: 0,
    state: "working",
    error: "",
  },
  codegraph_alive: "true",
  codegraph_error: "",
};

function renderProjects() {
  return render(<ProjectsPage />, {
    store: setUpStore({ config }),
  });
}

describe("Projects dashboard", () => {
  it("renders ready, crashed, and stopped workers and fetches indexes only for ready workers", async () => {
    const ragRequests: string[] = [];
    server.use(
      http.get("https://daemon.example.test/daemon/v1/workers", () =>
        HttpResponse.json([
          worker("ready-project", "ready", { pinned: true }),
          worker("crashed-project", "crashed", {
            last_error: "worker exited",
          }),
          worker("stopped-project", "stopped"),
        ]),
      ),
      http.get(
        "https://daemon.example.test/p/:projectId/v1/rag-status",
        ({ params }) => {
          ragRequests.push(String(params.projectId));
          return HttpResponse.json(ragStatus);
        },
      ),
    );

    renderProjects();

    const readyCard = await screen.findByLabelText("ready-project project");
    const crashedCard = screen.getByLabelText("crashed-project project");
    const stoppedCard = screen.getByLabelText("stopped-project project");
    expect(within(readyCard).getByText("Ready")).toBeInTheDocument();
    expect(within(crashedCard).getByText("Crashed")).toBeInTheDocument();
    expect(
      within(crashedCard).getByRole("button", { name: "Restart" }),
    ).toBeInTheDocument();
    expect(within(stoppedCard).getByText("Stopped")).toBeInTheDocument();
    expect(
      within(stoppedCard).getByText("Starts when you open it"),
    ).toBeInTheDocument();
    expect(within(readyCard).getByText("LSP 2")).toBeInTheDocument();
    expect(within(readyCard).getByText("Busy chats 1")).toBeInTheDocument();
    expect(within(readyCard).getByText("Exec 3")).toBeInTheDocument();
    expect(
      await within(readyCard).findByText("CodeGraph: working ✓"),
    ).toBeInTheDocument();
    expect(within(readyCard).getByText("VecDB: ready ✓")).toBeInTheDocument();
    expect(ragRequests).toEqual(["ready-project"]);
  });

  it("limits ready-worker index requests to three at a time", async () => {
    let activeRequests = 0;
    let maxActiveRequests = 0;
    let releaseRequests: (() => void) | undefined;
    const blocked = new Promise<void>((resolve) => {
      releaseRequests = resolve;
    });
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        activeRequests += 1;
        maxActiveRequests = Math.max(maxActiveRequests, activeRequests);
        await blocked;
        activeRequests -= 1;
        return new Response(JSON.stringify(ragStatus), {
          headers: { "Content-Type": "application/json" },
          status: 200,
        });
      }),
    );

    const request = fetchReadyProjectStatuses(
      "https://daemon.example.test",
      Array.from({ length: 5 }, (_, index) =>
        worker(`ready-${String(index)}`, "ready"),
      ),
    );
    await waitFor(() => expect(maxActiveRequests).toBe(3));
    releaseRequests?.();
    await request;

    expect(maxActiveRequests).toBe(3);
    vi.unstubAllGlobals();
  });

  it("shows the first-project hero for an empty registry", async () => {
    server.use(
      http.get("https://daemon.example.test/daemon/v1/workers", () =>
        HttpResponse.json([]),
      ),
    );

    renderProjects();

    expect(
      await screen.findByText("Add your first project"),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Add project" }),
    ).toBeInTheDocument();
  });

  it("browses, selects, and opens a project", async () => {
    const browsePaths: (string | undefined)[] = [];
    let openedRoot = "";
    server.use(
      http.get("https://daemon.example.test/daemon/v1/workers", () =>
        HttpResponse.json([]),
      ),
      http.post(
        "https://daemon.example.test/daemon/v1/fs/browse",
        async ({ request }) => {
          const body = (await request.json()) as { path?: string };
          browsePaths.push(body.path);
          if (body.path === "/home/refact") {
            return HttpResponse.json({
              path: "/home/refact",
              parent: "/home",
              dirs: [],
              can_open: true,
              truncated: false,
            });
          }
          return HttpResponse.json({
            path: "/home",
            parent: "/",
            dirs: [{ name: "refact", has_git: true }],
            can_open: true,
            truncated: false,
          });
        },
      ),
      http.post(
        "https://daemon.example.test/daemon/v1/projects/open",
        async ({ request }) => {
          const body = (await request.json()) as { root: string };
          openedRoot = body.root;
          return HttpResponse.json({
            project_id: "project-1",
            slug: "refact",
            root: body.root,
            pinned: false,
            worker: {
              project_id: "project-1",
              pid: 42,
              http_port: 8001,
              lsp_port: 9001,
              state: "ready",
            },
            cron_pending: null,
          });
        },
      ),
    );
    const view = renderProjects();

    await view.user.click(
      await screen.findByRole("button", { name: "Add project" }),
    );
    const dialog = await screen.findByRole("dialog");
    await view.user.click(
      await within(dialog).findByRole("button", { name: /refact/i }),
    );
    await waitFor(() => {
      expect(within(dialog).getByText("/home/refact")).toBeInTheDocument();
    });
    await view.user.click(
      within(dialog).getByRole("button", { name: "Select this folder" }),
    );
    expect(within(dialog).getByLabelText("Project path")).toHaveValue(
      "/home/refact",
    );
    await view.user.click(
      within(dialog).getByRole("button", { name: "Add project" }),
    );

    expect(await screen.findByLabelText("refact project")).toBeInTheDocument();
    expect(screen.getByText("Starting")).toBeInTheDocument();
    expect(browsePaths).toEqual([undefined, "/home/refact"]);
    expect(openedRoot).toBe("/home/refact");
  });

  it("renders the pin control as an icon-only button with an accessible label", async () => {
    server.use(
      http.get("https://daemon.example.test/daemon/v1/workers", () =>
        HttpResponse.json([worker("ready-project", "ready")]),
      ),
      http.get("https://daemon.example.test/p/:projectId/v1/rag-status", () =>
        HttpResponse.json(ragStatus),
      ),
    );

    renderProjects();

    const card = await screen.findByLabelText("ready-project project");
    const pinButton = within(card).getByRole("button", {
      name: "Pin ready-project",
    });
    expect(pinButton).toBeInTheDocument();
    expect(pinButton).toHaveTextContent("");
  });
});

describe("Project ordering", () => {
  it("orders pinned → ready → starting → stopped alphabetically and splits missing", () => {
    const { present, missing } = splitProjectWorkers([
      worker("zeta-stopped", "stopped"),
      worker("missing-b", "stopped", { root_exists: false }),
      worker("alpha-ready", "ready"),
      worker("pinned-z", "stopped", { pinned: true }),
      worker("beta-starting", "starting"),
      worker("missing-a", "ready", { root_exists: false }),
      worker("pinned-a", "ready", { pinned: true }),
      worker("alpha-stopped", "stopped"),
    ]);

    expect(present.map((entry) => entry.slug)).toEqual([
      "pinned-a",
      "pinned-z",
      "alpha-ready",
      "beta-starting",
      "alpha-stopped",
      "zeta-stopped",
    ]);
    expect(missing.map((entry) => entry.slug)).toEqual([
      "missing-a",
      "missing-b",
    ]);
  });

  it("treats workers without root_exists as present", () => {
    const { present, missing } = splitProjectWorkers([
      worker("legacy-daemon-row", "ready"),
    ]);

    expect(present).toHaveLength(1);
    expect(missing).toHaveLength(0);
  });
});

describe("Missing projects", () => {
  it("collapses a 49-project fixture to real cards plus a missing group", async () => {
    const realWorkers = Array.from({ length: 8 }, (_, index) =>
      worker(`real-${String(index)}`, index === 0 ? "ready" : "stopped"),
    );
    const missingWorkers = Array.from({ length: 41 }, (_, index) =>
      worker(`unittest-${String(index)}`, "stopped", { root_exists: false }),
    );
    server.use(
      http.get("https://daemon.example.test/daemon/v1/workers", () =>
        HttpResponse.json([
          ...missingWorkers.slice(0, 20),
          ...realWorkers,
          ...missingWorkers.slice(20),
        ]),
      ),
      http.get("https://daemon.example.test/p/:projectId/v1/rag-status", () =>
        HttpResponse.json(ragStatus),
      ),
    );

    renderProjects();

    await screen.findByLabelText("real-0 project");
    expect(screen.getAllByLabelText(/ project$/)).toHaveLength(8);
    const toggle = screen.getByRole("button", {
      name: /Missing projects \(41\)/,
    });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("/work/unittest-0")).not.toBeInTheDocument();
    expect(
      screen.queryByLabelText("unittest-0 project"),
    ).not.toBeInTheDocument();
  });

  it("expands missing projects and bulk-forgets them sequentially after confirmation", async () => {
    const forgotten: string[] = [];
    server.use(
      http.get("https://daemon.example.test/daemon/v1/workers", () =>
        HttpResponse.json([
          worker("real-project", "stopped"),
          worker("unittest-2", "stopped", { root_exists: false }),
          worker("unittest-0", "stopped", { root_exists: false }),
          worker("unittest-1", "stopped", { root_exists: false }),
        ]),
      ),
      http.delete(
        "https://daemon.example.test/daemon/v1/projects/:projectId",
        ({ params }) => {
          forgotten.push(String(params.projectId));
          return HttpResponse.json({ ok: true });
        },
      ),
    );

    const view = renderProjects();

    const toggle = await screen.findByRole("button", {
      name: /Missing projects \(3\)/,
    });
    await view.user.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("/work/unittest-0")).toBeInTheDocument();
    expect(screen.getByText("/work/unittest-1")).toBeInTheDocument();
    expect(screen.getByText("/work/unittest-2")).toBeInTheDocument();

    await view.user.click(
      screen.getByRole("button", { name: "Forget all missing" }),
    );
    const dialog = await screen.findByRole("dialog");
    expect(
      within(dialog).getByText("Forget 3 missing projects?"),
    ).toBeInTheDocument();
    await view.user.click(
      within(dialog).getByRole("button", { name: "Forget 3 projects" }),
    );

    await waitFor(() =>
      expect(forgotten).toEqual(["unittest-0", "unittest-1", "unittest-2"]),
    );
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );
  });
});
