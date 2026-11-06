import { describe, expect, test } from "vitest";
import {
  collectLeafIds,
  collectTabIds,
  findLeaf,
  type LeafPane,
} from "../ChatPanes/panesTree";
import {
  addSurfaceToPane,
  bindSurfaceToChat,
  clearWorkspaceChatState,
  closePane,
  closePaneTab,
  reorderPaneTabs,
  closeTab,
  ensureLiveEditSplit,
  focusPane,
  MAX_GROUP_LEAVES,
  MAX_WORKSPACE_TABS,
  hydrateWorkspace,
  openTab,
  reconcileWorkspace,
  selectActiveGroup,
  selectActiveTabId,
  selectFocusedChatWorkspaceRoot,
  selectFocusedWorkspaceChatId,
  selectFocusedWorkspaceSurfaceKey,
  selectGroupForTab,
  selectSurfaceChatId,
  selectWorkspaceContextChatIds,
  selectWorkspaceFocusedChatId,
  selectIsTabSplit,
  selectLiveEditsForChat,
  selectWorkspaceDock,
  selectWorkspaceDrawer,
  selectVisibleSurfaceKeys,
  selectVisibleThreadIds,
  selectPanelsForced,
  setActiveTab,
  setDockOpen,
  setDockSection,
  normalizeWorkspaceDock,
  setDockWidth,
  setDrawerHeight,
  setDrawerOpen,
  setPanelsForced,
  setLiveEditsForChat,
  setPaneActive,
  splitPaneWithSurface,
  splitTab,
  workspaceSlice,
  type PaneGroup,
  type WorkspaceState,
} from "./workspaceSlice";
import {
  isChatSurface,
  isDesignSurface,
  isFilesSurface,
  isGitSurface,
  isMainSurface,
  isTerminalSurface,
  makeSurfaceKey,
  makeGitDiffSurfaceKey,
  parseSurfaceKey,
  type SurfaceKey,
} from "./surfaceKey";

const reducer = workspaceSlice.reducer;

const chat = (id: string): SurfaceKey => makeSurfaceKey("chat", id);
const files = "files:main";
const git = makeSurfaceKey("git", "main");
const terminal = "terminal:main";

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

const contextState = (
  workspace: WorkspaceState,
  worktrees: Record<
    string,
    { root: string; source_workspace_root: string } | null
  >,
) => ({
  workspace,
  chat: {
    threads: Object.fromEntries(
      Object.entries(worktrees).map(([id, worktree]) => [
        id,
        { thread: { worktree } },
      ]),
    ),
  },
  current_project: { workspaceRoots: ["/project", "/secondary"] },
});

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
    expect(makeSurfaceKey("design", "surface-a")).toBe("design:surface-a");
    expect(makeSurfaceKey("dashboard")).toBe("dashboard");
    expect(files).toBe("files:main");
    expect(git).toBe("git:main");
    expect(terminal).toBe("terminal:main");
    expect(parseSurfaceKey("chat:thread-a")).toEqual({
      kind: "chat",
      id: "thread-a",
    });
    expect(parseSurfaceKey("design:surface-a")).toEqual({
      kind: "design",
      id: "surface-a",
    });
    expect(parseSurfaceKey("dashboard")).toEqual({
      kind: "dashboard",
      id: null,
    });
    expect(isChatSurface("chat:thread-a")).toBe(true);
    expect(isChatSurface("task:task-a")).toBe(false);
    expect(isDesignSurface("design:surface-a")).toBe(true);
    expect(() => parseSurfaceKey(files)).toThrow("invalid surface key");
    expect(isMainSurface(files)).toBe(false);
    expect(isFilesSurface(files)).toBe(true);
    expect(isGitSurface(git)).toBe(true);
    expect(isTerminalSurface(terminal)).toBe(true);
    expect(() => parseSurfaceKey("terminal:other")).toThrow(
      "invalid surface key",
    );
    expect(() => parseSurfaceKey("chat:")).toThrow("invalid surface key");
  });
});

