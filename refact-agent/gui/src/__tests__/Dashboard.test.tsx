import { http, HttpResponse } from "msw";
import { QueryStatus } from "@reduxjs/toolkit/query";
import { beforeEach, describe, expect, it } from "vitest";
import { render, screen } from "../utils/test-utils";
import { emptyTasks, server } from "../utils/mockServer";
import { Dashboard } from "../features/Dashboard/Dashboard";
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

describe("Dashboard progressive sidebar readiness", () => {
  beforeEach(() => {
    server.use(
      emptyTasks,
      http.get("http://127.0.0.1:8001/v1/setup/status", () =>
        HttpResponse.json({ configured: true }),
      ),
    );
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
          pagination: { cursor: null, hasMore: false },
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

  it("keeps sidebar readiness after duplicate config with unchanged lsp port", async () => {
    const { store } = render(<Dashboard />, {
      preloadedState: {
        ...CONFIG_STATE,
        history: {
          chats: {},
          isLoading: false,
          loadError: null,
          pagination: { cursor: null, hasMore: false },
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
          pagination: { cursor: null, hasMore: false },
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
          pagination: { cursor: null, hasMore: false },
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
          pagination: { cursor: null, hasMore: false },
        },
        sidebar: READY_SIDEBAR,
      },
    });

    expect(screen.getByText("Failed to load chats")).toBeInTheDocument();
    expect(screen.getByText("trajectory boom")).toBeInTheDocument();
    expect(screen.queryByText(/No chats yet/i)).not.toBeInTheDocument();
  });
});
