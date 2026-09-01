import { render, waitFor } from "../../utils/test-utils";
import { describe, expect, it } from "vitest";
import { http, HttpResponse } from "msw";
import {
  server,
  goodUser,
  goodPing,
  chatLinks,
  goodCaps,
  trajectorySave,
  chatSessionSubscribe,
  chatSessionCommand,
  chatSessionAbort,
  emptyTasks,
} from "../../utils/mockServer";
import { InnerApp } from "../../features/App";
import {
  deleteChatById,
  HistoryState,
} from "../../features/History/historySlice";
import type { TrajectoryMeta } from "../../services/refact/trajectories";

const now = new Date().toISOString();
const trajectory: TrajectoryMeta = {
  id: "abc123",
  title: "Test title",
  created_at: now,
  updated_at: now,
  model: "foo",
  mode: "AGENT",
  message_count: 0,
  total_lines_added: 0,
  total_lines_removed: 0,
  tasks_total: 0,
  tasks_done: 0,
  tasks_failed: 0,
};

const history: HistoryState = {
  chats: {
    abc123: {
      title: "Test title",
      isTitleGenerated: false,
      messages: [],
      id: "abc123",
      model: "foo",
      tool_use: "quick",
      new_chat_suggested: { wasSuggested: false },
      createdAt: now,
      updatedAt: now,
    },
  },
  isLoading: false,
  loadError: null,
  pagination: {
    cursor: null,
    hasMore: false,
    totalCount: null,
    generation: 0,
  },
};

function setup(options: { deleteStatus?: number; emitDeleted?: boolean } = {}) {
  let deleteRequests = 0;
  const events = [
    {
      protocol_version: 2,
      seq: 0,
      subscription_id: "test-sidebar",
      event: {
        type: "section_snapshot",
        section: "workspace",
        status: "ready",
        snapshot: { workspace_roots: ["/tmp/refact-test"] },
      },
    },
    {
      protocol_version: 2,
      seq: 1,
      subscription_id: "test-sidebar",
      event: {
        type: "section_snapshot",
        section: "chats",
        status: "ready",
        snapshot: { trajectories: [trajectory] },
      },
    },
    {
      protocol_version: 2,
      seq: 2,
      subscription_id: "test-sidebar",
      event: {
        type: "section_snapshot",
        section: "tasks",
        status: "ready",
        snapshot: { tasks: [] },
      },
    },
    {
      protocol_version: 2,
      seq: 3,
      subscription_id: "test-sidebar",
      event: {
        type: "section_snapshot",
        section: "buddy",
        status: "ready",
        snapshot: { buddy: null },
      },
    },
    ...(options.emitDeleted
      ? [
          {
            protocol_version: 2,
            seq: 4,
            subscription_id: "test-sidebar",
            event: {
              type: "section_update",
              section: "chats",
              update: { type: "deleted", id: trajectory.id },
            },
          },
        ]
      : []),
  ];

  server.use(
    goodUser,
    goodPing,
    chatLinks,
    goodCaps,
    http.get("http://127.0.0.1:8001/v1/trajectories", () =>
      HttpResponse.json({
        items: [trajectory],
        next_cursor: null,
        has_more: false,
      }),
    ),
    trajectorySave,
    http.delete("http://127.0.0.1:8001/v1/trajectories/:id", () => {
      deleteRequests += 1;
      const status = options.deleteStatus ?? 200;
      return status === 200
        ? HttpResponse.json({ success: true })
        : HttpResponse.json({ detail: "delete failed" }, { status });
    }),
    chatSessionSubscribe,
    chatSessionCommand,
    chatSessionAbort,
    http.get("http://127.0.0.1:8001/v1/chat-modes", () =>
      HttpResponse.json({ modes: [], errors: [] }),
    ),
    http.get("http://127.0.0.1:8001/v1/setup/status", () =>
      HttpResponse.json({
        configured: true,
        reasons: [],
        detail: {
          project_root: "/tmp/refact-test",
          has_agents_md: true,
          has_knowledge: false,
          has_trajectories: true,
        },
      }),
    ),
    http.get("http://127.0.0.1:8001/v1/sidebar/subscribe", () => {
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
    }),
    emptyTasks,
  );

  const rendered = render(<InnerApp />, {
    preloadedState: {
      history,
      pages: [{ name: "history" }],
      config: {
        apiKey: "test",
        lspPort: 8001,
        themeProps: {},
        host: "vscode",
        currentWorkspaceName: "refact-test",
      },
    },
  });

  return { ...rendered, getDeleteRequests: () => deleteRequests };
}

async function deleteChat(app: ReturnType<typeof setup>) {
  await app.findAllByText(trajectory.title);
  app.store.dispatch(deleteChatById(trajectory.id));
}

describe("Delete a Chat form history", () => {
  it("can delete a chat", async () => {
    const app = setup();

    await deleteChat(app);
    await waitFor(() => {
      expect(app.store.getState().history.chats).toEqual({});
      expect(app.getDeleteRequests()).toBe(1);
    });
  });

  it("removes a server-deleted chat locally without another DELETE", async () => {
    const app = setup({ emitDeleted: true });
    await waitFor(() => {
      expect(app.store.getState().history.chats).toEqual({});
      expect(app.getDeleteRequests()).toBe(0);
    });
  });

  it("keeps a chat removed without an error when DELETE returns 404", async () => {
    const app = setup({ deleteStatus: 404 });

    await deleteChat(app);
    await waitFor(() => {
      expect(app.store.getState().history.chats).toEqual({});
      expect(app.getDeleteRequests()).toBe(1);
    });
    expect(
      app.queryByText(
        "Failed to delete chat on the server — the list may resync.",
      ),
    ).not.toBeInTheDocument();
  });

  it("still sends DELETE for a chat that is not in local history", async () => {
    const app = setup();

    await app.findAllByText(trajectory.title);
    app.store.dispatch(deleteChatById("not-in-history"));

    await waitFor(() => {
      expect(app.getDeleteRequests()).toBe(1);
    });
    expect(app.store.getState().history.chats.abc123).toBeDefined();
  });

  it("restores a chat and shows an error when DELETE returns 500", async () => {
    const app = setup({ deleteStatus: 500 });

    await deleteChat(app);
    await waitFor(() => {
      expect(app.store.getState().error.message).toBe(
        "Failed to delete chat on the server — the list may resync.",
      );
      expect(app.store.getState().history.chats.abc123).toBeDefined();
      expect(app.getDeleteRequests()).toBe(1);
    });
  });
});
