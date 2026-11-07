import {
  openTask,
  addPlannerChat,
  closeTask,
} from "../features/Tasks/tasksSlice";
import { afterEach, describe, expect, it } from "vitest";
import { setUpStore } from "./store";
import { createChatWithId } from "../features/Chat/Thread/actions";
import { setCurrentProjectInfo } from "../features/Chat/currentProject";
import { push, selectCurrentPage } from "../features/Pages/pagesSlice";
import {
  workspaceSlice,
  selectFocusedWorkspaceChatId,
  selectSurfaceChatId,
} from "../features/Workspace/workspaceSlice";
import {
  loadPersistedWorkspace,
  setProjectStorageNamespace,
} from "../utils/chatUiPersistence";
import {
  collectTabIds,
  collectLeafIds,
  findLeaf,
} from "../features/ChatPanes/panesTree";

import { openFileInFilesPanel } from "../features/Workspace/FilesPanel/filesPanelSlice";
const actions = workspaceSlice.actions;
function setup() {
  const store = setUpStore();
  store.dispatch(
    setCurrentProjectInfo({
      name: "pane-listener-test",
      workspaceRoots: ["/workspace/pane-listener-test"],
    }),
  );
  store.dispatch(createChatWithId({ id: "pane-a" }));
  store.dispatch(createChatWithId({ id: "pane-b" }));
  store.dispatch(
    actions.addSurfaceToPane({
      tabId: "chat:pane-a",
      leafId: "root",
      surfaceKey: "chat:pane-b",
    }),
  );
  const group = store.getState().workspace.groups["chat:pane-a"];
  if (!group || group.root.kind !== "leaf")
    throw new Error("Expected local pane");
  return { store, leafId: group.root.id };
}
afterEach(() => {
  localStorage.clear();
  setProjectStorageNamespace(undefined);
});
describe("pane-tab listener integration", () => {
  it("persists reorder, local close, and open-in-new-tab immediately", () => {
    const { store, leafId } = setup();
    store.dispatch(
      actions.reorderPaneTabs({
        tabId: "chat:pane-a",
        leafId,
        fromIndex: 1,
        toIndex: 0,
      }),
    );
    expect(loadPersistedWorkspace().groups["chat:pane-a"]?.root).toEqual(
      store.getState().workspace.groups["chat:pane-a"]?.root,
    );
    store.dispatch(
      actions.closePaneTab({
        tabId: "chat:pane-a",
        leafId,
        surfaceKey: "chat:pane-b",
      }),
    );
    expect(loadPersistedWorkspace().tabs).toEqual(
      store.getState().workspace.tabs,
    );
    store.dispatch(
      actions.addSurfaceToPane({
        tabId: "chat:pane-a",
        leafId,
        surfaceKey: "file:/workspace/a.ts",
      }),
    );
    store.dispatch(actions.openSurfaceInNewTab("file:/workspace/a.ts"));
    expect(loadPersistedWorkspace().tabs).toEqual(
      store.getState().workspace.tabs,
    );
    expect(loadPersistedWorkspace().groups["chat:pane-a"]?.root).toMatchObject({
      activeTabId: "file:/workspace/a.ts",
    });
  });
  it("syncs focused chat and closes the removed local chat without ghosts", () => {
    const { store, leafId } = setup();
    store.dispatch(
      actions.setPaneActive({
        tabId: "chat:pane-a",
        leafId,
        surfaceKey: "chat:pane-b",
      }),
    );
    expect(store.getState().chat.current_thread_id).toBe("pane-b");
    store.dispatch(
      actions.closePaneTab({
        tabId: "chat:pane-a",
        leafId,
        surfaceKey: "chat:pane-b",
      }),
    );
    expect(store.getState().chat.current_thread_id).toBe(
      selectFocusedWorkspaceChatId(store.getState()),
    );
    expect(store.getState().chat.open_thread_ids).not.toContain("pane-b");
    expect(
      collectTabIds(
        store.getState().workspace.groups["chat:pane-a"]?.root ?? {
          kind: "leaf",
          id: "empty",
          tabIds: [],
          activeTabId: null,
        },
      ),
    ).not.toContain("chat:pane-b");
  });
});

