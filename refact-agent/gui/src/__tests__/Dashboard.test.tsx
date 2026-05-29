import { http, HttpResponse } from "msw";
import { QueryStatus } from "@reduxjs/toolkit/query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "../utils/test-utils";
import { emptyTasks, server } from "../utils/mockServer";
import { Dashboard } from "../features/Dashboard/Dashboard";
import { useSidebarSubscription } from "../hooks/useSidebarSubscription";
import { updateConfig } from "../features/Config/configSlice";
import { tasksApi, type TaskMeta } from "../services/refact/tasks";

const CONFIG_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "web" as const,
  },
  connection: {
    browserOnline: true,
    backendStatus: "online" as const,
    backendLastOkAt: Date.now(),
    backendError: null,
    sseConnections: {},
  },
  current_project: {
    name: "refact-test",
    workspaceRoots: ["/tmp/refact-test"],
  },
};

const READY_SIDEBAR = {
  subscriptionId: "test-sidebar",
  lspPort: 8001,
  sections: {
    workspace: { status: "ready" as const, error: null },
    chats: { status: "ready" as const, error: null },
    tasks: { status: "ready" as const, error: null },
    buddy: { status: "ready" as const, error: null },
  },
};

const task: TaskMeta = {
  id: "task-1",
  name: "Progressive task",
  status: "active",
  created_at: "2024-01-01T00:00:00Z",
  updated_at: "2024-01-01T00:00:00Z",
  cards_total: 2,
  cards_done: 1,
  cards_failed: 0,
  agents_active: 0,
};

const predefinedTask: TaskMeta = {
  ...task,
  id: "task-predefined",
  name: "Predefined workspace task",
};

const predefinedChat = {
  id: "chat-predefined",
  title: "Predefined workspace chat",
  created_at: "2024-01-01T00:00:00Z",
  updated_at: "2024-01-01T00:00:00Z",
  model: "gpt-4",
  mode: "agent",
  message_count: 1,
  total_lines_added: 0,
  total_lines_removed: 0,
  tasks_total: 0,
  tasks_done: 0,
  tasks_failed: 0,
};

function envelope(seq: number, event: Record<string, unknown>) {
  return {
    protocol_version: 2,
    seq,
    subscription_id: "test-sidebar",
    event,
  };
}

function sectionSnapshot(
  seq: number,
  section: "workspace" | "chats" | "tasks" | "buddy",
  snapshot: Record<string, unknown>,
) {
  return envelope(seq, {
    type: "section_snapshot",
    section,
    status: "ready",
    snapshot,
  });
}

function sidebarSseStream(events: unknown[]): ReadableStream<Uint8Array> {
  const encoder = new TextEncoder();
  return new ReadableStream({
    start(controller) {
      for (const event of events) {
        controller.enqueue(
          encoder.encode(`data: ${JSON.stringify(event)}\n\n`),
        );
      }
    },
  });
}

function DashboardWithSidebarSubscription() {
  useSidebarSubscription();
  return <Dashboard />;
}

