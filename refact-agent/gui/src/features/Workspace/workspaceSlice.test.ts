import { describe, expect, test } from "vitest";
import {
  collectLeafIds,
  findLeaf,
  type LeafPane,
} from "../ChatPanes/panesTree";
import {
  addSurfaceToPane,
  closePane,
  closeTab,
  focusPane,
  MAX_GROUP_LEAVES,
  MAX_WORKSPACE_TABS,
  openTab,
  reconcileWorkspace,
  reorderTabs,
  selectActiveGroup,
  selectActiveTabId,
  selectGroupForTab,
  selectIsTabSplit,
  selectTabs,
  selectVisibleSurfaceKeys,
  selectVisibleThreadIds,
  setActiveTab,
  setPaneActive,
  splitPaneWithSurface,
  splitTab,
  workspaceSlice,
  type PaneGroup,
  type WorkspaceState,
} from "./workspaceSlice";
import {
  isChatSurface,
  makeSurfaceKey,
  parseSurfaceKey,
  type SurfaceKey,
} from "./surfaceKey";

const reducer = workspaceSlice.reducer;

const chat = (id: string): SurfaceKey => makeSurfaceKey("chat", id);

const leaf = (
  id: string,
  tabIds: SurfaceKey[] = [],
  activeTabId: SurfaceKey | null = tabIds[0] ?? null,
): LeafPane => ({
  kind: "leaf",
  id,
  tabIds,
  activeTabId,
});

const rootState = (workspace: WorkspaceState) => ({ workspace });

const groupFor = (state: WorkspaceState, tabId: SurfaceKey): PaneGroup => {
  const group = state.groups[tabId];
  if (!group) {
    throw new Error(`missing group for ${tabId}`);
  }
  return group;
};

const splitAndAdd = (
  state: WorkspaceState,
  tabId: SurfaceKey,
  leafId: string,
  surfaceKey: SurfaceKey,
): WorkspaceState => {
  let next = reducer(state, openTab(surfaceKey));
  next = reducer(next, addSurfaceToPane({ tabId, leafId, surfaceKey }));
  return reducer(next, splitTab({ tabId, dir: "row" }));
};

describe("surfaceKey helpers", () => {
  test("creates and parses typed keys", () => {
    expect(makeSurfaceKey("chat", "thread-a")).toBe("chat:thread-a");
    expect(makeSurfaceKey("task", "task-a")).toBe("task:task-a");
    expect(makeSurfaceKey("buddy", "home")).toBe("buddy:home");
    expect(makeSurfaceKey("dashboard")).toBe("dashboard");
    expect(parseSurfaceKey("chat:thread-a")).toEqual({
      kind: "chat",
      id: "thread-a",
    });
    expect(parseSurfaceKey("dashboard")).toEqual({
      kind: "dashboard",
      id: null,
    });
    expect(isChatSurface("chat:thread-a")).toBe(true);
    expect(isChatSurface("task:task-a")).toBe(false);
    expect(() => parseSurfaceKey("chat:")).toThrow("invalid surface key");
  });
});