describe("closing the last workspace surface", () => {
  function setupSingleSurface(surfaceKey: string) {
    const store = setUpStore();
    store.dispatch(push({ name: "history" }));
    store.dispatch(push({ name: "chat" }));
    if (surfaceKey.startsWith("chat:")) {
      store.dispatch(
        createChatWithId({ id: surfaceKey.slice("chat:".length) }),
      );
    } else {
      store.dispatch(actions.openTab(surfaceKey));
    }
    const group = store.getState().workspace.groups[surfaceKey];
    if (!group) throw new Error("Expected workspace group");
    return { store, leafId: group.focusedLeafId };
  }

  it.each(["chat:last-chat", "file:/last.ts"])(
    "returns to Home when the final pane tab %s closes",
    (surfaceKey) => {
      const { store, leafId } = setupSingleSurface(surfaceKey);
      store.dispatch(
        actions.closePaneTab({ tabId: surfaceKey, leafId, surfaceKey }),
      );
      expect(store.getState().workspace.tabs).toEqual([]);
      expect(selectCurrentPage(store.getState())).toEqual({ name: "history" });
    },
  );

  it.each(["closeTab", "closePane"] as const)(
    "returns to Home when %s removes the final surface",
    (closeAction) => {
      const surfaceKey = "file:/last.ts";
      const { store, leafId } = setupSingleSurface(surfaceKey);
      store.dispatch(
        closeAction === "closeTab"
          ? actions.closeTab(surfaceKey)
          : actions.closePane({ tabId: surfaceKey, leafId }),
      );
      expect(store.getState().workspace.tabs).toEqual([]);
      expect(selectCurrentPage(store.getState())).toEqual({ name: "history" });
    },
  );

  it("stays in chat when a split leaf empties but a file remains in another leaf", () => {
    const tabId = "file:/first.ts";
    const { store, leafId } = setupSingleSurface(tabId);
    store.dispatch(actions.splitTab({ tabId, dir: "row" }));
    store.dispatch(actions.openTab("file:/remaining.ts"));
    store.dispatch(actions.closePaneTab({ tabId, leafId, surfaceKey: tabId }));
    const remainingTabId = store.getState().workspace.tabs[0];
    const group = store.getState().workspace.groups[remainingTabId];
    expect(group && collectTabIds(group.root)).toEqual(["file:/remaining.ts"]);
    expect(selectCurrentPage(store.getState())).toEqual({ name: "chat" });
    if (!group) throw new Error("Expected remaining file pane");
    store.dispatch(
      actions.closePaneTab({
        tabId: remainingTabId,
        leafId: group.focusedLeafId,
        surfaceKey: "file:/remaining.ts",
      }),
    );
    expect(store.getState().workspace.tabs).toEqual([]);
    expect(selectCurrentPage(store.getState())).toEqual({ name: "history" });
  });

  it("does not navigate for no-op closes in populated or empty workspaces", () => {
    const { store, leafId } = setupSingleSurface("file:/last.ts");
    store.dispatch(
      actions.closePaneTab({
        tabId: "file:/last.ts",
        leafId,
        surfaceKey: "file:/missing.ts",
      }),
    );
    expect(store.getState().workspace.tabs).toEqual(["file:/last.ts"]);
    expect(selectCurrentPage(store.getState())).toEqual({ name: "chat" });
    const emptyStore = setUpStore();
    emptyStore.dispatch(push({ name: "chat" }));
    emptyStore.dispatch(actions.closeTab("file:/missing.ts"));
    emptyStore.dispatch(
      actions.closePane({ tabId: "file:/missing.ts", leafId: "root" }),
    );
    emptyStore.dispatch(
      actions.closePaneTab({
        tabId: "file:/missing.ts",
        leafId: "root",
        surfaceKey: "file:/missing.ts",
      }),
    );
    expect(selectCurrentPage(emptyStore.getState())).toEqual({ name: "chat" });
  });

  it.each([
    { name: "general settings" } as const,
    { name: "task workspace", taskId: "owned-task" } as const,
  ])(
    "preserves the current $name page when the last workspace surface closes",
    (page) => {
      const surfaceKey = "file:/last.ts";
      const { store, leafId } = setupSingleSurface(surfaceKey);
      store.dispatch(push(page));
      store.dispatch(
        actions.closePaneTab({ tabId: surfaceKey, leafId, surfaceKey }),
      );
      expect(store.getState().workspace.tabs).toEqual([]);
      expect(selectCurrentPage(store.getState())).toEqual(page);
    },
  );
});

