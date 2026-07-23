import { act } from "react-dom/test-utils";
import { describe, expect, it, vi } from "vitest";

import { type AppStore, setUpStore } from "../../app/store";
import { fireEvent, render, screen, within } from "../../utils/test-utils";
import {
  createChatWithId,
  updateChatRuntimeFromSessionState,
} from "../Chat/Thread";
import {
  hasTabDragType,
  readTabDragData,
  readTabDragSurfaceKey,
  setTabDragData,
} from "../ChatPanes/tabDrag";
import { notificationAdded } from "../Notifications";
import type { ProcessCompletedNotification } from "../Notifications/notificationsSlice";
import { push } from "../Pages/pagesSlice";
import { updateConfig } from "../Config/configSlice";
import { openTask, reorderOpenTasks } from "../Tasks/tasksSlice";
import {
  addSurfaceToPane,
  closeTab,
  openTab,
  reorderTabs,
  setActiveTab,
  splitTab,
} from "./workspaceSlice";
import { makeSurfaceKey, type SurfaceKey } from "./surfaceKey";
import { TabBar } from "./TabBar";
import styles from "./TabBar.module.css";

const chat = (id: string): SurfaceKey => makeSurfaceKey("chat", id);
const task = (id: string): SurfaceKey => makeSurfaceKey("task", id);

