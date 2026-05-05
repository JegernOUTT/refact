import React from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { Provider } from "react-redux";
import { http, HttpResponse } from "msw";

import { setUpStore } from "../app/store";
import { useSidebarSubscription } from "../hooks/useSidebarSubscription";
import { server } from "../utils/mockServer";
import { setCurrentProjectInfo } from "../features/Chat/currentProject";
import { updateConfig } from "../features/Config/configSlice";
import { tasksApi } from "../services/refact/tasks";

function envelope(
  seq: number,
  event: Record<string, unknown>,
  subscriptionId = "test-sidebar",
) {
  return {
    protocol_version: 2,
    seq,
    subscription_id: subscriptionId,
    event,
  };
}

function sectionSnapshot(
  seq: number,
  section: "workspace" | "chats" | "tasks" | "buddy",
  snapshot: Record<string, unknown>,
  status: "ready" | "error" = "ready",
  error?: string,
  subscriptionId?: string,
) {
  return envelope(
    seq,
    {
      type: "section_snapshot",
      section,
      status,
      snapshot,
      ...(error ? { error } : {}),
    },
    subscriptionId,
  );
}

function sidebarSnapshotHandler(...events: Record<string, unknown>[]) {
  return http.get("http://127.0.0.1:8001/v1/sidebar/subscribe", () => {
    const encoder = new TextEncoder();
    const stream = new ReadableStream({
      start(controller) {
        for (const event of events) {
          controller.enqueue(
            encoder.encode(`data: ${JSON.stringify(event)}\n\n`),
          );
        }
      },
    });

    return new HttpResponse(stream, {
      headers: {
        "Content-Type": "text/event-stream",
        "Cache-Control": "no-cache",
        Connection: "keep-alive",
      },
    });
  });
}

function sidebarDelayedSnapshotHandler(
  delayMs: number,
  ...events: Record<string, unknown>[]
) {
  return http.get("http://127.0.0.1:8001/v1/sidebar/subscribe", () => {
    const encoder = new TextEncoder();
    let cancelled = false;
    const stream = new ReadableStream({
      start(controller) {
        void (async () => {
          for (const event of events) {
            if (cancelled) break;
            try {
              controller.enqueue(
                encoder.encode(`data: ${JSON.stringify(event)}\n\n`),
              );
            } catch {
              break;
            }
            await new Promise((resolve) => setTimeout(resolve, delayMs));
          }
        })();
      },
      cancel() {
        cancelled = true;
      },
    });

    return new HttpResponse(stream, {
      headers: {
        "Content-Type": "text/event-stream",
        "Cache-Control": "no-cache",
        Connection: "keep-alive",
      },
    });
  });
}

function taskMeta(
  id: string,
  name: string,
): {
  id: string;
  name: string;
  status: "planning";
  created_at: string;
  updated_at: string;
  cards_total: number;
  cards_done: number;
  cards_failed: number;
  agents_active: number;
} {
  return {
    id,
    name,
    status: "planning",
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    cards_total: 0,
    cards_done: 0,
    cards_failed: 0,
    agents_active: 0,
  };
}

