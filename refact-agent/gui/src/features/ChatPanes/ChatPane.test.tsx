import { act } from "react-dom/test-utils";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { render, screen, within } from "../../utils/test-utils";
import { backUpMessages, closeThread, createChatWithId } from "../Chat/Thread";
import { hydratePaneLayout } from "./panesSlice";
import { ChatPane } from "./ChatPane";

vi.mock("./PaneTabStrip", async () => {
  const React = await vi.importActual<typeof import("react")>("react");

  return {
    PaneTabStrip: ({ leafId }: { leafId: string }) =>
      React.createElement(
        "div",
        { "data-testid": `pane-tab-strip-${leafId}` },
        `Pane tabs ${leafId}`,
      ),
  };
});

vi.mock("../Chat/Chat", async () => {
  const React = await vi.importActual<typeof import("react")>("react");
  const thread =
    await vi.importActual<typeof import("../Chat/Thread")>("../Chat/Thread");
  const selectorHook = await vi.importActual<
    typeof import("../../hooks/useAppSelector")
  >("../../hooks/useAppSelector");

  return {
    Chat: ({ chatId }: { chatId?: string }) => {
      const contextChatId = thread.useThreadId();
      const resolvedChatId = chatId ?? contextChatId;
      const messages = selectorHook.useAppSelector((state) =>
        thread.selectMessagesById(state, resolvedChatId),
      );

      return React.createElement(
        "div",
        { "data-testid": `chat-transcript-${resolvedChatId}` },
        messages.map((message, index) =>
          React.createElement(
            "p",
            {
              key:
                "message_id" in message && message.message_id
                  ? message.message_id
                  : index,
            },
            typeof message.content === "string" ? message.content : "",
          ),
        ),
      );
    },
  };
});

const baseConfig = {
  host: "web" as const,
  lspPort: 8001,
  lspUrl: "http://127.0.0.1:8001/v1/ping/Refact",
  themeProps: {},
};

function renderChatPanes() {
  return render(
    <div>
      <ChatPane leafId="root" />
      <ChatPane leafId="right" />
    </div>,
    {
      preloadedState: { config: baseConfig },
    },
  );
}

function seedTwoPaneChats(view: ReturnType<typeof renderChatPanes>) {
  act(() => {
    const initialThreadId = view.store.getState().chat.open_thread_ids[0];
    if (initialThreadId) {
      view.store.dispatch(closeThread({ id: initialThreadId, force: true }));
    }

    view.store.dispatch(
      createChatWithId({ id: "chat-a", title: "Chat Alpha", mode: "agent" }),
    );
    view.store.dispatch(
      createChatWithId({ id: "chat-b", title: "Chat Beta", mode: "agent" }),
    );
    view.store.dispatch(
      backUpMessages({
        id: "chat-a",
        messages: [
          {
            role: "user",
            message_id: "chat-a-user",
            content: "Alpha transcript only",
          },
        ],
      }),
    );
    view.store.dispatch(
      backUpMessages({
        id: "chat-b",
        messages: [
          {
            role: "user",
            message_id: "chat-b-user",
            content: "Beta transcript only",
          },
        ],
      }),
    );
    view.store.dispatch(
      hydratePaneLayout({
        root: {
          kind: "split",
          id: "root:split:row",
          dir: "row",
          sizes: [0.5, 0.5],
          children: [
            {
              kind: "leaf",
              id: "root",
              tabIds: ["chat-a"],
              activeTabId: "chat-a",
            },
            {
              kind: "leaf",
              id: "right",
              tabIds: ["chat-b"],
              activeTabId: "chat-b",
            },
          ],
        },
        focusedLeafId: "root",
      }),
    );
  });
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("ChatPane", () => {
  it("renders two leaves with distinct chat transcripts", () => {
    const view = renderChatPanes();
    seedTwoPaneChats(view);

    const leftPane = screen.getByLabelText("Chat pane root");
    const rightPane = screen.getByLabelText("Chat pane right");

    expect(
      within(leftPane).getByTestId("pane-tab-strip-root"),
    ).toHaveTextContent("Pane tabs root");
    expect(
      within(leftPane).getByTestId("chat-transcript-chat-a"),
    ).toHaveTextContent("Alpha transcript only");
    expect(
      within(leftPane).queryByText("Beta transcript only"),
    ).not.toBeInTheDocument();
    expect(
      within(rightPane).getByTestId("chat-transcript-chat-b"),
    ).toHaveTextContent("Beta transcript only");
    expect(
      within(rightPane).queryByText("Alpha transcript only"),
    ).not.toBeInTheDocument();
  });

  it("focuses a pane when clicked", async () => {
    const view = renderChatPanes();
    seedTwoPaneChats(view);

    await userEvent.click(screen.getByLabelText("Chat pane right"));

    expect(view.store.getState().panes.focusedLeafId).toBe("right");
    expect(screen.getByLabelText("Chat pane right")).toHaveAttribute(
      "data-focused",
      "true",
    );
  });
});