function renderTabBar(store: AppStore) {
  return render(<TabBar />, { store });
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

function createProcessCompletedNotification(
  chatId: string,
  seq: string,
): ProcessCompletedNotification {
  return {
    id: `${chatId}:exec_${seq}:${seq}`,
    threadId: chatId,
    seq,
    processId: `exec_${seq}`,
    status: "failed",
    exitCode: 1,
    shortDescription: "Run workspace tab bar test",
    mode: "background",
    receivedAt: Date.now() + Number(seq),
  };
}

function createStoreWithChatTabs(): AppStore {
  const store = setUpStore();
  store.dispatch(
    createChatWithId({ id: "chat-a", title: "Chat Alpha", mode: "agent" }),
  );
  store.dispatch(
    createChatWithId({ id: "chat-b", title: "Chat Beta", mode: "agent" }),
  );
  store.dispatch(
    createChatWithId({ id: "chat-c", title: "Chat Gamma", mode: "agent" }),
  );
  store.dispatch(
    updateChatRuntimeFromSessionState({
      id: "chat-b",
      session_state: "generating",
    }),
  );
  store.dispatch(openTab(chat("chat-a")));
  store.dispatch(openTab(chat("chat-b")));
  store.dispatch(openTab(chat("chat-c")));
  store.dispatch(setActiveTab(chat("chat-a")));
  return store;
}

function createStoreWithGroupedTabs(): AppStore {
  const store = createStoreWithChatTabs();
  const groupTabId = chat("chat-a");
  store.dispatch(splitTab({ tabId: groupTabId, dir: "row" }));
  const group = store.getState().workspace.groups[groupTabId];
  if (!group) throw new Error("missing split group");
  store.dispatch(
    addSurfaceToPane({
      tabId: groupTabId,
      leafId: group.focusedLeafId,
      surfaceKey: chat("chat-b"),
    }),
  );
  return store;
}

function getTabWrap(name: RegExp): HTMLElement {
  const wrap = screen.getByRole("tab", { name }).closest("div");
  if (!wrap) throw new Error(`missing tab wrapper for ${name.source}`);
  return wrap;
}

describe("TabBar", () => {
  it("shows the dock toggle without a center-panel launcher", () => {
    const store = createStoreWithChatTabs();
    renderTabBar(store);
    expect(
      screen.getByRole("button", { name: "Toggle workspace dock" }),
    ).toBeInTheDocument();

    expect(
      screen.queryByRole("button", { name: "Open workspace panel" }),
    ).not.toBeInTheDocument();
  });

  it("renders the existing tab UI without panel chrome for IDE hosts", () => {
    const store = createStoreWithChatTabs();
    store.dispatch(updateConfig({ host: "vscode" }));
    renderTabBar(store);

    expect(
      screen.queryByRole("button", { name: "Open workspace panel" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Toggle workspace dock" }),
    ).not.toBeInTheDocument();
    expect(screen.getAllByRole("tab")).toHaveLength(3);
    expect(screen.getByRole("tab", { name: /Chat Alpha/ })).toBeInTheDocument();
  });

  it("renders all open tabs in one tablist with status and unread badges", () => {
    const store = createStoreWithChatTabs();
    store.dispatch({
      type: notificationAdded.type,
      payload: createProcessCompletedNotification("chat-a", "1"),
    });
    renderTabBar(store);

    expect(screen.getAllByRole("tablist")).toHaveLength(1);
    expect(screen.getAllByRole("tab")).toHaveLength(3);
    expect(screen.getByRole("tab", { name: /Chat Alpha/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(
      within(screen.getByRole("tab", { name: /Chat Beta/ })).getByLabelText(
        "In progress...",
      ),
    ).toBeInTheDocument();
    expect(
      within(screen.getByRole("tab", { name: /Chat Alpha/ })).getByLabelText(
        "1 unread process notifications",
      ),
    ).toHaveTextContent("1");
  });

  it("shows a split tab as a compact group with pane count and active pane title", () => {
    const store = createStoreWithGroupedTabs();
    renderTabBar(store);

    expect(screen.getAllByRole("tablist")).toHaveLength(1);
    expect(screen.getAllByRole("tab")).toHaveLength(2);
    const groupTab = screen.getByRole("tab", { name: /Chat Beta/ });

    expect(groupTab).toHaveAttribute("aria-selected", "true");
    expect(within(groupTab).getByLabelText("2 panes")).toHaveTextContent("2");
    expect(
      screen.queryByRole("tab", { name: /Chat Alpha/ }),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Chat Gamma/ })).toBeInTheDocument();
  });

  it("dispatches setActiveTab when a tab is clicked", async () => {
    const store = createStoreWithChatTabs();
    const dispatchSpy = vi.spyOn(store, "dispatch");
    const view = renderTabBar(store);

    await view.user.click(screen.getByRole("tab", { name: /Chat Beta/ }));

    expect(dispatchSpy).toHaveBeenCalledWith(setActiveTab(chat("chat-b")));
    expect(store.getState().workspace.activeTabId).toBe(chat("chat-b"));
  });

  it("dispatches closeTab from the close button", async () => {
    const store = createStoreWithChatTabs();
    const dispatchSpy = vi.spyOn(store, "dispatch");
    const view = renderTabBar(store);

    await view.user.click(
      within(getTabWrap(/Chat Beta/)).getByLabelText("Close Chat Beta"),
    );

    expect(dispatchSpy).toHaveBeenCalledWith(closeTab(chat("chat-b")));
    expect(store.getState().workspace.tabs).toEqual([
      chat("chat-a"),
      chat("chat-c"),
    ]);
  });

  it("navigates home when the last workspace tab is closed from the chat page", async () => {
    const store = setUpStore();
    store.dispatch(
      createChatWithId({ id: "chat-a", title: "Chat Alpha", mode: "agent" }),
    );
    store.dispatch(openTab(chat("chat-a")));
    store.dispatch(setActiveTab(chat("chat-a")));
    store.dispatch(push({ name: "history" }));
    store.dispatch(push({ name: "chat" }));
    const view = renderTabBar(store);

    await view.user.click(
      within(getTabWrap(/Chat Alpha/)).getByLabelText("Close Chat Alpha"),
    );

    expect(store.getState().workspace.tabs).toEqual([]);
    expect(store.getState().workspace.activeTabId).toBeNull();
    expect(store.getState().pages.at(-1)).toEqual({ name: "history" });
  });

  it("stays on the chat page when a non-last workspace tab is closed", async () => {
    const store = createStoreWithChatTabs();
    store.dispatch(push({ name: "history" }));
    store.dispatch(push({ name: "chat" }));
    const view = renderTabBar(store);

    await view.user.click(
      within(getTabWrap(/Chat Beta/)).getByLabelText("Close Chat Beta"),
    );

    expect(store.getState().workspace.tabs).toEqual([
      chat("chat-a"),
      chat("chat-c"),
    ]);
    expect(store.getState().pages.at(-1)).toEqual({ name: "chat" });
  });

  it("does not redirect home when the last chat tab is closed from a non-chat page", async () => {
    const store = setUpStore();
    store.dispatch(
      createChatWithId({ id: "chat-a", title: "Chat Alpha", mode: "agent" }),
    );
    store.dispatch(openTab(chat("chat-a")));
    store.dispatch(setActiveTab(chat("chat-a")));
    store.dispatch(openTask({ id: "task-a", name: "Task Alpha" }));
    store.dispatch(push({ name: "history" }));
    store.dispatch(push({ name: "task workspace", taskId: "task-a" }));
    const view = renderTabBar(store);

    await view.user.click(
      within(getTabWrap(/Chat Alpha/)).getByLabelText("Close Chat Alpha"),
    );

    expect(store.getState().workspace.tabs).toEqual([]);
    expect(store.getState().pages.at(-1)).toEqual({
      name: "task workspace",
      taskId: "task-a",
    });
  });

  it("renders task and buddy navigation tabs without split controls", async () => {
    const store = createStoreWithChatTabs();
    store.dispatch(openTask({ id: "task-a", name: "Task Alpha" }));
    store.dispatch(push({ name: "buddy" }));
    store.dispatch(push({ name: "chat" }));
    const dispatchSpy = vi.spyOn(store, "dispatch");
    const view = renderTabBar(store);

    expect(screen.getByRole("tab", { name: /Task Alpha/ })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Buddy/ })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Split/ })).toBeNull();

    await view.user.click(screen.getByRole("tab", { name: /Task Alpha/ }));

    expect(dispatchSpy).toHaveBeenCalledWith(
      push({ name: "task workspace", taskId: "task-a" }),
    );
    expect(store.getState().pages.at(-1)).toEqual({
      name: "task workspace",
      taskId: "task-a",
    });
    expect(screen.getByRole("tab", { name: /Task Alpha/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );

    await view.user.click(screen.getByRole("tab", { name: /Buddy/ }));

    expect(dispatchSpy).toHaveBeenCalledWith(push({ name: "buddy" }));
    expect(store.getState().pages.at(-1)).toEqual({ name: "buddy" });
  });

  it("makes chat, task, and buddy top-bar tabs draggable", () => {
    const store = createStoreWithChatTabs();
    store.dispatch(openTask({ id: "task-a", name: "Task Alpha" }));
    store.dispatch(push({ name: "buddy" }));
    store.dispatch(push({ name: "chat" }));
    renderTabBar(store);

    expect(screen.getByRole("tab", { name: /Chat Alpha/ })).toHaveAttribute(
      "draggable",
      "true",
    );
    expect(screen.getByRole("tab", { name: /Task Alpha/ })).toHaveAttribute(
      "draggable",
      "true",
    );
    expect(screen.getByRole("tab", { name: /Buddy/ })).toHaveAttribute(
      "draggable",
      "true",
    );
  });

  it("makes grouped top-bar tabs non-draggable while ordinary chat tabs stay draggable", () => {
    const store = createStoreWithGroupedTabs();
    renderTabBar(store);

    expect(screen.getByRole("tab", { name: /Chat Beta/ })).toHaveAttribute(
      "draggable",
      "false",
    );
    expect(screen.getByRole("tab", { name: /Chat Gamma/ })).toHaveAttribute(
      "draggable",
      "true",
    );
  });

  it("does not start a tab drag from a grouped top-bar tab", () => {
    const store = createStoreWithGroupedTabs();
    renderTabBar(store);
    const dataTransfer = createDataTransferStub();

    fireEvent.dragStart(screen.getByRole("tab", { name: /Chat Beta/ }), {
      dataTransfer,
    });

    expect(dataTransfer.types).toEqual([]);
    expect(dataTransfer.effectAllowed).toBe("uninitialized");
  });

  it("requires Refact drag MIME provenance before reading tab drag data", () => {
    const foreignDataTransfer = createDataTransferStub();
    foreignDataTransfer.setData("text/plain", "chat:chat-x");

    expect(readTabDragData(foreignDataTransfer)).toBeNull();
    expect(readTabDragSurfaceKey(foreignDataTransfer)).toBeNull();
    expect(hasTabDragType(foreignDataTransfer, "chat")).toBe(false);

    const refactDataTransfer = createDataTransferStub();
    setTabDragData(refactDataTransfer, "chat", "chat-a", chat("chat-a"));

    expect(readTabDragData(refactDataTransfer)).toEqual({
      type: "chat",
      id: "chat-a",
      surfaceKey: chat("chat-a"),
    });
    expect(readTabDragSurfaceKey(refactDataTransfer)).toBe(chat("chat-a"));
    expect(hasTabDragType(refactDataTransfer, "chat")).toBe(true);
  });

  it("closes task and active buddy navigation tabs without closing chat tabs", async () => {
    const store = createStoreWithChatTabs();
    store.dispatch(openTask({ id: "task-a", name: "Task Alpha" }));
    store.dispatch(push({ name: "task workspace", taskId: "task-a" }));
    const dispatchSpy = vi.spyOn(store, "dispatch");
    const view = renderTabBar(store);

    await view.user.click(
      within(getTabWrap(/Task Alpha/)).getByLabelText("Close Task Alpha"),
    );

    expect(store.getState().tasksUI.openTasks).toEqual([]);
    expect(store.getState().pages.at(-1)).toEqual({ name: "history" });
    expect(store.getState().workspace.tabs).toEqual([
      chat("chat-a"),
      chat("chat-b"),
      chat("chat-c"),
    ]);
    expect(dispatchSpy).not.toHaveBeenCalledWith(closeTab(task("task-a")));

    act(() => {
      store.dispatch(push({ name: "buddy" }));
    });

    await view.user.click(
      within(getTabWrap(/Buddy/)).getByLabelText("Close Buddy"),
    );

    expect(store.getState().pages.at(-1)).toEqual({ name: "history" });
    expect(store.getState().workspace.tabs).toEqual([
      chat("chat-a"),
      chat("chat-b"),
      chat("chat-c"),
    ]);
  });

  it("dispatches reorderTabs when a tab is dropped on another tab", () => {
    const store = createStoreWithChatTabs();
    const dispatchSpy = vi.spyOn(store, "dispatch");
    renderTabBar(store);

    const dataTransfer = createDataTransferStub();
    fireEvent.dragStart(screen.getByRole("tab", { name: /Chat Gamma/ }), {
      dataTransfer,
    });
    const target = getTabWrap(/Chat Alpha/);
    fireEvent.dragOver(target, { dataTransfer });
    fireEvent.drop(target, { dataTransfer });

    expect(dispatchSpy).toHaveBeenCalledWith(
      reorderTabs({ sourceKey: chat("chat-c"), targetKey: chat("chat-a") }),
    );
    expect(store.getState().workspace.tabs).toEqual([
      chat("chat-c"),
      chat("chat-a"),
      chat("chat-b"),
    ]);
  });

  it("dispatches reorderOpenTasks when a task tab is dropped on another task tab", () => {
    const store = createStoreWithChatTabs();
    store.dispatch(openTask({ id: "task-a", name: "Task Alpha" }));
    store.dispatch(openTask({ id: "task-b", name: "Task Beta" }));
    const dispatchSpy = vi.spyOn(store, "dispatch");
    renderTabBar(store);

    const dataTransfer = createDataTransferStub();
    fireEvent.dragStart(screen.getByRole("tab", { name: /Task Beta/ }), {
      dataTransfer,
    });
    const target = getTabWrap(/Task Alpha/);
    fireEvent.dragOver(target, { dataTransfer });
    fireEvent.drop(target, { dataTransfer });

    expect(dispatchSpy).toHaveBeenCalledWith(
      reorderOpenTasks({ sourceId: "task-b", targetId: "task-a" }),
    );
    expect(store.getState().tasksUI.openTasks.map((task) => task.id)).toEqual([
      "task-b",
      "task-a",
    ]);
  });

  it("keeps tab labels readable with a min-width wrap and a full-title tooltip", () => {
    const store = setUpStore();
    store.dispatch(
      createChatWithId({
        id: "chat-long",
        title: "Terminal Adjacent Extremely Long Chat Title",
        mode: "agent",
      }),
    );
    store.dispatch(openTab(chat("chat-long")));
    store.dispatch(setActiveTab(chat("chat-long")));
    renderTabBar(store);

    const tab = screen.getByRole("tab", { name: /Terminal Adjacent/ });
    expect(tab).toHaveAttribute(
      "title",
      "Terminal Adjacent Extremely Long Chat Title",
    );
    expect(getTabWrap(/Terminal Adjacent/)).toHaveClass(styles.tabWrap);
  });
});
