import { http, HttpResponse } from "msw";
import { QueryStatus } from "@reduxjs/toolkit/query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "../utils/test-utils";
import { emptyTasks, server } from "../utils/mockServer";
import { Dashboard } from "../features/Dashboard/Dashboard";
import { useSidebarSubscription } from "../hooks/useSidebarSubscription";
import { updateConfig } from "../features/Config/configSlice";
import { tasksApi, type TaskMeta } from "../services/refact/tasks";
import type { ChatHistoryItem } from "../features/History/historySlice";
import type { RootState } from "../app/store";

const CONFIG_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "web" as const,
    engineServed: true,
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

const SETTLED_HISTORY = {
  chats: {},
  isLoading: false,
  loadError: null,
  pagination: {
    cursor: null,
    hasMore: false,
    totalCount: null,
    generation: 0,
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

/**
 * Same fixture shape streamSelectors.test.ts builds: the stream selectors read
 * `history.chats` directly, so a plain ChatHistoryItem record is enough.
 */
function makeChat(
  partial: Partial<ChatHistoryItem> & { id: string },
): ChatHistoryItem {
  const now = new Date().toISOString();
  return {
    title: partial.title ?? `chat ${partial.id}`,
    model: "gpt-5",
    mode: "AGENT",
    tool_use: "agent",
    messages: [],
    boost_reasoning: false,
    include_project_info: true,
    increase_max_tokens: false,
    isTitleGenerated: false,
    createdAt: partial.createdAt ?? now,
    updatedAt: partial.updatedAt ?? now,
    last_user_message_id: "",
    session_state: "idle",
    message_count: 1,
    ...partial,
  } as ChatHistoryItem;
}

function historyWith(chats: ChatHistoryItem[]) {
  const byId: Record<string, ChatHistoryItem> = {};
  for (const chat of chats) byId[chat.id] = chat;
  return { ...SETTLED_HISTORY, chats: byId };
}

function fulfilledTasksApiState(tasks: TaskMeta[]) {
  return {
    queries: {
      "listTasks(undefined)": {
        status: QueryStatus.fulfilled,
        endpointName: "listTasks",
        error: undefined,
        originalArgs: undefined,
        requestId: "test",
        startedTimeStamp: Date.now(),
        data: tasks,
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
      invalidationBehavior: "delayed" as const,
    },
  } as unknown as RootState["tasksApi"];
}

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
      http.get("*/v1/setup/status", () =>
        HttpResponse.json({ configured: true }),
      ),
    );
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("shows the kit loading state before section snapshots arrive", () => {
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

    expect(screen.getByRole("status", { name: "Loading" })).toBeInTheDocument();
    // The stream feed (and therefore its empty copy) stays hidden while loading.
    expect(screen.queryByText("Nothing here yet.")).not.toBeInTheDocument();
  });

  it("opens an empty stream after all sidebar snapshots arrive", async () => {
    render(<Dashboard />, {
      preloadedState: {
        ...CONFIG_STATE,
        history: SETTLED_HISTORY,
        current_project: {
          name: "",
          workspaceRoots: [],
        },
        sidebar: READY_SIDEBAR,
      },
    });

    expect(await screen.findByText("Nothing here yet.")).toBeInTheDocument();
    expect(screen.queryByRole("status", { name: "Loading" })).not.toBe(
      screen.queryByText("Nothing here yet."),
    );
  });

  it("renders stream rows for chats and tasks once the sections are ready", async () => {
    render(<Dashboard />, {
      preloadedState: {
        ...CONFIG_STATE,
        history: historyWith([
          makeChat({ id: "chat-ready", title: "Ready workspace chat" }),
        ]),
        sidebar: READY_SIDEBAR,
        [tasksApi.reducerPath]: fulfilledTasksApiState([task]),
      },
    });

    expect(
      (await screen.findAllByRole("button", { name: /Ready workspace chat/ }))
        .length,
    ).toBeGreaterThan(0);
    expect(
      screen.getAllByRole("button", { name: /Progressive task/ }).length,
    ).toBeGreaterThan(0);
  });

  it("fetches tasks when the dashboard is ready without SSE data", async () => {
    const listHandler = vi.fn(() => HttpResponse.json([task]));
    server.use(http.get("*/v1/tasks", listHandler));

    render(<Dashboard />, {
      preloadedState: {
        ...CONFIG_STATE,
        history: SETTLED_HISTORY,
        sidebar: READY_SIDEBAR,
      },
    });

    expect(
      (await screen.findAllByRole("button", { name: /Progressive task/ }))
        .length,
    ).toBeGreaterThan(0);
    await waitFor(() => {
      expect(listHandler).toHaveBeenCalled();
    });
  });

  it("settles from predefined backend workspace snapshots", async () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation((key) =>
      key === "refact-trajectories-migrated" ? "true" : null,
    );
    server.use(
      http.get(
        "*/v1/sidebar/subscribe",
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
          ...SETTLED_HISTORY,
          isLoading: true,
        },
      },
    });

    await screen.findAllByRole("button", { name: /Predefined workspace task/ });

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
      screen.queryByRole("status", { name: "Loading" }),
    ).not.toBeInTheDocument();
  });

  it("keeps sidebar readiness after duplicate config with unchanged lsp port", async () => {
    const { store } = render(<Dashboard />, {
      preloadedState: {
        ...CONFIG_STATE,
        history: historyWith([
          makeChat({ id: "chat-stable", title: "Stable chat" }),
        ]),
        sidebar: READY_SIDEBAR,
      },
    });

    expect(
      (await screen.findAllByRole("button", { name: /Stable chat/ })).length,
    ).toBeGreaterThan(0);

    store.dispatch(updateConfig({ lspPort: 8001 }));

    expect(store.getState().sidebar.sections).toMatchObject({
      workspace: { status: "ready" },
      chats: { status: "ready" },
      tasks: { status: "ready" },
      buddy: { status: "ready" },
    });
    expect(
      screen.queryByRole("status", { name: "Loading" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getAllByRole("button", { name: /Stable chat/ }).length,
    ).toBeGreaterThan(0);
  });

  it("does not mask a ready stream while the workspace section is loading", async () => {
    render(<Dashboard />, {
      preloadedState: {
        ...CONFIG_STATE,
        history: SETTLED_HISTORY,
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

    expect(await screen.findByText("Nothing here yet.")).toBeInTheDocument();
    expect(
      screen.queryByRole("status", { name: "Loading" }),
    ).not.toBeInTheDocument();
  });

  it("does not mask a ready stream while the workspace section errored", async () => {
    render(<Dashboard />, {
      preloadedState: {
        ...CONFIG_STATE,
        history: SETTLED_HISTORY,
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

    expect(await screen.findByText("Nothing here yet.")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("keeps the loading state until chats are ready, even when tasks already are", async () => {
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
        [tasksApi.reducerPath]: fulfilledTasksApiState([task]),
      },
    });

    expect(
      await screen.findByRole("status", { name: "Loading" }),
    ).toBeInTheDocument();
    // Aggregate counters in the filter bar keep reporting the tasks that are
    // already known, even though the unified stream is still gated.
    expect(screen.getByRole("radio", { name: "Tasks 1" })).toBeInTheDocument();
    expect(screen.queryByText("Nothing here yet.")).not.toBeInTheDocument();
  });

  it("shows task load errors instead of a loading skeleton forever", () => {
    render(<Dashboard />, {
      preloadedState: {
        ...CONFIG_STATE,
        history: SETTLED_HISTORY,
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

    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent("Failed to load your workspace");
    expect(alert).toHaveTextContent("boom");
    expect(screen.queryByText("Nothing here yet.")).not.toBeInTheDocument();
  });

  it("shows chat load errors instead of a false empty state", () => {
    render(<Dashboard />, {
      preloadedState: {
        ...CONFIG_STATE,
        history: {
          ...SETTLED_HISTORY,
          loadError: "trajectory boom",
        },
        sidebar: READY_SIDEBAR,
      },
    });

    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent("Failed to load your workspace");
    expect(alert).toHaveTextContent("trajectory boom");
    expect(screen.queryByText("Nothing here yet.")).not.toBeInTheDocument();
  });
});

describe("Dashboard stream row deletion", () => {
  beforeEach(() => {
    server.use(
      emptyTasks,
      http.get("*/v1/setup/status", () =>
        HttpResponse.json({ configured: true }),
      ),
    );
  });

  function renderDashboardWithChat() {
    return render(<Dashboard />, {
      preloadedState: {
        ...CONFIG_STATE,
        history: historyWith([
          makeChat({ id: "chat-delete-test", title: "Chat to delete" }),
        ]),
        sidebar: READY_SIDEBAR,
      },
    });
  }

  it("requires confirmation before deleting a chat from the row peek", async () => {
    const { user } = renderDashboardWithChat();

    await user.click(
      await screen.findByTestId("stream-expand-chat-delete-test"),
    );

    await user.click(
      await screen.findByRole("button", { name: "Delete Chat to delete" }),
    );

    expect(await screen.findByText("Destructive action")).toBeInTheDocument();
    // Nothing happens until the destructive action is confirmed: the peek and
    // its Open action are still on screen.
    expect(screen.getByRole("button", { name: "Open" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Delete" }));

    await waitFor(() => {
      expect(
        screen.queryByRole("button", { name: "Open" }),
      ).not.toBeInTheDocument();
    });
  });

  it("closes delete confirmation without deleting when cancelled", async () => {
    const { user } = renderDashboardWithChat();

    await user.click(
      await screen.findByTestId("stream-expand-chat-delete-test"),
    );
    await user.click(
      await screen.findByRole("button", { name: "Delete Chat to delete" }),
    );
    expect(await screen.findByText("Destructive action")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Cancel" }));

    await waitFor(() => {
      expect(screen.queryByText("Destructive action")).not.toBeInTheDocument();
    });
    expect(screen.getByRole("button", { name: "Open" })).toBeInTheDocument();
    expect(
      screen.getAllByRole("button", { name: /Chat to delete/ }).length,
    ).toBeGreaterThan(0);
  });
});
