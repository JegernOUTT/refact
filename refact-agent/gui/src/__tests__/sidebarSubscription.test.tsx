import { http, HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";
import { render, waitFor } from "../utils/test-utils";
import { server } from "../utils/mockServer";
import { useSidebarSubscription } from "../hooks/useSidebarSubscription";
import { setBuddySnapshot } from "../features/Buddy/buddySlice";
import type { BuddySnapshot } from "../features/Buddy/types";
import type { TaskMeta } from "../services/refact/tasks";

const CONFIG_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "web" as const,
  },
};

function TestHarness() {
  useSidebarSubscription();
  return null;
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
  status: "ready" | "error" = "ready",
  error?: string,
) {
  return envelope(seq, {
    type: "section_snapshot",
    section,
    status,
    snapshot,
    ...(error ? { error } : {}),
  });
}

function sectionUpdate(
  seq: number,
  section: "chats" | "tasks" | "buddy",
  update: Record<string, unknown>,
) {
  return envelope(seq, {
    type: "section_update",
    section,
    update,
  });
}

function notification(seq: number, payload: Record<string, unknown>) {
  return envelope(seq, {
    type: "notification",
    notification: payload,
  });
}

function sseStream(events: unknown[]): ReadableStream<Uint8Array> {
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

const taskA: TaskMeta = {
  id: "task-a",
  name: "Task A",
  status: "active",
  created_at: "2024-01-01T00:00:00Z",
  updated_at: "2024-01-01T00:00:00Z",
  cards_total: 1,
  cards_done: 0,
  cards_failed: 0,
  agents_active: 0,
};

const taskB: TaskMeta = {
  ...taskA,
  id: "task-b",
  name: "Task B",
  updated_at: "2024-01-02T00:00:00Z",
};

const taskC: TaskMeta = {
  ...taskA,
  id: "task-c",
  name: "Task C",
  updated_at: "2024-01-03T00:00:00Z",
};

function trajectoryMeta(index: number) {
  const padded = index.toString().padStart(4, "0");
  return {
    id: `chat-${padded}`,
    title: `Chat ${padded}`,
    created_at: `2024-01-01T00:00:00Z`,
    updated_at: `2024-01-01T00:00:00Z`,
    model: "gpt-4",
    mode: "agent",
    message_count: index,
    total_lines_added: 0,
    total_lines_removed: 0,
    tasks_total: 0,
    tasks_done: 0,
    tasks_failed: 0,
  };
}

const chatA = {
  ...trajectoryMeta(1),
  id: "chat-a",
  title: "Chat A",
};

const chatB = {
  ...trajectoryMeta(2),
  id: "chat-b",
  title: "Chat B",
};

const chatC = {
  ...trajectoryMeta(3),
  id: "chat-c",
  title: "Chat C",
};

describe("useSidebarSubscription", () => {
  it("handles v2 section snapshots and null buddy snapshots", async () => {
    server.use(
      http.get(
        "http://127.0.0.1:8001/v1/sidebar/subscribe",
        () =>
          new HttpResponse(
            sseStream([
              sectionSnapshot(0, "workspace", {
                workspace_roots: ["/tmp/refact-test"],
              }),
              sectionSnapshot(1, "chats", { trajectories: [] }),
              sectionSnapshot(2, "tasks", { tasks: [] }),
              sectionSnapshot(3, "buddy", { buddy: null }),
            ]),
            { headers: { "Content-Type": "text/event-stream" } },
          ),
      ),
    );

    const { store } = render(<TestHarness />, { preloadedState: CONFIG_STATE });

    await waitFor(() => {
      expect(store.getState().current_project.workspaceRoots).toEqual([
        "/tmp/refact-test",
      ]);
      expect(store.getState().sidebar.sections.chats.status).toBe("ready");
      expect(store.getState().sidebar.sections.tasks.status).toBe("ready");
      expect(store.getState().sidebar.sections.buddy.status).toBe("ready");
      expect(store.getState().buddy.loaded).toBe(true);
      expect(store.getState().buddy.snapshot).toBeNull();
    });
  });

  it("routes v2 notification events without treating them as task events", async () => {
    const posted: unknown[] = [];
    const postMessageSpy = vi
      .spyOn(window, "postMessage")
      .mockImplementation((message) => {
        posted.push(message);
        return undefined;
      });

    server.use(
      http.get(
        "http://127.0.0.1:8001/v1/sidebar/subscribe",
        () =>
          new HttpResponse(
            sseStream([
              notification(0, {
                type: "task_done",
                chat_id: "chat-1",
                tool_call_id: "tool-1",
                summary: "Done",
              }),
              notification(1, {
                type: "ask_questions",
                chat_id: "chat-1",
                tool_call_id: "tool-2",
                questions: [{ id: "q1", type: "free_text", text: "Why?" }],
              }),
            ]),
            { headers: { "Content-Type": "text/event-stream" } },
          ),
      ),
    );

    render(<TestHarness />, { preloadedState: CONFIG_STATE });

    await waitFor(() => {
      expect(posted.length).toBeGreaterThanOrEqual(2);
    });
    expect(JSON.stringify(posted)).toContain("ide/taskDone");
    expect(JSON.stringify(posted)).toContain("Done");
    expect(JSON.stringify(posted)).toContain("ide/askQuestions");
    expect(JSON.stringify(posted)).toContain("tool-2");
    postMessageSpy.mockRestore();
  });

  it("clears stale buddy state when a later v2 buddy snapshot is null", async () => {
    server.use(
      http.get(
        "http://127.0.0.1:8001/v1/sidebar/subscribe",
        () =>
          new HttpResponse(
            sseStream([sectionSnapshot(0, "buddy", { buddy: null })]),
            {
              headers: { "Content-Type": "text/event-stream" },
            },
          ),
      ),
    );

    const existingSnapshot = {
      enabled: true,
      state: {
        identity: { name: "Old Buddy", created_at: "", palette_index: 0 },
      },
      settings: { enabled: true },
    } as BuddySnapshot;
    const { store } = render(<TestHarness />, { preloadedState: CONFIG_STATE });
    store.dispatch(setBuddySnapshot(existingSnapshot));

    await waitFor(() => {
      expect(store.getState().buddy.loaded).toBe(true);
      expect(store.getState().buddy.snapshot).toBeNull();
    });
  });

  it("section resync replaces only the tasks section", async () => {
    server.use(
      http.get(
        "http://127.0.0.1:8001/v1/sidebar/subscribe",
        () =>
          new HttpResponse(
            sseStream([
              sectionSnapshot(0, "tasks", { tasks: [taskA] }),
              sectionUpdate(1, "tasks", {
                type: "task_created",
                task_id: "task-b",
                meta: taskB,
              }),
              sectionSnapshot(2, "tasks", { tasks: [taskB] }),
            ]),
            { headers: { "Content-Type": "text/event-stream" } },
          ),
      ),
    );

    const { store } = render(<TestHarness />, { preloadedState: CONFIG_STATE });

    await waitFor(() => {
      expect(tasksFromStore(store.getState()).map((t) => t.id)).toEqual([
        "task-b",
      ]);
      expect(store.getState().sidebar.sections.tasks.status).toBe("ready");
    });
  });

  it("resync snapshots replace chats and tasks without reconnecting", async () => {
    server.use(
      http.get(
        "http://127.0.0.1:8001/v1/sidebar/subscribe",
        () =>
          new HttpResponse(
            sseStream([
              sectionSnapshot(0, "workspace", {
                workspace_roots: ["/tmp/refact-test"],
              }),
              sectionSnapshot(1, "chats", { trajectories: [chatA] }),
              sectionSnapshot(2, "tasks", { tasks: [taskA] }),
              sectionSnapshot(3, "buddy", { buddy: null }),
              sectionSnapshot(4, "chats", { trajectories: [chatB] }),
              sectionSnapshot(5, "tasks", { tasks: [taskB] }),
            ]),
            { headers: { "Content-Type": "text/event-stream" } },
          ),
      ),
    );

    const { store } = render(<TestHarness />, { preloadedState: CONFIG_STATE });

    await waitFor(() => {
      expect(store.getState().sidebar.subscriptionId).toBe("test-sidebar");
      expect(Object.keys(store.getState().history.chats)).toEqual(["chat-b"]);
      expect(tasksFromStore(store.getState()).map((t) => t.id)).toEqual([
        "task-b",
      ]);
      expect(store.getState().current_project.workspaceRoots).toEqual([
        "/tmp/refact-test",
      ]);
    });
  });

  it("keeps section readiness when workspace snapshot arrives after other sections", async () => {
    server.use(
      http.get(
        "http://127.0.0.1:8001/v1/sidebar/subscribe",
        () =>
          new HttpResponse(
            sseStream([
              sectionSnapshot(0, "buddy", { buddy: null }),
              sectionSnapshot(1, "tasks", { tasks: [] }),
              sectionSnapshot(2, "chats", { trajectories: [] }),
              sectionSnapshot(3, "workspace", {
                workspace_roots: ["/tmp/refact-test"],
              }),
            ]),
            { headers: { "Content-Type": "text/event-stream" } },
          ),
      ),
    );

    const { store } = render(<TestHarness />, { preloadedState: CONFIG_STATE });

    await waitFor(() => {
      expect(store.getState().sidebar.sections.workspace.status).toBe("ready");
      expect(store.getState().sidebar.sections.chats.status).toBe("ready");
      expect(store.getState().sidebar.sections.tasks.status).toBe("ready");
      expect(store.getState().sidebar.sections.buddy.status).toBe("ready");
    });
  });

  it("processes other sections while one section errors and recovers on retry", async () => {
    server.use(
      http.get(
        "http://127.0.0.1:8001/v1/sidebar/subscribe",
        () =>
          new HttpResponse(
            sseStream([
              sectionSnapshot(0, "workspace", {
                workspace_roots: ["/tmp/refact-test"],
              }),
              sectionSnapshot(
                1,
                "chats",
                { trajectories: [] },
                "error",
                "temporary trajectory error",
              ),
              sectionSnapshot(2, "tasks", { tasks: [taskA] }),
              sectionSnapshot(3, "buddy", { buddy: null }),
              sectionUpdate(4, "tasks", {
                type: "task_created",
                task_id: "task-c",
                meta: taskC,
              }),
              sectionSnapshot(5, "chats", { trajectories: [chatC] }),
            ]),
            { headers: { "Content-Type": "text/event-stream" } },
          ),
      ),
    );

    const { store } = render(<TestHarness />, { preloadedState: CONFIG_STATE });

    await waitFor(() => {
      expect(store.getState().sidebar.sections.chats).toEqual({
        status: "ready",
        error: null,
      });
      expect(Object.keys(store.getState().history.chats)).toEqual(["chat-c"]);
      expect(tasksFromStore(store.getState()).map((t) => t.id)).toEqual([
        "task-c",
        "task-a",
      ]);
      expect(store.getState().sidebar.sections.tasks.status).toBe("ready");
      expect(store.getState().sidebar.sections.buddy.status).toBe("ready");
    });
  });

  it("keeps existing chats during a transient chat error and replaces them on retry success", async () => {
    const existing = {
      id: "existing-chat",
      title: "Existing chat",
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
    const recovered = {
      ...existing,
      id: "recovered-chat",
      title: "Recovered chat",
      updated_at: "2024-01-02T00:00:00Z",
    };
    server.use(
      http.get(
        "http://127.0.0.1:8001/v1/sidebar/subscribe",
        () =>
          new HttpResponse(
            sseStream([
              sectionSnapshot(0, "chats", { trajectories: [existing] }),
              sectionSnapshot(
                1,
                "chats",
                { trajectories: [] },
                "error",
                "temporary timeout",
              ),
              sectionSnapshot(2, "chats", { trajectories: [recovered] }),
            ]),
            { headers: { "Content-Type": "text/event-stream" } },
          ),
      ),
    );

    const { store } = render(<TestHarness />, { preloadedState: CONFIG_STATE });

    await waitFor(() => {
      expect(store.getState().history.loadError).toBeNull();
      expect(Object.keys(store.getState().history.chats)).toEqual([
        "recovered-chat",
      ]);
      expect(store.getState().sidebar.sections.chats.status).toBe("ready");
    });
  });

  it("replaces large trajectory snapshots deterministically", async () => {
    const firstSnapshot = Array.from({ length: 1_000 }, (_, index) =>
      trajectoryMeta(index),
    );
    const secondSnapshot = Array.from({ length: 1_000 }, (_, index) =>
      trajectoryMeta(index + 1_000),
    );
    server.use(
      http.get(
        "http://127.0.0.1:8001/v1/sidebar/subscribe",
        () =>
          new HttpResponse(
            sseStream([
              sectionSnapshot(0, "chats", { trajectories: firstSnapshot }),
              sectionSnapshot(1, "chats", { trajectories: secondSnapshot }),
            ]),
            { headers: { "Content-Type": "text/event-stream" } },
          ),
      ),
    );

    const { store } = render(<TestHarness />, { preloadedState: CONFIG_STATE });

    await waitFor(() => {
      expect(Object.keys(store.getState().history.chats)).toHaveLength(1_000);
      expect(store.getState().history.chats["chat-0999"]).toBeUndefined();
      expect(store.getState().history.chats["chat-1000"].title).toBe(
        "Chat 1000",
      );
      expect(store.getState().history.chats["chat-1999"].title).toBe(
        "Chat 1999",
      );
    });
  });

  it("turns a chat error snapshot into an error state instead of forever loading", async () => {
    server.use(
      http.get(
        "http://127.0.0.1:8001/v1/sidebar/subscribe",
        () =>
          new HttpResponse(
            sseStream([
              sectionSnapshot(
                0,
                "chats",
                { trajectories: [] },
                "error",
                "trajectory boom",
              ),
            ]),
            { headers: { "Content-Type": "text/event-stream" } },
          ),
      ),
    );

    const { store } = render(<TestHarness />, { preloadedState: CONFIG_STATE });

    await waitFor(() => {
      expect(store.getState().history.loadError).toBe("trajectory boom");
      expect(store.getState().history.isLoading).toBe(false);
      expect(store.getState().sidebar.sections.chats.status).toBe("error");
    });
  });
});

function tasksFromStore(
  state: ReturnType<ReturnType<typeof render>["store"]["getState"]>,
) {
  const entry = tasksQueryFromStore(state);
  return (entry?.data as TaskMeta[] | undefined) ?? [];
}

function tasksQueryFromStore(
  state: ReturnType<ReturnType<typeof render>["store"]["getState"]>,
) {
  const queries = state.tasksApi.queries;
  return Object.values(queries).find(
    (query) => query?.endpointName === "listTasks",
  );
}