describe("workspaceSlice", () => {
  test("stores the explicit workspace panels opt-in", () => {
    let state = reducer(undefined, setPanelsForced(true));
    expect(selectPanelsForced(rootState(state))).toBe(true);

    state = reducer(state, setPanelsForced(false));
    expect(selectPanelsForced(rootState(state))).toBe(false);
  });

  test("updates normalized dock and drawer layout state", () => {
    let state = reducer(undefined, setDockOpen(false));
    state = reducer(state, setDockWidth(1000));
    state = reducer(state, setDockSection("activity"));
    state = reducer(state, setDrawerOpen(true));
    state = reducer(state, setDrawerHeight(40));

    expect(selectWorkspaceDock(rootState(state))).toEqual({
      open: false,
      width: 640,
      section: "activity",
    });
    expect(selectWorkspaceDrawer(rootState(state))).toEqual({
      open: true,
      height: 160,
    });
  });

  test("migrates the legacy saved tasks dock section to active chats", () => {
    expect(
      normalizeWorkspaceDock({ open: true, width: 300, section: "tasks" }),
    ).toEqual({ open: true, width: 300, section: "activity" });
    expect(normalizeWorkspaceDock({ section: "activity" }).section).toBe(
      "activity",
    );
    expect(normalizeWorkspaceDock({ section: "bogus" }).section).toBe("files");
  });

  test("migrates a legacy Terminal center tab into the drawer", () => {
    const chatA = chat("a");
    const state = reducer(
      undefined,
      hydrateWorkspace({
        tabs: [chatA, terminal],
        activeTabId: terminal,
        groups: {},
      }),
    );

    expect(state.tabs).toEqual([chatA]);
    expect(state.activeTabId).toBe(chatA);
    expect(state.drawer).toEqual({ open: true, height: 220 });
  });

  test("drops legacy Files and Terminal surfaces while keeping file viewers", () => {
    const chatA = chat("a");
    const file = makeSurfaceKey("file", "/workspace/src/main.ts");
    let state = reducer(undefined, openTab(chatA));
    state = reducer(state, openTab(files));
    state = reducer(state, openTab(terminal));
    state = reducer(state, openTab(file));

    expect(renderedSurfaces(state)).toEqual([chatA, file]);
    expect(selectFocusedWorkspaceSurfaceKey(rootState(state))).toBe(file);

    state = reducer(state, closeTab(file));
    expect(state.tabs).toEqual([chatA]);
    expect(state.activeTabId).toBe(chatA);

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
        workspaceCapabilities: {
          filesPanel: true,
          gitPanel: true,
          terminalPanel: false,
        },
      }),
    );

    expect(hydrated.tabs).toEqual([chatA]);
    expect(hydrated.activeTabId).toBe(chatA);
  });

  test("opens independent design surfaces and binds chat ownership only explicitly", () => {
    let state = reducer(undefined, openTab(chat("a")));
    state = reducer(state, openTab("design:a"));
    expect(selectWorkspaceFocusedChatId({ workspace: state })).toBeNull();
    expect(state.contextChatByTab?.["design:a"]).toBeUndefined();
    state = reducer(
      state,
      bindSurfaceToChat({ surfaceKey: "design:a", chatId: "a" }),
    );
    expect(renderedSurfaces(state)).toEqual([chat("a"), "design:a"]);
    expect(selectWorkspaceFocusedChatId({ workspace: state })).toBe("a");
    expect(state.contextChatByTab?.["design:a"]).toBe("a");
    const restored = reducer(
      undefined,
      hydrateWorkspace({ ...state, openThreadIds: [] }),
    );
    expect(renderedSurfaces(restored)).toEqual(["design:a"]);
  });

  test("opens, activates, reorders, and closes tabs", () => {
    const chatA = chat("a");
    const chatB = chat("b");
    const chatC = chat("c");
    let state = reducer(undefined, openTab(chatA));
    state = reducer(state, openTab(chatB));
    state = reducer(state, openTab(chatB));
    state = reducer(state, openTab(chatC));

    expect(state.tabs).toEqual([chatA]);
    expect(renderedSurfaces(state)).toEqual([chatA, chatB, chatC]);
    expect(selectFocusedWorkspaceSurfaceKey(rootState(state))).toBe(chatC);

    state = reducer(state, setActiveTab(chatA));
    expect(selectActiveTabId(rootState(state))).toBe(chatA);

    state = reducer(
      state,
      reorderPaneTabs({
        tabId: chatA,
        leafId: "root",
        fromIndex: 0,
        toIndex: 2,
      }),
    );
    expect(renderedSurfaces(state)).toEqual([chatB, chatC, chatA]);

    state = reducer(state, closeTab(chatA));
    expect(renderedSurfaces(state)).toEqual([chatB, chatC]);
    expect(selectFocusedWorkspaceSurfaceKey(rootState(state))).toBe(chatB);
  });

  test("keeps task and buddy panes through hydration and chat pruning", () => {
    let state = reducer(undefined, openTab("task:a"));
    state = reducer(state, openTab("buddy:home"));
    state = reducer(state, openTab(chat("a")));
    state = reducer(
      undefined,
      hydrateWorkspace({ ...state, openThreadIds: [] }),
    );
    expect(renderedSurfaces(state)).toEqual(["task:a", "buddy:home"]);
    expect(state.tabs).toEqual(["task:a"]);
    expect(selectWorkspaceFocusedChatId({ workspace: state })).toBeNull();
  });

  test("hydrateWorkspace merges legacy groups into one unique pane tree", () => {
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

    expect(hydrated.tabs).toEqual([chatA]);
    expect(Object.keys(hydrated.groups)).toEqual([chatA]);
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
    expect(state.groups[chatA]?.root).toEqual(leaf("root", [chatA], chatA));
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

  test("closing a last split preserves local tabs in the remaining pane", () => {
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

    expect(state.groups[chatA]?.root).toEqual(
      leaf("root", [chatA, chatC], chatC),
    );
    expect(state.tabs).toEqual([chatA]);
    expect(state.activeTabId).toBe(chatA);
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

    expect(state.groups[chatA]?.root).toEqual(leaf("root", [chatA], chatA));
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

  test("live edits default off everywhere and the last choice sticks for new chats", () => {
    const state = reducer(undefined, openTab(chat("a")));
    expect(
      selectLiveEditsForChat(
        { ...contextState(state, { a: null }), config: { host: "web" } },
        "a",
      ),
    ).toBe(false);
    expect(
      selectLiveEditsForChat(
        { ...contextState(state, { a: null }), config: { host: "vscode" } },
        "a",
      ),
    ).toBe(false);

    const enabled = reducer(
      state,
      setLiveEditsForChat({ chatId: "a", enabled: true }),
    );
    expect(
      selectLiveEditsForChat(
        { ...contextState(enabled, { a: null }), config: { host: "web" } },
        "a",
      ),
    ).toBe(true);
    // Sticky default: chats without an explicit entry inherit the last choice.
    expect(
      selectLiveEditsForChat(
        { ...contextState(enabled, { a: null }), config: { host: "web" } },
        "b",
      ),
    ).toBe(true);

    const disabled = reducer(
      enabled,
      setLiveEditsForChat({ chatId: "a", enabled: false }),
    );
    expect(
      selectLiveEditsForChat(
        { ...contextState(disabled, { a: null }), config: { host: "web" } },
        "a",
      ),
    ).toBe(false);
    expect(
      selectLiveEditsForChat(
        { ...contextState(disabled, { a: null }), config: { host: "web" } },
        "b",
      ),
    ).toBe(false);
  });

  test("closes an active top-level contextual file with its chat state", () => {
    const fileA = makeSurfaceKey("file", "/worktrees/a/main.ts");
    const chatA = chat("a");
    const chatB = chat("b");
    const state: WorkspaceState = {
      tabs: [chatA, fileA, chatB],
      activeTabId: fileA,
      groups: {},
      liveEditsByChat: { a: true, b: false },
      contextChatByTab: { [fileA]: "a" },
    };

    const next = reducer(state, clearWorkspaceChatState("a"));

    expect(next.tabs).toEqual([chatA]);
    expect(renderedSurfaces(next)).toEqual([chatA, chatB]);
    expect(selectFocusedWorkspaceSurfaceKey(rootState(next))).toBe(chatA);
    expect(next.liveEditsByChat).toEqual({ b: false });
    expect(next.contextChatByTab).toBeUndefined();
  });

  test("collapses a split after closing its contextual file surface", () => {
    const chatA = chat("a");
    const fileA = makeSurfaceKey("file", "/worktrees/a/main.ts");
    const state: WorkspaceState = {
      tabs: [chatA],
      activeTabId: chatA,
      groups: {
        [chatA]: {
          focusedLeafId: "file",
          root: {
            kind: "split",
            id: "root:split:row",
            dir: "row",
            sizes: [0.5, 0.5],
            children: [
              leaf("chat", [chatA], chatA),
              leaf("file", [fileA], fileA),
            ],
          },
        },
      },
      contextChatByTab: { [chatA]: "a" },
    };

    const next = reducer(state, clearWorkspaceChatState("a"));

    expect(next.tabs).toEqual([chatA]);
    expect(collectTabIds(groupFor(next, chatA).root)).toEqual([chatA]);
    expect(groupFor(next, chatA).root.kind).toBe("leaf");
    expect(next.activeTabId).toBe(chatA);
    expect(next.contextChatByTab).toBeUndefined();
  });

  test("closing one chat preserves surfaces owned by another chat", () => {
    const chatA = chat("a");
    const chatB = chat("b");
    const fileA = makeSurfaceKey("file", "/worktrees/a/main.ts");
    const fileB = makeSurfaceKey("file", "/worktrees/b/main.ts");
    const state: WorkspaceState = {
      tabs: [chatA, fileA, chatB, fileB],
      activeTabId: fileB,
      groups: {},
      liveEditsByChat: { a: true, b: false },
      contextChatByTab: { [fileA]: "a", [fileB]: "b" },
    };

    const next = reducer(state, clearWorkspaceChatState("a"));

    expect(next.tabs).toEqual([chatA]);
    expect(renderedSurfaces(next)).toEqual([fileB, chatA, chatB]);
    expect(selectFocusedWorkspaceSurfaceKey(rootState(next))).toBe(fileB);
    expect(next.liveEditsByChat).toEqual({ b: false });
    expect(next.contextChatByTab?.[fileB]).toBe("b");
    expect(next.contextChatByTab?.[fileA]).toBeUndefined();
  });

  test("live edit split reuses one horizontal file pane and follows paths", () => {
    const chatA = chat("a");
    let state = reducer(undefined, openTab(chatA));
    state = reducer(
      state,
      ensureLiveEditSplit({ chatId: "a", filePath: "/workspace/one.ts" }),
    );
    const first = groupFor(state, chatA);
    expect(first.root.kind).toBe("split");
    if (first.root.kind !== "split") throw new Error("missing live split");
    expect(first.root.dir).toBe("row");
    expect(collectLeafIds(first.root)).toHaveLength(2);
    expect(collectTabIds(first.root)).toEqual([
      chatA,
      "file:/workspace/one.ts",
    ]);
    expect(findLeaf(first.root, first.focusedLeafId)?.activeTabId).toBe(
      "file:/workspace/one.ts",
    );

    state = reducer(
      state,
      ensureLiveEditSplit({ chatId: "a", filePath: "/workspace/two.ts" }),
    );
    const second = groupFor(state, chatA);
    expect(collectLeafIds(second.root)).toHaveLength(2);
    expect(collectTabIds(second.root)).toEqual([
      chatA,
      "file:/workspace/one.ts",
      "file:/workspace/two.ts",
    ]);
    expect(findLeaf(second.root, second.focusedLeafId)?.activeTabId).toBe(
      "file:/workspace/two.ts",
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
    expect(collectTabIds(groupFor(state, chatA).root)).toEqual([chatA]);
    expect(groupFor(state, chatA).root.kind).toBe("leaf");
    expect(selectVisibleThreadIds(rootState(state))).toEqual(["a"]);
  });

  test("reconcileWorkspace closes files whose context chat is no longer open", () => {
    const chatA = chat("a");
    const fileA = makeSurfaceKey("file", "/worktrees/a/main.ts");
    const state: WorkspaceState = {
      tabs: [chatA, fileA],
      activeTabId: fileA,
      groups: {},
      contextChatByTab: { [fileA]: "a" },
    };

    const next = reducer(state, reconcileWorkspace({ openThreadIds: [] }));

    expect(next.tabs).toEqual([]);
    expect(next.activeTabId).toBeNull();
    expect(next.groups).toEqual({});
    expect(next.contextChatByTab).toBeUndefined();
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

  test("top-level file focus retains its originating chat context", () => {
    const chatA = chat("a");
    const file = makeSurfaceKey("file", "/worktrees/a/src/main.ts");
    let state = reducer(undefined, openTab(chatA));
    state = reducer(state, openTab(file));

    expect(state.contextChatByTab?.[file]).toBe("a");
    expect(selectFocusedWorkspaceChatId(rootState(state))).toBe("a");

    state = reducer(state, closeTab(file));
    expect(state.contextChatByTab?.[file]).toBeUndefined();
    expect(selectFocusedWorkspaceChatId(rootState(state))).toBe("a");
  });

  test("focused files in a multi-chat group retain the most recently associated chat", () => {
    const chatA = chat("a");
    const chatB = chat("b");
    const file = makeSurfaceKey("file", "/worktrees/b/src/main.ts");
    let state = reducer(undefined, openTab(chatA));
    state = reducer(state, splitTab({ tabId: chatA, dir: "row" }));
    state = reducer(state, openTab(chatB));
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
      ensureLiveEditSplit({
        chatId: "b",
        filePath: "/worktrees/b/src/main.ts",
      }),
    );
    state = reducer(
      state,
      bindSurfaceToChat({ surfaceKey: file, chatId: "b" }),
    );

    expect(state.contextChatByTab).toEqual({ [chatA]: "b", [file]: "b" });
    expect(selectFocusedWorkspaceChatId(rootState(state))).toBe("b");
    expect(
      selectFocusedChatWorkspaceRoot(
        contextState(state, {
          a: { root: "/worktrees/a", source_workspace_root: "/project" },
          b: { root: "/worktrees/b", source_workspace_root: "/project" },
        }),
      ),
    ).toBe("/worktrees/b");
  });

  test("hydrateWorkspace prunes stale context mappings", () => {
    const chatA = chat("a");
    const file = makeSurfaceKey("file", "/project/src/main.ts");
    const state = reducer(
      undefined,
      hydrateWorkspace({
        tabs: [chatA, file],
        activeTabId: file,
        groups: {},
        contextChatByTab: { [file]: "a", missing: "b" },
      }),
    );

    expect(state.contextChatByTab?.[file]).toBe("a");
    expect(state.contextChatByTab?.missing).toBeUndefined();
    expect(selectFocusedWorkspaceChatId(rootState(state))).toBe("a");
  });

  test("hydrateWorkspace drops contextual files owned by closed chats", () => {
    const chatA = chat("a");
    const fileA = makeSurfaceKey("file", "/worktrees/a/src/main.ts");
    const state = reducer(
      undefined,
      hydrateWorkspace({
        tabs: [chatA],
        activeTabId: chatA,
        groups: {
          [chatA]: {
            focusedLeafId: "file",
            root: {
              kind: "split",
              id: "root:split:row",
              dir: "row",
              sizes: [0.5, 0.5],
              children: [
                leaf("chat", [chatA], chatA),
                leaf("file", [fileA], fileA),
              ],
            },
          },
        },
        contextChatByTab: { [chatA]: "b" },
        openThreadIds: ["a"],
      }),
    );

    expect(state.tabs).toEqual([chatA]);
    expect(state.activeTabId).toBe(chatA);
    expect(collectTabIds(groupFor(state, chatA).root)).toEqual([chatA]);
    expect(groupFor(state, chatA).root.kind).toBe("leaf");
    expect(state.contextChatByTab).toBeUndefined();
  });

  test("focused chat workspace root follows worktrees and falls back to the project root", () => {
    const chatA = chat("a");
    const chatB = chat("b");
    let state = reducer(undefined, openTab(chatA));
    state = reducer(state, openTab(chatB));

    const roots = {
      a: { root: "/worktrees/a", source_workspace_root: "/project" },
      b: null,
    };

    expect(selectFocusedChatWorkspaceRoot(contextState(state, roots))).toBe(
      "/project",
    );

    state = reducer(state, setActiveTab(chatA));
    expect(selectFocusedChatWorkspaceRoot(contextState(state, roots))).toBe(
      "/worktrees/a",
    );

    state = reducer(state, setActiveTab(chatB));
    expect(selectFocusedChatWorkspaceRoot(contextState(state, roots))).toBe(
      "/project",
    );
  });

  test("openTab enforces file admission limits without dropping open chats", () => {
    let state = reducer(undefined, { type: "@@INIT" });
    for (let index = 0; index <= MAX_WORKSPACE_TABS; index += 1) {
      state = reducer(state, openTab(makeSurfaceKey("file", `/file-${index}`)));
    }
    expect(state.tabs).toEqual(["file:/file-0"]);
    expect(renderedSurfaces(state)).toEqual(
      Array.from(
        { length: MAX_WORKSPACE_TABS },
        (_, index) => `file:/file-${index}`,
      ),
    );
    expect(selectFocusedWorkspaceSurfaceKey(rootState(state))).toBe(
      `file:/file-${MAX_WORKSPACE_TABS - 1}`,
    );
    for (let index = 0; index < 50; index += 1) {
      state = reducer(state, openTab(chat(String(index))));
    }
    expect(renderedSurfaces(state)).toEqual([
      ...Array.from(
        { length: MAX_WORKSPACE_TABS },
        (_, index) => `file:/file-${index}`,
      ),
      ...Array.from({ length: 50 }, (_, index) => chat(String(index))),
    ]);
    state = reducer(state, openTab(chat("25")));
    expect(renderedSurfaces(state)).toHaveLength(MAX_WORKSPACE_TABS + 50);
    expect(selectFocusedWorkspaceSurfaceKey(rootState(state))).toBe(chat("25"));
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

describe("task-aware workspace context", () => {
  const fileKey = makeSurfaceKey("file", "/repo/a.ts");
  const workspace = (): WorkspaceState => {
    let state = reducer(undefined, openTab(makeSurfaceKey("chat", "chat-a")));
    state = reducer(state, openTab(fileKey));
    return reducer(
      state,
      bindSurfaceToChat({ surfaceKey: fileKey, chatId: "agent-1" }),
    );
  };
  const tasksUI = {
    openTasks: [
      {
        id: "task-1",
        activeChat: { chatId: "planner-1" },
        plannerChats: [{ id: "planner-1" }],
      },
    ],
  };

  test("uses the active task chat only on task pages", () => {
    const base = { workspace: workspace(), tasksUI };
    expect(
      selectFocusedWorkspaceChatId({ ...base, pages: [{ name: "chat" }] }),
    ).toBe("agent-1");
    expect(
      selectFocusedWorkspaceChatId({
        ...base,
        pages: [{ name: "task workspace", taskId: "task-1" }],
      }),
    ).toBe("planner-1");
    expect(
      selectWorkspaceFocusedChatId({
        ...base,
        pages: [{ name: "task workspace", taskId: "task-1" }],
      }),
    ).toBe("agent-1");
    expect(
      selectFocusedWorkspaceChatId({
        ...base,
        pages: [{ name: "task workspace", taskId: "missing" }],
      }),
    ).toBeNull();
  });

  test("collects open task chats including card agents from thread metadata", () => {
    expect(
      selectWorkspaceContextChatIds({
        workspace: workspace(),
        tasksUI,
        chat: {
          threads: {
            "agent-1": { thread: { task_meta: { task_id: "task-1" } } },
            "other-agent": { thread: { task_meta: { task_id: "task-2" } } },
            "chat-a": { thread: {} },
          },
        },
      }).sort(),
    ).toEqual(["agent-1", "planner-1"]);
  });

  test("reconcile keeps surfaces owned by task context chats", () => {
    const state = workspace();
    const dropped = reducer(
      state,
      reconcileWorkspace({ openThreadIds: ["chat-a"] }),
    );
    expect(renderedSurfaces(dropped)).toEqual([chat("chat-a")]);

    const kept = reducer(
      state,
      reconcileWorkspace({
        openThreadIds: ["chat-a"],
        contextChatIds: ["agent-1"],
      }),
    );
    expect(renderedSurfaces(kept)).toEqual([chat("chat-a"), fileKey]);
    expect(selectSurfaceChatId({ workspace: kept }, fileKey)).toBe("agent-1");
  });
});

describe("file split groups", () => {
  const fileA = "file:/project/a.ts";
  const fileB = "file:/project/b.ts";

  test("hydrates and reconciles file-rooted mixed and file-only groups", () => {
    for (const other of [chat("a"), fileB]) {
      const group: PaneGroup = {
        focusedLeafId: "other",
        root: {
          kind: "split",
          id: "split",
          dir: "row",
          sizes: [0.4, 0.6],
          children: [leaf("file", [fileA]), leaf("other", [other])],
        },
      };
      const state = reducer(
        undefined,
        hydrateWorkspace({
          tabs: [fileA],
          activeTabId: fileA,
          groups: { [fileA]: group },
          openThreadIds: other === chat("a") ? ["a"] : [],
        }),
      );
      expect(state.groups[other === chat("a") ? other : fileA]).toEqual(group);
      expect(selectVisibleSurfaceKeys({ workspace: state })).toEqual([
        fileA,
        other,
      ]);
      expect(
        reducer(state, reconcileWorkspace({ openThreadIds: ["a"] })),
      ).toEqual(state);
    }
  });

  test("live edits preserve the file-rooted split across hydration and reconciliation", () => {
    let state = reducer(undefined, openTab(fileA));
    state = reducer(
      state,
      splitTab({ tabId: fileA, surfaceKey: chat("a"), dir: "row" }),
    );
    state = reducer(
      state,
      ensureLiveEditSplit({ chatId: "a", filePath: "/project/b.ts" }),
    );

    expect(state.tabs).toEqual([fileA]);
    expect(collectTabIds(groupFor(state, fileA).root)).toEqual([
      fileA,
      fileB,
      chat("a"),
    ]);
    expect(selectFocusedWorkspaceSurfaceKey({ workspace: state })).toBe(fileB);
    expect(state.contextChatByTab).toEqual({ [fileA]: "a", [fileB]: "a" });
    const hydrated = reducer(
      undefined,
      hydrateWorkspace({ ...state, openThreadIds: ["a"] }),
    );
    expect(groupFor(hydrated, chat("a")).root).toEqual(
      groupFor(state, fileA).root,
    );
    const reconciled = reducer(
      state,
      reconcileWorkspace({ openThreadIds: ["a"] }),
    );
    expect(reconciled.tabs).toEqual([chat("a")]);
    expect(groupFor(reconciled, chat("a")).root).toEqual(
      groupFor(state, fileA).root,
    );
    expect(selectFocusedWorkspaceSurfaceKey({ workspace: reconciled })).toBe(
      fileB,
    );
    expect(reconciled.contextChatByTab?.[fileA]).toBe("a");
    expect(reconciled.contextChatByTab?.[fileB]).toBe("a");
    expect(selectFocusedWorkspaceSurfaceKey(rootState(reconciled))).toBe(fileB);
  });

  test.each(["clear", "reconcile", "hydrate"] as const)(
    "%s chat cleanup preserves unrelated chats in a file-rooted group",
    (operation) => {
      let state = reducer(undefined, openTab(fileA));
      state = reducer(
        state,
        splitTab({ tabId: fileA, surfaceKey: chat("a"), dir: "row" }),
      );
      state = reducer(
        state,
        splitTab({ tabId: fileA, surfaceKey: chat("b"), dir: "col" }),
      );
      state = reducer(
        state,
        bindSurfaceToChat({ surfaceKey: fileA, chatId: "a" }),
      );
      expect(state.contextChatByTab).toEqual({ [fileA]: "a" });
      const next =
        operation === "clear"
          ? reducer(state, clearWorkspaceChatState("a"))
          : operation === "reconcile"
            ? reducer(state, reconcileWorkspace({ openThreadIds: ["b"] }))
            : reducer(
                undefined,
                hydrateWorkspace({ ...state, openThreadIds: ["b"] }),
              );

      expect(renderedSurfaces(next)).toEqual(
        operation === "clear" ? [chat("a"), chat("b")] : [chat("b")],
      );
      expect(next.tabs).not.toContain(fileA);
      expect(next.activeTabId).toBe(chat(operation === "clear" ? "a" : "b"));
      expect(next.contextChatByTab?.[fileA]).toBeUndefined();
      expect(Object.values(next.contextChatByTab ?? {})).not.toContain("a");
    },
  );

  test.each(["reconcile", "hydrate"] as const)(
    "%s removes empty file groups and keeps chats when files are disabled",
    (operation) => {
      let state = reducer(undefined, openTab(fileA));
      state = reducer(
        state,
        splitTab({ tabId: fileA, surfaceKey: fileB, dir: "row" }),
      );
      const workspaceCapabilities = {
        filesPanel: false,
        gitPanel: true,
        terminalPanel: true,
      };
      const empty =
        operation === "reconcile"
          ? reducer(
              state,
              reconcileWorkspace({ openThreadIds: [], workspaceCapabilities }),
            )
          : reducer(
              undefined,
              hydrateWorkspace({
                ...state,
                openThreadIds: [],
                workspaceCapabilities,
              }),
            );
      expect(empty.tabs).toEqual([]);
      expect(empty.groups).toEqual({});
      expect(empty.activeTabId).toBeNull();

      state = reducer(
        state,
        addSurfaceToPane({
          tabId: fileA,
          leafId: "root",
          surfaceKey: chat("a"),
        }),
      );
      const withChat =
        operation === "reconcile"
          ? reducer(
              state,
              reconcileWorkspace({
                openThreadIds: ["a"],
                workspaceCapabilities,
              }),
            )
          : reducer(
              undefined,
              hydrateWorkspace({
                ...state,
                openThreadIds: ["a"],
                workspaceCapabilities,
              }),
            );
      expect(withChat.tabs).toEqual([chat("a")]);
      expect(groupFor(withChat, chat("a")).root.kind).toBe("leaf");
      expect(renderedSurfaces(withChat)).toEqual([chat("a")]);
      expect(withChat.activeTabId).toBe(chat("a"));
    },
  );

  test("closing the canonical file surface preserves the other pane", () => {
    let state = reducer(undefined, openTab(fileA));
    state = reducer(
      state,
      splitTab({ tabId: fileA, surfaceKey: chat("a"), dir: "row" }),
    );
    state = reducer(state, closeTab(fileA));

    expect(state.tabs).toEqual([chat("a")]);
    expect(renderedSurfaces(state)).toEqual([chat("a")]);
    expect(groupFor(state, chat("a")).root.kind).toBe("leaf");
    expect(state.activeTabId).toBe(chat("a"));
  });

  test("splits files, focuses deduplicated opens and closes the file pane cleanly", () => {
    let state = reducer(undefined, openTab(fileA));
    state = reducer(state, openTab(fileB));
    state = reducer(state, setActiveTab(fileA));
    state = reducer(
      state,
      splitTab({ tabId: fileA, surfaceKey: fileB, dir: "row" }),
    );
    const group = state.groups[fileA];
    if (!group) throw new Error("missing file group");
    expect(state.tabs).toEqual([fileA]);
    expect(collectTabIds(group.root)).toEqual([fileA, fileB]);
    state = reducer(state, openTab(fileB));
    expect(state.tabs).toEqual([fileA]);
    expect(selectFocusedWorkspaceSurfaceKey({ workspace: state })).toBe(fileB);
    state = reducer(
      state,
      closePane({ tabId: fileA, leafId: group.focusedLeafId }),
    );
    expect(groupFor(state, fileA).root).toEqual(leaf("root", [fileA], fileA));
    expect(state.tabs).toEqual([fileA]);
    expect(state.activeTabId).toBe(fileA);
  });

  test("closing an interior file leaves no dangling surface or group references", () => {
    let state = reducer(undefined, openTab(chat("a")));
    state = reducer(
      state,
      splitTab({ tabId: chat("a"), surfaceKey: fileA, dir: "row" }),
    );
    const group = state.groups[chat("a")];
    if (!group) throw new Error("missing mixed group");
    state = reducer(
      state,
      splitPaneWithSurface({
        tabId: chat("a"),
        leafId: group.focusedLeafId,
        surfaceKey: fileB,
        dir: "col",
      }),
    );
    state = reducer(state, closeTab(fileA));
    const remaining = state.groups[chat("a")];
    if (!remaining) throw new Error("missing remaining group");
    expect(collectTabIds(remaining.root)).toEqual([chat("a"), fileB]);
    state = reducer(state, closeTab(fileB));
    expect(groupFor(state, chat("a")).root).toEqual(
      leaf("root", [chat("a")], chat("a")),
    );
    expect(state.tabs).toEqual([chat("a")]);
  });
});

describe("durable pane-local tabs", () => {
  test("retains ordering and selection through reorder, root close, rekey and hydration", () => {
    let state = reducer(undefined, openTab(chat("a")));
    for (const surfaceKey of ["file:/a", "file:/b"]) {
      state = reducer(
        state,
        addSurfaceToPane({ tabId: chat("a"), leafId: "root", surfaceKey }),
      );
    }
    state = reducer(
      state,
      reorderPaneTabs({
        tabId: chat("a"),
        leafId: "root",
        fromIndex: 2,
        toIndex: 0,
      }),
    );
    state = reducer(
      state,
      setPaneActive({
        tabId: chat("a"),
        leafId: "root",
        surfaceKey: "file:/a",
      }),
    );
    expect(state.tabs).toEqual([chat("a")]);
    expect(state.groups[chat("a")]?.root).toEqual(
      leaf("root", ["file:/b", chat("a"), "file:/a"], "file:/a"),
    );
    state = reducer(
      state,
      closePaneTab({ tabId: chat("a"), leafId: "root", surfaceKey: chat("a") }),
    );
    expect(state.tabs).toEqual(["file:/a"]);
    expect(state.activeTabId).toBe("file:/a");
    expect(state.groups["file:/a"]?.root).toEqual(
      leaf("root", ["file:/b", "file:/a"], "file:/a"),
    );
    const hydrated = reducer(
      undefined,
      hydrateWorkspace({ ...state, openThreadIds: [], contextChatIds: ["a"] }),
    );
    expect(hydrated.groups).toEqual(state.groups);
    expect(hydrated.tabs).toEqual(state.tabs);
    state = reducer(
      state,
      closePaneTab({ tabId: "file:/a", leafId: "root", surfaceKey: "file:/a" }),
    );
    expect(state.tabs).toEqual(["file:/b"]);
    expect(state.activeTabId).toBe("file:/b");
    expect(groupFor(state, "file:/b").root).toEqual(
      leaf("root", ["file:/b"], "file:/b"),
    );
  });

  test("closing the final split preserves a remaining multi-tab leaf and moves deduplicated surfaces", () => {
    let state = reducer(undefined, openTab(chat("a")));
    state = reducer(
      state,
      addSurfaceToPane({
        tabId: chat("a"),
        leafId: "root",
        surfaceKey: "file:/a",
      }),
    );
    state = reducer(
      state,
      splitTab({ tabId: chat("a"), dir: "row", surfaceKey: chat("b") }),
    );
    const splitLeafId = state.groups[chat("a")]?.focusedLeafId ?? "missing";
    state = reducer(
      state,
      closePaneTab({
        tabId: chat("a"),
        leafId: splitLeafId,
        surfaceKey: chat("b"),
      }),
    );
    expect(state.tabs).toEqual([chat("a")]);
    expect(state.groups[chat("a")]?.root).toEqual(
      leaf("root", [chat("a"), "file:/a"], "file:/a"),
    );
    state = reducer(state, openTab("file:/b"));
    state = reducer(
      state,
      addSurfaceToPane({
        tabId: chat("a"),
        leafId: "root",
        surfaceKey: "file:/b",
      }),
    );
    state = reducer(
      state,
      addSurfaceToPane({
        tabId: chat("a"),
        leafId: "root",
        surfaceKey: "file:/b",
      }),
    );
    expect(state.tabs).toEqual([chat("a")]);
    expect(renderedSurfaces(state)).toEqual([chat("a"), "file:/a", "file:/b"]);
    expect(selectVisibleSurfaceKeys(rootState(state))).toEqual(["file:/b"]);
  });
});

describe("all seven pane-capable surfaces", () => {
  const keys = [
    "chat:a",
    "file:/a",
    "task:a",
    "buddy:home",
    "design:a",
    makeGitDiffSurfaceKey({
      root: "/repo",
      path: "a",
      old_path: null,
      left: { rev: "HEAD", path: "a" },
      right: { rev: "WORKTREE", path: "a" },
      kind: "working",
    }),
    "git:main",
  ];
  test.each(
    keys.flatMap((source) =>
      keys
        .filter((target) => target !== source)
        .flatMap((target) =>
          (["row", "col"] as const).map((dir) => ({ source, target, dir })),
        ),
    ),
  )(
    "splits $source into $target ($dir), moves, deduplicates, reorders and closes UI only",
    ({ source, target, dir }) => {
      let state = reducer(undefined, openTab(target));
      state = reducer(
        state,
        splitPaneWithSurface({
          tabId: target,
          leafId: "root",
          surfaceKey: source,
          dir,
        }),
      );
      expect(collectLeafIds(groupFor(state, target).root)).toHaveLength(2);
      expect(renderedSurfaces(state)).toEqual([target, source]);
      state = reducer(
        state,
        addSurfaceToPane({ tabId: target, leafId: "root", surfaceKey: source }),
      );
      expect(renderedSurfaces(state)).toEqual([target, source]);
      expect(collectLeafIds(groupFor(state, target).root)).toHaveLength(1);
      state = reducer(
        state,
        addSurfaceToPane({ tabId: target, leafId: "root", surfaceKey: source }),
      );
      state = reducer(
        state,
        reorderPaneTabs({
          tabId: target,
          leafId: "root",
          fromIndex: 1,
          toIndex: 0,
        }),
      );
      expect(renderedSurfaces(state)).toEqual([source, target]);
      state = reducer(
        state,
        closePaneTab({ tabId: target, leafId: "root", surfaceKey: source }),
      );
      expect(renderedSurfaces(state)).toEqual([target]);
    },
  );
  test.each(keys)(
    "restores root %s without requiring open chat threads",
    (key) => {
      const state = reducer(undefined, openTab(key));
      const restored = reducer(
        undefined,
        hydrateWorkspace({
          ...state,
          openThreadIds: key === "chat:a" ? ["a"] : [],
        }),
      );
      expect(renderedSurfaces(restored)).toEqual([key]);
    },
  );
  test("file capacity does not block other kinds and rejection does not detach source", () => {
    let state = reducer(undefined, openTab("chat:a"));
    for (let index = 0; index < 30; index++)
      state = reducer(state, openTab(`file:/${index}`));
    for (const surfaceKey of keys.filter((key) => !key.startsWith("file:")))
      state = reducer(
        state,
        addSurfaceToPane({ tabId: "chat:a", leafId: "root", surfaceKey }),
      );
    const source: WorkspaceState = {
      ...state,
      tabs: [...state.tabs, "file:/overflow"],
      groups: {
        ...state.groups,
        "file:/overflow": {
          root: leaf("source", ["file:/overflow"]),
          focusedLeafId: "source",
        },
      },
    };
    expect(
      reducer(
        source,
        addSurfaceToPane({
          tabId: "chat:a",
          leafId: "root",
          surfaceKey: "file:/overflow",
        }),
      ),
    ).toEqual(source);
    expect(
      reducer(
        source,
        splitPaneWithSurface({
          tabId: "chat:a",
          leafId: "root",
          surfaceKey: "file:/overflow",
          dir: "row",
        }),
      ),
    ).toEqual(source);
  });
  test("prunes only closed chats and explicit owned files, preserves independent surfaces", () => {
    let state = reducer(undefined, openTab("chat:a"));
    for (const key of keys.slice(1)) state = reducer(state, openTab(key));
    const next = reducer(state, reconcileWorkspace({ openThreadIds: [] }));
    expect(renderedSurfaces(next)).toEqual(
      keys.filter((key) => key !== "chat:a" && key !== "file:/a"),
    );
    expect(selectWorkspaceFocusedChatId({ workspace: next })).toBeNull();
    for (const key of keys.slice(2))
      expect(selectSurfaceChatId({ workspace: next }, key)).toBeNull();
  });
  test("focused task resolves its actual active chat without an ownership mapping", () => {
    const workspace = reducer(undefined, openTab("task:a"));
    expect(
      selectFocusedWorkspaceChatId({
        workspace,
        tasksUI: {
          openTasks: [{ id: "a", activeChat: { chatId: "planner" } }],
        },
      }),
    ).toBe("planner");
    expect(workspace.contextChatByTab).toBeUndefined();
  });
});

describe("independent pane capability pruning", () => {
  test.each(["file:/a", "git:main"])(
    "disabled root %s retains enabled nested Task/Buddy/Design",
    (root) => {
      let state = reducer(undefined, openTab(root));
      for (const key of ["task:a", "buddy:home", "design:a"])
        state = reducer(state, openTab(key));
      const workspaceCapabilities = {
        filesPanel: false,
        gitPanel: false,
        terminalPanel: false,
      };
      for (const next of [
        reducer(
          state,
          reconcileWorkspace({ openThreadIds: [], workspaceCapabilities }),
        ),
        reducer(
          undefined,
          hydrateWorkspace({
            ...state,
            openThreadIds: [],
            workspaceCapabilities,
          }),
        ),
      ]) {
        expect(renderedSurfaces(next)).toEqual([
          "task:a",
          "buddy:home",
          "design:a",
        ]);
        expect(next.tabs).toEqual(["design:a"]);
        expect(next.contextChatByTab).toBeUndefined();
      }
    },
  );
});
