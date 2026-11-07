import { act } from "react-dom/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setUpStore } from "../../app/store";
import { fireEvent, render, screen, waitFor } from "../../utils/test-utils";
import { findLeaf } from "../ChatPanes/panesTree";
import { setTabDragData } from "../ChatPanes/tabDrag";
import { PaneTabStrip } from "./PaneTabStrip";
import { displayInfoForSurface } from "./tabDisplay";
import { GroupSplitView } from "./GroupSplitView";
import { WorkspaceToolbarTabs } from "./WorkspacePaneTabs";
import { addSurfaceToPane, openTab } from "./workspaceSlice";
import { closeTask, openTask } from "../Tasks/tasksSlice";
import { change } from "../Pages/pagesSlice";

const a = "file:/project/alpha.ts";
const b = "file:/project/beta.ts";

function setup() {
  const store = setUpStore();
  store.dispatch(change({ name: "chat" }));
  store.dispatch(openTab(a));
  store.dispatch(openTab(b));
  store.dispatch(addSurfaceToPane({ tabId: a, leafId: "root", surfaceKey: b }));
  return store;
}

function transfer(): DataTransfer {
  const data = new Map<string, string>();
  return {
    get types() {
      return [...data.keys()];
    },
    setData: (type: string, value: string) => data.set(type, value),
    getData: (type: string) => data.get(type) ?? "",
    effectAllowed: "move",
    dropEffect: "move",
  } as unknown as DataTransfer;
}

