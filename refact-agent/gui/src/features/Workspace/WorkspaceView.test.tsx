import { describe, expect, it } from "vitest";

import { type AppStore, setUpStore } from "../../app/store";
import { render, screen } from "../../utils/test-utils";
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
});
