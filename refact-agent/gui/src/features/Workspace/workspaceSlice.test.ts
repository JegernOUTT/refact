import { describe, expect, test } from "vitest";
import {
  collectLeafIds,
  collectTabIds,
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
  hydrateWorkspace,
  openTab,
  reconcileWorkspace,
  reorderTabs,
  selectActiveGroup,
  selectActiveTabId,
  selectFocusedWorkspaceChatId,
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
  isFilesSurface,
  isGitSurface,
  isPanelSurface,
  isTerminalSurface,
  makeSurfaceKey,
  parseSurfaceKey,
  type SurfaceKey,
} from "./surfaceKey";

const reducer = workspaceSlice.reducer;

const chat = (id: string): SurfaceKey => makeSurfaceKey("chat", id);
const task = (id: string): SurfaceKey => makeSurfaceKey("task", id);
const files = makeSurfaceKey("files", "main");
const git = makeSurfaceKey("git", "main");
const terminal = makeSurfaceKey("terminal", "main");

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

const renderedSurfaces = (workspace: WorkspaceState): SurfaceKey[] =>
  workspace.tabs.flatMap((tabId) => {
    const group = workspace.groups[tabId];
    return group ? collectTabIds(group.root) : [tabId];
  });

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
    expect(files).toBe("files:main");
    expect(git).toBe("git:main");
    expect(terminal).toBe("terminal:main");
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
    expect(parseSurfaceKey(files)).toEqual({ kind: "files", id: "main" });
    expect(isPanelSurface(files)).toBe(true);
    expect(isFilesSurface(files)).toBe(true);
    expect(isGitSurface(git)).toBe(true);
    expect(isTerminalSurface(terminal)).toBe(true);
    expect(() => makeSurfaceKey("files", "other" as "main")).toThrow(
      "invalid files surface id",
    );
    expect(() => parseSurfaceKey("terminal:other")).toThrow(
      "invalid surface key",
    );
    expect(() => parseSurfaceKey("chat:")).toThrow("invalid surface key");
  });
});

