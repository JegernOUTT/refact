import { readFileSync } from "node:fs";

import { describe, expect, it, vi } from "vitest";
import { waitFor } from "@testing-library/react";

import { type AppStore, setUpStore } from "../../app/store";
import { fireEvent, render, screen, within } from "../../utils/test-utils";
import { createChatWithId, switchToThread } from "../Chat/Thread";
import {
  collectLeafIds,
  collectTabIds,
  findLeaf,
} from "../ChatPanes/panesTree";
import { setTabDragData } from "../ChatPanes/tabDrag";
import {
  addSurfaceToPane,
  openTab,
  setActiveTab,
  splitTab,
} from "./workspaceSlice";
import { makeSurfaceKey, type SurfaceKey } from "./surfaceKey";
import { WorkspaceView } from "./WorkspaceView";

const groupSplitViewCss = readFileSync(
  "src/features/Workspace/GroupSplitView.module.css",
  "utf8",
);

const chat = (id: string): SurfaceKey => makeSurfaceKey("chat", id);
const task = (id: string): SurfaceKey => makeSurfaceKey("task", id);

function createDataTransferStub(): DataTransfer {
  const data = new Map<string, string>();
  return {
    dropEffect: "none" as DataTransfer["dropEffect"],
    effectAllowed: "uninitialized" as DataTransfer["effectAllowed"],
    files: [] as unknown as FileList,
    items: [] as unknown as DataTransferItemList,
    get types() {
      return Array.from(data.keys());
    },
    clearData: (type?: string) => {
      if (type) {
        data.delete(type);
      } else {
        data.clear();
      }
    },
    getData: (type: string) => data.get(type) ?? "",
    setData: (type: string, value: string) => {
      data.set(type, value);
    },
    setDragImage: () => undefined,
  } as DataTransfer;
}

function createWorkspaceStore(): AppStore {
  const store = setUpStore();
  store.dispatch(createChatWithId({ id: "chat-a", title: "Chat Alpha" }));
  store.dispatch(createChatWithId({ id: "chat-b", title: "Chat Beta" }));
  store.dispatch(openTab(chat("chat-a")));
  store.dispatch(openTab(chat("chat-b")));
  store.dispatch(setActiveTab(chat("chat-a")));
  return store;
}

function createSplitWorkspaceStore(): AppStore {
  const store = createWorkspaceStore();
  store.dispatch(splitTab({ tabId: chat("chat-a"), dir: "row" }));
  const group = store.getState().workspace.groups[chat("chat-a")];
  if (!group) throw new Error("missing split group");
  store.dispatch(
    addSurfaceToPane({
      tabId: chat("chat-a"),
      leafId: group.focusedLeafId,
      surfaceKey: chat("chat-b"),
    }),
  );
  return store;
}

function createEmptySplitWorkspaceStore(): AppStore {
  const store = createWorkspaceStore();
  store.dispatch(splitTab({ tabId: chat("chat-a"), dir: "row" }));
  return store;
}

function renderWorkspaceView(store: AppStore) {
  return render(<WorkspaceView />, { store });
}

function expectSurface(key: SurfaceKey) {
  const element = document.querySelector(`[data-surface-key="${key}"]`);
  expect(element).toBeInTheDocument();
}

