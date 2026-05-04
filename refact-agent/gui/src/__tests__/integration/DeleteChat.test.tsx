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
            controller.enqueue(
              encoder.encode(
                `data: ${JSON.stringify({
                  seq: 0,
                  category: "snapshot",
                  trajectories: [trajectory],
                  tasks: [],
                  workspace_roots: ["/tmp/refact-test"],
                  buddy: { enabled: false },
                })}\n\n`,
              ),
            );
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
      pagination: { cursor: null, hasMore: false },
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

    const restoreButtonText = await app.findByText(itemTitleToDelete);

    // Find the delete button - uses aria-label="Delete chat"
    let container = restoreButtonText.parentElement;
    while (
      container &&
      !container.querySelector('[aria-label="Delete chat"]')
    ) {
      container = container.parentElement;
    }
    const deleteButton = container?.querySelector('[aria-label="Delete chat"]');

    expect(deleteButton).not.toBeNull();

    // eslint-disable-next-line @typescript-eslint/no-non-null-assertion
    await user.click(deleteButton!);

    // Wait for the deletion to be processed
    await waitFor(() => {
      expect(store.getState().history.chats).toEqual({});
    });
  });
});
