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
  trajectoryDelete,
  chatSessionSubscribe,
  chatSessionCommand,
  chatSessionAbort,
  emptyTasks,
} from "../../utils/mockServer";
import { InnerApp } from "../../features/App";
import { HistoryState } from "../../features/History/historySlice";
import type { TrajectoryMeta } from "../../services/refact/trajectories";

describe("Delete a Chat form history", () => {
  it("can delete a chat", async () => {
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

    server.use(
      goodUser,
      goodPing,
      chatLinks,
      goodCaps,
      http.get("http://127.0.0.1:8001/v1/trajectories", () => {
        return HttpResponse.json({
          items: [trajectory],
          next_cursor: null,
          has_more: false,
        });
      }),
      trajectorySave,
      trajectoryDelete,
      chatSessionSubscribe,
      chatSessionCommand,
      chatSessionAbort,
      http.get("http://127.0.0.1:8001/v1/chat-modes", () => {
        return HttpResponse.json({ modes: [], errors: [] });
      }),
      http.get("http://127.0.0.1:8001/v1/setup/status", () => {
        return HttpResponse.json({
          configured: true,
          reasons: [],
          detail: {
            project_root: "/tmp/refact-test",
            has_agents_md: true,
            has_knowledge: false,
            has_trajectories: true,
          },
        });
      }),
      http.get("http://127.0.0.1:8001/v1/sidebar/subscribe", () => {
        const encoder = new TextEncoder();
        const stream = new ReadableStream({
          start(controller) {
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
            ];
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
    const history: HistoryState = {
      chats: {
        abc123: {
          title: "Test title",
          isTitleGenerated: false,
          messages: [],
          id: "abc123",
          model: "foo",
          tool_use: "quick",
          new_chat_suggested: {
            wasSuggested: false,
          },
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
    const { user, store, ...app } = render(<InnerApp />, {
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

    const itemTitleToDelete = "Test title";

    await app.findByText(itemTitleToDelete);

    const deleteTrigger = await app.findByRole("button", {
      name: `Delete ${itemTitleToDelete}`,
    });
    await user.click(deleteTrigger);

    await app.findByText("Destructive action");
    const confirmDeleteButton = await app.findByRole("button", {
      name: "Delete",
    });
    await user.click(confirmDeleteButton);

    // Wait for the deletion to be processed
    await waitFor(() => {
      expect(store.getState().history.chats).toEqual({});
    });
  });
});