describe("workspaceSlice", () => {
  test("opens, activates, reorders, and closes tabs", () => {
    const chatA = chat("a");
    const chatB = chat("b");
    const chatC = chat("c");
    let state = reducer(undefined, openTab(chatA));
    state = reducer(state, openTab(chatB));
    state = reducer(state, openTab(chatB));
    state = reducer(state, openTab(chatC));

    expect(selectTabs(rootState(state))).toEqual([chatA, chatB, chatC]);
    expect(selectActiveTabId(rootState(state))).toBe(chatC);

    state = reducer(state, setActiveTab(chatA));
    expect(selectActiveTabId(rootState(state))).toBe(chatA);

    state = reducer(state, reorderTabs({ sourceKey: chatA, targetKey: chatC }));
    expect(state.tabs).toEqual([chatB, chatC, chatA]);

    state = reducer(state, closeTab(chatA));
    expect(state.tabs).toEqual([chatB, chatC]);
    expect(state.activeTabId).toBe(chatC);
  });

  test("splitTab creates a group with the surface kept and an empty sibling", () => {
    const chatA = chat("a");
    let state = reducer(undefined, openTab(chatA));

    state = reducer(state, splitTab({ tabId: chatA, dir: "row" }));

    const group = groupFor(state, chatA);
    expect(selectGroupForTab(rootState(state), chatA)).toEqual(group);
    expect(selectIsTabSplit(rootState(state), chatA)).toBe(true);
    expect(selectActiveGroup(rootState(state))).toEqual(group);
    expect(group.focusedLeafId).toBe("root:sibling:chat:a");
    expect(findLeaf(group.root, "root")).toEqual(leaf("root", [chatA], chatA));
    expect(findLeaf(group.root, "root:sibling:chat:a")).toEqual(
      leaf("root:sibling:chat:a", [], null),
    );
  });

  test("closePane collapses a two-pane group back to a normal tab", () => {
    const chatA = chat("a");
    let state = reducer(undefined, openTab(chatA));
    state = reducer(state, splitTab({ tabId: chatA, dir: "row" }));

    state = reducer(
      state,
      closePane({ tabId: chatA, leafId: "root:sibling:chat:a" }),
    );

    expect(state.tabs).toEqual([chatA]);
    expect(state.groups[chatA]).toBeUndefined();
    expect(state.activeTabId).toBe(chatA);
    expect(selectVisibleSurfaceKeys(rootState(state))).toEqual([chatA]);
  });

  test("addSurfaceToPane moves a top-level tab into the target pane", () => {
    const chatA = chat("a");
    const chatB = chat("b");
    let state = reducer(undefined, openTab(chatA));
    state = reducer(state, openTab(chatB));
    state = reducer(state, setActiveTab(chatA));
    state = reducer(state, splitTab({ tabId: chatA, dir: "row" }));

    state = reducer(
      state,
      addSurfaceToPane({
        tabId: chatA,
        leafId: "root:sibling:chat:a",
        surfaceKey: chatB,
      }),
    );

    const group = groupFor(state, chatA);
    expect(state.tabs).toEqual([chatA]);
    expect(state.activeTabId).toBe(chatA);
    expect(findLeaf(group.root, "root")).toEqual(leaf("root", [chatA], chatA));
    expect(findLeaf(group.root, "root:sibling:chat:a")).toEqual(
      leaf("root:sibling:chat:a", [chatB], chatB),
    );
    expect(selectVisibleSurfaceKeys(rootState(state))).toEqual([chatA, chatB]);
    expect(selectVisibleThreadIds(rootState(state))).toEqual(["a", "b"]);
  });

  test("addSurfaceToPane collapses an emptied source pane", () => {
    const chatA = chat("a");
    const chatB = chat("b");
    const chatC = chat("c");
    let state = reducer(undefined, openTab(chatA));
    state = reducer(state, openTab(chatB));
    state = reducer(state, openTab(chatC));
    state = reducer(state, setActiveTab(chatA));
    state = reducer(state, splitTab({ tabId: chatA, dir: "row" }));
    state = reducer(
      state,
      addSurfaceToPane({
        tabId: chatA,
        leafId: "root:sibling:chat:a",
        surfaceKey: chatB,
      }),
    );
    state = reducer(state, splitTab({ tabId: chatA, dir: "row" }));
    const sourceLeafId = "root:sibling:chat:a:sibling:chat:b";
    state = reducer(
      state,
      addSurfaceToPane({
        tabId: chatA,
        leafId: sourceLeafId,
        surfaceKey: chatC,
      }),
    );

    state = reducer(
      state,
      addSurfaceToPane({ tabId: chatA, leafId: "root", surfaceKey: chatC }),
    );

    const group = groupFor(state, chatA);
    expect(findLeaf(group.root, sourceLeafId)).toBeNull();
    expect(collectLeafIds(group.root)).toEqual(["root", "root:sibling:chat:a"]);
    expect(findLeaf(group.root, "root")?.tabIds).toEqual([chatA, chatC]);
    expect(findLeaf(group.root, "root:sibling:chat:a")).toEqual(
      leaf("root:sibling:chat:a", [chatB], chatB),
    );
  });

  test("splitPaneWithSurface adds a dragged tab as a new sibling and cleans its source", () => {
    const chatA = chat("a");
    const chatB = chat("b");
    let state = reducer(undefined, openTab(chatA));
    state = reducer(state, openTab(chatB));
    state = reducer(state, setActiveTab(chatA));
    state = reducer(state, splitTab({ tabId: chatA, dir: "row" }));
    state = reducer(
      state,
      addSurfaceToPane({
        tabId: chatA,
        leafId: "root:sibling:chat:a",
        surfaceKey: chatB,
      }),
    );

    state = reducer(
      state,
      splitPaneWithSurface({
        tabId: chatA,
        leafId: "root:sibling:chat:a",
        surfaceKey: chatA,
        dir: "row",
        placement: "before",
      }),
    );

    const group = groupFor(state, chatA);
    expect(findLeaf(group.root, "root")).toBeNull();
    expect(findLeaf(group.root, "root:sibling:chat:a")).toEqual(
      leaf("root:sibling:chat:a", [chatB], chatB),
    );
    expect(findLeaf(group.root, "root:sibling:chat:a:sibling:chat:a")).toEqual(
      leaf("root:sibling:chat:a:sibling:chat:a", [chatA], chatA),
    );
    expect(group.focusedLeafId).toBe("root:sibling:chat:a:sibling:chat:a");
    expect(state.tabs).toEqual([chatA]);
  });

  test("closing a last pane with multiple surfaces ungroups into normal tabs", () => {
    const chatA = chat("a");
    const chatB = chat("b");
    const chatC = chat("c");
    let state = reducer(undefined, openTab(chatA));
    state = reducer(state, openTab(chatB));
    state = reducer(state, openTab(chatC));
    state = reducer(state, setActiveTab(chatA));
    state = reducer(state, splitTab({ tabId: chatA, dir: "row" }));
    state = reducer(
      state,
      addSurfaceToPane({
        tabId: chatA,
        leafId: "root:sibling:chat:a",
        surfaceKey: chatB,
      }),
    );
    state = reducer(
      state,
      addSurfaceToPane({ tabId: chatA, leafId: "root", surfaceKey: chatC }),
    );

    state = reducer(
      state,
      closePane({ tabId: chatA, leafId: "root:sibling:chat:a" }),
    );

    expect(state.groups[chatA]).toBeUndefined();
    expect(state.tabs).toEqual([chatA, chatC]);
    expect(state.activeTabId).toBe(chatC);
  });

  test("closePane removes dangling empty non-root leaves", () => {
    const chatA = chat("a");
    const chatB = chat("b");
    let state = reducer(undefined, openTab(chatA));
    state = reducer(state, openTab(chatB));
    state = reducer(state, setActiveTab(chatA));
    state = reducer(state, splitTab({ tabId: chatA, dir: "row" }));
    state = reducer(
      state,
      addSurfaceToPane({
        tabId: chatA,
        leafId: "root:sibling:chat:a",
        surfaceKey: chatB,
      }),
    );
    state = reducer(state, splitTab({ tabId: chatA, dir: "row" }));

    state = reducer(
      state,
      closePane({ tabId: chatA, leafId: "root:sibling:chat:a" }),
    );

    expect(state.groups[chatA]).toBeUndefined();
    expect(state.tabs).toEqual([chatA]);
    expect(state.activeTabId).toBe(chatA);
  });

  test("setPaneActive and focusPane update a split group without changing top tabs", () => {
    const chatA = chat("a");
    const chatB = chat("b");
    const chatC = chat("c");
    const leafId = "root:sibling:chat:a";
    const spareLeafId = "root:sibling:chat:a:sibling:chat:b";
    let state = reducer(undefined, openTab(chatA));
    state = reducer(state, openTab(chatB));
    state = reducer(state, openTab(chatC));
    state = reducer(state, setActiveTab(chatA));
    state = reducer(state, splitTab({ tabId: chatA, dir: "row" }));
    state = reducer(
      state,
      addSurfaceToPane({ tabId: chatA, leafId, surfaceKey: chatB }),
    );
    state = reducer(state, splitTab({ tabId: chatA, dir: "row" }));
    state = reducer(
      state,
      addSurfaceToPane({
        tabId: chatA,
        leafId: spareLeafId,
        surfaceKey: chatC,
      }),
    );
    state = reducer(state, focusPane({ tabId: chatA, leafId: "root" }));
    let group = groupFor(state, chatA);

    expect(group.focusedLeafId).toBe("root");

    state = reducer(
      state,
      addSurfaceToPane({ tabId: chatA, leafId: "root", surfaceKey: chatB }),
    );
    state = reducer(
      state,
      addSurfaceToPane({ tabId: chatA, leafId: "root", surfaceKey: chatA }),
    );

    group = groupFor(state, chatA);
    expect(findLeaf(group.root, "root")?.tabIds).toEqual([chatB, chatA]);
    expect(findLeaf(group.root, leafId)).toBeNull();
    expect(findLeaf(group.root, spareLeafId)).toEqual(
      leaf(spareLeafId, [chatC], chatC),
    );

    state = reducer(
      state,
      setPaneActive({ tabId: chatA, leafId: "root", surfaceKey: chatB }),
    );

    group = groupFor(state, chatA);
    expect(findLeaf(group.root, "root")?.activeTabId).toBe(chatB);
    expect(state.tabs).toEqual([chatA]);
  });

  test("reconcileWorkspace prunes stale chat surfaces and drops or ungroups groups", () => {
    const chatA = chat("a");
    const chatB = chat("b");
    const chatC = chat("c");
    let state = reducer(undefined, openTab(chatA));
    state = reducer(state, openTab(chatB));
    state = reducer(state, openTab(chatC));
    state = reducer(state, setActiveTab(chatA));
    state = reducer(state, splitTab({ tabId: chatA, dir: "row" }));
    state = reducer(
      state,
      addSurfaceToPane({
        tabId: chatA,
        leafId: "root:sibling:chat:a",
        surfaceKey: chatB,
      }),
    );
    state = reducer(state, splitTab({ tabId: chatC, dir: "row" }));
    state = reducer(state, setActiveTab(chatC));

    state = reducer(state, reconcileWorkspace({ openThreadIds: ["a"] }));

    expect(state.tabs).toEqual([chatA]);
    expect(state.activeTabId).toBe(chatA);
    expect(state.groups).toEqual({});
    expect(selectVisibleThreadIds(rootState(state))).toEqual(["a"]);
  });

  test("selectVisibleSurfaceKeys returns the active tab when unsplit", () => {
    const chatA = chat("a");
    const chatB = chat("b");
    let state = reducer(undefined, openTab(chatA));
    state = reducer(state, openTab(chatB));

    expect(selectVisibleSurfaceKeys(rootState(state))).toEqual([chatB]);
    expect(selectVisibleThreadIds(rootState(state))).toEqual(["b"]);
  });

  test("openTab enforces the top-level tab cap", () => {
    let state = reducer(undefined, { type: "@@INIT" });

    for (let index = 0; index < MAX_WORKSPACE_TABS + 1; index += 1) {
      state = reducer(state, openTab(chat(String(index))));
    }

    expect(state.tabs).toHaveLength(MAX_WORKSPACE_TABS);
    expect(state.tabs.at(-1)).toBe(chat(String(MAX_WORKSPACE_TABS - 1)));
    expect(state.activeTabId).toBe(chat(String(MAX_WORKSPACE_TABS - 1)));
  });

  test("splitTab enforces the group leaf cap", () => {
    const tabId = chat("0");
    let state = reducer(undefined, openTab(tabId));
    state = reducer(state, splitTab({ tabId, dir: "row" }));

    let emptyLeafId = "root:sibling:chat:0";
    for (let index = 1; index < MAX_GROUP_LEAVES; index += 1) {
      state = splitAndAdd(state, tabId, emptyLeafId, chat(String(index)));
      emptyLeafId = `${emptyLeafId}:sibling:chat:${index}`;
    }

    const group = groupFor(state, tabId);
    expect(collectLeafIds(group.root)).toHaveLength(MAX_GROUP_LEAVES);

    const leafIdsBefore = collectLeafIds(group.root);
    state = reducer(state, openTab(chat("overflow")));
    state = reducer(
      state,
      addSurfaceToPane({
        tabId,
        leafId: emptyLeafId,
        surfaceKey: chat("overflow"),
      }),
    );
    state = reducer(state, splitTab({ tabId, dir: "row" }));

    expect(collectLeafIds(groupFor(state, tabId).root)).toEqual(leafIdsBefore);
  });
});
