import { http, HttpResponse } from "msw";
import { afterEach, describe, expect, it, vi } from "vitest";
import { act } from "react-dom/test-utils";
import userEvent from "@testing-library/user-event";

import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { Toolbar, type Tab } from "./Toolbar";
import {
  createChatWithId,
  newBuddyChatAction,
  updateChatRuntimeFromSessionState,
} from "../../features/Chat/Thread";
import { processCompleted } from "../../features/Notifications";
import type { ProcessCompletedEvent } from "../../features/Notifications";
import { openTask } from "../../features/Tasks";
import type { TaskMeta } from "../../services/refact/tasks";

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

function useToolbarHandlers(tasks: TaskMeta[] = []) {
  server.use(
    http.get("*/v1/tasks", () => HttpResponse.json(tasks)),
    http.get("*/v1/chat-modes", () => HttpResponse.json(chatModesResponse)),
    http.post("*/v1/chats/:id/commands", () =>
      HttpResponse.json({ status: "queued" }),
    ),
  );
}

function renderToolbar(activeTab: Tab) {
  return render(<Toolbar activeTab={activeTab} />, {
    preloadedState: { config: baseConfig },
  });
}

function dispatchAndRerender(
  view: ReturnType<typeof renderToolbar>,
  activeTab: Tab,
  callback: () => void,
) {
  act(callback);
  view.rerender(<Toolbar activeTab={activeTab} />);
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
    short_description: "Run toolbar parity test",
    mode: "background",
  };
}

function getTabWrap(title: string): HTMLElement {
  const wrap = screen.getByTitle(title).closest("div");
  if (!wrap) throw new Error(`missing tab wrapper for ${title}`);
  return wrap;
}

