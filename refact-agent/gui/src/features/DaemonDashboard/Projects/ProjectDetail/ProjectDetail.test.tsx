import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";

import { setUpStore } from "../../../../app/store";
import type { DaemonWorker } from "../../../../services/refact/daemon";
import { server } from "../../../../utils/mockServer";
import { render, screen, waitFor, within } from "../../../../utils/test-utils";
import { daemonEventsReceived } from "../../dashboardSlice";
import { ProjectsPage } from "../ProjectsPage";
import { ProjectDetailPage } from "./ProjectDetailPage";

const config = {
  apiKey: "",
  host: "web" as const,
  lspPort: 8488,
  lspUrl: "https://daemon.example.test",
  surface: "dashboard" as const,
  themeProps: {},
};

const BASE = "https://daemon.example.test";
const PROJECT = "demo-project";

function worker(
  state: string,
  extra: Partial<DaemonWorker> = {},
): DaemonWorker {
  return {
    project_id: PROJECT,
    slug: PROJECT,
    root: `/work/${PROJECT}`,
    pinned: false,
    last_active_ms: 1,
    state,
    pid: state === "stopped" ? null : 10,
    rss_bytes: state === "ready" ? 209_715_200 : null,
    cpu_percent: state === "ready" ? 12.3 : null,
    uptime_secs: state === "ready" ? 4_000 : null,
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

const codeIntelOverview = {
  counts: { nodes: 120, edges: 340, files: 25 },
  index_state: { queued: 0, cross_file_edges: 340, cross_file_ready: true },
  scc_count: 1,
  largest_scc: 4,
  component_count: 2,
  top_pagerank: [],
  top_betweenness: [],
  file_centrality: { top_pagerank: [], top_betweenness: [] },
  community_count: 6,
  dead_code_count: 7,
};

const codeIntelHealth = {
  index_state: { queued: 0, cross_file_edges: 340, cross_file_ready: true },
  aggregate: {
    file_count: 25,
    function_count: 90,
    avg_score: 7.4,
    grade: "B",
    max_complexity: 14,
    avg_maintainability: 61.2,
    avg_duplication_pct: 4.5,
    biomarker_count: 11,
    refactoring_count: 3,
  },
  files: [
    {
      path: "src/messy.ts",
      lang: "ts",
      score: 3.1,
      grade: "D",
      complexity: 14,
      maintainability: 40,
      max_complexity: 14,
      avg_maintainability: 40,
      function_count: 9,
      duplication_pct: 12,
      dry_violation: true,
      defect_score: 4,
      maintainability_score: 3,
      performance_score: 6,
      biomarker_count: 6,
      refactoring_count: 2,
      functions: [],
      findings: [],
      refactorings: [],
    },
  ],
};

const gitStatus = {
  roots: [
    {
      root: `/work/${PROJECT}`,
      branch: "main",
      head_detached: false,
      ahead: 0,
      behind: 0,
      staged: [
        {
          relative_path: "a.ts",
          absolute_path: `/work/${PROJECT}/a.ts`,
          status: "MODIFIED",
        },
      ],
      unstaged: [],
      untracked_included: false,
    },
  ],
};

const gitLog = {
  roots: [
    {
      root: `/work/${PROJECT}`,
      commits: [
        {
          oid: "abcdef1234567890",
          short_oid: "abcdef1",
          time_ms: Date.now() - 60_000,
          author_name: "Ada",
          author_email: "ada@example.test",
          message_first_line: "feat: add detail page",
          message: "feat: add detail page",
        },
      ],
    },
  ],
};

const gitBranches = {
  roots: [
    {
      root: `/work/${PROJECT}`,
      current: "main",
      branches: [
        { name: "main", is_head: true, upstream: null },
        { name: "dev", is_head: false, upstream: null },
      ],
    },
  ],
};

const trajectories = [
  {
    id: "chat-1",
    title: "Fix the parser",
    created_at: "2026-07-18T00:00:00Z",
    updated_at: "2026-07-18T01:00:00Z",
    model: "claude",
    mode: "agent",
    message_count: 12,
  },
  {
    id: "chat-2",
    title: "Improve tests",
    created_at: "2026-07-17T00:00:00Z",
    updated_at: "2026-07-17T01:00:00Z",
    model: "claude",
    mode: "agent",
    message_count: 4,
  },
];

const tasks = [
  {
    id: "task-1",
    name: "Ship detail page",
    status: "active",
    created_at: "2026-07-18T00:00:00Z",
    updated_at: "2026-07-18T01:00:00Z",
    cards_total: 5,
    cards_done: 2,
    cards_failed: 0,
    agents_active: 1,
  },
  {
    id: "task-2",
    name: "Old idea",
    status: "planning",
    created_at: "2026-07-18T00:00:00Z",
    updated_at: "2026-07-18T00:30:00Z",
    cards_total: 0,
    cards_done: 0,
    cards_failed: 0,
    agents_active: 0,
  },
];

type ProxyCounters = {
  "rag-status": number;
  overview: number;
  health: number;
  "git-status": number;
  "git-log": number;
  "git-branches": number;
  trajectories: number;
  tasks: number;
};

function registerProxyHandlers(workers: () => DaemonWorker[]): ProxyCounters {
  const counters: ProxyCounters = {
    "rag-status": 0,
    overview: 0,
    health: 0,
    "git-status": 0,
    "git-log": 0,
    "git-branches": 0,
    trajectories: 0,
    tasks: 0,
  };
  const count = (key: keyof ProxyCounters) => {
    counters[key] += 1;
  };
  server.use(
    http.get(`${BASE}/daemon/v1/workers`, () => HttpResponse.json(workers())),
    http.get(`${BASE}/p/${PROJECT}/v1/rag-status`, () => {
      count("rag-status");
      return HttpResponse.json(ragStatus);
    }),
    http.get(`${BASE}/p/${PROJECT}/v1/code-intel/overview`, () => {
      count("overview");
      return HttpResponse.json(codeIntelOverview);
    }),
    http.get(`${BASE}/p/${PROJECT}/v1/code-intel/health`, () => {
      count("health");
      return HttpResponse.json(codeIntelHealth);
    }),
    http.get(`${BASE}/p/${PROJECT}/v1/git/status`, () => {
      count("git-status");
      return HttpResponse.json(gitStatus);
    }),
    http.get(`${BASE}/p/${PROJECT}/v1/git/log`, () => {
      count("git-log");
      return HttpResponse.json(gitLog);
    }),
    http.get(`${BASE}/p/${PROJECT}/v1/git/branches`, () => {
      count("git-branches");
      return HttpResponse.json(gitBranches);
    }),
    http.get(`${BASE}/p/${PROJECT}/v1/trajectories`, () => {
      count("trajectories");
      return HttpResponse.json(trajectories);
    }),
    http.get(`${BASE}/p/${PROJECT}/v1/tasks`, () => {
      count("tasks");
      return HttpResponse.json(tasks);
    }),
  );
  return counters;
}

function renderDetail() {
  const store = setUpStore({ config });
  const view = render(<ProjectDetailPage projectId={PROJECT} />, { store });
  return { store, view };
}

describe("ProjectDetailPage", () => {
  it("gates proxied content behind a stopped worker and starts it on demand", async () => {
    let restarted = false;
    registerProxyHandlers(() => [worker(restarted ? "ready" : "stopped")]);
    server.use(
      http.post(`${BASE}/daemon/v1/projects/${PROJECT}/restart`, () => {
        restarted = true;
        return HttpResponse.json({
          project_id: PROJECT,
          pid: 42,
          http_port: 8001,
          lsp_port: 9001,
          state: "ready",
        });
      }),
    );

    const { view } = renderDetail();

    expect(await screen.findByText("Worker is stopped")).toBeInTheDocument();
    expect(screen.getAllByText(`/work/${PROJECT}`).length).toBeGreaterThan(0);

    await view.user.click(screen.getByRole("button", { name: "Start worker" }));

    expect(await screen.findByText("Index brain")).toBeInTheDocument();
    expect(await screen.findByText("Code graph")).toBeInTheDocument();
    expect(restarted).toBe(true);
  });

  it("renders overview, health, git, chats, tasks, activity, and settings tabs from fixtures", async () => {
    registerProxyHandlers(() => [worker("ready")]);
    const { store, view } = renderDetail();
    store.dispatch(
      daemonEventsReceived([
        {
          seq: 1,
          ts_ms: Date.now(),
          kind: "worker_started",
          project_id: PROJECT,
          payload: {},
        },
        {
          seq: 2,
          ts_ms: Date.now(),
          kind: "other_project_event",
          project_id: "someone-else",
          payload: {},
        },
      ]),
    );

    expect(await screen.findByText("Index brain")).toBeInTheDocument();
    expect(screen.getByText("200 MB")).toBeInTheDocument();
    expect(screen.getByText("12.3%")).toBeInTheDocument();

    await view.user.click(screen.getByRole("tab", { name: "Health" }));
    expect(await screen.findByText("B")).toBeInTheDocument();
    expect(screen.getByText("4.5%")).toBeInTheDocument();
    const hotspots = await screen.findByLabelText("Health hotspots");
    expect(within(hotspots).getByText("src/messy.ts")).toBeInTheDocument();
    expect(await screen.findByText("7 candidates")).toBeInTheDocument();

    await view.user.click(screen.getByRole("tab", { name: "Git" }));
    expect(await screen.findByText("main")).toBeInTheDocument();
    expect(
      await screen.findByText("feat: add detail page"),
    ).toBeInTheDocument();

    await view.user.click(screen.getByRole("tab", { name: "Activity" }));
    expect(await screen.findByText("worker_started")).toBeInTheDocument();
    expect(screen.queryByText("other_project_event")).not.toBeInTheDocument();

    await view.user.click(screen.getByRole("tab", { name: "Chats" }));
    expect(await screen.findByText("Fix the parser")).toBeInTheDocument();
    const resumeLinks = screen.getAllByRole("link", { name: "Resume" });
    expect(resumeLinks).toHaveLength(2);
    expect(resumeLinks[0]).toHaveAttribute(
      "href",
      `${BASE}/p/${PROJECT}/?chat=chat-1`,
    );
    expect(resumeLinks[1]).toHaveAttribute(
      "href",
      `${BASE}/p/${PROJECT}/?chat=chat-2`,
    );

    await view.user.click(screen.getByRole("tab", { name: "Tasks" }));
    expect(await screen.findByText("Ship detail page")).toBeInTheDocument();
    expect(screen.getByText("2/5 cards · 1 agents")).toBeInTheDocument();
    expect(
      screen.getByRole("link", { name: "Open board" }),
    ).toBeInTheDocument();

    await view.user.click(screen.getByRole("tab", { name: "Settings" }));
    expect(await screen.findByText("Worker controls")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Forget" })).toBeInTheDocument();
    expect(
      screen.getByRole("link", { name: "daemon log tail" }),
    ).toBeInTheDocument();
  });

  it("fetches proxied tab data lazily on tab activation", async () => {
    const counters = registerProxyHandlers(() => [worker("ready")]);
    const { view } = renderDetail();

    expect(await screen.findByText("Index brain")).toBeInTheDocument();
    await waitFor(() => expect(counters["rag-status"]).toBe(1));
    expect(counters.health).toBe(0);
    expect(counters["git-status"]).toBe(0);
    expect(counters.trajectories).toBe(0);
    expect(counters.tasks).toBe(0);

    await view.user.click(screen.getByRole("tab", { name: "Health" }));
    await waitFor(() => expect(counters.health).toBe(1));
    expect(counters["git-status"]).toBe(0);

    await view.user.click(screen.getByRole("tab", { name: "Git" }));
    await waitFor(() => expect(counters["git-status"]).toBe(1));
  });

  it("navigates to the detail route from a project card title", async () => {
    server.use(
      http.get(`${BASE}/daemon/v1/workers`, () =>
        HttpResponse.json([worker("ready")]),
      ),
      http.get(`${BASE}/p/${PROJECT}/v1/rag-status`, () =>
        HttpResponse.json(ragStatus),
      ),
    );
    const store = setUpStore({ config });
    const view = render(<ProjectsPage />, { store });

    await view.user.click(
      await screen.findByRole("button", {
        name: `Open ${PROJECT} details`,
      }),
    );

    expect(store.getState().daemonDashboard.navigation).toEqual({
      page: "projects",
      params: { projectId: PROJECT },
    });
  });
});
