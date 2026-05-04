import { http, HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";
import { render, waitFor } from "../utils/test-utils";
import { server } from "../utils/mockServer";
import { useSidebarSubscription } from "../hooks/useSidebarSubscription";
import { setBuddySnapshot } from "../features/Buddy/buddySlice";
import type { BuddySnapshot } from "../features/Buddy/types";

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

describe("useSidebarSubscription", () => {
  it("handles progressive snapshots and null buddy snapshots", async () => {
    server.use(
      http.get(
        "http://127.0.0.1:8001/v1/sidebar/subscribe",
        () =>
          new HttpResponse(
            sseStream([
              {
                seq: 0,
                category: "workspace_snapshot",
                workspace_roots: ["/tmp/refact-test"],
              },
              { seq: 1, category: "trajectories_snapshot", trajectories: [] },
              { seq: 2, category: "tasks_snapshot", tasks: [] },
              { seq: 3, category: "buddy_snapshot", buddy: null },
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
      expect(
        store.getState().current_project.trajectoriesSnapshotReceived,
      ).toBe(true);
      expect(store.getState().current_project.tasksSnapshotReceived).toBe(true);
      expect(store.getState().current_project.buddySnapshotReceived).toBe(true);
      expect(store.getState().buddy.loaded).toBe(true);
      expect(store.getState().buddy.snapshot).toBeNull();
    });
  });

  it("routes notification events without treating them as task events", async () => {
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
              {
                seq: 0,
                category: "notification",
                type: "task_done",
                chat_id: "chat-1",
                tool_call_id: "tool-1",
                summary: "Done",
              },
              {
                seq: 1,
                category: "notification",
                type: "ask_questions",
                chat_id: "chat-1",
                tool_call_id: "tool-2",
                questions: [{ id: "q1", type: "free_text", text: "Why?" }],
              },
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

  it("clears stale buddy state when a later buddy snapshot is null", async () => {
    server.use(
      http.get(
        "http://127.0.0.1:8001/v1/sidebar/subscribe",
        () =>
          new HttpResponse(
            sseStream([{ seq: 0, category: "buddy_snapshot", buddy: null }]),
            { headers: { "Content-Type": "text/event-stream" } },
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
});