describe("compact pane tabs", () => {
  it("renders a singleton group, switches with roving keys, retains panels and closes a local file", async () => {
    const store = setup();
    render(
      <>
        <WorkspaceToolbarTabs />
        <GroupSplitView tabId={a} />
      </>,
      { store },
    );
    const alpha = screen.getByRole("tab", { name: "alpha.ts" });
    const beta = screen.getByRole("tab", { name: "beta.ts" });
    const panel = document.querySelector(`[data-surface-key="${b}"]`);
    expect(beta).toHaveAttribute("aria-selected", "true");
    const inactivePanel = document.getElementById(
      alpha.getAttribute("aria-controls") ?? "",
    );
    expect(inactivePanel).toHaveAttribute("role", "tabpanel");
    expect(inactivePanel).toHaveAttribute("hidden");
    expect(
      inactivePanel?.querySelector(`[data-surface-key="${a}"]`),
    ).toBeNull();
    fireEvent.keyDown(beta, { key: "Home" });
    expect(alpha).toHaveFocus();
    expect(alpha).toHaveAttribute("aria-selected", "true");
    fireEvent.keyDown(alpha, { key: "ArrowLeft" });
    expect(beta).toHaveFocus();
    expect(document.querySelector(`[data-surface-key="${b}"]`)).toBe(panel);
    fireEvent.click(screen.getByRole("button", { name: "Close alpha.ts" }));
    await waitFor(() =>
      expect(screen.queryByRole("tab", { name: "alpha.ts" })).toBeNull(),
    );
    expect(store.getState().workspace.tabs).toEqual([b]);
  });

  it("associates whitespace file tabs with panels and restores focus after keyboard close", async () => {
    const store = setUpStore();
    const spaced = "file:/project/my file.ts";
    store.dispatch(openTab(a));
    store.dispatch(openTab(spaced));
    const { user } = render(
      <>
        <WorkspaceToolbarTabs />
        <GroupSplitView tabId={a} />
      </>,
      { store },
    );
    const tab = screen.getByRole("tab", { name: "my file.ts" });
    expect(tab.id).not.toMatch(/\s/);
    const panel = document.getElementById(
      tab.getAttribute("aria-controls") ?? "",
    );
    expect(panel).toHaveAttribute("role", "tabpanel");
    expect(panel).toHaveAccessibleName("my file.ts");
    expect(panel).toHaveAccessibleName("my file.ts");
    screen.getByRole("button", { name: "Close my file.ts" }).focus();
    await user.keyboard("{Enter}");
    await waitFor(() =>
      expect(screen.getByRole("tab", { name: "alpha.ts" })).toHaveFocus(),
    );
    expect(screen.queryByRole("tab", { name: "my file.ts" })).toBeNull();
    expect(store.getState().workspace.tabs).toEqual([a]);
    expect(store.getState().workspace.groups[a]?.root).toMatchObject({
      kind: "leaf",
      tabIds: [a],
      activeTabId: a,
    });
  });

  it("uses native payloads for tab reordering rather than pane splitting", () => {
    const store = setup();
    render(
      <>
        <WorkspaceToolbarTabs />
        <GroupSplitView tabId={a} />
      </>,
      { store },
    );
    const dataTransfer = transfer();
    fireEvent.dragStart(screen.getByRole("tab", { name: "alpha.ts" }), {
      dataTransfer,
    });
    expect(dataTransfer.types).toContain("application/x-refact-surface-tab");
    setTabDragData(dataTransfer, "file", "/project/alpha.ts", a);
    fireEvent.drop(screen.getByRole("tab", { name: "beta.ts" }), {
      dataTransfer,
    });
    const group = store.getState().workspace.groups[a];
    expect(group?.root.kind).toBe("leaf");
    expect(group && findLeaf(group.root, "root")?.tabIds).toEqual([b, a]);
  });

  it("shrinks titles before overflow and removes fit contributions on unmount", () => {
    const onFitChange = vi.fn();
    const width = vi
      .spyOn(HTMLElement.prototype, "clientWidth", "get")
      .mockReturnValue(170);
    const rect = vi
      .spyOn(HTMLElement.prototype, "getBoundingClientRect")
      .mockImplementation(function (this: HTMLElement) {
        return {
          width: this.className.includes("title") ? 100 : 180,
          height: 30,
          left: 0,
          right: 180,
          top: 0,
          bottom: 30,
          x: 0,
          y: 0,
          toJSON: () => ({}),
        };
      });
    try {
      const { unmount } = render(
        <PaneTabStrip
          leaf={{ kind: "leaf", id: "root", tabIds: [a, b], activeTabId: b }}
          tabId={a}
          hidePaneActions
          onFitChange={onFitChange}
        />,
      );
      expect(screen.getByRole("tab", { name: "alpha.ts" })).toBeInTheDocument();
      expect(screen.getByRole("tab", { name: "beta.ts" })).toBeInTheDocument();
      expect(
        screen.queryByRole("button", { name: /more pane tabs/ }),
      ).toBeNull();
      const element = onFitChange.mock.calls[0][0] as HTMLElement;
      expect(onFitChange).toHaveBeenCalledWith(element, 360);
      unmount();
      expect(onFitChange).toHaveBeenLastCalledWith(element, null);
    } finally {
      width.mockRestore();
      rect.mockRestore();
    }
  });

  it("measures overflow and retains the active tab with keyboard access to hidden tabs", () => {
    const store = setup();
    const width = vi
      .spyOn(HTMLElement.prototype, "clientWidth", "get")
      .mockReturnValue(150);
    const rect = vi
      .spyOn(HTMLElement.prototype, "getBoundingClientRect")
      .mockImplementation(function (this: HTMLElement) {
        return {
          width: this.className.includes("title") ? 0 : 100,
          height: 30,
          left: 0,
          right: 100,
          top: 0,
          bottom: 30,
          x: 0,
          y: 0,
          toJSON: () => ({}),
        };
      });
    try {
      render(
        <>
          <WorkspaceToolbarTabs />
          <GroupSplitView tabId={a} />
        </>,
        { store },
      );
      expect(
        screen.getByRole("button", { name: "1 more pane tabs" }),
      ).toBeInTheDocument();
      expect(screen.queryByRole("tab", { name: "alpha.ts" })).toBeNull();
      expect(
        screen
          .getByRole("tab", { name: "beta.ts" })
          .closest("[data-pane-tab-key]"),
      ).toHaveStyle({ minWidth: "50px" });
      fireEvent.keyDown(screen.getByRole("tab", { name: "beta.ts" }), {
        key: "Home",
      });
      expect(screen.getByRole("tab", { name: "alpha.ts" })).toHaveFocus();
      expect(screen.queryByRole("tab", { name: "beta.ts" })).toBeNull();
    } finally {
      width.mockRestore();
      rect.mockRestore();
    }
  });
});

