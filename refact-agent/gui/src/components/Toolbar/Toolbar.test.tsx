import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { http, HttpResponse } from "msw";
import { afterEach, describe, expect, it, vi } from "vitest";
import { act } from "react-dom/test-utils";
import userEvent from "@testing-library/user-event";

import { render, screen, waitFor, within } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { Toolbar, type Tab } from "./Toolbar";
import { createChatWithId } from "../../features/Chat/Thread";
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

  it("maps task states to the current status dot labels", async () => {
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
      view.store.dispatch(openTask({ id: "task-status", name: "Task Status" }));
    });

    await waitFor(() => {
      expectTabToContainStatus("Task Status", "Needs your attention");
    });
  });

  it("does not render chat tabs in the global toolbar", () => {
    useToolbarHandlers();
    const view = renderToolbar({ type: "dashboard" });

    dispatchAndRerender(view, { type: "dashboard" }, () => {
      view.store.dispatch(
        createChatWithId({ id: "normal-chat", title: "Normal Chat" }),
      );
    });

    expect(
      screen.queryByRole("tab", { name: /Normal Chat/ }),
    ).not.toBeInTheDocument();
  });

  it("does not render an empty task tab strip", () => {
    useToolbarHandlers();
    renderToolbar({ type: "dashboard" });

    expect(screen.queryByRole("tablist")).not.toBeInTheDocument();
  });

  it("wheel-scrolls an overflowing tab container horizontally", () => {
    useToolbarHandlers();
    const view = renderToolbar({ type: "dashboard" });

    dispatchAndRerender(view, { type: "dashboard" }, () => {
      view.store.dispatch(openTask({ id: "task-scroll", name: "Task Scroll" }));
    });

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

  it("scrolls the active task tab into view when the active tab changes", () => {
    useToolbarHandlers();
    const scrollIntoView = vi.spyOn(Element.prototype, "scrollIntoView");

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

    view.rerender(
      <Toolbar
        activeTab={{ type: "task", taskId: "task-b", taskName: "Task Beta" }}
      />,
    );

    expect(scrollIntoView).toHaveBeenCalledWith({
      behavior: "smooth",
      block: "nearest",
      inline: "nearest",
    });
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
});
