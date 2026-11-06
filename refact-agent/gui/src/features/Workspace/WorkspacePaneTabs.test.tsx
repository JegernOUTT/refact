import { describe, expect, it, vi } from "vitest";
import { act } from "react-dom/test-utils";
import { setUpStore } from "../../app/store";
import {
  fireEvent,
  render,
  screen,
  within,
  waitFor,
} from "../../utils/test-utils";
import { createChatWithId } from "../Chat/Thread";
import { setTabDragData } from "../ChatPanes/tabDrag";
import { collectTabIds, findLeaf } from "../ChatPanes/panesTree";
import { updateConfig } from "../Config/configSlice";
import { push } from "../Pages/pagesSlice";
import { openTask } from "../Tasks/tasksSlice";
import { panePanelId, paneTabId } from "./paneTabIds";
import { makeGitDiffSurfaceKey } from "./surfaceKey";
import {
  bindSurfaceToChat,
  openTab,
  setActiveTab,
  splitTab,
} from "./workspaceSlice";
import { WorkspaceSurfaceRoutes } from "./WorkspaceSurfaceRoutes";
import { WorkspaceToolbarTabs } from "./WorkspacePaneTabs";

const root = "chat:alpha";
const file = "file:/repo/example.ts";
const diff = makeGitDiffSurfaceKey({
  root: "/repo",
  path: "changed.ts",
  old_path: null,
  left: { rev: "HEAD", path: "changed.ts" },
  right: { rev: "WORKTREE", path: "changed.ts" },
  kind: "working",
});

function setup(host: "web" | "jetbrains" = "web") {
  const store = setUpStore({
    pages: [{ name: "history" }, { name: "buddy" }, { name: "chat" }],
    config: { host, lspPort: 8001, themeProps: {} },
  });
  store.dispatch(createChatWithId({ id: "alpha", title: "Alpha" }));
  store.dispatch(openTab(root));
  store.dispatch(openTab(file));
  store.dispatch(bindSurfaceToChat({ surfaceKey: file, chatId: "alpha" }));
  store.dispatch(openTask({ id: "task-one", name: "Release" }));
  store.dispatch(openTab("buddy:home"));
  for (const key of ["git:main", diff, "design:main"])
    store.dispatch(openTab(key));
  store.dispatch(setActiveTab(file));
  return store;
}

function getGroup(store: ReturnType<typeof setup>) {
  const group = store.getState().workspace.groups[root];
  if (!group) throw new Error("Expected canonical group");
  return group;
}