function renderSidebarSubscription(
  preloadedState: Parameters<typeof setUpStore>[0] = {},
) {
  const store = setUpStore({
    config: {
      apiKey: "test",
      lspPort: 8001,
      themeProps: {},
      host: "vscode",
    },
    ...preloadedState,
  });

  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <Provider store={store}>{children}</Provider>
  );

  renderHook(() => useSidebarSubscription(), { wrapper });

  return store;
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("useSidebarSubscription", () => {
  it("keeps local project info while waiting for an explicit workspace snapshot", async () => {
    server.use(
      sidebarSnapshotHandler(
        sectionSnapshot(0, "chats", { trajectories: [] }),
        sectionSnapshot(1, "tasks", { tasks: [] }),
        sectionSnapshot(2, "buddy", { buddy: null }),
      ),
    );

    const store = renderSidebarSubscription({
      current_project: {
        name: "local-refact",
        workspaceRoots: ["/local/refact"],
      },
    });

    await waitFor(() => {
      expect(store.getState().sidebar.sections.chats.status).toBe("ready");
      expect(store.getState().sidebar.sections.tasks.status).toBe("ready");
    });

    expect(store.getState().current_project).toEqual({
      name: "local-refact",
      workspaceRoots: ["/local/refact"],
    });
    expect(store.getState().sidebar.sections.workspace.status).toBe("loading");
  });

  it("accepts an explicit empty server workspace snapshot as loaded", async () => {
    server.use(
      sidebarSnapshotHandler(
        sectionSnapshot(0, "workspace", { workspace_roots: [] }),
        sectionSnapshot(1, "chats", { trajectories: [] }),
        sectionSnapshot(2, "tasks", { tasks: [] }),
        sectionSnapshot(3, "buddy", { buddy: null }),
      ),
    );

    const store = renderSidebarSubscription();

    await waitFor(() => {
      expect(store.getState().current_project).toEqual({
        name: "",
        workspaceRoots: [],
      });
      expect(store.getState().sidebar.sections.workspace.status).toBe("ready");
      expect(store.getState().sidebar.sections.chats.status).toBe("ready");
      expect(store.getState().sidebar.sections.tasks.status).toBe("ready");
      expect(store.getState().sidebar.sections.buddy.status).toBe("ready");
    });
  });

  it("keeps sidebar readiness for workspace-name-only config updates", async () => {
    server.use(
      sidebarSnapshotHandler(
        sectionSnapshot(0, "workspace", {
          workspace_roots: ["/workspace/refact"],
        }),
        sectionSnapshot(1, "chats", { trajectories: [] }),
        sectionSnapshot(2, "tasks", { tasks: [] }),
        sectionSnapshot(3, "buddy", { buddy: null }),
      ),
    );

    const store = renderSidebarSubscription({
      config: {
        apiKey: "test",
        lspPort: 8001,
        themeProps: {},
        host: "vscode",
        currentWorkspaceName: "refact",
      },
    });

    await waitFor(() => {
      expect(store.getState().sidebar.sections.chats.status).toBe("ready");
      expect(store.getState().sidebar.sections.tasks.status).toBe("ready");
    });

    store.dispatch(updateConfig({ currentWorkspaceName: "renamed-refact" }));

    expect(store.getState().sidebar.sections).toMatchObject({
      workspace: { status: "ready" },
      chats: { status: "ready" },
      tasks: { status: "ready" },
      buddy: { status: "ready" },
    });
  });

  it("resets section readiness and cached tasks when the same subscription switches workspace", async () => {
    server.use(
      sidebarSnapshotHandler(
        sectionSnapshot(0, "workspace", {
          workspace_roots: ["/workspace/old"],
        }),
        sectionSnapshot(1, "chats", { trajectories: [] }),
        sectionSnapshot(2, "tasks", { tasks: [taskMeta("old", "Old task")] }),
        sectionSnapshot(3, "buddy", { buddy: null }),
        sectionSnapshot(4, "workspace", {
          workspace_roots: ["/workspace/new"],
        }),
      ),
    );

    const store = renderSidebarSubscription();

    await waitFor(() => {
      expect(store.getState().current_project).toEqual({
        name: "new",
        workspaceRoots: ["/workspace/new"],
      });
      expect(store.getState().sidebar.sections.workspace.status).toBe("ready");
    });

    expect(store.getState().sidebar.sections.chats.status).toBe("loading");
    expect(store.getState().sidebar.sections.tasks.status).toBe("loading");
    expect(store.getState().sidebar.sections.buddy.status).toBe("loading");

    expect(
      tasksApi.endpoints.listTasks.select(undefined)(store.getState()).data,
    ).toEqual([]);
  });

  it("settles after a same-subscription workspace switch receives replacement section snapshots", async () => {
    server.use(
      sidebarSnapshotHandler(
        sectionSnapshot(0, "workspace", {
          workspace_roots: ["/workspace/old"],
        }),
        sectionSnapshot(1, "chats", { trajectories: [] }),
        sectionSnapshot(2, "tasks", { tasks: [taskMeta("old", "Old task")] }),
        sectionSnapshot(3, "buddy", { buddy: null }),
        sectionSnapshot(4, "workspace", {
          workspace_roots: ["/workspace/new"],
        }),
        sectionSnapshot(5, "chats", { trajectories: [] }),
        sectionSnapshot(6, "tasks", { tasks: [taskMeta("new", "New task")] }),
        sectionSnapshot(7, "buddy", { buddy: null }),
      ),
    );

    const store = renderSidebarSubscription();

    await waitFor(() => {
      expect(store.getState().current_project).toEqual({
        name: "new",
        workspaceRoots: ["/workspace/new"],
      });
      expect(store.getState().sidebar.subscriptionId).toBe("test-sidebar");
      expect(store.getState().sidebar.sections).toMatchObject({
        workspace: { status: "ready" },
        chats: { status: "ready" },
        tasks: { status: "ready" },
        buddy: { status: "ready" },
      });
      expect(
        tasksApi.endpoints.listTasks.select(undefined)(store.getState()).data,
      ).toEqual([taskMeta("new", "New task")]);
      expect(store.getState().history.isLoading).toBe(false);
    });
  });

  it("does not poison canonical roots with workspace error snapshots", async () => {
    server.use(
      sidebarSnapshotHandler(
        sectionSnapshot(0, "workspace", {
          workspace_roots: ["/workspace/refact"],
        }),
        sectionSnapshot(1, "chats", { trajectories: [] }),
        sectionSnapshot(2, "tasks", { tasks: [] }),
        sectionSnapshot(3, "buddy", { buddy: null }),
        sectionSnapshot(
          4,
          "workspace",
          { workspace_roots: [] },
          "error",
          "workspace timeout",
        ),
        sectionSnapshot(5, "workspace", {
          workspace_roots: ["/workspace/refact"],
        }),
      ),
    );

    const store = renderSidebarSubscription();

    await waitFor(() => {
      expect(store.getState().sidebar.sections.workspace.status).toBe("ready");
      expect(store.getState().sidebar.sections.workspace.error).toBeNull();
      expect(store.getState().sidebar.sections.chats.status).toBe("ready");
      expect(store.getState().sidebar.sections.tasks.status).toBe("ready");
      expect(store.getState().sidebar.sections.buddy.status).toBe("ready");
      expect(store.getState().current_project).toEqual({
        name: "refact",
        workspaceRoots: ["/workspace/refact"],
      });
    });
  });

  it("does not reset readiness for normalized-equivalent workspace snapshots", async () => {
    server.use(
      sidebarSnapshotHandler(
        sectionSnapshot(0, "workspace", {
          workspace_roots: ["/workspace/refact/"],
        }),
        sectionSnapshot(1, "chats", { trajectories: [] }),
        sectionSnapshot(2, "tasks", { tasks: [] }),
        sectionSnapshot(3, "buddy", { buddy: null }),
        sectionSnapshot(4, "workspace", {
          workspace_roots: ["/workspace/refact"],
        }),
      ),
    );

    const store = renderSidebarSubscription();

    await waitFor(() => {
      expect(store.getState().sidebar.sections.workspace.status).toBe("ready");
      expect(store.getState().sidebar.sections.chats.status).toBe("ready");
      expect(store.getState().sidebar.sections.tasks.status).toBe("ready");
      expect(store.getState().sidebar.sections.buddy.status).toBe("ready");
    });
  });

  it("does not reset readiness for reordered duplicate equivalent roots", async () => {
    server.use(
      sidebarSnapshotHandler(
        sectionSnapshot(0, "workspace", {
          workspace_roots: ["/workspace/first", "/workspace/second"],
        }),
        sectionSnapshot(1, "chats", { trajectories: [] }),
        sectionSnapshot(2, "tasks", { tasks: [taskMeta("old", "Old task")] }),
        sectionSnapshot(3, "buddy", { buddy: null }),
        sectionSnapshot(4, "workspace", {
          workspace_roots: [
            "/workspace/second/",
            "/workspace/first",
            "/workspace/second",
          ],
        }),
      ),
    );

    const store = renderSidebarSubscription();

    await waitFor(() => {
      expect(store.getState().sidebar.sections).toMatchObject({
        workspace: { status: "ready" },
        chats: { status: "ready" },
        tasks: { status: "ready" },
        buddy: { status: "ready" },
      });
      expect(
        tasksApi.endpoints.listTasks.select(undefined)(store.getState()).data,
      ).toEqual([taskMeta("old", "Old task")]);
    });
  });

  it("preserves task cache on task errors and replaces it on ready retry", async () => {
    server.use(
      sidebarDelayedSnapshotHandler(
        250,
        sectionSnapshot(0, "workspace", {
          workspace_roots: ["/workspace/refact"],
        }),
        sectionSnapshot(1, "chats", { trajectories: [] }),
        sectionSnapshot(2, "tasks", { tasks: [taskMeta("old", "Old task")] }),
        sectionSnapshot(3, "buddy", { buddy: null }),
        sectionSnapshot(4, "tasks", { tasks: [] }, "error", "task timeout"),
        sectionSnapshot(5, "tasks", { tasks: [taskMeta("new", "New task")] }),
      ),
    );

    const store = renderSidebarSubscription();

    await waitFor(
      () => {
        expect(store.getState().sidebar.sections.tasks).toEqual({
          status: "error",
          error: "task timeout",
        });
        expect(
          tasksApi.endpoints.listTasks.select(undefined)(store.getState()).data,
        ).toEqual([taskMeta("old", "Old task")]);
      },
      { timeout: 2_000, interval: 20 },
    );

    await waitFor(
      () => {
        expect(store.getState().sidebar.sections.tasks).toEqual({
          status: "ready",
          error: null,
        });
        expect(
          tasksApi.endpoints.listTasks.select(undefined)(store.getState()).data,
        ).toEqual([taskMeta("new", "New task")]);
      },
      { timeout: 2_000, interval: 20 },
    );
  });

  it("does not reset roots after workspace error recovers with equivalent roots", async () => {
    server.use(
      sidebarSnapshotHandler(
        sectionSnapshot(0, "workspace", {
          workspace_roots: ["/workspace/first", "/workspace/second"],
        }),
        sectionSnapshot(1, "chats", { trajectories: [] }),
        sectionSnapshot(2, "tasks", { tasks: [taskMeta("old", "Old task")] }),
        sectionSnapshot(3, "buddy", { buddy: null }),
        sectionSnapshot(
          4,
          "workspace",
          { workspace_roots: ["/workspace/other"] },
          "error",
          "workspace timeout",
        ),
        sectionSnapshot(5, "workspace", {
          workspace_roots: ["/workspace/second/", "/workspace/first"],
        }),
      ),
    );

    const store = renderSidebarSubscription();

    await waitFor(() => {
      expect(store.getState().sidebar.sections).toMatchObject({
        workspace: { status: "ready", error: null },
        chats: { status: "ready" },
        tasks: { status: "ready" },
        buddy: { status: "ready" },
      });
      expect(
        tasksApi.endpoints.listTasks.select(undefined)(store.getState()).data,
      ).toEqual([taskMeta("old", "Old task")]);
    });
  });

  it("keeps out-of-order new section snapshots after a same-port workspace switch", async () => {
    server.use(
      sidebarSnapshotHandler(
        sectionSnapshot(0, "workspace", {
          workspace_roots: ["/workspace/old"],
        }),
        sectionSnapshot(1, "chats", { trajectories: [] }),
        sectionSnapshot(2, "tasks", { tasks: [taskMeta("old", "Old task")] }),
        sectionSnapshot(3, "buddy", { buddy: null }),
        sectionSnapshot(4, "chats", { trajectories: [] }),
        sectionSnapshot(5, "tasks", { tasks: [taskMeta("new", "New task")] }),
        sectionSnapshot(6, "buddy", { buddy: null }),
        sectionSnapshot(7, "workspace", {
          workspace_roots: ["/workspace/new"],
        }),
      ),
    );

    const store = renderSidebarSubscription();

    await waitFor(() => {
      expect(store.getState().current_project).toEqual({
        name: "new",
        workspaceRoots: ["/workspace/new"],
      });
      expect(store.getState().sidebar.sections).toMatchObject({
        workspace: { status: "ready" },
        chats: { status: "ready" },
        tasks: { status: "ready" },
        buddy: { status: "ready" },
      });
      expect(
        tasksApi.endpoints.listTasks.select(undefined)(store.getState()).data,
      ).toEqual([taskMeta("new", "New task")]);
      expect(store.getState().history.isLoading).toBe(false);
    });
  });

  it("keeps history loading false after an empty chat snapshot", async () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation((key) =>
      key === "refact-trajectories-migrated" ? "true" : null,
    );
    server.use(
      sidebarSnapshotHandler(
        sectionSnapshot(0, "workspace", {
          workspace_roots: ["/workspace/refact"],
        }),
        sectionSnapshot(1, "chats", { trajectories: [] }),
      ),
    );

    const store = renderSidebarSubscription();

    await waitFor(() => {
      expect(store.getState().history.isLoading).toBe(false);
    });
  });

  it("does not return to project loading when local IDE project info matches the server snapshot", async () => {
    server.use(
      sidebarSnapshotHandler(
        sectionSnapshot(0, "workspace", {
          workspace_roots: ["/workspace/refact"],
        }),
      ),
    );

    const store = renderSidebarSubscription();

    await waitFor(() => {
      expect(store.getState().sidebar.sections.workspace.status).toBe("ready");
    });

    store.dispatch(
      setCurrentProjectInfo({
        name: "refact",
        workspaceRoots: ["/workspace/refact"],
      }),
    );

    expect(store.getState().sidebar.sections.workspace.status).toBe("ready");
  });

  it("tracks changed local IDE project info separately from sidebar section status", async () => {
    server.use(
      sidebarSnapshotHandler(
        sectionSnapshot(0, "workspace", {
          workspace_roots: ["/workspace/refact"],
        }),
      ),
    );

    const store = renderSidebarSubscription();

    await waitFor(() => {
      expect(store.getState().sidebar.sections.workspace.status).toBe("ready");
    });

    store.dispatch(
      setCurrentProjectInfo({
        name: "other-project",
        workspaceRoots: ["/workspace/other-project"],
      }),
    );

    expect(store.getState().current_project).toEqual({
      name: "other-project",
      workspaceRoots: ["/workspace/other-project"],
    });
    expect(store.getState().sidebar.sections.workspace.status).toBe("ready");
  });
});
