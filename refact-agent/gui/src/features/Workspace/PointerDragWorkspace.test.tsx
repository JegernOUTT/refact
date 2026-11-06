import { act } from "react-dom/test-utils";
import { afterEach, describe, expect, it } from "vitest";

import { type AppStore, setUpStore } from "../../app/store";
import { fireEvent, render, screen, within } from "../../utils/test-utils";
import { createChatWithId } from "../Chat/Thread";
import {
  collectLeafIds,
  collectTabIds,
  findLeaf,
} from "../ChatPanes/panesTree";
import { pointerDragController } from "../ChatPanes/pointerDrag";
import type { TabDragPayload } from "../ChatPanes/tabDrag";
import {
  addSurfaceToPane,
  openTab,
  setActiveTab,
  setPanelsForced,
  splitTab,
} from "./workspaceSlice";
import { openTask } from "../Tasks/tasksSlice";
import { makeSurfaceKey, type SurfaceKey } from "./surfaceKey";
import { Toolbar } from "../../components/Toolbar/Toolbar";
import { WorkspaceShell } from "./WorkspaceShell";
import { WorkspaceView } from "./WorkspaceView";

function renderWorkspace(store: AppStore) {
  return render(
    <>
      <Toolbar
        activeTab={{
          type: "chat",
          id: store.getState().chat.current_thread_id,
        }}
      />
      <WorkspaceShell>
        <WorkspaceView />
      </WorkspaceShell>
    </>,
    { store },
  );
}

function getChatGroup(store: AppStore) {
  const group =
    store.getState().workspace.groups[makeSurfaceKey("chat", "chat-a")];
  if (!group) throw new Error("Missing canonical chat group");
  return group;
}

const chat = (id: string): SurfaceKey => makeSurfaceKey("chat", id);
const task = (id: string): SurfaceKey => makeSurfaceKey("task", id);

function taskPayload(id: string): TabDragPayload {
  return { type: "task", id, surfaceKey: task(id) };
}

const jetbrainsConfig = {
  host: "jetbrains" as const,
  lspPort: 8001,
  apiKey: "test",
  themeProps: {},
};

function chatPayload(id: string): TabDragPayload {
  return { type: "chat", id, surfaceKey: chat(id) };
}

function mockRect(
  element: Element | null,
  bounds: { left: number; top: number; width: number; height: number },
): void {
  if (!element) throw new Error("missing element to mock rect");
  const { left, top, width, height } = bounds;
  const controls = element.querySelector("[data-pane-controls]");
  if (controls) {
    controls.getBoundingClientRect = () =>
      new DOMRect(left + width - 50, top, 50, 28);
  }
  element.getBoundingClientRect = () =>
    ({
      left,
      top,
      width,
      height,
      right: left + width,
      bottom: top + height,
      x: left,
      y: top,
      toJSON: () => ({}),
    }) as DOMRect;
}

function firePointerDown(
  element: Element,
  { clientX = 0, clientY = 0, pointerId = 1, button = 0 } = {},
): void {
  const event = new MouseEvent("pointerdown", {
    clientX,
    clientY,
    button,
    bubbles: true,
    cancelable: true,
  });
  Object.defineProperty(event, "pointerId", {
    value: pointerId,
    configurable: true,
  });
  fireEvent(element, event);
}

function fireWindowPointer(
  type: "pointermove" | "pointerup",
  { clientX = 0, clientY = 0, pointerId = 1 } = {},
): void {
  const event = new MouseEvent(type, { clientX, clientY });
  Object.defineProperty(event, "pointerId", {
    value: pointerId,
    configurable: true,
  });
  act(() => {
    window.dispatchEvent(event);
  });
}

function getTabWrap(name: RegExp): HTMLElement {
  const wrap = screen.getByRole("tab", { name }).closest("div");
  if (!wrap) throw new Error(`missing tab wrapper for ${name.source}`);
  return wrap;
}

function createChatTabsStore(): AppStore {
  const store = setUpStore({ config: jetbrainsConfig });
  store.dispatch(
    createChatWithId({ id: "chat-a", title: "Chat Alpha", mode: "agent" }),
  );
  store.dispatch(
    createChatWithId({ id: "chat-b", title: "Chat Beta", mode: "agent" }),
  );
  store.dispatch(
    createChatWithId({ id: "chat-c", title: "Chat Gamma", mode: "agent" }),
  );
  store.dispatch(openTab(chat("chat-a")));
  store.dispatch(openTab(chat("chat-b")));
  store.dispatch(openTab(chat("chat-c")));
  store.dispatch(setActiveTab(chat("chat-a")));
  return store;
}

function createWorkspaceStore(): AppStore {
  const store = setUpStore({ config: jetbrainsConfig });
  store.dispatch(createChatWithId({ id: "chat-a", title: "Chat Alpha" }));
  store.dispatch(createChatWithId({ id: "chat-b", title: "Chat Beta" }));
  store.dispatch(openTab(chat("chat-a")));
  store.dispatch(openTab(chat("chat-b")));
  store.dispatch(setActiveTab(chat("chat-a")));
  return store;
}

