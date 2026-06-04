import { http, HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";
import { render, waitFor } from "../utils/test-utils";
import { server } from "../utils/mockServer";
import { useSidebarSubscription } from "../hooks/useSidebarSubscription";
import { setBuddySnapshot } from "../features/Buddy/buddySlice";
import type { BuddySnapshot } from "../features/Buddy/types";
import {
  subscribeToSidebarEvents,
  type SidebarSubscriptionCallbacks,
} from "../services/refact/sidebarSubscription";
import { type TaskBoard, type TaskMeta } from "../services/refact/tasks";

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
  const blocks = events.map((event) => `data: ${JSON.stringify(event)}\n\n`);
  return sseRawStream(blocks);
}

function sseRawStream(blocks: string[]): ReadableStream<Uint8Array> {
  const encoder = new TextEncoder();
  return new ReadableStream({
    start(controller) {
      for (const block of blocks) {
        controller.enqueue(encoder.encode(block));
      }
    },
  });
}

function sidebarHandler(events: unknown[]) {
  return http.get(
    "*/v1/sidebar/subscribe",
    () =>
      new HttpResponse(sseStream(events), {
        headers: { "Content-Type": "text/event-stream" },
      }),
  );
}

function sidebarRawHandler(blocks: string[]) {
  return http.get(
    "*/v1/sidebar/subscribe",
    () =>
      new HttpResponse(sseRawStream(blocks), {
        headers: { "Content-Type": "text/event-stream" },
      }),
  );
}

function pendingSidebarResponse() {
  return new HttpResponse(
    new ReadableStream<Uint8Array>({
      start() {
        return undefined;
      },
    }),
    { headers: { "Content-Type": "text/event-stream" } },
  );
}

function subscribeForTest(
  overrides: Partial<SidebarSubscriptionCallbacks> = {},
) {
  const events: Parameters<SidebarSubscriptionCallbacks["onEvent"]>[0][] = [];
  const errors: Error[] = [];
  const liveness = vi.fn();
  const disconnect = subscribeToSidebarEvents(
    { host: "vscode", lspPort: 8001 },
    "test",
    {
      onEvent: (event) => events.push(event),
      onError: (error) => errors.push(error),
      onLiveness: liveness,
      ...overrides,
    },
  );
  return { events, errors, liveness, disconnect };
}