describe("workspaceSlice", () => {
  test("opens, focuses, and closes singleton panel tabs", () => {
    const chatA = chat("a");
    let state = reducer(undefined, openTab(chatA));
    state = reducer(state, openTab(files));
    state = reducer(state, openTab(terminal));
    state = reducer(state, openTab(files));

    expect(state.tabs).toEqual([chatA, files, terminal]);
    expect(state.activeTabId).toBe(files);

    state = reducer(state, closeTab(files));
    expect(state.tabs).toEqual([chatA, terminal]);
    expect(state.activeTabId).toBe(terminal);

    const beforeSplit = state;
    state = reducer(state, splitTab({ tabId: terminal, dir: "row" }));
    expect(state).toEqual(beforeSplit);
  });

  test("drops disabled panel tabs while hydrating persisted workspace state", () => {
    const chatA = chat("a");
    const hydrated = reducer(
      undefined,
      hydrateWorkspace({
        tabs: [chatA, files, terminal],
        activeTabId: terminal,
        groups: {},
        panelCapabilities: {
          filesPanel: true,
          gitPanel: true,
          terminalPanel: false,
        },
      }),
    );

    expect(hydrated.tabs).toEqual([chatA, files]);
    expect(hydrated.activeTabId).toBe(chatA);
  });

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

  test("keeps task and buddy surfaces out of pane-backed workspace state", () => {
    const chatA = chat("a");
    const taskA = task("a");
    const buddyHome = makeSurfaceKey("buddy", "home");
    let state = reducer(undefined, openTab(taskA));
    state = reducer(state, openTab(buddyHome));

    expect(state.tabs).toEqual([]);
    expect(state.activeTabId).toBeNull();

    state = reducer(state, openTab(chatA));
    state = reducer(state, splitTab({ tabId: taskA, dir: "row" }));
    state = reducer(state, splitTab({ tabId: chatA, dir: "row" }));
    const leafIdsBefore = collectLeafIds(groupFor(state, chatA).root);

    state = reducer(
      state,
      addSurfaceToPane({
        tabId: chatA,
        leafId: "root:sibling:chat:a",
        surfaceKey: taskA,
      }),
    );
    state = reducer(
      state,
      splitPaneWithSurface({
        tabId: chatA,
        leafId: "root:sibling:chat:a",
        surfaceKey: buddyHome,
        dir: "row",
      }),
    );

    expect(state.tabs).toEqual([chatA]);
    expect(collectLeafIds(groupFor(state, chatA).root)).toEqual(leafIdsBefore);
    expect(collectTabIds(groupFor(state, chatA).root)).toEqual([chatA]);
  });

  test("hydrate and reconcile defensively drop non-chat pane state", () => {
    const chatA = chat("a");
    const taskA = task("a");
    const buddyHome = makeSurfaceKey("buddy", "home");
    const hydrated = reducer(
      undefined,
      hydrateWorkspace({
        tabs: [chatA, taskA, buddyHome],
        activeTabId: taskA,
        groups: {
          [chatA]: {
            focusedLeafId: "right",
            root: {
              kind: "split",
              id: "root:split:row",
              dir: "row",
              sizes: [0.5, 0.5],
              children: [
                leaf("left", [chatA], chatA),
                leaf("right", [taskA], taskA),
              ],
            },
          },
        },
      }),
    );

    expect(hydrated.tabs).toEqual([chatA]);
    expect(hydrated.activeTabId).toBe(chatA);
    expect(hydrated.groups).toEqual({});

    const reconciled = reducer(
      {
        tabs: [chatA, taskA],
        activeTabId: taskA,
        groups: {
          [taskA]: {
            focusedLeafId: "root",
            root: leaf("root", [taskA], taskA),
          },
        },
      },
      reconcileWorkspace({ openThreadIds: ["a"] }),
    );

    expect(reconciled).toEqual({
      tabs: [chatA],
      activeTabId: chatA,
      groups: {},
    });
  });

  test("hydrateWorkspace drops duplicate surface groups back to unsplit tabs", () => {
    const chatA = chat("a");
    const chatB = chat("b");
    const chatC = chat("c");
    const chatD = chat("d");
    const chatE = chat("e");

    const hydrated = reducer(
      undefined,
      hydrateWorkspace({
        tabs: [chatA, chatB, chatC, chatD],
        activeTabId: chatA,
        groups: {
          [chatA]: {
            focusedLeafId: "left",
            root: {
              kind: "split",
              id: "duplicate:split",
              dir: "row",
              sizes: [0.5, 0.5],
              children: [
                leaf("left", [chatA], chatA),
                leaf("right", [chatA], chatA),
              ],
            },
          },
          [chatB]: {
            focusedLeafId: "left",
            root: {
              kind: "split",
              id: "top-level-collision:split",
              dir: "row",
              sizes: [0.5, 0.5],
              children: [
                leaf("left", [chatB], chatB),
                leaf("right", [chatC], chatC),
              ],
            },
          },
          [chatD]: {
            focusedLeafId: "right",
            root: {
              kind: "split",
              id: "valid:split",
              dir: "row",
              sizes: [0.5, 0.5],
              children: [
                leaf("left", [chatD], chatD),
                leaf("right", [chatE], chatE),
              ],
            },
          },
        },
      }),
    );

    expect(hydrated.tabs).toEqual([chatA, chatB, chatC, chatD]);
    expect(hydrated.groups[chatA]).toBeUndefined();
    expect(hydrated.groups[chatB]).toBeUndefined();
    expect(hydrated.groups[chatD]).toBeDefined();
    expect(renderedSurfaces(hydrated)).toEqual([
      chatA,
      chatB,
      chatC,
      chatD,
      chatE,
    ]);
    expect(new Set(renderedSurfaces(hydrated)).size).toBe(
      renderedSurfaces(hydrated).length,
    );
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

  test("splitTab can move a dragged top-level chat into the new sibling", () => {
    const chatA = chat("a");
    const chatB = chat("b");
    let state = reducer(undefined, openTab(chatA));
    state = reducer(state, openTab(chatB));
    state = reducer(state, setActiveTab(chatA));

    state = reducer(
      state,
      splitTab({ tabId: chatA, dir: "row", surfaceKey: chatB }),
    );

    const group = groupFor(state, chatA);
    expect(state.tabs).toEqual([chatA]);
    expect(state.activeTabId).toBe(chatA);
    expect(findLeaf(group.root, "root")).toEqual(leaf("root", [chatA], chatA));
    expect(findLeaf(group.root, "root:sibling:chat:a")).toEqual(
      leaf("root:sibling:chat:a", [chatB], chatB),
    );
    expect(group.focusedLeafId).toBe("root:sibling:chat:a");
    expect(selectVisibleSurfaceKeys(rootState(state))).toEqual([chatA, chatB]);
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

  test("focusPane is a no-op for the already focused leaf", () => {
    const chatA = chat("a");
    let state = reducer(undefined, openTab(chatA));
    state = reducer(state, splitTab({ tabId: chatA, dir: "row" }));

    const next = reducer(
      state,
      focusPane({
        tabId: chatA,
        leafId: groupFor(state, chatA).focusedLeafId,
      }),
    );

    expect(next).toBe(state);
    expect(groupFor(next, chatA).focusedLeafId).toBe(
      groupFor(state, chatA).focusedLeafId,
    );
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

  test("workspace selectors expose the active group's visible and focused chats", () => {
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

    expect(selectVisibleThreadIds(rootState(state))).toEqual(["a", "b"]);
    expect(selectFocusedWorkspaceChatId(rootState(state))).toBe("b");
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
