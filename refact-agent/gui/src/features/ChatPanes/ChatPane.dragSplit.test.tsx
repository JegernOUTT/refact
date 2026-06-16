import { http, HttpResponse } from "msw";
import { act } from "react-dom/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";

import { fireEvent, render, screen, within } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import {
  closeThread,
  createChatWithId,
  reorderOpenThreads,
} from "../Chat/Thread";
import { ChatPane } from "./ChatPane";
import { hydratePaneLayout, moveTabToPane, splitPane } from "./panesSlice";

vi.mock("../Chat/Chat", async () => {
  const React = await vi.importActual<typeof import("react")>("react");

  return {
    Chat: ({ chatId }: { chatId?: string }) =>
      React.createElement("div", { "data-testid": `chat-${chatId}` }),
  };
});

const baseConfig = {
  host: "web" as const,
  lspPort: 8001,
  lspUrl: "http://127.0.0.1:8001/v1/ping/Refact",
  themeProps: {},
};

const chatModesResponse = {
  modes: [
    {
      id: "agent",
      title: "Agent",
      description: "Agent mode",
      tools_count: 1,
      thread_defaults: {
        include_project_info: true,
        checkpoints_enabled: true,
        auto_approve_editing_tools: false,
        auto_approve_dangerous_commands: false,
      },
      ui: { order: 1, tags: [] },
    },
  ],
  errors: [],
};

function usePaneTabStripHandlers() {
  server.use(
    http.get("*/v1/chat-modes", () => HttpResponse.json(chatModesResponse)),
  );
}

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

function createDataTransferStub(): DataTransfer {
  const data = new Map<string, string>();
  const dataTransfer = {
    dropEffect: "none" as DataTransfer["dropEffect"],
    effectAllowed: "uninitialized" as DataTransfer["effectAllowed"],
    files: [] as unknown as FileList,
    items: [] as unknown as DataTransferItemList,
    get types() {
      return Array.from(data.keys());
    },
    clearData: vi.fn((type?: string) => {
      if (type) {
        data.delete(type);
      } else {
        data.clear();
      }
    }),
    getData: vi.fn((type: string) => data.get(type) ?? ""),
    setData: vi.fn((type: string, value: string) => {
      data.set(type, value);
    }),
    setDragImage: vi.fn(),
  } satisfies Partial<DataTransfer>;

  return dataTransfer as DataTransfer;
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

function seedSinglePaneChats(view: ReturnType<typeof renderChatPanes>) {
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
      hydratePaneLayout({
        root: {
          kind: "leaf",
          id: "root",
          tabIds: ["chat-a", "chat-b"],
          activeTabId: "chat-a",
        },
        focusedLeafId: "root",
      }),
    );
  });
}

function startTabDrag(title: string): DataTransfer {
  const dataTransfer = createDataTransferStub();
  fireEvent.dragStart(screen.getByTitle(title), { dataTransfer });
  return dataTransfer;
}

function rerenderPanes(view: ReturnType<typeof renderChatPanes>) {
  view.rerender(
    <div>
      <ChatPane leafId="root" />
      <ChatPane leafId="right" />
    </div>,
  );
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("ChatPane drag split", () => {
  it.each([
    ["left", "row", "before"],
    ["right", "row", "after"],
    ["top", "col", "before"],
    ["bottom", "col", "after"],
  ] as const)(
    "dispatches splitPane for the %s edge",
    (edge, dir, placement) => {
      usePaneTabStripHandlers();
      const view = renderChatPanes();
      seedTwoPaneChats(view);
      const dispatchSpy = vi.spyOn(view.store, "dispatch");
      rerenderPanes(view);

      const dataTransfer = startTabDrag("Chat Alpha");
      const rightPane = screen.getByLabelText("Chat pane right");
      fireEvent.dragEnter(rightPane, { dataTransfer });
      const edgeZone = within(rightPane).getByTestId(
        `pane-edge-drop-right-${edge}`,
      );

      fireEvent.dragOver(edgeZone, { dataTransfer });
      fireEvent.drop(edgeZone, { dataTransfer });

      expect(dispatchSpy).toHaveBeenCalledWith(
        splitPane({ leafId: "right", dir, tabId: "chat-a", placement }),
      );
    },
  );

  it("moves a tab on plain strip drop without splitting", () => {
    usePaneTabStripHandlers();
    const view = renderChatPanes();
    seedTwoPaneChats(view);
    const dispatchSpy = vi.spyOn(view.store, "dispatch");
    rerenderPanes(view);

    const dataTransfer = startTabDrag("Chat Alpha");
    const rightPane = screen.getByLabelText("Chat pane right");
    const rightStrip = within(rightPane).getByLabelText("Pane chat tabs");

    fireEvent.dragOver(rightStrip, { dataTransfer });
    fireEvent.drop(rightStrip, { dataTransfer });

    expect(dispatchSpy).toHaveBeenCalledWith(
      moveTabToPane({
        fromLeafId: "root",
        toLeafId: "right",
        tabId: "chat-a",
      }),
    );
    expect(dispatchSpy).not.toHaveBeenCalledWith(
      splitPane({ leafId: "right", dir: "row", tabId: "chat-a" }),
    );
  });

  it("keeps within-strip tab reorder as a reorder action", () => {
    usePaneTabStripHandlers();
    const view = renderChatPanes();
    seedSinglePaneChats(view);
    const dispatchSpy = vi.spyOn(view.store, "dispatch");
    rerenderPanes(view);

    const dataTransfer = startTabDrag("Chat Beta");
    const rootPane = screen.getByLabelText("Chat pane root");
    const targetTab = within(rootPane).getByTitle("Chat Alpha").closest("div");
    if (!targetTab) throw new Error("missing target tab wrapper");

    fireEvent.dragOver(targetTab, { dataTransfer });
    fireEvent.drop(targetTab, { dataTransfer });

    expect(dispatchSpy).toHaveBeenCalledWith(
      reorderOpenThreads({ sourceId: "chat-b", targetId: "chat-a" }),
    );
    expect(dispatchSpy).not.toHaveBeenCalledWith(
      moveTabToPane({
        fromLeafId: "root",
        toLeafId: "root",
        tabId: "chat-b",
      }),
    );
  });
});