describe("Dashboard progressive sidebar readiness", () => {
  beforeEach(() => {
    server.use(
      emptyTasks,
      http.get("http://127.0.0.1:8001/v1/setup/status", () =>
        HttpResponse.json({ configured: true }),
      ),
    );
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("does not show empty states before section snapshots arrive", () => {
    render(<Dashboard />, {
      preloadedState: {
        ...CONFIG_STATE,
        sidebar: {
          subscriptionId: null,
          lspPort: 8001,
          sections: {
            workspace: { status: "ready", error: null },
            chats: { status: "loading", error: null },
            tasks: { status: "loading", error: null },
            buddy: { status: "loading", error: null },
          },
        },
      },
    });

    expect(screen.getAllByText("Loading").length).toBeGreaterThan(0);
    expect(screen.queryByText(/No chats yet/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/No tasks yet/i)).not.toBeInTheDocument();
  });

  it("opens an empty workspace after all sidebar snapshots arrive", async () => {
    render(<Dashboard />, {
      preloadedState: {
        ...CONFIG_STATE,
        history: {
          chats: {},
          isLoading: false,
          loadError: null,
          pagination: {
            cursor: null,
            hasMore: false,
            totalCount: null,
            generation: 0,
          },
        },
        current_project: {
          name: "",
          workspaceRoots: [],
        },
        sidebar: READY_SIDEBAR,
      },
    });

    expect(screen.getByText(/No chats yet/i)).toBeInTheDocument();
    expect(await screen.findByText(/No tasks yet/i)).toBeInTheDocument();
  });

  it("settles from predefined backend workspace snapshots", async () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation((key) =>
      key === "refact-trajectories-migrated" ? "true" : null,
    );
    server.use(
      http.get(
        "http://127.0.0.1:8001/v1/sidebar/subscribe",
        () =>
          new HttpResponse(
            sidebarSseStream([
              sectionSnapshot(0, "workspace", {
                workspace_roots: ["/workspace/predefined-refact"],
              }),
              sectionSnapshot(1, "chats", {
                trajectories: [predefinedChat],
              }),
              sectionSnapshot(2, "tasks", { tasks: [predefinedTask] }),
              sectionSnapshot(3, "buddy", { buddy: null }),
            ]),
            { headers: { "Content-Type": "text/event-stream" } },
          ),
      ),
    );

    const { store } = render(<DashboardWithSidebarSubscription />, {
      preloadedState: {
        ...CONFIG_STATE,
        history: {
          chats: {},
          isLoading: true,
          loadError: null,
          pagination: {
            cursor: null,
            hasMore: false,
            totalCount: null,
            generation: 0,
          },
        },
      },
    });

    await screen.findByText("Predefined workspace task");

    expect(store.getState().current_project).toEqual({
      name: "predefined-refact",
      workspaceRoots: ["/workspace/predefined-refact"],
    });
    expect(store.getState().sidebar.sections).toMatchObject({
      workspace: { status: "ready" },
      chats: { status: "ready" },
      tasks: { status: "ready" },
      buddy: { status: "ready" },
    });
    expect(store.getState().history.chats["chat-predefined"].title).toBe(
      "Predefined workspace chat",
    );
    expect(
      tasksApi.endpoints.listTasks.select(undefined)(store.getState()).data,
    ).toEqual([predefinedTask]);
    expect(screen.queryByText("Loading")).not.toBeInTheDocument();
  });

  it("keeps sidebar readiness after duplicate config with unchanged lsp port", async () => {
    const { store } = render(<Dashboard />, {
      preloadedState: {
        ...CONFIG_STATE,
        history: {
          chats: {},
          isLoading: false,
          loadError: null,
          pagination: {
            cursor: null,
            hasMore: false,
            totalCount: null,
            generation: 0,
          },
        },
        sidebar: READY_SIDEBAR,
      },
    });

    expect(screen.getByText(/No chats yet/i)).toBeInTheDocument();
    expect(await screen.findByText(/No tasks yet/i)).toBeInTheDocument();

    store.dispatch(updateConfig({ lspPort: 8001 }));

    expect(store.getState().sidebar.sections).toMatchObject({
      workspace: { status: "ready" },
      chats: { status: "ready" },
      tasks: { status: "ready" },
      buddy: { status: "ready" },
    });
    expect(screen.queryByText("Loading")).not.toBeInTheDocument();
    expect(screen.getByText(/No chats yet/i)).toBeInTheDocument();
    expect(screen.getByText(/No tasks yet/i)).toBeInTheDocument();
  });

  it("does not mask ready empty chats and tasks while workspace is loading", async () => {
    render(<Dashboard />, {
      preloadedState: {
        ...CONFIG_STATE,
        history: {
          chats: {},
          isLoading: false,
          loadError: null,
          pagination: {
            cursor: null,
            hasMore: false,
            totalCount: null,
            generation: 0,
          },
        },
        sidebar: {
          subscriptionId: "test-sidebar",
          lspPort: 8001,
          sections: {
            workspace: { status: "loading", error: null },
            chats: { status: "ready", error: null },
            tasks: { status: "ready", error: null },
            buddy: { status: "ready", error: null },
          },
        },
      },
    });

    expect(screen.getByText(/No chats yet/i)).toBeInTheDocument();
    expect(await screen.findByText(/No tasks yet/i)).toBeInTheDocument();
    expect(screen.queryByText("Loading")).not.toBeInTheDocument();
  });

  it("does not mask ready empty chats and tasks while workspace is error", async () => {
    render(<Dashboard />, {
      preloadedState: {
        ...CONFIG_STATE,
        history: {
          chats: {},
          isLoading: false,
          loadError: null,
          pagination: {
            cursor: null,
            hasMore: false,
            totalCount: null,
            generation: 0,
          },
        },
        sidebar: {
          subscriptionId: "test-sidebar",
          lspPort: 8001,
          sections: {
            workspace: { status: "error", error: "workspace boom" },
            chats: { status: "ready", error: null },
            tasks: { status: "ready", error: null },
            buddy: { status: "ready", error: null },
          },
        },
      },
    });

    expect(screen.getByText(/No chats yet/i)).toBeInTheDocument();
    expect(await screen.findByText(/No tasks yet/i)).toBeInTheDocument();
    expect(screen.queryByText("Loading")).not.toBeInTheDocument();
  });

  it("lets tasks become ready while chats are still loading", async () => {
    render(<Dashboard />, {
      preloadedState: {
        ...CONFIG_STATE,
        sidebar: {
          subscriptionId: "test-sidebar",
          lspPort: 8001,
          sections: {
            workspace: { status: "ready", error: null },
            chats: { status: "loading", error: null },
            tasks: { status: "ready", error: null },
            buddy: { status: "ready", error: null },
          },
        },
        [tasksApi.reducerPath]: {
          queries: {
            "listTasks(undefined)": {
              status: QueryStatus.fulfilled,
              endpointName: "listTasks",
              error: undefined,
              originalArgs: undefined,
              requestId: "test",
              startedTimeStamp: Date.now(),
              data: [task],
              fulfilledTimeStamp: Date.now(),
            },
          },
          mutations: {},
          provided: {
            Tasks: {},
            Board: {},
            TaskTrajectories: {},
          },
          subscriptions: {},
          config: {
            online: true,
            focused: true,
            middlewareRegistered: true,
            refetchOnFocus: false,
            refetchOnReconnect: false,
            refetchOnMountOrArgChange: false,
            keepUnusedDataFor: 60,
            reducerPath: tasksApi.reducerPath,
            invalidationBehavior: "delayed",
          },
        },
      },
    });

    expect(await screen.findByText("Progressive task")).toBeInTheDocument();
    expect(screen.getByText("CHATS")).toBeInTheDocument();
    expect(screen.queryByText(/No chats yet/i)).not.toBeInTheDocument();
  });

  it("does not repeatedly load the same chat page boundary", async () => {
    const listHandler = vi.fn(() =>
      HttpResponse.json({
        items: [],
        next_cursor: "next-page-2",
        has_more: true,
        total_count: 2,
      }),
    );
    server.use(http.get("http://127.0.0.1:8001/v1/trajectories", listHandler));

    render(<Dashboard />, {
      preloadedState: {
        ...CONFIG_STATE,
        history: {
          chats: {
            "chat-predefined": {
              id: "chat-predefined",
              title: "Predefined workspace chat",
              updatedAt: "2024-01-01T00:00:00Z",
              model: "gpt-4",
              mode: "agent",
              tool_use: "agent",
              messages: [],
              boost_reasoning: false,
              include_project_info: true,
              increase_max_tokens: false,
              createdAt: "2024-01-01T00:00:00Z",
              last_user_message_id: "",
              message_count: 1,
              total_lines_added: 0,
              total_lines_removed: 0,
              tasks_total: 0,
              tasks_done: 0,
              tasks_failed: 0,
            },
          },
          isLoading: false,
          loadError: null,
          pagination: {
            cursor: "next-page",
            hasMore: true,
            totalCount: 2,
            generation: 0,
          },
        },
        sidebar: READY_SIDEBAR,
      },
    });

    const calls = (globalThis as Record<string, unknown>).__VIRTUOSO_CALLS__ as
      | { endReached?: () => void }[]
      | undefined;
    const endReached = [...(calls ?? [])]
      .reverse()
      .find((call) => call.endReached)?.endReached;
    expect(endReached).toBeDefined();
    endReached?.();
    endReached?.();

    await screen.findByText("Predefined workspace chat");
    expect(listHandler).toHaveBeenCalledTimes(1);
  });

  it("shows task load errors instead of a loading skeleton forever", () => {
    render(<Dashboard />, {
      preloadedState: {
        ...CONFIG_STATE,
        sidebar: {
          subscriptionId: "test-sidebar",
          lspPort: 8001,
          sections: {
            workspace: { status: "ready", error: null },
            chats: { status: "ready", error: null },
            tasks: { status: "error", error: "boom" },
            buddy: { status: "ready", error: null },
          },
        },
      },
    });

    expect(screen.getByText("Failed to load tasks")).toBeInTheDocument();
    expect(screen.getByText("boom")).toBeInTheDocument();
    expect(screen.queryByText(/No tasks yet/i)).not.toBeInTheDocument();
  });

  it("shows chat load errors instead of a false empty state", () => {
    render(<Dashboard />, {
      preloadedState: {
        ...CONFIG_STATE,
        history: {
          chats: {},
          isLoading: false,
          loadError: "trajectory boom",
          pagination: {
            cursor: null,
            hasMore: false,
            totalCount: null,
            generation: 0,
          },
        },
        sidebar: READY_SIDEBAR,
      },
    });

    expect(screen.getByText("Failed to load chats")).toBeInTheDocument();
    expect(screen.getByText("trajectory boom")).toBeInTheDocument();
    expect(screen.queryByText(/No chats yet/i)).not.toBeInTheDocument();
  });
});
