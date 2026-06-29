import { act } from "react-dom/test-utils";
import { afterEach, describe, expect, it } from "vitest";

import { type AppStore, setUpStore } from "../../app/store";
import { fireEvent, render, screen, within } from "../../utils/test-utils";
import { createChatWithId } from "../Chat/Thread";
import { collectLeafIds, collectTabIds } from "../ChatPanes/panesTree";
import { pointerDragController } from "../ChatPanes/pointerDrag";
import type { TabDragPayload } from "../ChatPanes/tabDrag";
import {
  addSurfaceToPane,
  openTab,
  setActiveTab,
  splitTab,
} from "./workspaceSlice";
import { openTask } from "../Tasks/tasksSlice";
import { makeSurfaceKey, type SurfaceKey } from "./surfaceKey";
import { TabBar } from "./TabBar";
import { WorkspaceView } from "./WorkspaceView";

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
    render(<TabBar />, { store });

    const gammaTab = screen.getByRole("tab", { name: /Chat Gamma/ });
    expect(gammaTab).toHaveAttribute("draggable", "false");

    firePointerDown(gammaTab, { clientX: 0, clientY: 0, pointerId: 7 });
    fireWindowPointer("pointermove", { clientX: 40, clientY: 0, pointerId: 7 });

    expect(pointerDragController.isDragging()).toBe(true);
    expect(pointerDragController.getSnapshot().label).toBe("Chat Gamma");
  });

  it("reorders chat tabs on a pointer drop", () => {
    const store = createChatTabsStore();
    render(<TabBar />, { store });

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

    expect(store.getState().workspace.tabs).toEqual([
      chat("chat-c"),
      chat("chat-a"),
      chat("chat-b"),
    ]);
  });

  it("reorders task tabs on a pointer drop", () => {
    const store = createChatTabsStore();
    store.dispatch(openTask({ id: "task-a", name: "Task Alpha" }));
    store.dispatch(openTask({ id: "task-b", name: "Task Beta" }));
    render(<TabBar />, { store });

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

    expect(
      store.getState().tasksUI.openTasks.map((openTaskItem) => openTaskItem.id),
    ).toEqual(["task-b", "task-a"]);
  });

  it("suppresses the click synthesized at the end of a pointer drag", () => {
    const store = createChatTabsStore();
    render(<TabBar />, { store });

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

  it("splits the unsplit surface on a pointer drop", () => {
    const store = createWorkspaceStore();
    render(<WorkspaceView />, { store });

    const dropTarget = document.querySelector(
      "[data-workspace-unsplit-drop-target='true']",
    );
    mockRect(dropTarget, { left: 0, top: 0, width: 400, height: 300 });

    act(() => {
      pointerDragController.startDrag(
        { payload: chatPayload("chat-b") },
        { x: 200, y: 150 },
      );
    });
    fireWindowPointer("pointermove", { clientX: 200, clientY: 150 });
    fireWindowPointer("pointerup", { clientX: 200, clientY: 150 });

    expect(store.getState().workspace.groups[chat("chat-a")]).toBeDefined();
  });

  it("fills an empty split pane on a pointer drop", () => {
    const store = createWorkspaceStore();
    store.dispatch(splitTab({ tabId: chat("chat-a"), dir: "row" }));
    render(<WorkspaceView />, { store });

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
    render(<WorkspaceView />, { store });

    const rootPane = screen.getByLabelText("Workspace pane root");
    mockRect(rootPane, { left: 0, top: 0, width: 200, height: 200 });

    act(() => {
      pointerDragController.startDrag(
        { payload: chatPayload("chat-c") },
        { x: 100, y: 100 },
      );
    });

    const rightEdge = await within(rootPane).findByTestId(
      "workspace-pane-edge-drop-root-right",
    );
    mockRect(rightEdge, { left: 180, top: 0, width: 20, height: 200 });

    fireWindowPointer("pointermove", { clientX: 190, clientY: 100 });
    fireWindowPointer("pointerup", { clientX: 190, clientY: 100 });

    const updated = store.getState().workspace.groups[chat("chat-a")];
    if (!updated) throw new Error("missing group after edge drop");
    expect(collectTabIds(updated.root)).toContain(chat("chat-c"));
    expect(collectLeafIds(updated.root).length).toBeGreaterThanOrEqual(3);
  });
});
