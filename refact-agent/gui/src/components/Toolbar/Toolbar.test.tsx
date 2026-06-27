import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { http, HttpResponse } from "msw";
import { act } from "react-dom/test-utils";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { render, screen, waitFor } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { Toolbar, type Tab } from "./Toolbar";
import { createChatWithId, switchToThread } from "../../features/Chat/Thread";
import { push } from "../../features/Pages/pagesSlice";
import { openTask } from "../../features/Tasks";
import {
  makeSurfaceKey,
  openTab,
  setActiveTab,
} from "../../features/Workspace";
import type { TaskMeta } from "../../services/refact/tasks";

const baseConfig = {
  host: "web" as const,
  lspPort: 8001,
  lspUrl: "http://127.0.0.1:8001/v1/ping/Refact",
  themeProps: { appearance: "dark" as const },
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

const createdTask: TaskMeta = {
  id: "task-new",
  name: "New Task",
  status: "planning",
  created_at: "2026-06-07T00:00:00.000Z",
  updated_at: "2026-06-07T00:00:00.000Z",
  cards_total: 0,
  cards_done: 0,
  cards_failed: 0,
  agents_active: 0,
};

function useToolbarHandlers(tasks: TaskMeta[] = []) {
  server.use(
    http.get("*/v1/tasks", () => HttpResponse.json(tasks)),
    http.post("*/v1/tasks", () => HttpResponse.json(createdTask)),
    http.get("*/v1/chat-modes", () => HttpResponse.json(chatModesResponse)),
    http.post("*/v1/chats/:id/commands", () =>
      HttpResponse.json({ status: "queued" }),
    ),
  );
}

function pagesForActiveTab(activeTab: Tab) {
  if (activeTab.type === "dashboard") return [{ name: "history" as const }];
  if (activeTab.type === "task") {
    return [
      { name: "history" as const },
      { name: "task workspace" as const, taskId: activeTab.taskId },
    ];
  }
  if (activeTab.type === "buddy") {
    return [{ name: "history" as const }, { name: "buddy" as const }];
  }
  return [{ name: "history" as const }, { name: "chat" as const }];
}

function renderToolbar(activeTab: Tab) {
  return render(<Toolbar activeTab={activeTab} />, {
    preloadedState: {
      config: baseConfig,
      pages: pagesForActiveTab(activeTab),
    },
  });
}

function rerenderToolbar(
  view: ReturnType<typeof renderToolbar>,
  activeTab: Tab,
) {
  view.rerender(<Toolbar activeTab={activeTab} />);
}

function arrangeFocusedWorkspaceChatWithHiddenCurrent(
  view: ReturnType<typeof renderToolbar>,
  focusedId = "chat-focused",
  hiddenId = "chat-hidden",
) {
  const focusedSurface = makeSurfaceKey("chat", focusedId);

  act(() => {
    view.store.dispatch(createChatWithId({ id: focusedId, title: "Focused" }));
    view.store.dispatch(openTab(focusedSurface));
    view.store.dispatch(setActiveTab(focusedSurface));
    view.store.dispatch(
      createChatWithId({ id: hiddenId, title: "Hidden", openTab: false }),
    );
    view.store.dispatch(switchToThread({ id: hiddenId, openTab: false }));
  });
}

async function expectNewChatCleansFocusedWorkspaceChat(
  view: ReturnType<typeof renderToolbar>,
  focusedId = "chat-focused",
  hiddenId = "chat-hidden",
) {
  await view.user.click(screen.getByRole("button", { name: "New Chat" }));

  await waitFor(() => {
    expect(view.store.getState().chat.current_thread_id).not.toBe(hiddenId);
  });
  expect(view.store.getState().chat.threads[focusedId]).toBeUndefined();
  expect(view.store.getState().chat.threads[hiddenId]).toBeDefined();
  expect(view.store.getState().pages.at(-1)?.name).toBe("chat");
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

describe("Toolbar single workspace tab row", () => {
  it("renders the unified workspace tab bar on chat and task pages without legacy KitTabs", () => {
    useToolbarHandlers();
    const activeTab = { type: "chat" as const, id: "chat-a" };
    const view = renderToolbar(activeTab);
    const chatA = makeSurfaceKey("chat", "chat-a");

    act(() => {
      view.store.dispatch(
        createChatWithId({ id: "chat-a", title: "Chat Alpha" }),
      );
      view.store.dispatch(openTab(chatA));
      view.store.dispatch(setActiveTab(chatA));
      view.store.dispatch(openTask({ id: "task-a", name: "Task Alpha" }));
    });
    rerenderToolbar(view, activeTab);

    expect(screen.getAllByRole("tablist")).toHaveLength(1);
    expect(
      screen.getByRole("tablist", { name: "Open workspace tabs" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Chat Alpha/ })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Task Alpha/ })).toBeInTheDocument();

    act(() => {
      view.store.dispatch(push({ name: "task workspace", taskId: "task-a" }));
    });
    rerenderToolbar(view, {
      type: "task",
      taskId: "task-a",
      taskName: "Task Alpha",
    });

    expect(screen.getAllByRole("tablist")).toHaveLength(1);
    expect(screen.getByRole("tab", { name: /Task Alpha/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  it("renders the unified workspace tab bar on dashboard when tabs are open", () => {
    useToolbarHandlers();
    const view = renderToolbar({ type: "dashboard" });
    const chatA = makeSurfaceKey("chat", "chat-a");

    act(() => {
      view.store.dispatch(
        createChatWithId({ id: "chat-a", title: "Chat Alpha" }),
      );
      view.store.dispatch(openTab(chatA));
      view.store.dispatch(setActiveTab(chatA));
      view.store.dispatch(openTask({ id: "task-a", name: "Task Alpha" }));
    });
    rerenderToolbar(view, { type: "dashboard" });

    expect(screen.getAllByRole("tablist")).toHaveLength(1);
    expect(
      screen.getByRole("tablist", { name: "Open workspace tabs" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Chat Alpha/ })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Task Alpha/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Home" })).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "New Chat" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "New Task" }),
    ).toBeInTheDocument();
  });

  it("does not render an empty tab strip on dashboard", () => {
    useToolbarHandlers();
    renderToolbar({ type: "dashboard" });

    expect(screen.queryByRole("tablist")).not.toBeInTheDocument();
  });

  it("renders a flex spacer instead of the tab bar when no tabs are open", () => {
    useToolbarHandlers();
    const view = renderToolbar({ type: "dashboard" });

    expect(screen.queryByRole("tablist")).not.toBeInTheDocument();
    expect(
      view.container.querySelector('[data-element="ToolbarSpacer"]'),
    ).toBeInTheDocument();
  });

  it("drops the spacer when the workspace tab bar takes over the row", () => {
    useToolbarHandlers();
    const view = renderToolbar({ type: "dashboard" });
    const chatA = makeSurfaceKey("chat", "chat-a");

    act(() => {
      view.store.dispatch(
        createChatWithId({ id: "chat-a", title: "Chat Alpha" }),
      );
      view.store.dispatch(openTab(chatA));
      view.store.dispatch(setActiveTab(chatA));
    });
    rerenderToolbar(view, { type: "dashboard" });

    expect(screen.getByRole("tablist")).toBeInTheDocument();
    expect(
      view.container.querySelector('[data-element="ToolbarSpacer"]'),
    ).not.toBeInTheDocument();
  });

  it("keeps Home, New Chat, New Task, theme, and menu actions functional", async () => {
    useToolbarHandlers();
    const activeTab = { type: "chat" as const, id: "chat-a" };
    const view = renderToolbar(activeTab);
    const initialThreadId = view.store.getState().chat.current_thread_id;

    expect(screen.getByRole("button", { name: "Home" })).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "New Chat" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "New Task" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Toggle Dark Mode" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Menu" })).toBeInTheDocument();

    await view.user.click(screen.getByRole("button", { name: "Home" }));
    expect(view.store.getState().pages.at(-1)?.name).toBe("history");

    await view.user.click(screen.getByRole("button", { name: "New Chat" }));
    expect(view.store.getState().chat.current_thread_id).not.toBe(
      initialThreadId,
    );
    expect(view.store.getState().pages.at(-1)?.name).toBe("chat");

    await view.user.click(
      screen.getByRole("button", { name: "Toggle Dark Mode" }),
    );
    expect(view.store.getState().config.themeProps.appearance).toBe("light");

    await view.user.click(screen.getByRole("button", { name: "New Task" }));
    await waitFor(() => {
      expect(view.store.getState().tasksUI.openTasks).toEqual(
        expect.arrayContaining([
          expect.objectContaining({ id: "task-new", name: "New Task" }),
        ]),
      );
      expect(view.store.getState().pages.at(-1)).toEqual({
        name: "task workspace",
        taskId: "task-new",
      });
    });
  });

  it("uses the active workspace chat for New Chat cleanup", async () => {
    useToolbarHandlers();
    const activeTab = { type: "chat" as const, id: "chat-visible" };
    const view = renderToolbar(activeTab);

    act(() => {
      view.store.dispatch(
        createChatWithId({ id: "chat-visible", title: "Visible Chat" }),
      );
      view.store.dispatch(
        createChatWithId({
          id: "task-hidden",
          title: "Task Hidden",
          openTab: false,
        }),
      );
      view.store.dispatch(
        switchToThread({ id: "task-hidden", openTab: false }),
      );
    });
    rerenderToolbar(view, activeTab);

    await view.user.click(screen.getByRole("button", { name: "New Chat" }));

    await waitFor(() => {
      expect(view.store.getState().chat.current_thread_id).not.toBe(
        "task-hidden",
      );
    });
    expect(view.store.getState().chat.threads["chat-visible"]).toBeUndefined();
    expect(view.store.getState().chat.threads["task-hidden"]).toBeDefined();
    expect(view.store.getState().chat.open_thread_ids).not.toContain(
      "task-hidden",
    );
    expect(view.store.getState().chat.open_thread_ids).toContain(
      view.store.getState().chat.current_thread_id,
    );
    expect(view.store.getState().chat.current_thread_id).not.toBe(
      "chat-visible",
    );
  });

  it("uses the focused workspace chat for New Chat cleanup from dashboard", async () => {
    useToolbarHandlers();
    const view = renderToolbar({ type: "dashboard" });
    arrangeFocusedWorkspaceChatWithHiddenCurrent(view);
    rerenderToolbar(view, { type: "dashboard" });

    await expectNewChatCleansFocusedWorkspaceChat(view);
  });

  it("does not clean a hidden current chat from dashboard without focused workspace chat", async () => {
    useToolbarHandlers();
    const view = renderToolbar({ type: "dashboard" });

    act(() => {
      view.store.dispatch(
        createChatWithId({
          id: "chat-hidden",
          title: "Hidden",
          openTab: false,
        }),
      );
      view.store.dispatch(
        switchToThread({ id: "chat-hidden", openTab: false }),
      );
    });
    rerenderToolbar(view, { type: "dashboard" });

    await view.user.click(screen.getByRole("button", { name: "New Chat" }));

    expect(view.store.getState().chat.threads["chat-hidden"]).toBeDefined();
    expect(view.store.getState().pages.at(-1)?.name).toBe("chat");
  });

  it("uses the focused workspace chat for New Chat cleanup from task workspace", async () => {
    useToolbarHandlers();
    const activeTab = {
      type: "task" as const,
      taskId: "task-a",
      taskName: "Task Alpha",
    };
    const view = renderToolbar(activeTab);
    arrangeFocusedWorkspaceChatWithHiddenCurrent(view);
    rerenderToolbar(view, activeTab);

    await expectNewChatCleansFocusedWorkspaceChat(view);
  });

  it("uses the focused workspace chat for New Chat cleanup from buddy", async () => {
    useToolbarHandlers();
    const activeTab = { type: "buddy" as const };
    const view = renderToolbar(activeTab);
    arrangeFocusedWorkspaceChatWithHiddenCurrent(view);
    rerenderToolbar(view, activeTab);

    await expectNewChatCleansFocusedWorkspaceChat(view);
  });
});

describe("Toolbar chrome containment", () => {
  it("toolbar_is_fixed_height_chrome_that_never_flex_shrinks", async () => {
    const css = await readFile(
      resolve(process.cwd(), "src", "components/Toolbar/Toolbar.module.css"),
      "utf8",
    );
    const match = /\.toolbar \{[^}]*\}/.exec(css);
    expect(match).not.toBeNull();
    const block = match?.[0] ?? "";
    expect(block).toContain("flex-shrink: 0");
    expect(block).toContain("height: 36px");
  });

  it("toolbar_spacer_grows_to_keep_right_controls_pinned_to_the_edge", async () => {
    const css = await readFile(
      resolve(process.cwd(), "src", "components/Toolbar/Toolbar.module.css"),
      "utf8",
    );
    const match = /\.toolbarSpacer \{[^}]*\}/.exec(css);
    expect(match).not.toBeNull();
    const block = match?.[0] ?? "";
    expect(block).toContain("flex: 1 1 auto");
  });
});