describe("single-layer workspace migration", () => {
  it("keeps all newly opened chats and files reachable in the focused pane", () => {
    const { store } = setup();
    store.dispatch(actions.splitTab({ tabId: "chat:pane-a", dir: "row" }));
    const id =
      store.getState().workspace.groups["chat:pane-a"]?.focusedLeafId ?? "";
    store.dispatch(createChatWithId({ id: "pane-c" }));
    store.dispatch(
      openFileInFilesPanel({ path: "/workspace/new.ts", openInNewTab: true }),
    );
    const workspace = store.getState().workspace;
    expect(workspace.tabs).toEqual(["chat:pane-a"]);
    const group = workspace.groups["chat:pane-a"];
    expect(group && findLeaf(group.root, id)?.tabIds).toEqual([
      "chat:pane-c",
      "file:/workspace/new.ts",
    ]);
    expect(group && collectTabIds(group.root)).toEqual([
      "chat:pane-a",
      "chat:pane-b",
      "chat:pane-c",
      "file:/workspace/new.ts",
    ]);
    expect(
      selectSurfaceChatId(store.getState(), "file:/workspace/new.ts"),
    ).toBe("pane-c");
    expect(loadPersistedWorkspace().groups).toEqual(workspace.groups);
  });
  it("merges legacy groups deterministically, preserves active splits and reloads idempotently", () => {
    const { store } = setup();
    const pane = (id: string, tabs: string[]) => ({
      kind: "leaf" as const,
      id,
      tabIds: tabs,
      activeTabId: tabs[0],
    });
    store.dispatch(
      actions.hydrateWorkspace({
        tabs: ["chat:pane-a", "chat:pane-b", "file:/orphan"],
        activeTabId: "chat:pane-a",
        groups: {
          "chat:pane-a": {
            root: {
              kind: "split",
              id: "active-split",
              dir: "row",
              sizes: [0.7, 0.3],
              children: [
                pane("left", ["chat:pane-a", "file:/a"]),
                pane("right", ["file:/b"]),
              ],
            },
            focusedLeafId: "right",
          },
          "chat:pane-b": {
            root: pane("root", ["chat:pane-b", "file:/task"]),
            focusedLeafId: "root",
          },
        },
        contextChatByTab: { "chat:pane-b": "pane-b", "chat:pane-a": "pane-a" },
        openThreadIds: ["pane-a", "pane-b"],
      }),
    );
    const state = store.getState().workspace;
    const group = state.groups["chat:pane-a"];
    expect(state.tabs).toEqual(["chat:pane-a"]);
    expect(group?.root).toMatchObject({
      id: "active-split",
      sizes: [0.7, 0.3],
    });
    expect(group && findLeaf(group.root, "right")?.tabIds).toEqual([
      "file:/b",
      "chat:pane-b",
      "file:/task",
      "file:/orphan",
    ]);
    expect(selectSurfaceChatId(store.getState(), "file:/task")).toBe("pane-b");
    store.dispatch(actions.consolidateWorkspaceTabs());
    expect(store.getState().workspace).toBe(state);
    const persisted = loadPersistedWorkspace();
    store.dispatch(
      actions.hydrateWorkspace({
        ...persisted,
        openThreadIds: ["pane-a", "pane-b"],
      }),
    );
    expect(store.getState().workspace.groups).toEqual(state.groups);
  });
  it("reopening an existing editor does not steal its owning chat", () => {
    const { store } = setup();
    store.dispatch(openFileInFilesPanel({ path: "/owned.ts" }));
    store.dispatch(actions.setActiveTab("chat:pane-a"));
    store.dispatch(openFileInFilesPanel({ path: "/owned.ts" }));
    expect(selectSurfaceChatId(store.getState(), "file:/owned.ts")).toBe(
      "pane-b",
    );
    store.dispatch(actions.setActiveTab("chat:pane-a"));
    store.dispatch(actions.openTab("file:/owned.ts"));
    expect(selectSurfaceChatId(store.getState(), "file:/owned.ts")).toBe(
      "pane-b",
    );
  });
  it("missing hydrated chats append without stealing restored selection", () => {
    const { store } = setup();
    store.dispatch(
      actions.hydrateWorkspace({
        tabs: ["chat:pane-a"],
        activeTabId: "chat:pane-a",
        groups: {},
        openThreadIds: ["pane-a", "pane-b"],
      }),
    );
    const group = store.getState().workspace.groups["chat:pane-a"];
    expect(group?.root).toMatchObject({
      activeTabId: "chat:pane-a",
      tabIds: ["chat:pane-a", "chat:pane-b"],
    });
    expect(store.getState().chat.current_thread_id).toBe("pane-a");
  });
  it("allows thirty files in addition to fifty already open chats", () => {
    let state = workspaceSlice.getInitialState();
    for (let i = 0; i < 50; i++)
      state = workspaceSlice.reducer(state, actions.openTab(`chat:${i}`));
    for (let i = 0; i < 30; i++)
      state = workspaceSlice.reducer(state, actions.openTab(`file:/${i}`));
    const root = state.groups["chat:0"]?.root;
    expect(root && collectTabIds(root)).toHaveLength(80);
    state = workspaceSlice.reducer(state, actions.openTab("file:/overflow"));
    expect(state.groups["chat:0"]?.root).toEqual(root);
  });
  it("live edits keep manually opened editors and previous live files reachable", () => {
    const { store } = setup();
    store.dispatch(openFileInFilesPanel({ path: "/manual.ts" }));
    store.dispatch(
      actions.ensureLiveEditSplit({
        chatId: "pane-b",
        filePath: "/live-one.ts",
      }),
    );
    store.dispatch(
      actions.ensureLiveEditSplit({
        chatId: "pane-b",
        filePath: "/live-two.ts",
      }),
    );
    store.dispatch(
      actions.ensureLiveEditSplit({
        chatId: "pane-b",
        filePath: "/live-two.ts",
      }),
    );
    const root = store.getState().workspace.groups["chat:pane-a"]?.root;
    const surfaces = root ? collectTabIds(root) : [];
    expect(surfaces).toEqual([
      "chat:pane-a",
      "chat:pane-b",
      "file:/manual.ts",
      "file:/live-one.ts",
      "file:/live-two.ts",
    ]);
    expect(new Set(surfaces).size).toBe(surfaces.length);
    expect(selectSurfaceChatId(store.getState(), "file:/live-two.ts")).toBe(
      "pane-b",
    );
  });
  it("bounds migrated panes at six without dropping inactive group tabs", () => {
    let state = workspaceSlice.getInitialState();
    const groups: typeof state.groups = {};
    const tabs: string[] = [];
    for (let i = 0; i < 4; i++) {
      const key = `chat:${i}`;
      tabs.push(key);
      groups[key] = {
        focusedLeafId: `left-${i}`,
        root: {
          kind: "split",
          id: `split-${i}`,
          dir: "row",
          sizes: [0.5, 0.5],
          children: [
            { kind: "leaf", id: `left-${i}`, tabIds: [key], activeTabId: key },
            {
              kind: "leaf",
              id: `right-${i}`,
              tabIds: [`file:/${i}`],
              activeTabId: `file:/${i}`,
            },
          ],
        },
      };
    }
    state = workspaceSlice.reducer(
      state,
      actions.hydrateWorkspace({ tabs, groups, activeTabId: tabs[0] }),
    );
    const root = state.groups[tabs[0]]?.root;
    expect(root && collectLeafIds(root)).toHaveLength(6);
    expect(root && collectTabIds(root).length).toBe(8);
    expect(root && new Set(collectTabIds(root)).size).toBe(8);
  });
});