function expectTabToContainStatus(title: string, label: string) {
  const tab = screen.getByTitle(title);
  const status = within(tab).getByLabelText(label);
  expect(tab).toContainElement(status);
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

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("Dropdown navigation", () => {
  it("clicking Settings dispatches push({name:'general settings'})", async () => {
    useToolbarHandlers();
    const { store } = renderToolbar({ type: "dashboard" });

    await userEvent.click(screen.getByRole("button", { name: "Menu" }));
    await userEvent.click(
      await screen.findByRole("menuitem", { name: "Settings" }),
    );

    expect(store.getState().pages.at(-1)?.name).toBe("general settings");
  });

  it("clicking Extension Settings sends openSettings postMessage", async () => {
    useToolbarHandlers();
    const postMessageSpy = vi.spyOn(window, "postMessage");

    renderToolbar({ type: "dashboard" });

    await userEvent.click(screen.getByRole("button", { name: "Menu" }));
    await userEvent.click(
      await screen.findByRole("menuitem", { name: "Extension Settings" }),
    );

    expect(postMessageSpy).toHaveBeenCalledWith(
      expect.objectContaining({ type: "ide/openSettings" }),
      "*",
    );
  });
});

describe("Toolbar tab parity", () => {
  it("closes the active chat tab and falls back to the first remaining chat tab", async () => {
    useToolbarHandlers();
    const view = renderToolbar({ type: "chat", id: "chat-b" });

    const initialThreadId = view.store.getState().chat.open_thread_ids[0];
    dispatchAndRerender(view, { type: "chat", id: "chat-b" }, () => {
      if (initialThreadId) {
        view.store.dispatch({
          type: "chatThread/closeThread",
          payload: { id: initialThreadId },
        });
      }
      view.store.dispatch(
        createChatWithId({ id: "chat-a", title: "Chat Alpha" }),
      );
      view.store.dispatch(
        createChatWithId({ id: "chat-b", title: "Chat Beta" }),
      );
    });

    await userEvent.click(
      within(getTabWrap("Chat Beta")).getByTitle("Close tab"),
    );

    expect(view.store.getState().chat.open_thread_ids).toContain("chat-a");
    expect(view.store.getState().chat.open_thread_ids).not.toContain("chat-b");
    expect(view.store.getState().chat.current_thread_id).toBe("chat-a");
    expect(view.store.getState().pages.at(-1)?.name).toBe("chat");
  });

  it("closes the last active chat tab and falls back to the dashboard", async () => {
    useToolbarHandlers();
    const view = renderToolbar({ type: "chat", id: "solo-chat" });

    const initialThreadId = view.store.getState().chat.open_thread_ids[0];
    if (!initialThreadId) throw new Error("missing initial thread");
    dispatchAndRerender(view, { type: "chat", id: "solo-chat" }, () => {
      view.store.dispatch(
        createChatWithId({ id: "solo-chat", title: "Solo Chat" }),
      );
      view.store.dispatch({
        type: "chatThread/closeThread",
        payload: { id: initialThreadId },
      });
    });

    await userEvent.click(
      within(getTabWrap("Solo Chat")).getByTitle("Close tab"),
    );

    expect(view.store.getState().chat.open_thread_ids).not.toContain(
      "solo-chat",
    );
    expect(view.store.getState().pages.at(-1)?.name).toBe("history");
  });

  it("closes the active task tab and falls back to the dashboard", async () => {
    useToolbarHandlers();
    const view = renderToolbar({
      type: "task",
      taskId: "task-a",
      taskName: "Task Alpha",
    });

    dispatchAndRerender(
      view,
      { type: "task", taskId: "task-a", taskName: "Task Alpha" },
      () => {
        view.store.dispatch(openTask({ id: "task-a", name: "Task Alpha" }));
      },
    );

    await userEvent.click(
      within(getTabWrap("Task Alpha")).getByTitle("Close task tab"),
    );

    expect(view.store.getState().tasksUI.openTasks).toEqual([]);
    expect(view.store.getState().pages.at(-1)?.name).toBe("history");
  });

  it("closes a chat tab with middle click", () => {
    useToolbarHandlers();
    const view = renderToolbar({ type: "chat", id: "middle-chat" });

    const initialThreadId = view.store.getState().chat.open_thread_ids[0];
    dispatchAndRerender(view, { type: "chat", id: "middle-chat" }, () => {
      if (initialThreadId) {
        view.store.dispatch({
          type: "chatThread/closeThread",
          payload: { id: initialThreadId },
        });
      }
      view.store.dispatch(
        createChatWithId({ id: "middle-chat", title: "Middle Chat" }),
      );
    });

    screen.getByTitle("Middle Chat").dispatchEvent(
      new MouseEvent("auxclick", {
        button: 1,
        bubbles: true,
        cancelable: true,
      }),
    );

    expect(view.store.getState().chat.open_thread_ids).not.toContain(
      "middle-chat",
    );
    expect(view.store.getState().pages.at(-1)?.name).toBe("history");
  });

  it("persists reordered chat tabs after drag and drop", () => {
    useToolbarHandlers();
    const view = renderToolbar({ type: "chat", id: "chat-a" });

    const initialThreadId = view.store.getState().chat.open_thread_ids[0];
    dispatchAndRerender(view, { type: "chat", id: "chat-a" }, () => {
      if (initialThreadId) {
        view.store.dispatch({
          type: "chatThread/closeThread",
          payload: { id: initialThreadId },
        });
      }
      view.store.dispatch(
        createChatWithId({ id: "chat-a", title: "Chat Alpha" }),
      );
      view.store.dispatch(
        createChatWithId({ id: "chat-b", title: "Chat Beta" }),
      );
    });

    const dragged = screen.getByTitle("Chat Beta");
    const target = getTabWrap("Chat Alpha");
    const { dataTransfer } = createDataTransferStub();

    const dragStart = new Event("dragstart", { bubbles: true });
    Object.defineProperty(dragStart, "dataTransfer", { value: dataTransfer });
    dragged.dispatchEvent(dragStart);
    const drop = new Event("drop", { bubbles: true, cancelable: true });
    Object.defineProperty(drop, "dataTransfer", { value: dataTransfer });
    target.dispatchEvent(drop);

    expect(view.store.getState().chat.open_thread_ids).toEqual([
      "chat-b",
      "chat-a",
    ]);
  });

  it("closes an inactive chat tab without switching to it", async () => {
    useToolbarHandlers();
    const view = renderToolbar({ type: "chat", id: "chat-a" });

    const initialThreadId = view.store.getState().chat.open_thread_ids[0];
    dispatchAndRerender(view, { type: "chat", id: "chat-a" }, () => {
      if (initialThreadId) {
        view.store.dispatch({
          type: "chatThread/closeThread",
          payload: { id: initialThreadId },
        });
      }
      view.store.dispatch(
        createChatWithId({ id: "chat-a", title: "Chat Alpha" }),
      );
      view.store.dispatch(
        createChatWithId({ id: "chat-b", title: "Chat Beta" }),
      );
    });

    const pagesLengthBefore = view.store.getState().pages.length;
    const pagesTopBefore = view.store.getState().pages.at(-1)?.name;

    await userEvent.click(
      within(getTabWrap("Chat Beta")).getByTitle("Close tab"),
    );

    expect(view.store.getState().chat.open_thread_ids).not.toContain("chat-b");
    expect(view.store.getState().pages.length).toBe(pagesLengthBefore);
    expect(view.store.getState().pages.at(-1)?.name).toBe(pagesTopBefore);
  });

  it("closes an inactive task tab without switching to it", async () => {
    useToolbarHandlers();
    const view = renderToolbar({
      type: "task",
      taskId: "task-a",
      taskName: "Task Alpha",
    });

    dispatchAndRerender(
      view,
      { type: "task", taskId: "task-a", taskName: "Task Alpha" },
      () => {
        view.store.dispatch(openTask({ id: "task-a", name: "Task Alpha" }));
        view.store.dispatch(openTask({ id: "task-b", name: "Task Beta" }));
      },
    );

    const pagesLengthBefore = view.store.getState().pages.length;
    const pagesTopBefore = view.store.getState().pages.at(-1)?.name;

    await userEvent.click(
      within(getTabWrap("Task Beta")).getByTitle("Close task tab"),
    );

    expect(
      view.store.getState().tasksUI.openTasks.map((task) => task.id),
    ).toEqual(["task-a"]);
    expect(view.store.getState().pages.length).toBe(pagesLengthBefore);
    expect(view.store.getState().pages.at(-1)?.name).toBe(pagesTopBefore);
  });

  it("does not start tab drag from a close button", () => {
    useToolbarHandlers();
    const view = renderToolbar({ type: "chat", id: "chat-a" });

    const initialThreadId = view.store.getState().chat.open_thread_ids[0];
    dispatchAndRerender(view, { type: "chat", id: "chat-a" }, () => {
      if (initialThreadId) {
        view.store.dispatch({
          type: "chatThread/closeThread",
          payload: { id: initialThreadId },
        });
      }
      view.store.dispatch(
        createChatWithId({ id: "chat-a", title: "Chat Alpha" }),
      );
      view.store.dispatch(
        createChatWithId({ id: "chat-b", title: "Chat Beta" }),
      );
    });

    const closeButton = within(getTabWrap("Chat Beta")).getByTitle("Close tab");
    const { data, dataTransfer } = createDataTransferStub();

    fireEvent.dragStart(closeButton, { dataTransfer });

    expect(data.size).toBe(0);
    expect(view.store.getState().chat.open_thread_ids).toEqual([
      "chat-a",
      "chat-b",
    ]);
  });

  it("commits and cancels chat title renames from double-click rename mode", async () => {
    useToolbarHandlers();
    const view = renderToolbar({ type: "chat", id: "rename-chat" });

    dispatchAndRerender(view, { type: "chat", id: "rename-chat" }, () => {
      view.store.dispatch(
        createChatWithId({ id: "rename-chat", title: "Original Chat" }),
      );
    });

    await userEvent.dblClick(
      screen.getByRole("tab", { name: /Original Chat/ }),
    );
    const renameInput = screen.getByDisplayValue("Original Chat");
    await userEvent.clear(renameInput);
    await userEvent.type(renameInput, "Renamed Chat{Enter}");

    expect(
      view.store.getState().chat.threads["rename-chat"]?.thread.title,
    ).toBe("Renamed Chat");
    expect(
      screen.getByRole("tab", { name: /Renamed Chat/ }),
    ).toBeInTheDocument();

    await userEvent.dblClick(screen.getByRole("tab", { name: /Renamed Chat/ }));
    const cancelInput = screen.getByDisplayValue("Renamed Chat");
    await userEvent.clear(cancelInput);
    await userEvent.type(cancelInput, "Cancelled Chat{Escape}");

    expect(
      view.store.getState().chat.threads["rename-chat"]?.thread.title,
    ).toBe("Renamed Chat");
    expect(
      screen.getByRole("tab", { name: /Renamed Chat/ }),
    ).toBeInTheDocument();
  });

  it("commits and cancels task title renames from double-click rename mode", async () => {
    const updatedTask: TaskMeta = {
      id: "task-rename",
      name: "Renamed Task",
      status: "planning",
      created_at: "2026-06-07T00:00:00.000Z",
      updated_at: "2026-06-07T00:00:00.000Z",
      cards_total: 0,
      cards_done: 0,
      cards_failed: 0,
      agents_active: 0,
    };
    let patchBody: unknown;
    server.use(
      http.get("*/v1/tasks", () => HttpResponse.json([])),
      http.get("*/v1/chat-modes", () => HttpResponse.json(chatModesResponse)),
      http.patch("*/v1/tasks/task-rename/meta", async ({ request }) => {
        patchBody = await request.json();
        return HttpResponse.json(updatedTask);
      }),
    );
    const view = renderToolbar({
      type: "task",
      taskId: "task-rename",
      taskName: "Original Task",
    });

    dispatchAndRerender(
      view,
      {
        type: "task",
        taskId: "task-rename",
        taskName: "Original Task",
      },
      () => {
        view.store.dispatch(
          openTask({ id: "task-rename", name: "Original Task" }),
        );
      },
    );

    await userEvent.dblClick(
      screen.getByRole("tab", { name: /Original Task/ }),
    );
    const renameInput = screen.getByDisplayValue("Original Task");
    await userEvent.clear(renameInput);
    await userEvent.type(renameInput, "Renamed Task{Enter}");

    await waitFor(() => expect(patchBody).toEqual({ name: "Renamed Task" }));

    await userEvent.dblClick(
      screen.getByRole("tab", { name: /Original Task/ }),
    );
    const cancelInput = screen.getByDisplayValue("Original Task");
    await userEvent.clear(cancelInput);
    await userEvent.type(cancelInput, "Cancelled Task{Escape}");

    expect(
      screen.getByRole("tab", { name: /Original Task/ }),
    ).toBeInTheDocument();
  });

  it("renders unread process notification badges and caps counts above nine", () => {
    useToolbarHandlers();
    const view = renderToolbar({ type: "dashboard" });

    dispatchAndRerender(view, { type: "dashboard" }, () => {
      view.store.dispatch(
        createChatWithId({ id: "badge-chat", title: "Badge Chat" }),
      );
      for (let i = 1; i <= 10; i += 1) {
        view.store.dispatch(
          processCompleted(
            createProcessCompletedEvent("badge-chat", String(i)),
          ),
        );
      }
    });

    expect(
      screen.getByLabelText("10 unread process notifications"),
    ).toHaveTextContent("9+");
  });

  it("maps chat and task states to the current status dot labels", async () => {
    const task: TaskMeta = {
      id: "task-status",
      name: "Task Status",
      status: "planning",
      created_at: "2026-06-07T00:00:00.000Z",
      updated_at: "2026-06-07T00:00:00.000Z",
      cards_total: 0,
      cards_done: 0,
      cards_failed: 0,
      agents_active: 0,
      planner_session_state: "waiting_ide",
    };
    useToolbarHandlers([task]);
    const view = renderToolbar({ type: "dashboard" });

    dispatchAndRerender(view, { type: "dashboard" }, () => {
      view.store.dispatch(
        createChatWithId({ id: "idle-chat", title: "Idle Chat" }),
      );
      view.store.dispatch(
        createChatWithId({ id: "running-chat", title: "Running Chat" }),
      );
      view.store.dispatch(
        createChatWithId({ id: "paused-chat", title: "Paused Chat" }),
      );
      view.store.dispatch(
        createChatWithId({ id: "done-chat", title: "Done Chat" }),
      );
      view.store.dispatch(
        createChatWithId({ id: "error-chat", title: "Error Chat" }),
      );
      view.store.dispatch(
        updateChatRuntimeFromSessionState({
          id: "running-chat",
          session_state: "generating",
        }),
      );
      view.store.dispatch(
        updateChatRuntimeFromSessionState({
          id: "paused-chat",
          session_state: "waiting_user_input",
        }),
      );
      view.store.dispatch(
        updateChatRuntimeFromSessionState({
          id: "done-chat",
          session_state: "completed",
        }),
      );
      view.store.dispatch(
        updateChatRuntimeFromSessionState({
          id: "error-chat",
          session_state: "error",
        }),
      );
      view.store.dispatch(openTask({ id: "task-status", name: "Task Status" }));
    });

    expectTabToContainStatus("Idle Chat", "Idle");
    expectTabToContainStatus("Running Chat", "In progress...");
    expectTabToContainStatus("Paused Chat", "Needs your attention");
    expectTabToContainStatus("Done Chat", "Completed");
    expectTabToContainStatus("Error Chat", "An error occurred");

    await waitFor(() => {
      expectTabToContainStatus("Task Status", "Needs your attention");
    });
  });

  it("excludes buddy and task-owned chat threads from the tab strip", () => {
    useToolbarHandlers();
    const view = renderToolbar({ type: "dashboard" });

    dispatchAndRerender(view, { type: "dashboard" }, () => {
      view.store.dispatch(
        createChatWithId({ id: "normal-chat", title: "Normal Chat" }),
      );
      view.store.dispatch(newBuddyChatAction({ chat_id: "buddy-chat" }));
      view.store.dispatch(
        createChatWithId({
          id: "task-owned-chat",
          title: "Task Owned Chat",
          isTaskChat: true,
        }),
      );
    });

    expect(
      screen.getByRole("tab", { name: /Normal Chat/ }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("tab", { name: /buddy-chat/i }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("tab", { name: /Task Owned Chat/ }),
    ).not.toBeInTheDocument();
  });
  it("wheel-scrolls an overflowing tab container horizontally", () => {
    useToolbarHandlers();
    renderToolbar({ type: "dashboard" });

    const tabList = screen.getByRole("tablist");
    const container = tabList.parentElement;
    if (!container) throw new Error("missing tabs container");
    Object.defineProperty(container, "scrollWidth", {
      configurable: true,
      value: 300,
    });
    Object.defineProperty(container, "clientWidth", {
      configurable: true,
      value: 100,
    });

    expect(container.scrollLeft).toBe(0);
    container.dispatchEvent(
      new WheelEvent("wheel", { deltaY: 24, bubbles: true, cancelable: true }),
    );

    expect(container.scrollLeft).toBe(24);
  });

  it("scrolls the active tab into view when the active tab changes", () => {
    useToolbarHandlers();
    const scrollIntoView = vi.spyOn(Element.prototype, "scrollIntoView");

    const view = renderToolbar({ type: "chat", id: "chat-a" });

    dispatchAndRerender(view, { type: "chat", id: "chat-a" }, () => {
      view.store.dispatch(
        createChatWithId({ id: "chat-a", title: "Chat Alpha" }),
      );
      view.store.dispatch(
        createChatWithId({ id: "chat-b", title: "Chat Beta" }),
      );
    });

    view.rerender(<Toolbar activeTab={{ type: "chat", id: "chat-b" }} />);

    expect(scrollIntoView).toHaveBeenCalledWith({
      behavior: "smooth",
      block: "nearest",
      inline: "nearest",
    });
  });
});
