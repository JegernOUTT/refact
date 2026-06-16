import { http, HttpResponse } from "msw";
import { act } from "react-dom/test-utils";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { render, screen, within } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import {
  closeThread,
  createChatWithId,
  reorderOpenThreads,
  saveTitle,
  updateChatRuntimeFromSessionState,
} from "../Chat/Thread";
import { processCompleted } from "../Notifications";
import type { ProcessCompletedEvent } from "../Notifications";
import { findLeaf } from "./panesTree";
import {
  hydratePaneLayout,
  removeTabEverywhere,
  setPaneActiveTab,
} from "./panesSlice";
import { PaneTabStrip } from "./PaneTabStrip";

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
    http.post("*/v1/chats/:id/commands", () =>
      HttpResponse.json({ status: "queued" }),
    ),
  );
}

function renderPaneTabStrip() {
  return render(<PaneTabStrip leafId="root" />, {
    preloadedState: { config: baseConfig },
  });
}

function seedPaneTabs(view: ReturnType<typeof renderPaneTabStrip>) {
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
      updateChatRuntimeFromSessionState({
        id: "chat-b",
        session_state: "generating",
      }),
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

function getTabWrap(title: string): HTMLElement {
  const wrap = screen.getByTitle(title).closest("div");
  if (!wrap) throw new Error(`missing tab wrapper for ${title}`);
  return wrap;
}

function createDataTransferStub() {
  const data = new Map<string, string>();
  return {
    data,
    dataTransfer: {
      effectAllowed: "",
      dropEffect: "",
      setData: (type: string, value: string) => data.set(type, value),
      getData: (type: string) => data.get(type) ?? "",
    },
  };
}

function createProcessCompletedEvent(
  chatId: string,
  seq: string,
): ProcessCompletedEvent {
  return {
    chat_id: chatId,
    seq,
    type: "process_completed",
    process_id: `exec_${seq}`,
    status: "failed",
    exit_code: 1,
    short_description: "Run pane tab strip test",
    mode: "background",
  };
}

function rootPaneTabIds(view: ReturnType<typeof renderPaneTabStrip>): string[] {
  const leaf = findLeaf(view.store.getState().panes.root, "root");
  if (!leaf) throw new Error("missing root leaf");
  return leaf.tabIds;
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("PaneTabStrip", () => {
  it("renders leaf chat tabs with active highlight, status dot, and mode badge", async () => {
    usePaneTabStripHandlers();
    const view = renderPaneTabStrip();
    seedPaneTabs(view);

    const chatAlpha = screen.getByRole("tab", { name: /Chat Alpha/ });
    const chatBeta = screen.getByRole("tab", { name: /Chat Beta/ });

    expect(chatAlpha).toHaveAttribute("aria-selected", "true");
    expect(chatBeta).toHaveAttribute("aria-selected", "false");
    expect(within(chatAlpha).getByLabelText("Idle")).toBeInTheDocument();
    expect(
      within(chatBeta).getByLabelText("In progress..."),
    ).toBeInTheDocument();
    expect(await within(chatAlpha).findByText("Agent")).toBeInTheDocument();
  });

  it("sets the pane active tab when a tab is clicked", async () => {
    usePaneTabStripHandlers();
    const view = renderPaneTabStrip();
    seedPaneTabs(view);

    await userEvent.click(screen.getByRole("tab", { name: /Chat Beta/ }));

    expect(view.store.getState().panes.focusedLeafId).toBe("root");
    expect(
      findLeaf(view.store.getState().panes.root, "root")?.activeTabId,
    ).toBe("chat-b");
    expect(screen.getByRole("tab", { name: /Chat Beta/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  it("closes a tab and removes it from pane state", async () => {
    usePaneTabStripHandlers();
    const view = renderPaneTabStrip();
    seedPaneTabs(view);

    await userEvent.click(
      within(getTabWrap("Chat Beta")).getByTitle("Close tab"),
    );

    expect(view.store.getState().chat.open_thread_ids).toEqual(["chat-a"]);
    expect(rootPaneTabIds(view)).toEqual(["chat-a"]);
    expect(
      screen.queryByRole("tab", { name: /Chat Beta/ }),
    ).not.toBeInTheDocument();
  });

  it("dispatches closeThread and removeTabEverywhere on close", async () => {
    usePaneTabStripHandlers();
    const view = renderPaneTabStrip();
    seedPaneTabs(view);
    const dispatchSpy = vi.spyOn(view.store, "dispatch");
    view.rerender(<PaneTabStrip leafId="root" />);

    await userEvent.click(
      within(getTabWrap("Chat Beta")).getByTitle("Close tab"),
    );

    expect(dispatchSpy).toHaveBeenCalledWith(closeThread({ id: "chat-b" }));
    expect(dispatchSpy).toHaveBeenCalledWith(removeTabEverywhere("chat-b"));
  });

  it("reorders open chat tabs after drag and drop", () => {
    usePaneTabStripHandlers();
    const view = renderPaneTabStrip();
    seedPaneTabs(view);
    const dispatchSpy = vi.spyOn(view.store, "dispatch");
    view.rerender(<PaneTabStrip leafId="root" />);

    const dragged = screen.getByTitle("Chat Beta");
    const target = getTabWrap("Chat Alpha");
    const { dataTransfer } = createDataTransferStub();

    const dragStart = new Event("dragstart", { bubbles: true });
    Object.defineProperty(dragStart, "dataTransfer", { value: dataTransfer });
    dragged.dispatchEvent(dragStart);
    const drop = new Event("drop", { bubbles: true, cancelable: true });
    Object.defineProperty(drop, "dataTransfer", { value: dataTransfer });
    target.dispatchEvent(drop);

    expect(dispatchSpy).toHaveBeenCalledWith(
      reorderOpenThreads({ sourceId: "chat-b", targetId: "chat-a" }),
    );
    expect(view.store.getState().chat.open_thread_ids).toEqual([
      "chat-b",
      "chat-a",
    ]);
  });

  it("dispatches saveTitle when renaming from double-click rename mode", async () => {
    usePaneTabStripHandlers();
    const view = renderPaneTabStrip();
    seedPaneTabs(view);
    const dispatchSpy = vi.spyOn(view.store, "dispatch");
    view.rerender(<PaneTabStrip leafId="root" />);

    await userEvent.dblClick(screen.getByRole("tab", { name: /Chat Alpha/ }));
    const renameInput = screen.getByDisplayValue("Chat Alpha");
    await userEvent.clear(renameInput);
    await userEvent.type(renameInput, "Renamed Chat{Enter}");

    expect(view.store.getState().chat.threads["chat-a"]?.thread.title).toBe(
      "Renamed Chat",
    );
    expect(dispatchSpy).toHaveBeenCalledWith(
      saveTitle({
        id: "chat-a",
        title: "Renamed Chat",
        isTitleGenerated: true,
      }),
    );
  });

  it("renders unread process notification badges and caps counts above nine", () => {
    usePaneTabStripHandlers();
    const view = renderPaneTabStrip();
    seedPaneTabs(view);

    act(() => {
      for (let i = 1; i <= 10; i += 1) {
        view.store.dispatch(
          processCompleted(createProcessCompletedEvent("chat-a", String(i))),
        );
      }
    });

    expect(
      screen.getByLabelText("10 unread process notifications"),
    ).toHaveTextContent("9+");
  });

  it("dispatches setPaneActiveTab when a tab is clicked", async () => {
    usePaneTabStripHandlers();
    const view = renderPaneTabStrip();
    seedPaneTabs(view);
    const dispatchSpy = vi.spyOn(view.store, "dispatch");
    view.rerender(<PaneTabStrip leafId="root" />);

    await userEvent.click(screen.getByRole("tab", { name: /Chat Beta/ }));

    expect(dispatchSpy).toHaveBeenCalledWith(
      setPaneActiveTab({ leafId: "root", tabId: "chat-b" }),
    );
  });
});
