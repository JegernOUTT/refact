import { http, HttpResponse } from "msw";
import { beforeEach, describe, expect, it } from "vitest";
import { render, screen, waitFor } from "../utils/test-utils";
import { emptyTasks, server } from "../utils/mockServer";
import { Dashboard } from "../features/Dashboard/Dashboard";
import type { TaskMeta } from "../services/refact/tasks";

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
      preloadedState: CONFIG_STATE,
    });

    expect(screen.getAllByText("Loading").length).toBeGreaterThan(0);
    expect(screen.queryByText(/No chats yet/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/No tasks yet/i)).not.toBeInTheDocument();
  });

  it("lets tasks become ready while chats are still loading", async () => {
    server.use(
      http.get("http://127.0.0.1:8001/v1/tasks", () =>
        HttpResponse.json([task]),
      ),
    );

    render(<Dashboard />, {
      preloadedState: {
        ...CONFIG_STATE,
        current_project: {
          name: "refact-test",
          workspaceRoots: ["/tmp/refact-test"],
          tasksSnapshotReceived: true,
          trajectoriesSnapshotReceived: false,
        },
      },
    });

    expect(await screen.findByText("Progressive task")).toBeInTheDocument();
    expect(screen.getByText("CHATS")).toBeInTheDocument();
    expect(screen.queryByText(/No chats yet/i)).not.toBeInTheDocument();
  });

  it("shows task load errors instead of a loading skeleton forever", async () => {
    server.use(
      http.get("http://127.0.0.1:8001/v1/tasks", () =>
        HttpResponse.json({ detail: "boom" }, { status: 500 }),
      ),
    );

    render(<Dashboard />, {
      preloadedState: {
        ...CONFIG_STATE,
        current_project: {
          name: "refact-test",
          workspaceRoots: ["/tmp/refact-test"],
          tasksSnapshotReceived: true,
          trajectoriesSnapshotReceived: true,
        },
      },
    });

    await waitFor(() => {
      expect(screen.getByText("Failed to load tasks")).toBeInTheDocument();
    });
    expect(screen.queryByText(/No tasks yet/i)).not.toBeInTheDocument();
  });
});