describe("WorkspaceView", () => {
  it("renders an unsplit surface without pane chrome", () => {
    renderWorkspaceView(createWorkspaceStore());

    expectSurface(chat("chat-a"));
    expect(screen.queryByRole("tablist")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Workspace pane controls")).toBeNull();
    expect(screen.queryByRole("button", { name: "Close Pane" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Split Right" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Split Down" })).toBeNull();
  });

  it("does not render a pane or split affordance for a non-chat active tab", () => {
    const store = setUpStore({
      workspace: {
        tabs: [task("task-a")],
        activeTabId: task("task-a"),
        groups: {},
      },
    });
    renderWorkspaceView(store);

    expect(
      document.querySelector(`[data-surface-key="${task("task-a")}"]`),
    ).toBeNull();
    expect(screen.queryByLabelText("Workspace pane controls")).toBeNull();
    expect(
      screen.queryByRole("button", { name: "Split active tab" }),
    ).toBeNull();
  });

  it("reconciles current thread to the active workspace chat on entry", async () => {
    const store = createWorkspaceStore();
    store.dispatch(
      createChatWithId({
        id: "task-hidden",
        title: "Task Hidden",
        openTab: false,
      }),
    );
    store.dispatch(switchToThread({ id: "task-hidden", openTab: false }));

    renderWorkspaceView(store);

    expectSurface(chat("chat-a"));
    await waitFor(() => {
      expect(store.getState().chat.current_thread_id).toBe("chat-a");
    });
    expect(store.getState().workspace.activeTabId).toBe(chat("chat-a"));
    expect(store.getState().workspace.tabs).toEqual([
      chat("chat-a"),
      chat("chat-b"),
    ]);
  });

  it("renders split panes with distinct surfaces and pane close controls", () => {
    renderWorkspaceView(createSplitWorkspaceStore());

    expectSurface(chat("chat-a"));
    expectSurface(chat("chat-b"));
    expect(screen.getAllByLabelText("Workspace pane controls")).toHaveLength(2);
    expect(screen.getAllByRole("button", { name: "Close Pane" })).toHaveLength(
      2,
    );
    expect(screen.getAllByRole("button", { name: "Split Right" })).toHaveLength(
      2,
    );
    expect(screen.getAllByRole("button", { name: "Split Down" })).toHaveLength(
      2,
    );
  });

  it("uses a vertically scrollable stacked layout at narrow breakpoints", async () => {
    const clientWidthSpy = vi
      .spyOn(HTMLElement.prototype, "clientWidth", "get")
      .mockReturnValue(360);

    try {
      renderWorkspaceView(createSplitWorkspaceStore());

      await waitFor(() => {
        const container = document.querySelector<HTMLElement>(
          `[data-workspace-group-tab-id="${chat("chat-a")}"]`,
        );
        expect(container).toBeInTheDocument();
        expect(container).toHaveAttribute("data-breakpoint", "narrow");
        expect(container).toHaveAttribute("data-stacked", "true");
      });
      expectSurface(chat("chat-a"));
      expectSurface(chat("chat-b"));
      expect(screen.queryByTestId("workspace-vertical-divider")).toBeNull();
      expect(groupSplitViewCss).toMatch(
        /\.stackedLayout\s*\{[^}]*overflow:\s*hidden auto;/u,
      );
    } finally {
      clientWidthSpy.mockRestore();
    }
  });

  it("dropping a chat tab on an unsplit chat surface creates a split", () => {
    const store = createWorkspaceStore();
    renderWorkspaceView(store);
    const dataTransfer = createDataTransferStub();
    setTabDragData(dataTransfer, "chat", "chat-b", chat("chat-b"));
    const dropTarget = document.querySelector(
      "[data-workspace-unsplit-drop-target='true']",
    );
    if (!dropTarget) throw new Error("missing unsplit drop target");

    fireEvent.dragEnter(dropTarget, { dataTransfer });
    fireEvent.dragOver(dropTarget, { dataTransfer });
    fireEvent.drop(dropTarget, { dataTransfer });

    const group = store.getState().workspace.groups[chat("chat-a")];
    expect(group).toBeDefined();
    if (!group) throw new Error("missing split group");
    expect(store.getState().workspace.tabs).toEqual([chat("chat-a")]);
    expectSurface(chat("chat-a"));
    expectSurface(chat("chat-b"));
    expect(screen.getAllByLabelText("Workspace pane controls")).toHaveLength(2);
  });

  it("clicking a pane close ungroups the split and closes the removed chat", async () => {
    const store = createSplitWorkspaceStore();
    const view = renderWorkspaceView(store);
    const siblingPane = screen.getByLabelText(
      "Workspace pane root:sibling:chat:chat-a",
    );

    expect(store.getState().chat.open_thread_ids).toEqual(["chat-a", "chat-b"]);
    expect(store.getState().chat.threads["chat-b"]).toBeDefined();

    await view.user.click(
      within(siblingPane).getByRole("button", { name: "Close Pane" }),
    );

    expect(store.getState().workspace.groups[chat("chat-a")]).toBeUndefined();
    expect(store.getState().workspace.tabs).toEqual([chat("chat-a")]);
    expect(store.getState().chat.open_thread_ids).toEqual(["chat-a"]);
    expect(store.getState().chat.threads["chat-a"]).toBeDefined();
    expect(store.getState().chat.threads["chat-b"]).toBeUndefined();
    expect(screen.queryByLabelText("Workspace pane controls")).toBeNull();
    expectSurface(chat("chat-a"));
  });

  it("dropping a workspace tab on a pane edge creates a split", () => {
    const store = createEmptySplitWorkspaceStore();
    renderWorkspaceView(store);
    const dataTransfer = createDataTransferStub();
    setTabDragData(dataTransfer, "chat", "chat-b", chat("chat-b"));
    const targetPane = screen.getByLabelText("Workspace pane root");

    fireEvent.dragEnter(targetPane, { dataTransfer });
    const edgeDropZone = within(targetPane).getByTestId(
      "workspace-pane-edge-drop-root-right",
    );
    fireEvent.dragOver(edgeDropZone, { dataTransfer });
    fireEvent.drop(edgeDropZone, { dataTransfer });

    const group = store.getState().workspace.groups[chat("chat-a")];
    expect(group).toBeDefined();
    if (!group) throw new Error("missing split group");
    expect(store.getState().workspace.tabs).toEqual([chat("chat-a")]);
    expectSurface(chat("chat-a"));
    expectSurface(chat("chat-b"));
    expect(screen.getAllByLabelText("Workspace pane controls")).toHaveLength(2);
  });

  it("dropping a workspace tab on an empty pane fills the pane", () => {
    const store = createEmptySplitWorkspaceStore();
    renderWorkspaceView(store);
    const dataTransfer = createDataTransferStub();
    setTabDragData(dataTransfer, "chat", "chat-b", chat("chat-b"));
    const emptyPane = screen.getByLabelText(
      "Workspace pane root:sibling:chat:chat-a",
    );

    fireEvent.dragEnter(emptyPane, { dataTransfer });
    fireEvent.dragOver(emptyPane, { dataTransfer });
    fireEvent.drop(emptyPane, { dataTransfer });

    const group = store.getState().workspace.groups[chat("chat-a")];
    expect(group).toBeDefined();
    if (!group) throw new Error("missing split group");
    const leafIds = collectLeafIds(group.root);
    expect(leafIds).toHaveLength(2);
    expect(
      leafIds.map((leafId) => findLeaf(group.root, leafId)?.tabIds.length),
    ).toEqual([1, 1]);
    expect(store.getState().workspace.tabs).toEqual([chat("chat-a")]);
    expectSurface(chat("chat-a"));
    expectSurface(chat("chat-b"));
  });

  it("dropping a workspace tab on an occupied pane center creates a sibling pane", () => {
    const store = createSplitWorkspaceStore();
    store.dispatch(createChatWithId({ id: "chat-c", title: "Chat Gamma" }));
    store.dispatch(openTab(chat("chat-c")));
    store.dispatch(setActiveTab(chat("chat-a")));
    renderWorkspaceView(store);
    const dataTransfer = createDataTransferStub();
    setTabDragData(dataTransfer, "chat", "chat-c", chat("chat-c"));
    const occupiedPane = screen.getByLabelText("Workspace pane root");

    fireEvent.dragEnter(occupiedPane, { dataTransfer });
    fireEvent.dragOver(occupiedPane, { dataTransfer });
    fireEvent.drop(occupiedPane, { dataTransfer });

    const group = store.getState().workspace.groups[chat("chat-a")];
    expect(group).toBeDefined();
    if (!group) throw new Error("missing split group");
    const leafIds = collectLeafIds(group.root);
    expect(leafIds).toHaveLength(3);
    expect(
      leafIds.every((leafId) => {
        const leaf = findLeaf(group.root, leafId);
        return Boolean(leaf && leaf.tabIds.length <= 1);
      }),
    ).toBe(true);
    expect(collectTabIds(group.root).sort()).toEqual(
      [chat("chat-a"), chat("chat-b"), chat("chat-c")].sort(),
    );
    expect(store.getState().workspace.tabs).toEqual([chat("chat-a")]);
    expectSurface(chat("chat-a"));
    expectSurface(chat("chat-b"));
    expectSurface(chat("chat-c"));
    expect(screen.getAllByLabelText("Workspace pane controls")).toHaveLength(3);
  });

  it("keeps split sizes unchanged for non-finite resize input", async () => {
    const clientWidthSpy = vi
      .spyOn(HTMLElement.prototype, "clientWidth", "get")
      .mockReturnValue(1024);
    const store = createEmptySplitWorkspaceStore();
    renderWorkspaceView(store);

    try {
      const divider = await screen.findByTestId("workspace-vertical-divider");
      const split = document.querySelector<HTMLElement>(
        "[data-pane-split-id='root:split:row']",
      );
      if (!split) throw new Error("missing split");
      const groupBefore = store.getState().workspace.groups[chat("chat-a")];
      if (!groupBefore || groupBefore.root.kind !== "split") {
        throw new Error("missing split group");
      }
      const sizesBefore = [...groupBefore.root.sizes];
      const rectSpy = vi.spyOn(split, "getBoundingClientRect").mockReturnValue({
        x: Number.POSITIVE_INFINITY,
        y: 0,
        width: 1024,
        height: 768,
        top: 0,
        right: Number.POSITIVE_INFINITY,
        bottom: 768,
        left: Number.POSITIVE_INFINITY,
        toJSON: () => ({}),
      });

      try {
        fireEvent.mouseDown(divider);
        fireEvent.mouseMove(window, { clientX: 500 });
        fireEvent.mouseUp(window);
      } finally {
        rectSpy.mockRestore();
      }

      const groupAfter = store.getState().workspace.groups[chat("chat-a")];
      expect(groupAfter?.root.kind).toBe("split");
      if (!groupAfter || groupAfter.root.kind !== "split") {
        throw new Error("missing split group");
      }
      expect(groupAfter.root.sizes).toEqual(sizesBefore);
    } finally {
      clientWidthSpy.mockRestore();
    }
  });
});