const testBoard: TaskBoard = {
  schema_version: 1,
  rev: 2,
  columns: [],
  cards: [],
};

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
  it("uses a relative SSE URL in Vite and engine-served web mode", async () => {
    const requests: string[] = [];
    server.use(
      http.get("/v1/sidebar/subscribe", ({ request }) => {
        requests.push(request.url);
        return pendingSidebarResponse();
      }),
    );

    const devDisconnect = subscribeToSidebarEvents(
      { host: "web", dev: true },
      "test",
      {
        onEvent: vi.fn(),
        onError: vi.fn(),
      },
    );
    const engineServedDisconnect = subscribeToSidebarEvents(
      { host: "web", engineServed: true, lspUrl: "http://127.0.0.1:8001" },
      "test",
      {
        onEvent: vi.fn(),
        onError: vi.fn(),
      },
    );

    await waitFor(() => {
      expect(requests).toHaveLength(2);
    });
    expect(
      requests.every(
        (url) => new URL(url).pathname === "/v1/sidebar/subscribe",
      ),
    ).toBe(true);
    devDisconnect();
    engineServedDisconnect();
  });

  it("uses the configured remote origin in web mode", async () => {
    const requests: string[] = [];
    server.use(
      http.get(
        "https://remote.example.com/proxy/v1/sidebar/subscribe",
        ({ request }) => {
          requests.push(request.url);
          return pendingSidebarResponse();
        },
      ),
    );

    const disconnect = subscribeToSidebarEvents(
      { host: "web", lspUrl: "https://remote.example.com/proxy/v1/ping" },
      "test",
      {
        onEvent: vi.fn(),
        onError: vi.fn(),
      },
    );

    await waitFor(() => {
      expect(requests).toEqual([
        "https://remote.example.com/proxy/v1/sidebar/subscribe",
      ]);
    });
    disconnect();
  });

  it("unknown_event_type_is_ignored_not_fatal", async () => {
    server.use(
      sidebarHandler([
        envelope(0, { type: "future_event", payload: { ok: true } }),
        sectionSnapshot(1, "tasks", { tasks: [taskA] }),
      ]),
    );

    const { store } = render(<TestHarness />, { preloadedState: CONFIG_STATE });

    await waitFor(() => {
      expect(tasksFromStore(store.getState()).map((task) => task.id)).toEqual([
        "task-a",
      ]);
      expect(store.getState().sidebar.sections.tasks.status).toBe("ready");
    });
    expect(store.getState().history.loadError).toBeNull();
  });

  it("malformed_json_block_is_skipped_not_fatal", async () => {
    server.use(
      sidebarRawHandler([
        "data: {not json}\n\n",
        `data: ${JSON.stringify(
          sectionSnapshot(0, "tasks", { tasks: [taskA] }),
        )}\n\n`,
      ]),
    );

    const { store } = render(<TestHarness />, { preloadedState: CONFIG_STATE });

    await waitFor(() => {
      expect(tasksFromStore(store.getState()).map((task) => task.id)).toEqual([
        "task-a",
      ]);
      expect(store.getState().sidebar.sections.tasks.status).toBe("ready");
    });
    expect(store.getState().history.loadError).toBeNull();
  });

  it("known_event_with_bad_payload_is_skipped_not_fatal", async () => {
    server.use(
      sidebarHandler([
        envelope(0, {
          type: "section_snapshot",
          section: "tasks",
          status: "ready",
          snapshot: { tasks: "oops" },
        }),
        sectionSnapshot(1, "tasks", { tasks: [taskB] }),
      ]),
    );

    const { store } = render(<TestHarness />, { preloadedState: CONFIG_STATE });

    await waitFor(() => {
      expect(tasksFromStore(store.getState()).map((task) => task.id)).toEqual([
        "task-b",
      ]);
      expect(store.getState().sidebar.sections.tasks.status).toBe("ready");
    });
    expect(store.getState().history.loadError).toBeNull();
  });

  it("accepts chat snapshot pagination metadata", async () => {
    server.use(
      sidebarHandler([
        sectionSnapshot(0, "chats", {
          trajectories: [],
          pagination: {
            next_cursor: "next-page",
            has_more: true,
            total_count: 51,
          },
        }),
      ]),
    );

    const { events, disconnect } = subscribeForTest();

    await waitFor(() => {
      expect(events[0]?.event).toEqual({
        type: "section_snapshot",
        section: "chats",
        status: "ready",
        snapshot: {
          trajectories: [],
          pagination: {
            next_cursor: "next-page",
            has_more: true,
            total_count: 51,
          },
        },
      });
    });
    disconnect();
  });

  it("heartbeat_event_does_not_trigger_invalidation", async () => {
    server.use(sidebarHandler([envelope(0, { type: "heartbeat" })]));
    const events: Parameters<SidebarSubscriptionCallbacks["onEvent"]>[0][] = [];
    const liveness = vi.fn();
    const disconnect = subscribeToSidebarEvents(
      { host: "vscode", lspPort: 8001 },
      "test",
      {
        onEvent: (event) => events.push(event),
        onError: () => undefined,
        onLiveness: liveness,
      },
    );

    await waitFor(() => {
      expect(liveness).toHaveBeenCalled();
    });
    expect(events).toEqual([]);
    disconnect();
  });

  it("heartbeat_event_resets_liveness_timer", async () => {
    server.use(sidebarHandler([envelope(0, { type: "heartbeat" })]));
    const { events, errors, liveness, disconnect } = subscribeForTest();

    await waitFor(() => {
      expect(liveness).toHaveBeenCalled();
    });
    expect(events).toEqual([]);
    expect(errors).toEqual([]);
    disconnect();
  });

  it("sse_block_exceeding_4mib_triggers_reconnect", async () => {
    server.use(sidebarRawHandler([`data: ${"x".repeat(4 * 1024 * 1024 + 1)}`]));
    const { errors, disconnect } = subscribeForTest();

    await waitFor(() => {
      expect(errors.map((error) => error.message)).toContain(
        "sse_block_too_large",
      );
    });
    disconnect();
  });

  it("allows one read chunk with multiple valid blocks over 1mib total", async () => {
    const first = sectionSnapshot(0, "chats", {
      trajectories: [{ id: "a", title: "A".repeat(600 * 1024) }],
    });
    const second = sectionSnapshot(1, "chats", {
      trajectories: [{ id: "b", title: "B".repeat(600 * 1024) }],
    });
    server.use(
      sidebarRawHandler([
        `data: ${JSON.stringify(first)}\n\ndata: ${JSON.stringify(second)}\n\n`,
      ]),
    );

    const { events, errors, disconnect } = subscribeForTest();

    await waitFor(() => {
      expect(events.map((event) => event.seq)).toEqual([0, 1]);
    });
    expect(errors).toEqual([]);
    disconnect();
  });

  it("subsequent_known_event_after_unknown_event_is_dispatched_correctly", async () => {
    server.use(
      sidebarHandler([
        envelope(0, { type: "future_event", payload: { ok: true } }),
        sectionUpdate(1, "tasks", {
          type: "board_changed",
          task_id: "task-a",
          rev: testBoard.rev,
          board: testBoard,
        }),
      ]),
    );
    const { events, errors, disconnect } = subscribeForTest();

    await waitFor(() => {
      expect(events).toHaveLength(1);
    });
    expect(errors).toEqual([]);
    expect(events[0].event).toEqual({
      type: "section_update",
      section: "tasks",
      update: {
        type: "board_changed",
        task_id: "task-a",
        rev: testBoard.rev,
        board: testBoard,
      },
    });
    disconnect();
  });

  it("handles v2 section snapshots and null buddy snapshots", async () => {
    server.use(
      http.get(
        "*/v1/sidebar/subscribe",
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
        "*/v1/sidebar/subscribe",
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

  it("routes conductor ghost messages into Buddy state", async () => {
    server.use(
      http.get(
        "*/v1/sidebar/subscribe",
        () =>
          new HttpResponse(
            sseStream([
              sectionUpdate(0, "buddy", {
                event_type: "ConductorGhostMessage",
                ghost: {
                  id: "ghost-sidebar",
                  goal_id: "goal-sidebar",
                  role: "ask",
                  content: "Answer from overlay?",
                  created_at: "2024-01-01T00:00:00Z",
                  source_chat_id: "chat-a",
                  question_id: "question-sidebar",
                },
              }),
            ]),
            { headers: { "Content-Type": "text/event-stream" } },
          ),
      ),
    );

    const { store } = render(<TestHarness />, { preloadedState: CONFIG_STATE });

    await waitFor(() => {
      expect(store.getState().buddy.ghostMessages).toEqual([
        expect.objectContaining({
          id: "ghost-sidebar",
          role: "ask",
          question_id: "question-sidebar",
        }),
      ]);
    });
  });

  it("clears stale buddy state when a later v2 buddy snapshot is null", async () => {
    server.use(
      http.get(
        "*/v1/sidebar/subscribe",
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
        "*/v1/sidebar/subscribe",
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
        "*/v1/sidebar/subscribe",
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
        "*/v1/sidebar/subscribe",
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
        "*/v1/sidebar/subscribe",
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
        "*/v1/sidebar/subscribe",
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
        "*/v1/sidebar/subscribe",
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
        "*/v1/sidebar/subscribe",
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