describe("rich pane navigation and motion", () => {
  afterEach(() => {
    delete document.documentElement.dataset.reducedMotion;
  });
  const leaf = {
    kind: "leaf" as const,
    id: "root",
    tabIds: [a],
    activeTabId: a,
  };

  it("resolves meaningful file and auxiliary titles", () => {
    const resolve = (key: string) =>
      displayInfoForSurface(key, new Map(), {}, new Map());
    expect(resolve(a)).toMatchObject({
      title: "alpha.ts",
      tooltip: "/project/alpha.ts",
      kind: "file",
    });
    expect(resolve("buddy:home").title).toBe("Buddy");
    expect(resolve("git:main").title).toBe("Git");
    expect(resolve("design:main").title).toBe("Design");
    expect(resolve("dashboard").title).toBe("Dashboard");
  });

  it("renders rich auxiliary content without changing leaf ownership before action", () => {
    const store = setup();
    const before = store.getState().workspace;
    const onActivate = vi.fn();
    const onActivateLeaf = vi.fn();
    render(
      <PaneTabStrip
        leaf={leaf}
        tabId={a}
        hidePaneActions
        selectedNavigationKey="task"
        onActivateLeaf={onActivateLeaf}
        navigationEntries={[
          {
            key: "task",
            surfaceKey: "task:demo",
            onActivate,
            display: {
              kind: "task",
              title: "Full meaningful task title",
              unreadNotificationCount: 12,
              activity: "working",
            },
          },
        ]}
      />,
      { store },
    );
    expect(store.getState().workspace).toBe(before);
    const task = screen.getByRole("tab", {
      name: /Full meaningful task title/,
    });
    expect(task).toHaveAttribute("title", "Full meaningful task title");
    expect(task).toHaveAttribute("aria-selected", "true");
    expect(
      screen.getAllByLabelText("12 unread process notifications")[0],
    ).toHaveTextContent("9+");
    expect(screen.getByRole("tab", { name: "alpha.ts" })).toHaveAttribute(
      "aria-selected",
      "false",
    );
    fireEvent.click(task);
    expect(onActivate).toHaveBeenCalledOnce();
    fireEvent.click(screen.getByRole("tab", { name: "alpha.ts" }));
    expect(onActivateLeaf).toHaveBeenCalledWith(a);
    expect(screen.queryByRole("button", { name: "Pane actions" })).toBeNull();
  });

  it("describes focused file tabs with full paths without replacing their accessible name", async () => {
    const { user } = render(<PaneTabStrip leaf={leaf} tabId={a} />);
    const tab = screen.getByRole("tab", { name: "alpha.ts" });
    await user.tab();
    const tooltip = await screen.findByRole("tooltip");
    expect(tab).toHaveFocus();
    expect(tab).toHaveAccessibleName("alpha.ts");
    expect(tab).toHaveAttribute("aria-describedby", tooltip.id);
    expect(tooltip).toHaveTextContent("/project/alpha.ts");
    expect(tooltip.querySelector("button, a, [tabindex]")).toBeNull();
    expect(tab.querySelector("button")).toBeNull();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("tooltip")).toBeNull();
  });

  it("shows rich metadata on hover while retaining visible activity, badges and unread counts", async () => {
    const { user } = render(
      <PaneTabStrip
        leaf={leaf}
        tabId={a}
        navigationEntries={[
          {
            key: "chat-details",
            surfaceKey: "chat:details",
            onActivate: vi.fn(),
            display: {
              title: "A complete chat title",
              kind: "chat",
              activity: "waiting_agents",
              mode: "task_planner",
              session_state: "executing_tools",
              is_task_chat: true,
              unreadNotificationCount: 12,
              project: "Example project",
              branch: "feature/details",
              root: "/project/worktree",
              taskId: "task-123",
            },
          },
        ]}
      />,
    );
    const tab = screen.getByRole("tab", { name: /A complete chat title/ });
    expect(tab).toHaveTextContent("Task");
    expect(tab).toHaveTextContent("9+");
    expect(
      tab.querySelector('[data-chat-activity="waiting_agents"]'),
    ).not.toBeNull();
    await user.hover(tab);
    const tooltip = await screen.findByRole("tooltip");
    for (const value of [
      "A complete chat title",
      "Chat · Task",
      "Waiting for subagents",
      "Task Planner",
      "Executing tools",
      "12 unread process notifications",
      "Example project",
      "feature/details",
      "/project/worktree",
      "task-123",
    ])
      expect(tooltip).toHaveTextContent(value);
    expect(tab).toHaveAttribute("aria-describedby", tooltip.id);
  });

  it("does not accept native drops on unowned workspace tabs", () => {
    render(<PaneTabStrip leaf={leaf} tabId={a} ownerBySurface={{}} />);
    const dataTransfer = transfer();
    setTabDragData(dataTransfer, "file", "/project/beta.ts", b);
    expect(
      fireEvent.dragOver(screen.getByRole("tab", { name: "alpha.ts" }), {
        dataTransfer,
      }),
    ).toBe(true);
    expect(
      fireEvent.drop(screen.getByRole("tab", { name: "alpha.ts" }), {
        dataTransfer,
      }),
    ).toBe(true);
  });

  it("dispatches close immediately and removes only an inert exit snapshot on cancellation", () => {
    const onClose = vi.fn();
    const { unmount } = render(
      <PaneTabStrip
        leaf={leaf}
        tabId={a}
        navigationEntries={[
          {
            key: "buddy",
            surfaceKey: "buddy:home",
            onActivate: vi.fn(),
            onClose,
          },
        ]}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Close Buddy" }));
    expect(onClose).toHaveBeenCalledOnce();
    const wrapper = document.querySelector('[data-pane-tab-ghost="nav:buddy"]');
    expect(wrapper).not.toBeNull();
    expect(wrapper).toHaveAttribute("aria-hidden", "true");
    expect(wrapper?.querySelector("button, [role=tab], [tabindex]")).toBeNull();
    act(() => {
      wrapper?.dispatchEvent(new Event("animationcancel", { bubbles: true }));
    });
    expect(document.querySelector("[data-pane-tab-ghost]")).toBeNull();
    expect(onClose).toHaveBeenCalledOnce();
    unmount();
  });

  it.each(["navigate Buddy", "focus another pane"])(
    "never loses closeTask on immediate %s and unmount",
    (handoff) => {
      vi.useFakeTimers();
      const store = setup();
      store.dispatch(openTask({ id: "closing-task", name: "Closing task" }));
      const onClose = vi.fn(() => store.dispatch(closeTask("closing-task")));
      const { rerender, unmount } = render(
        <PaneTabStrip
          leaf={leaf}
          tabId={a}
          navigationEntries={[
            {
              key: "closing-task",
              surfaceKey: "task:closing-task",
              onActivate: vi.fn(),
              onClose,
            },
          ]}
        />,
        { store },
      );
      const close = screen.getByRole("button", { name: "Close Closing task" });
      fireEvent.click(close);
      fireEvent.click(close);
      expect(onClose).toHaveBeenCalledOnce();
      expect(
        store
          .getState()
          .tasksUI.openTasks.some((task) => task.id === "closing-task"),
      ).toBe(false);
      expect(screen.getByRole("tab", { name: "alpha.ts" })).toHaveFocus();
      if (handoff === "navigate Buddy")
        act(() => {
          store.dispatch(change({ name: "buddy" }));
        });
      rerender(<PaneTabStrip leaf={{ ...leaf, id: "other-pane" }} tabId={a} />);
      unmount();
      act(() => {
        vi.runAllTimers();
      });
      expect(onClose).toHaveBeenCalledOnce();
      expect(
        store
          .getState()
          .tasksUI.openTasks.some((task) => task.id === "closing-task"),
      ).toBe(false);
      vi.useRealTimers();
    },
  );

  it("expires a visible exit snapshot within 200ms without replaying close", () => {
    vi.useFakeTimers();
    const onClose = vi.fn();
    const { unmount } = render(
      <PaneTabStrip
        leaf={leaf}
        tabId={a}
        navigationEntries={[
          {
            key: "buddy",
            surfaceKey: "buddy:home",
            onActivate: vi.fn(),
            onClose,
          },
        ]}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Close Buddy" }));
    expect(document.querySelector("[data-pane-tab-ghost]")).not.toBeNull();
    act(() => {
      vi.advanceTimersByTime(200);
    });
    expect(document.querySelector("[data-pane-tab-ghost]")).toBeNull();
    expect(onClose).toHaveBeenCalledOnce();
    unmount();
    vi.useRealTimers();
  });

  it("bypasses delayed exit for HTML reduced motion and suppresses selection for null", () => {
    document.documentElement.dataset.reducedMotion = "on";
    const onClose = vi.fn();
    render(
      <PaneTabStrip
        leaf={leaf}
        tabId={a}
        selectedNavigationKey={null}
        navigationEntries={[
          {
            key: "buddy",
            surfaceKey: "buddy:home",
            onActivate: vi.fn(),
            onClose,
          },
        ]}
      />,
    );
    expect(
      screen
        .getAllByRole("tab")
        .every((tab) => tab.getAttribute("aria-selected") === "false"),
    ).toBe(true);
    expect(screen.getByRole("tab", { name: "alpha.ts" })).toHaveAttribute(
      "tabindex",
      "0",
    );
    fireEvent.click(screen.getByRole("button", { name: "Close Buddy" }));
    expect(onClose).toHaveBeenCalledOnce();
    expect(document.querySelector("[data-pane-tab-ghost]")).toBeNull();
  });
});