afterEach(() => {
  pointerDragController.cancel();
});

describe("Workspace pointer drag-and-drop (JCEF hosts)", () => {
  it("disables native dragging and starts a pointer drag from a tab", () => {
    const store = createChatTabsStore();
    renderWorkspace(store);

    const gammaTab = screen.getByRole("tab", { name: /Chat Gamma/ });
    expect(gammaTab).toHaveAttribute("draggable", "false");

    firePointerDown(gammaTab, { clientX: 0, clientY: 0, pointerId: 7 });
    fireWindowPointer("pointermove", { clientX: 40, clientY: 0, pointerId: 7 });

    expect(pointerDragController.isDragging()).toBe(true);
    expect(pointerDragController.getSnapshot().label).toBe("Chat Gamma");
  });

  it("reorders chat tabs on a pointer drop", () => {
    const store = createChatTabsStore();
    renderWorkspace(store);

    mockRect(getTabWrap(/Chat Alpha/), {
      left: 0,
      top: 0,
      width: 120,
      height: 32,
    });

    act(() => {
      pointerDragController.startDrag(
        { payload: chatPayload("chat-c"), label: "Chat Gamma" },
        { x: 60, y: 16 },
      );
    });
    fireWindowPointer("pointermove", { clientX: 60, clientY: 16 });
    fireWindowPointer("pointerup", { clientX: 60, clientY: 16 });

    expect(collectTabIds(getChatGroup(store).root)).toEqual([
      chat("chat-c"),
      chat("chat-a"),
      chat("chat-b"),
    ]);
  });

  it("reorders task tabs on a pointer drop", () => {
    const store = createChatTabsStore();
    store.dispatch(openTask({ id: "task-a", name: "Task Alpha" }));
    store.dispatch(openTask({ id: "task-b", name: "Task Beta" }));
    renderWorkspace(store);

    mockRect(getTabWrap(/Task Alpha/), {
      left: 0,
      top: 0,
      width: 120,
      height: 32,
    });

    act(() => {
      pointerDragController.startDrag(
        { payload: taskPayload("task-b"), label: "Task Beta" },
        { x: 60, y: 16 },
      );
    });
    fireWindowPointer("pointermove", { clientX: 60, clientY: 16 });
    fireWindowPointer("pointerup", { clientX: 60, clientY: 16 });

    expect(collectTabIds(getChatGroup(store).root)).toEqual([
      chat("chat-a"),
      chat("chat-b"),
      chat("chat-c"),
      task("task-b"),
      task("task-a"),
    ]);
    expect(collectLeafIds(getChatGroup(store).root)).toHaveLength(1);
    expect(store.getState().tasksUI.openTasks.map((item) => item.id)).toEqual([
      "task-a",
      "task-b",
    ]);
  });

  it("suppresses the click synthesized at the end of a pointer drag", () => {
    const store = createChatTabsStore();
    renderWorkspace(store);

    const gammaTab = screen.getByRole("tab", { name: /Chat Gamma/ });
    firePointerDown(gammaTab, { clientX: 0, clientY: 0, pointerId: 3 });
    fireWindowPointer("pointermove", { clientX: 40, clientY: 0, pointerId: 3 });
    expect(pointerDragController.isDragging()).toBe(true);

    const clickEvent = new MouseEvent("click", {
      bubbles: true,
      cancelable: true,
    });
    act(() => {
      gammaTab.dispatchEvent(clickEvent);
    });
    expect(clickEvent.defaultPrevented).toBe(true);
  });

  it("splits the singleton pane at its edge on a pointer drop", async () => {
    const store = createWorkspaceStore();
    renderWorkspace(store);

    const dropTarget = document.querySelector(
      "[data-workspace-leaf-id='root']",
    );
    mockRect(dropTarget, { left: 0, top: 0, width: 400, height: 300 });

    act(() => {
      pointerDragController.startDrag(
        { payload: chatPayload("chat-b") },
        { x: 200, y: 150 },
      );
    });
    fireWindowPointer("pointermove", { clientX: 390, clientY: 150 });
    expect(
      await screen.findByTestId("workspace-pane-drop-preview"),
    ).toHaveAttribute("data-drop-target", "right");
    fireWindowPointer("pointerup", { clientX: 390, clientY: 150 });

    expect(store.getState().workspace.groups[chat("chat-a")]).toBeDefined();
  });

  it("drags a file tab with pointer payload and splits a file surface", async () => {
    const store = createWorkspaceStore();
    store.dispatch(setPanelsForced(true));
    const fileA = "file:/project/a.ts";
    const fileB = "file:/project/b.ts";
    store.dispatch(openTab(fileA));
    store.dispatch(openTab(fileB));
    store.dispatch(setActiveTab(fileA));
    renderWorkspace(store);
    const dropTarget = document.querySelector(
      "[data-workspace-leaf-id='root']",
    );
    mockRect(dropTarget, { left: 100, top: 100, width: 400, height: 300 });
    const tab = screen.getByRole("tab", { name: /b.ts/ });
    firePointerDown(tab, { clientX: 0, clientY: 0, pointerId: 9 });
    fireWindowPointer("pointermove", {
      clientX: 490,
      clientY: 250,
      pointerId: 9,
    });
    expect(pointerDragController.getSnapshot().active).toBe(true);
    expect(
      await screen.findByTestId("workspace-pane-drop-preview"),
    ).toHaveAttribute("data-drop-target", "right");
    fireWindowPointer("pointerup", {
      clientX: 490,
      clientY: 150,
      pointerId: 9,
    });
    const group = store.getState().workspace.groups[chat("chat-a")];
    if (!group) throw new Error("missing file pointer split");
    expect(collectTabIds(group.root)).toEqual([
      chat("chat-a"),
      chat("chat-b"),
      fileA,
      fileB,
    ]);
    expect(store.getState().workspace.tabs).not.toContain(fileB);
  });

  it("fills an empty split pane on a pointer drop", () => {
    const store = createWorkspaceStore();
    store.dispatch(splitTab({ tabId: chat("chat-a"), dir: "row" }));
    renderWorkspace(store);

    const emptyPane = screen.getByLabelText(
      "Workspace pane root:sibling:chat:chat-a",
    );
    mockRect(emptyPane, { left: 200, top: 0, width: 200, height: 300 });

    act(() => {
      pointerDragController.startDrag(
        { payload: chatPayload("chat-b") },
        { x: 300, y: 150 },
      );
    });
    fireWindowPointer("pointermove", { clientX: 300, clientY: 150 });
    fireWindowPointer("pointerup", { clientX: 300, clientY: 150 });

    const group = store.getState().workspace.groups[chat("chat-a")];
    if (!group) throw new Error("missing group after fill");
    expect(collectTabIds(group.root)).toContain(chat("chat-b"));
  });

  it("joins occupied pane centers and reorders local tabs without splitting", () => {
    const store = createWorkspaceStore();
    store.dispatch(splitTab({ tabId: chat("chat-a"), dir: "row" }));
    renderWorkspace(store);
    const pane = screen.getByLabelText("Workspace pane root");
    mockRect(pane, { left: 0, top: 0, width: 400, height: 300 });
    act(() =>
      pointerDragController.startDrag(
        { payload: chatPayload("chat-b") },
        { x: 200, y: 150 },
      ),
    );
    fireWindowPointer("pointerup", { clientX: 200, clientY: 150 });
    const group = store.getState().workspace.groups[chat("chat-a")];
    expect(group && collectLeafIds(group.root)).toHaveLength(1);
    expect(group && findLeaf(group.root, "root")?.tabIds).toEqual([
      chat("chat-a"),
      chat("chat-b"),
    ]);
    const joinedPane = screen.getByLabelText("Workspace pane root");
    mockRect(joinedPane, { left: 0, top: 0, width: 400, height: 300 });
    const beta = screen.getByRole("tab", { name: /Chat Beta/ });
    mockRect(beta.closest("div"), {
      left: 100,
      top: 0,
      width: 100,
      height: 30,
    });
    firePointerDown(screen.getByRole("tab", { name: /Chat Alpha/ }), {
      clientX: 0,
      clientY: 0,
      pointerId: 11,
    });
    fireWindowPointer("pointermove", {
      clientX: 150,
      clientY: 15,
      pointerId: 11,
    });
    expect(pointerDragController.isDragging()).toBe(true);
    fireWindowPointer("pointerup", {
      clientX: 150,
      clientY: 15,
      pointerId: 11,
    });
    const reordered = store.getState().workspace.groups[chat("chat-a")];
    expect(reordered && findLeaf(reordered.root, "root")?.tabIds).toEqual([
      chat("chat-b"),
      chat("chat-a"),
    ]);
    expect(reordered && collectLeafIds(reordered.root)).toHaveLength(1);
  });

  it("splits a pane along an edge on a pointer drop", async () => {
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
    store.dispatch(createChatWithId({ id: "chat-c", title: "Chat Gamma" }));
    store.dispatch(openTab(chat("chat-c")));
    store.dispatch(setActiveTab(chat("chat-a")));
    renderWorkspace(store);

    const rootPane = screen.getByLabelText("Workspace pane root");
    mockRect(rootPane, { left: 0, top: 0, width: 200, height: 200 });

    act(() => {
      pointerDragController.startDrag(
        { payload: chatPayload("chat-c") },
        { x: 100, y: 100 },
      );
    });

    fireWindowPointer("pointermove", { clientX: 190, clientY: 100 });
    expect(
      await within(rootPane).findByTestId("workspace-pane-drop-preview"),
    ).toHaveAttribute("data-drop-target", "right");
    fireWindowPointer("pointerup", { clientX: 190, clientY: 100 });

    const updated = store.getState().workspace.groups[chat("chat-a")];
    if (!updated) throw new Error("missing group after edge drop");
    expect(collectTabIds(updated.root)).toContain(chat("chat-c"));
    expect(collectLeafIds(updated.root).length).toBeGreaterThanOrEqual(3);
  });
});
