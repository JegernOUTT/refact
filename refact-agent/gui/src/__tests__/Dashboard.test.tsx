import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { render, screen } from "../utils/test-utils";
import { Dashboard } from "../features/Dashboard/Dashboard";
import { setUpStore, type RootState } from "../app/store";
import { setHistoryLoading } from "../features/History/historySlice";
import { tasksApi } from "../services/refact/tasks";
import {
  markProjectHistorySnapshotReceived,
  markProjectTasksSnapshotReceived,
  setCurrentProjectInfo,
} from "../features/Chat/currentProject";
import { server } from "../utils/mockServer";

const CONFIG_STATE: Partial<RootState> = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode",
    currentWorkspaceName: "refact-test",
  },
  connection: {
    browserOnline: true,
    backendStatus: "online",
    backendLastOkAt: Date.now(),
    backendError: null,
    sseConnections: {},
  },
};

function mockSetupStatus() {
  server.use(
    http.get("http://127.0.0.1:8001/v1/setup/status", () =>
      HttpResponse.json({ configured: true }),
    ),
  );
}

function mockEmptyTasks() {
  server.use(
    http.get("http://127.0.0.1:8001/v1/tasks", () => HttpResponse.json([])),
  );
}

describe("Dashboard startup loading", () => {
  it("keeps chats and tasks loading until the sidebar snapshot is processed", () => {
    mockSetupStatus();
    mockEmptyTasks();

    render(<Dashboard />, { preloadedState: CONFIG_STATE });

    expect(screen.getAllByText("Loading").length).toBeGreaterThanOrEqual(2);
    expect(screen.queryByText("No chats yet — start a new one!")).toBeNull();
    expect(screen.queryByText("No tasks yet — start a new one!")).toBeNull();
    expect(screen.queryByText("0 total")).toBeNull();
  });

  it("lets tasks become ready while chats are still loading", async () => {
    mockEmptyTasks();
    mockSetupStatus();

    const store = setUpStore({ ...CONFIG_STATE });
    void store.dispatch(
      tasksApi.util.upsertQueryData("listTasks", undefined, []),
    );
    store.dispatch(markProjectTasksSnapshotReceived());

    render(<Dashboard />, { store });

    expect(screen.getByText("TASKS")).toBeInTheDocument();
    expect(
      await screen.findByText("No tasks yet — start a new one!"),
    ).toBeInTheDocument();
    expect(screen.getByText("CHATS")).toBeInTheDocument();
    expect(screen.queryByText("No chats yet — start a new one!")).toBeNull();
  });

  it("shows real empty states only after the sidebar snapshot is processed", async () => {
    mockEmptyTasks();
    mockSetupStatus();

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(
      setCurrentProjectInfo({
        name: "refact-test",
        workspaceRoots: ["/tmp/refact-test"],
      }),
    );
    store.dispatch(setHistoryLoading(false));
    store.dispatch(markProjectHistorySnapshotReceived());
    void store.dispatch(
      tasksApi.util.upsertQueryData("listTasks", undefined, []),
    );
    store.dispatch(markProjectTasksSnapshotReceived());

    render(<Dashboard />, { store });

    expect(
      await screen.findByText("No chats yet — start a new one!"),
    ).toBeInTheDocument();
    expect(
      await screen.findByText("No tasks yet — start a new one!"),
    ).toBeInTheDocument();
    expect(screen.getAllByText("0 total").length).toBeGreaterThanOrEqual(2);
  });
});
