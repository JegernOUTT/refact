import { render, waitFor } from "../../utils/test-utils";
import { describe, expect, it } from "vitest";
import {
  server,
  goodUser,
  goodPing,
  chatLinks,
  telemetryChat,
  telemetryNetwork,
  goodCaps,
  emptyTrajectories,
  trajectorySave,
  trajectoryDelete,
  chatSessionSubscribe,
  chatSessionCommand,
  chatSessionAbort,
  sidebarSubscribe,
  emptyTasks,
} from "../../utils/mockServer";
import { InnerApp } from "../../features/App";
import { HistoryState } from "../../features/History/historySlice";

describe("Delete a Chat form history", () => {
  it("can delete a chat", async () => {
    server.use(
      goodUser,
      goodPing,
      chatLinks,
      telemetryChat,
      telemetryNetwork,
      goodCaps,
      emptyTrajectories,
      trajectorySave,
      trajectoryDelete,
      chatSessionSubscribe,
      chatSessionCommand,
      chatSessionAbort,
      sidebarSubscribe,
      emptyTasks,
    );
    const now = new Date().toISOString();
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
        teams: {
          group: { id: "123", name: "test" },
        },
        pages: [{ name: "history" }],
        config: {
          apiKey: "test",
          lspPort: 8001,
          themeProps: {},
          host: "vscode",
          addressURL: "Refact",
        },
      },
    });

    const itemTitleToDelete = "Test title";

    const restoreButtonText = await app.findByText(itemTitleToDelete);

    // Find the delete button - in compact view, it uses aria-label="Delete"
    let container = restoreButtonText.parentElement;
    while (container && !container.querySelector('[aria-label="Delete"]')) {
      container = container.parentElement;
    }
    const deleteButton = container?.querySelector('[aria-label="Delete"]');

    expect(deleteButton).not.toBeNull();

    // eslint-disable-next-line @typescript-eslint/no-non-null-assertion
    await user.click(deleteButton!);

    // Wait for the deletion to be processed
    await waitFor(() => {
      expect(store.getState().history.chats).toEqual({});
    });
  });
});