describe("real Task pane lifecycle", () => {
  it("closes pane UI without deleting planner runtime and does not resurrect on reconciliation", () => {
    const { store, leafId } = setup();
    store.dispatch(openTask({ id: "owned-task", name: "Owned" }));
    store.dispatch(
      addPlannerChat({
        taskId: "owned-task",
        planner: {
          id: "pane-b",
          title: "Planner",
          createdAt: "now",
          updatedAt: "now",
        },
      }),
    );
    store.dispatch(
      actions.closePaneTab({
        tabId: "chat:pane-a",
        leafId,
        surfaceKey: "task:owned-task",
      }),
    );
    expect(
      store
        .getState()
        .tasksUI.openTasks.some((task) => task.id === "owned-task"),
    ).toBe(false);
    expect(store.getState().chat.threads["pane-b"]).toBeDefined();
    expect(store.getState().chat.open_thread_ids).toContain("pane-b");
    store.dispatch(
      actions.reconcileWorkspace({
        openThreadIds: store.getState().chat.open_thread_ids,
      }),
    );
    const group = store.getState().workspace.groups["chat:pane-a"];
    expect(group && collectTabIds(group.root)).not.toContain("task:owned-task");
  });
  it("explicit Task UI close removes its real owned surface", () => {
    const { store } = setup();
    store.dispatch(openTask({ id: "owned-task", name: "Owned" }));
    store.dispatch(closeTask("owned-task"));
    const group = store.getState().workspace.groups["chat:pane-a"];
    expect(group && collectTabIds(group.root)).not.toContain("task:owned-task");
    expect(store.getState().chat.threads["pane-a"]).toBeDefined();
  });
});