describe("WorkspacePaneTabs", () => {
  it("projects actual leaf sections into one top band while preserving owners and toolbar IDs", async () => {
    const store = setup();
    store.dispatch(splitTab({ tabId: root, dir: "row", surfaceKey: root }));
    const group = getGroup(store);
    const sibling = group.focusedLeafId;
    const before = store.getState().workspace;
    const rect = vi
      .spyOn(HTMLElement.prototype, "getBoundingClientRect")
      .mockImplementation(function (this: HTMLElement) {
        const id = this.dataset.workspaceLeafId;
        const left = id === "root" ? 332 : id === sibling ? 881 : 36;
        const right = id === "root" ? 881 : id === sibling ? 1430 : 1167;
        return {
          left,
          right,
          top: 36,
          bottom: 800,
          width: right - left,
          height: 764,
          x: left,
          y: 36,
          toJSON: () => ({}),
        };
      });
    try {
      const view = render(
        <>
          <WorkspaceSurfaceRoutes>
            <WorkspaceToolbarTabs />
          </WorkspaceSurfaceRoutes>
          <div data-workspace-group-tab-id={root}>
            <section data-workspace-leaf-id="root" />
            <section data-workspace-leaf-id={sibling} />
          </div>
        </>,
        { store },
      );
      await waitFor(() =>
        expect(screen.getAllByRole("tablist")).toHaveLength(2),
      );
      const sections = view.container.querySelectorAll<HTMLElement>(
        "[data-toolbar-pane-id]",
      );
      expect(sections[0].style.getPropertyValue("--toolbar-pane-left")).toBe(
        "296px",
      );
      expect(sections[1].style.getPropertyValue("--toolbar-pane-width")).toBe(
        "286px",
      );
      expect(screen.getByRole("tab", { name: "example.ts" }).id).toBe(
        paneTabId("toolbar", file),
      );
      expect(screen.getAllByRole("tab", { selected: true })).toHaveLength(2);
      expect(screen.getAllByRole("tab")).toHaveLength(7);
      expect(store.getState().workspace).toEqual(before);
      await view.user.click(screen.getByRole("tab", { name: "example.ts" }));
      expect(getGroup(store).focusedLeafId).toBe("root");
    } finally {
      rect.mockRestore();
    }
  });

  it("places every previous tab kind beside local chat/file tabs in one band without mutating workspace", () => {
    const store = setup();
    const before = store.getState().workspace;
    render(
      <WorkspaceSurfaceRoutes>
        <WorkspaceToolbarTabs />
      </WorkspaceSurfaceRoutes>,
      { store },
    );
    const band = screen.getByRole("tablist", { name: "Workspace tabs" });
    for (const name of [
      "Alpha",
      "example.ts",
      "Git",
      "Design",
      "Release",
      "Buddy",
    ])
      expect(
        within(band).getByRole("tab", { name: new RegExp(name) }),
      ).toBeVisible();
    expect(within(band).getAllByRole("tab")).toHaveLength(7);
    expect(screen.getAllByRole("tablist")).toHaveLength(1);
    expect(
      screen.queryByRole("navigation", { name: "Workspace tabs" }),
    ).not.toBeInTheDocument();
    expect(store.getState().workspace).toEqual(before);
  });

  it("activates actual task/Buddy pane tabs without changing owners, order or file context", async () => {
    const store = setup();
    const before = collectTabIds(getGroup(store).root);
    const view = render(
      <WorkspaceSurfaceRoutes>
        <WorkspaceToolbarTabs />
      </WorkspaceSurfaceRoutes>,
      { store },
    );
    for (const [name, key] of [
      [/Release/, "task:task-one"],
      [/Buddy/, "buddy:home"],
      [/example.ts/, file],
      [/Alpha/, root],
    ] as const) {
      await view.user.click(screen.getByRole("tab", { name }));
      expect(store.getState().pages.at(-1)).toEqual({ name: "chat" });
      expect(findLeaf(getGroup(store).root, "root")?.activeTabId).toBe(key);
      expect(collectTabIds(getGroup(store).root)).toEqual(before);
      expect(getGroup(store).focusedLeafId).toBe("root");
      expect(store.getState().workspace.contextChatByTab?.[file]).toBe("alpha");
    }
  });

  it("collects every leaf once and activates the actual owner without flattening", async () => {
    const store = setup();
    store.dispatch(splitTab({ tabId: root, dir: "row", surfaceKey: root }));
    const before = getGroup(store);
    const view = render(
      <WorkspaceSurfaceRoutes>
        <WorkspaceToolbarTabs />
      </WorkspaceSurfaceRoutes>,
      { store },
    );
    expect(screen.getAllByRole("tab")).toHaveLength(7);
    const tab = screen.getByRole("tab", { name: "example.ts" });
    expect(tab.id).toBe(paneTabId("toolbar", file));
    expect(tab).toHaveAttribute("aria-controls", panePanelId("root", file));
    await view.user.click(tab);
    const after = getGroup(store);
    expect(after.focusedLeafId).toBe("root");
    expect(after.root).toEqual(before.root);
    expect(store.getState().workspace.activeTabId).toBe(root);
    fireEvent.click(screen.getByRole("button", { name: "Close example.ts" }));
    expect(findLeaf(getGroup(store).root, "root")?.tabIds ?? []).not.toContain(
      file,
    );
    expect(
      findLeaf(getGroup(store).root, before.focusedLeafId)?.tabIds,
    ).toContain(root);
  });

  it("moves a cross-leaf native drop into the actual target owner", () => {
    const store = setup();
    store.dispatch(splitTab({ tabId: root, dir: "row", surfaceKey: root }));
    render(
      <WorkspaceSurfaceRoutes>
        <WorkspaceToolbarTabs />
      </WorkspaceSurfaceRoutes>,
      { store },
    );
    const data = new Map<string, string>();
    const dataTransfer = {
      get types() {
        return [...data.keys()];
      },
      setData: (key: string, value: string) => data.set(key, value),
      getData: (key: string) => data.get(key) ?? "",
    } as unknown as DataTransfer;
    setTabDragData(dataTransfer, "chat", "alpha", root);
    fireEvent.drop(screen.getByRole("tab", { name: "example.ts" }), {
      dataTransfer,
    });
    expect(findLeaf(getGroup(store).root, "root")?.tabIds).toEqual([
      root,
      file,
      "task:task-one",
      "buddy:home",
      "git:main",
      diff,
      "design:main",
    ]);
    expect(
      collectTabIds(getGroup(store).root).filter((key) => key === root),
    ).toHaveLength(1);
    expect(screen.getByRole("tab", { name: /Alpha/ })).toHaveAttribute(
      "aria-controls",
      panePanelId("root", root),
    );
  });

  it("retains the actual selected surface and reaches hidden entries at 240px", async () => {
    const store = setup();
    const width = vi
      .spyOn(HTMLElement.prototype, "clientWidth", "get")
      .mockReturnValue(240);
    const rect = vi
      .spyOn(HTMLElement.prototype, "getBoundingClientRect")
      .mockReturnValue({
        width: 100,
        height: 30,
        left: 0,
        right: 100,
        top: 0,
        bottom: 30,
        x: 0,
        y: 0,
        toJSON: () => ({}),
      });
    try {
      const view = render(
        <WorkspaceSurfaceRoutes>
          <WorkspaceToolbarTabs />
        </WorkspaceSurfaceRoutes>,
        { store },
      );
      expect(screen.getByRole("tab", { name: "example.ts" })).toHaveAttribute(
        "aria-selected",
        "true",
      );
      await view.user.click(
        screen.getByRole("button", { name: /more pane tabs/ }),
      );
      await view.user.click(screen.getByRole("menuitem", { name: /Release/ }));
      await screen.findByRole("tab", { name: /Release/, selected: true });
      expect(store.getState().pages.at(-1)).toEqual({ name: "chat" });
      expect(findLeaf(getGroup(store).root, "root")?.activeTabId).toBe(
        "task:task-one",
      );
    } finally {
      width.mockRestore();
      rect.mockRestore();
    }
  });

  it("bridges auxiliary routes into the same pane band without toolbar-owned controls", async () => {
    const store = setup();
    store.dispatch(push({ name: "task workspace", taskId: "task-one" }));
    const before = collectTabIds(getGroup(store).root);
    const view = render(
      <WorkspaceSurfaceRoutes>
        <WorkspaceToolbarTabs />
      </WorkspaceSurfaceRoutes>,
      { store },
    );
    expect(screen.getAllByRole("tablist")).toHaveLength(1);
    expect(screen.getByRole("tab", { name: /Release/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("tab", { name: "example.ts" })).toHaveAttribute(
      "aria-selected",
      "false",
    );
    expect(
      screen.queryByRole("button", { name: "Pane actions" }),
    ).not.toBeInTheDocument();
    expect(collectTabIds(getGroup(store).root)).toEqual(before);
    await view.user.click(screen.getByRole("tab", { name: "example.ts" }));
    expect(findLeaf(getGroup(store).root, "root")?.activeTabId).toBe(file);
    expect(store.getState().pages.at(-1)?.name).toBe("chat");
    expect(screen.getAllByRole("tablist")).toHaveLength(1);
  });

  it("closes actual task and Buddy surfaces with task lifecycle cleanup", async () => {
    const store = setup();
    store.dispatch(push({ name: "task workspace", taskId: "task-one" }));
    const view = render(
      <WorkspaceSurfaceRoutes>
        <WorkspaceToolbarTabs />
      </WorkspaceSurfaceRoutes>,
      { store },
    );
    await view.user.click(
      screen.getByRole("button", { name: "Close Release" }),
    );
    await screen.findByRole("tab", { name: /Alpha/ });
    await new Promise((resolve) => setTimeout(resolve, 300));
    expect(store.getState().tasksUI.openTasks).toHaveLength(0);
    expect(collectTabIds(getGroup(store).root)).not.toContain("task:task-one");
    expect(store.getState().pages.at(-1)?.name).toBe("chat");
    act(() => {
      store.dispatch(push({ name: "buddy" }));
    });
    await view.user.click(screen.getByRole("button", { name: "Close Buddy" }));
    await new Promise((resolve) => setTimeout(resolve, 300));
    expect(store.getState().pages.at(-1)?.name).toBe("chat");
    expect(
      screen.queryByRole("tab", { name: /Buddy/ }),
    ).not.toBeInTheDocument();
  });

  it.each(["git:main", diff, "design:main"])(
    "closing %s preserves remaining pane order and bound file context",
    async (key) => {
      const store = setup();
      store.dispatch(setActiveTab(key));
      const before = collectTabIds(getGroup(store).root);
      const view = render(
        <WorkspaceSurfaceRoutes>
          <WorkspaceToolbarTabs />
        </WorkspaceSurfaceRoutes>,
        { store },
      );
      const selected = screen.getByRole("tab", { selected: true });
      const close = selected.parentElement?.querySelector<HTMLButtonElement>(
        'button[aria-label^="Close "]',
      );
      if (!close) throw new Error("Expected selected tab close button");
      await view.user.click(close);
      await new Promise((resolve) => setTimeout(resolve, 300));
      expect(collectTabIds(getGroup(store).root)).not.toContain(key);
      expect(store.getState().workspace.activeTabId).toBe(root);
      expect(collectTabIds(getGroup(store).root)).toEqual(
        before.filter((surface) => surface !== key),
      );
      expect(store.getState().workspace.contextChatByTab?.[file]).toBe("alpha");
      expect(findLeaf(getGroup(store).root, "root")?.activeTabId).toBe(root);
    },
  );

  it("does not select a fake editor on history or create a chat for auxiliary-only state", () => {
    const store = setUpStore({ pages: [{ name: "history" }] });
    store.dispatch(openTask({ id: "task-one", name: "Release" }));
    const before = store.getState();
    render(
      <WorkspaceSurfaceRoutes>
        <WorkspaceToolbarTabs />
      </WorkspaceSurfaceRoutes>,
      { store },
    );
    expect(screen.getAllByRole("tab")).toHaveLength(1);
    expect(
      screen.queryByRole("tab", { selected: true }),
    ).not.toBeInTheDocument();
    expect(store.getState().workspace).toEqual(before.workspace);
    expect(store.getState().chat).toEqual(before.chat);
  });

  it("preserves tab navigation for IDE hosts without adding pane controls", () => {
    const store = setup();
    store.dispatch(updateConfig({ host: "vscode" }));
    store.dispatch(push({ name: "buddy" }));
    render(
      <WorkspaceSurfaceRoutes>
        <WorkspaceToolbarTabs />
      </WorkspaceSurfaceRoutes>,
      { store },
    );
    expect(
      screen.getByRole("tablist", { name: "Workspace tabs" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Buddy/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(
      screen.queryByRole("button", { name: "Pane actions" }),
    ).not.toBeInTheDocument();
  });
});
