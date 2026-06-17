import { describe, expect, it } from "vitest";

import { type AppStore, setUpStore } from "../../app/store";
import { fireEvent, render, screen, within } from "../../utils/test-utils";
import { createChatWithId } from "../Chat/Thread";
import {
  addSurfaceToPane,
  openTab,
  setActiveTab,
  splitTab,
} from "./workspaceSlice";
import { makeSurfaceKey, type SurfaceKey } from "./surfaceKey";
import { WorkspaceView } from "./WorkspaceView";

const chat = (id: string): SurfaceKey => makeSurfaceKey("chat", id);

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
    expect(screen.getAllByRole("tablist")).toHaveLength(1);
    expect(screen.queryByLabelText("Workspace pane controls")).toBeNull();
    expect(screen.queryByRole("button", { name: "Close Pane" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Split Right" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Split Down" })).toBeNull();
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

  it("clicking a pane close ungroups the split", async () => {
    const store = createSplitWorkspaceStore();
    const view = renderWorkspaceView(store);
    const siblingPane = screen.getByLabelText(
      "Workspace pane root:sibling:chat:chat-a",
    );

    await view.user.click(
      within(siblingPane).getByRole("button", { name: "Close Pane" }),
    );

    expect(store.getState().workspace.groups[chat("chat-a")]).toBeUndefined();
    expect(store.getState().workspace.tabs).toEqual([chat("chat-a")]);
    expect(screen.queryByLabelText("Workspace pane controls")).toBeNull();
    expectSurface(chat("chat-a"));
  });

  it("dropping a workspace tab on a pane edge creates a split", () => {
    const store = createEmptySplitWorkspaceStore();
    renderWorkspaceView(store);
    const dataTransfer = createDataTransferStub();
    dataTransfer.setData("text/plain", "chat:chat-b");
    const targetPane = screen.getByLabelText("Workspace pane root");

    fireEvent.dragEnter(targetPane, { dataTransfer });
    const edgeDropZone = within(targetPane).getByTestId(
      "workspace-pane-edge-drop-root-right",
    );
    fireEvent.dragOver(edgeDropZone, { dataTransfer });
    fireEvent.drop(edgeDropZone, { dataTransfer });

    const group = store.getState().workspace.groups[chat("chat-a")];
    expect(group).toBeDefined();
    expect(store.getState().workspace.tabs).toEqual([chat("chat-a")]);
    expectSurface(chat("chat-a"));
    expectSurface(chat("chat-b"));
    expect(screen.getAllByLabelText("Workspace pane controls")).toHaveLength(2);
  });
});
