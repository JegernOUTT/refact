import { describe, expect, it } from "vitest";
import {
  collectLeafIds,
  collectTabIds,
  findLeafByTab,
  type PaneNode,
} from "../ChatPanes/panesTree";
import {
  isBrowserTabSurface,
  isChatBoundSurface,
  isSplittableSurface,
  isValidBrowserTabId,
  makeBrowserTabSurfaceKey,
  makeGitDiffSurfaceKey,
} from "./surfaceKey";
import {
  addSurfaceToPane,
  bindSurfaceToChat,
  canShowBrowserTabForChat,
  focusPane,
  hideBrowserTabsForChat,
  hydrateWorkspace,
  MAX_GROUP_LEAVES,
  normalizeWorkspaceDock,
  openTab,
  pruneBrowserTabSurfaces,
  reconcileWorkspace,
  resizeGroupSplit,
  restoreAppWorkspace,
  selectBrowserViewOwners,
  selectWorkspaceFocusedChatId,
  selectWorkspaceGroups,
  showBrowserTabForChat,
  splitPaneWithSurface,
  workspaceSlice,
  type WorkspaceState,
} from "./workspaceSlice";

const reducer = workspaceSlice.reducer;

const allSurfaces = (state: WorkspaceState): string[] => [
  ...state.tabs,
  ...Object.values(state.groups).flatMap((group) =>
    group ? collectTabIds(group.root) : [],
  ),
];

const rowChildren = (root: PaneNode): string[][] => {
  if (root.kind !== "split" || root.dir !== "row")
    throw new Error("expected a row split");
  return root.children.map((child) => collectTabIds(child));
};

function twoChats(): WorkspaceState {
  let state = reducer(undefined, openTab("chat:a"));
  state = reducer(state, openTab("chat:b"));
  return state;
}

describe("browser tab surface keys", () => {
  it("accepts Chromium tab ids and rejects the retired singleton", () => {
    expect(isValidBrowserTabId("A1B2C3")).toBe(true);
    expect(isValidBrowserTabId("tab-1")).toBe(true);
    expect(isValidBrowserTabId("main")).toBe(false);
    expect(isValidBrowserTabId("has space")).toBe(false);
    expect(isBrowserTabSurface("browser:tab-1")).toBe(true);
    expect(isBrowserTabSurface("browser:main")).toBe(false);
    expect(isSplittableSurface("browser:main")).toBe(false);
    expect(isSplittableSurface("browser:tab-1")).toBe(true);
    expect(isChatBoundSurface("browser:tab-1")).toBe(true);
    expect(isChatBoundSurface("app:binding")).toBe(true);
    expect(isChatBoundSurface("file:/a.ts")).toBe(false);
    expect(() => makeBrowserTabSurfaceKey("main")).toThrow();
  });

  it("drops the legacy browser:main surface when hydrating", () => {
    const state = reducer(
      undefined,
      hydrateWorkspace({
        tabs: ["chat:a", "browser:main"],
        activeTabId: "chat:a",
        groups: {
          "chat:a": {
            root: {
              kind: "split",
              id: "split",
              dir: "row",
              sizes: [0.5, 0.5],
              children: [
                {
                  kind: "leaf",
                  id: "left",
                  tabIds: ["chat:a"],
                  activeTabId: "chat:a",
                },
                {
                  kind: "leaf",
                  id: "right",
                  tabIds: ["browser:main"],
                  activeTabId: "browser:main",
                },
              ],
            },
            focusedLeafId: "left",
          },
        },
        openThreadIds: ["a"],
      }),
    );
    expect(allSurfaces(state)).not.toContain("browser:main");
    expect(allSurfaces(state)).toContain("chat:a");
  });

  it("keeps the browser dock section when normalizing a persisted dock", () => {
    expect(
      normalizeWorkspaceDock({ open: true, width: 300, section: "browser" })
        .section,
    ).toBe("browser");
  });
});

describe("chat-bound browser views", () => {
  it("splits the visible chat with the browser on the right, keeping focus on the chat", () => {
    let state = reducer(undefined, openTab("chat:a"));
    state = reducer(state, showBrowserTabForChat({ chatId: "a", tabId: "t1" }));
    const group = state.groups["chat:a"];
    expect(group).toBeDefined();
    if (!group) return;
    expect(rowChildren(group.root)).toEqual([["chat:a"], ["browser:t1"]]);
    expect(group.focusedLeafId).toBe(findLeafByTab(group.root, "chat:a")?.id);
    expect(state.activeTabId).toBe("chat:a");
  });

  it("gives a background chat its own group without switching the visible group", () => {
    const before = twoChats();
    expect(before.activeTabId).toBeTruthy();
    const visibleGroup = before.activeTabId;
    const state = reducer(
      before,
      showBrowserTabForChat({ chatId: "a", tabId: "t1" }),
    );
    expect(state.activeTabId).not.toBe("chat:a");
    expect(state.activeTabId).toBe(
      visibleGroup === "chat:a" ? "chat:b" : visibleGroup,
    );
    const visible = state.groups[state.activeTabId ?? ""];
    expect(visible && collectTabIds(visible.root)).toEqual(["chat:b"]);
    const own = state.groups["chat:a"];
    expect(own && rowChildren(own.root)).toEqual([["chat:a"], ["browser:t1"]]);
  });

  it("adds a second browser tab to the existing browser leaf", () => {
    let state = reducer(undefined, openTab("chat:a"));
    state = reducer(state, showBrowserTabForChat({ chatId: "a", tabId: "t1" }));
    state = reducer(state, showBrowserTabForChat({ chatId: "a", tabId: "t2" }));
    const group = state.groups["chat:a"];
    expect(group && rowChildren(group.root)).toEqual([
      ["chat:a"],
      ["browser:t1", "browser:t2"],
    ]);
    expect(group && findLeafByTab(group.root, "browser:t2")?.activeTabId).toBe(
      "browser:t2",
    );
  });

  it("moves a view to the requested chat instead of duplicating it", () => {
    let state = twoChats();
    state = reducer(state, showBrowserTabForChat({ chatId: "a", tabId: "t1" }));
    state = reducer(state, showBrowserTabForChat({ chatId: "b", tabId: "t1" }));
    expect(
      allSurfaces(state).filter((key) => key === "browser:t1"),
    ).toHaveLength(1);
    expect(selectBrowserViewOwners({ workspace: state })).toEqual({ t1: "b" });
  });

  it("keeps chat-bound groups out of the consolidated workspace group", () => {
    let state = reducer(undefined, openTab("chat:a"));
    state = reducer(state, showBrowserTabForChat({ chatId: "a", tabId: "t1" }));
    state = reducer(state, openTab("chat:b"));
    const own = state.groups["chat:a"];
    expect(own && collectTabIds(own.root)).toEqual(["chat:a", "browser:t1"]);
    expect(state.activeTabId).not.toBe("chat:a");
  });

  it("reports the owning chat when the browser leaf has focus", () => {
    let state = reducer(undefined, openTab("chat:a"));
    state = reducer(state, showBrowserTabForChat({ chatId: "a", tabId: "t1" }));
    const group = state.groups["chat:a"];
    const browserLeaf = group && findLeafByTab(group.root, "browser:t1");
    expect(browserLeaf).toBeTruthy();
    state = reducer(
      state,
      focusPane({ tabId: "chat:a", leafId: browserLeaf?.id ?? "" }),
    );
    expect(selectWorkspaceFocusedChatId({ workspace: state })).toBe("a");
  });

  it("keeps a chat's browser leaf when its app workspace is restored", () => {
    let state = reducer(undefined, openTab("chat:a"));
    state = reducer(state, showBrowserTabForChat({ chatId: "a", tabId: "t1" }));
    state = reducer(
      state,
      restoreAppWorkspace({
        chatId: "a",
        root: {
          kind: "split",
          id: "app-root:a",
          dir: "row",
          sizes: [0.5, 0.5],
          children: [
            {
              kind: "leaf",
              id: "app-root:a:0",
              tabIds: ["chat:a"],
              activeTabId: "chat:a",
            },
            {
              kind: "leaf",
              id: "app-root:a:1",
              tabIds: ["app:binding"],
              activeTabId: "app:binding",
            },
          ],
        },
        focusedLeafId: "app-root:a:0",
      }),
    );
    const group = state.groups["chat:a"];
    expect(group && collectTabIds(group.root)).toEqual([
      "chat:a",
      "app:binding",
      "browser:t1",
    ]);
    const root = group?.root;
    expect(root?.kind === "split" && root.dir).toBe("row");
    expect(
      root?.kind === "split" &&
        collectTabIds(root.children[root.children.length - 1]),
    ).toEqual(["browser:t1"]);
  });

  it("prunes views whose Chromium tab is gone", () => {
    let state = reducer(undefined, openTab("chat:a"));
    state = reducer(state, showBrowserTabForChat({ chatId: "a", tabId: "t1" }));
    state = reducer(state, showBrowserTabForChat({ chatId: "a", tabId: "t2" }));
    state = reducer(state, pruneBrowserTabSurfaces({ liveTabIds: ["t2"] }));
    expect(allSurfaces(state)).not.toContain("browser:t1");
    expect(allSurfaces(state)).toContain("browser:t2");
  });

  it("closes a chat's browser views when the chat closes", () => {
    let state = twoChats();
    state = reducer(state, showBrowserTabForChat({ chatId: "a", tabId: "t1" }));
    state = reducer(state, reconcileWorkspace({ openThreadIds: ["b"] }));
    expect(allSurfaces(state).filter(isBrowserTabSurface)).toEqual([]);
    expect(allSurfaces(state)).toContain("chat:b");
  });

  it("hides only the requested chat's browser views", () => {
    let state = twoChats();
    state = reducer(state, showBrowserTabForChat({ chatId: "a", tabId: "t1" }));
    state = reducer(state, showBrowserTabForChat({ chatId: "b", tabId: "t2" }));
    state = reducer(state, hideBrowserTabsForChat("a"));
    expect(selectBrowserViewOwners({ workspace: state })).toEqual({ t2: "b" });
    expect(allSurfaces(state)).toContain("chat:a");
  });

  it("refuses an open that would exceed the pane cap", () => {
    let state = reducer(undefined, openTab("chat:a"));
    for (let index = 1; index < MAX_GROUP_LEAVES; index += 1) {
      const group = state.groups["chat:a"];
      if (!group) throw new Error("missing chat group");
      state = reducer(
        state,
        splitPaneWithSurface({
          tabId: "chat:a",
          leafId: collectLeafIds(group.root)[0],
          surfaceKey: `app:binding-${index}`,
          dir: "col",
        }),
      );
    }
    const full = state.groups["chat:a"];
    expect(full && collectLeafIds(full.root)).toHaveLength(MAX_GROUP_LEAVES);
    expect(canShowBrowserTabForChat(state, "a", "t1")).toBe(false);
    const unchanged = reducer(
      state,
      showBrowserTabForChat({ chatId: "a", tabId: "t1" }),
    );
    expect(allSurfaces(unchanged)).not.toContain("browser:t1");
  });
});

describe("chat-bound browser views: review fixes", () => {
  it("moves a visible but unfocused chat without activating it", () => {
    let state = twoChats();
    const key = state.activeTabId ?? "";
    const main = state.groups[key];
    const leaf = main && findLeafByTab(main.root, "chat:a");
    if (!leaf) throw new Error("missing chat leaf");
    state = reducer(
      state,
      splitPaneWithSurface({
        tabId: key,
        leafId: leaf.id,
        surfaceKey: "chat:b",
        dir: "row",
        placement: "after",
      }),
    );
    const split = state.groups[state.activeTabId ?? ""];
    const left = split && findLeafByTab(split.root, "chat:a");
    if (!left) throw new Error("missing left leaf");
    state = reducer(
      state,
      focusPane({ tabId: state.activeTabId ?? "", leafId: left.id }),
    );
    const activeBefore = state.activeTabId;
    state = reducer(state, showBrowserTabForChat({ chatId: "b", tabId: "t1" }));
    expect(state.activeTabId).toBe(activeBefore);
    expect(selectWorkspaceFocusedChatId({ workspace: state })).toBe("a");
    expect(selectBrowserViewOwners({ workspace: state })).toEqual({ t1: "b" });
  });

  it("opens a file inside the focused chat-bound group", () => {
    let state = reducer(undefined, openTab("chat:a"));
    state = reducer(state, showBrowserTabForChat({ chatId: "a", tabId: "t1" }));
    state = reducer(state, openTab("file:/repo/a.ts"));
    expect(state.tabs).toEqual(["chat:a"]);
    expect(state.activeTabId).toBe("chat:a");
    const group = state.groups["chat:a"];
    expect(group && collectTabIds(group.root).sort()).toEqual(
      ["browser:t1", "chat:a", "file:/repo/a.ts"].sort(),
    );
    expect(group && findLeafByTab(group.root, "file:/repo/a.ts")?.id).toBe(
      group?.focusedLeafId,
    );
    expect(state.contextChatByTab?.["file:/repo/a.ts"]).toBe("a");
    expect(selectWorkspaceFocusedChatId({ workspace: state })).toBe("a");
    const visible = selectWorkspaceGroups({ workspace: state })["chat:a"];
    expect(visible && collectTabIds(visible.root)).toContain("browser:t1");
  });

  it("keeps other chats out of a chat-bound group", () => {
    let state = reducer(undefined, openTab("chat:a"));
    state = reducer(state, showBrowserTabForChat({ chatId: "a", tabId: "t1" }));
    state = reducer(state, openTab("chat:b"));
    const group = state.groups["chat:a"];
    expect(group && collectTabIds(group.root)).not.toContain("chat:b");
  });

  it("hides another chat's browser views from the visible workspace", () => {
    let state = twoChats();
    state = reducer(state, showBrowserTabForChat({ chatId: "a", tabId: "t1" }));
    const active = state.activeTabId ?? "";
    expect(active).not.toBe("chat:a");
    const groups = selectWorkspaceGroups({ workspace: state });
    expect(
      Object.values(groups).flatMap((group) =>
        group ? collectTabIds(group.root) : [],
      ),
    ).not.toContain("browser:t1");
    state = reducer(state, openTab("chat:a"));
    const own = selectWorkspaceGroups({ workspace: state })["chat:a"];
    expect(own && collectTabIds(own.root)).toContain("browser:t1");
  });

  it("never lets an automatic hint take a view out of another chat's group", () => {
    let state = twoChats();
    state = reducer(state, openTab("chat:a"));
    state = reducer(state, showBrowserTabForChat({ chatId: "a", tabId: "t1" }));
    const before = state;
    state = reducer(
      state,
      showBrowserTabForChat({ chatId: "b", tabId: "t1", automatic: true }),
    );
    expect(state).toBe(before);
    expect(canShowBrowserTabForChat(state, "b", "t1", true)).toBe(false);
    state = reducer(state, showBrowserTabForChat({ chatId: "b", tabId: "t1" }));
    expect(selectBrowserViewOwners({ workspace: state })).toEqual({ t1: "b" });
  });

  const appRoot = (leaves: string[][]): PaneNode =>
    leaves.length === 1
      ? {
          kind: "leaf",
          id: "app-root:a:0",
          tabIds: leaves[0],
          activeTabId: leaves[0][0],
        }
      : {
          kind: "split",
          id: "app-root:a",
          dir: "row",
          sizes: leaves.map(() => 1 / leaves.length),
          children: leaves.map((tabIds, index) => ({
            kind: "leaf" as const,
            id: `app-root:a:${index}`,
            tabIds,
            activeTabId: tabIds[0],
          })),
        };

  it("keeps the browser leaf's side, size and id when an app workspace is restored", () => {
    let state = reducer(undefined, openTab("chat:a"));
    state = reducer(state, showBrowserTabForChat({ chatId: "a", tabId: "t1" }));
    const root = state.groups["chat:a"]?.root;
    if (root?.kind !== "split") throw new Error("expected a split");
    const browserLeafId = root.children[1].id;
    state = reducer(
      state,
      resizeGroupSplit({
        tabId: "chat:a",
        splitId: root.id,
        sizes: [0.7, 0.3],
      }),
    );
    state = reducer(
      state,
      restoreAppWorkspace({
        chatId: "a",
        root: appRoot([["chat:a"], ["app:binding"]]),
        focusedLeafId: "app-root:a:0",
      }),
    );
    const restored = state.groups["chat:a"]?.root;
    if (restored?.kind !== "split") throw new Error("expected a split");
    expect(restored.id).toBe(root.id);
    expect(restored.children[1].id).toBe(browserLeafId);
    expect(collectTabIds(restored.children[1])).toEqual(["browser:t1"]);
    expect(restored.sizes[1]).toBeCloseTo(0.3);
  });

  it("keeps browser tabs out of the chat leaf when a restored app workspace is at the pane cap", () => {
    let state = reducer(undefined, openTab("chat:a"));
    state = reducer(state, showBrowserTabForChat({ chatId: "a", tabId: "t1" }));
    const leaves = [
      ["chat:a"],
      ...Array.from({ length: MAX_GROUP_LEAVES - 1 }, (_, index) => [
        `app:b${index}`,
      ]),
    ];
    state = reducer(
      state,
      restoreAppWorkspace({
        chatId: "a",
        root: appRoot(leaves),
        focusedLeafId: "app-root:a:0",
      }),
    );
    const group = state.groups["chat:a"];
    const chatLeaf = group && findLeafByTab(group.root, "chat:a");
    expect(chatLeaf?.tabIds).toEqual(["chat:a"]);
    const browserLeaf = group && findLeafByTab(group.root, "browser:t1");
    expect(browserLeaf?.id).toBe(`app-root:a:${MAX_GROUP_LEAVES - 1}`);
    expect(group && collectLeafIds(group.root)).toHaveLength(MAX_GROUP_LEAVES);
  });
});

describe("chat-bound browser views: review round 2", () => {
  const gitDiff = (path: string) =>
    makeGitDiffSurfaceKey({
      root: "/repo",
      path,
      old_path: null,
      left: { rev: "HEAD", path },
      right: { rev: "WORKTREE", path },
      kind: "working",
    });

  const widths = (
    node: PaneNode,
    width = 1,
    out: [string[], number][] = [],
  ): [string[], number][] => {
    if (node.kind === "leaf") out.push([node.tabIds, width]);
    else
      node.children.forEach((child, index) =>
        widths(
          child,
          node.dir === "row" ? width * node.sizes[index] : width,
          out,
        ),
      );
    return out;
  };

  it("shares one context pane for every git diff opened from a chat with a browser view", () => {
    let state = reducer(undefined, openTab("chat:a"));
    state = reducer(state, showBrowserTabForChat({ chatId: "a", tabId: "t1" }));
    const diffs = ["a.ts", "b.ts", "c.ts"].map(gitDiff);
    for (const diff of diffs) state = reducer(state, openTab(diff));
    const group = state.groups["chat:a"];
    if (!group) throw new Error("missing group");
    expect(collectLeafIds(group.root)).toHaveLength(3);
    expect(findLeafByTab(group.root, diffs[0])?.tabIds).toEqual(diffs);
    const sizes = Object.fromEntries(
      widths(group.root).map(([tabIds, width]) => [tabIds[0], width]),
    );
    expect(sizes["chat:a"]).toBeCloseTo(0.25);
    expect(sizes[diffs[0]]).toBeCloseTo(0.25);
    expect(sizes["browser:t1"]).toBeCloseTo(0.5);
  });

  it("puts design and git surfaces into the existing file pane", () => {
    let state = reducer(undefined, openTab("chat:a"));
    state = reducer(state, showBrowserTabForChat({ chatId: "a", tabId: "t1" }));
    state = reducer(state, openTab("file:/repo/a.ts"));
    state = reducer(state, openTab("design:d1"));
    state = reducer(state, openTab("git:main"));
    const group = state.groups["chat:a"];
    expect(group && collectLeafIds(group.root)).toHaveLength(3);
    expect(group && findLeafByTab(group.root, "git:main")?.tabIds).toEqual([
      "file:/repo/a.ts",
      "design:d1",
      "git:main",
    ]);
  });

  it("never opens a file in the browser pane at the pane cap", () => {
    let state = reducer(undefined, openTab("chat:a"));
    state = reducer(state, showBrowserTabForChat({ chatId: "a", tabId: "t1" }));
    const groupOf = (current: WorkspaceState) => {
      const group = current.groups["chat:a"];
      if (!group) throw new Error("missing group");
      return group;
    };
    for (let index = 0; index < MAX_GROUP_LEAVES - 2; index += 1) {
      const chatLeaf = findLeafByTab(groupOf(state).root, "chat:a");
      if (!chatLeaf) throw new Error("missing chat leaf");
      state = reducer(
        state,
        splitPaneWithSurface({
          tabId: "chat:a",
          leafId: chatLeaf.id,
          surfaceKey: `app:b${index}`,
          dir: "col",
        }),
      );
    }
    const browserLeaf = findLeafByTab(groupOf(state).root, "browser:t1");
    if (!browserLeaf) throw new Error("missing browser leaf");
    state = reducer(
      state,
      focusPane({ tabId: "chat:a", leafId: browserLeaf.id }),
    );
    expect(collectLeafIds(groupOf(state).root)).toHaveLength(MAX_GROUP_LEAVES);
    state = reducer(state, openTab("file:/repo/a.ts"));
    const group = groupOf(state);
    expect(findLeafByTab(group.root, "browser:t1")?.tabIds).toEqual([
      "browser:t1",
    ]);
    expect(findLeafByTab(group.root, "file:/repo/a.ts")?.tabIds).toContain(
      "chat:a",
    );
  });

  it("lets an automatic hint take a view from a background chat's group", () => {
    let state = twoChats();
    state = reducer(state, showBrowserTabForChat({ chatId: "a", tabId: "t1" }));
    expect(state.activeTabId).not.toBe("chat:a");
    expect(canShowBrowserTabForChat(state, "b", "t1", true)).toBe(true);
    state = reducer(
      state,
      showBrowserTabForChat({ chatId: "b", tabId: "t1", automatic: true }),
    );
    expect(selectBrowserViewOwners({ workspace: state })).toEqual({ t1: "b" });
  });

  it("keeps files opened next to the chat when its app workspace is restored", () => {
    const appRoot: PaneNode = {
      kind: "split",
      id: "app-root:a",
      dir: "row",
      sizes: [0.5, 0.5],
      children: [
        {
          kind: "leaf",
          id: "app-root:a:0",
          tabIds: ["chat:a"],
          activeTabId: "chat:a",
        },
        {
          kind: "leaf",
          id: "app-root:a:1",
          tabIds: ["app:bind"],
          activeTabId: "app:bind",
        },
      ],
    };
    let state = reducer(undefined, openTab("chat:a"));
    state = reducer(
      state,
      restoreAppWorkspace({
        chatId: "a",
        root: appRoot,
        focusedLeafId: "app-root:a:0",
      }),
    );
    state = reducer(state, showBrowserTabForChat({ chatId: "a", tabId: "t1" }));
    state = reducer(state, openTab("file:/repo/a.ts"));
    state = reducer(
      state,
      restoreAppWorkspace({
        chatId: "a",
        root: appRoot,
        focusedLeafId: "app-root:a:0",
      }),
    );
    expect(state.tabs).toEqual(["chat:a"]);
    const group = state.groups["chat:a"];
    if (!group) throw new Error("missing group");
    expect(collectTabIds(group.root).sort()).toEqual(
      ["app:bind", "browser:t1", "chat:a", "file:/repo/a.ts"].sort(),
    );
    expect(findLeafByTab(group.root, "file:/repo/a.ts")?.tabIds).toEqual([
      "file:/repo/a.ts",
    ]);
    expect(state.contextChatByTab?.["file:/repo/a.ts"]).toBe("a");
  });
});

describe("chat-bound browser views: review round 3", () => {
  const appRoot = (extra: string[][]): PaneNode => ({
    kind: "split",
    id: "app-root:a",
    dir: "row",
    sizes: [["chat:a"], ...extra].map((_, __, all) => 1 / all.length),
    children: [["chat:a"], ...extra].map((tabIds, index) => ({
      kind: "leaf" as const,
      id: `app-root:a:${index}`,
      tabIds,
      activeTabId: tabIds[0],
    })),
  });
  const shape = (node: PaneNode): unknown =>
    node.kind === "leaf"
      ? node.tabIds
      : { [node.dir]: node.children.map(shape) };

  it("leaves the shared layout alone when the key chat's app workspace is restored", () => {
    let state = reducer(undefined, openTab("chat:a"));
    state = reducer(state, openTab("chat:b"));
    state = reducer(state, openTab("file:/repo/b.ts"));
    state = reducer(
      state,
      bindSurfaceToChat({ surfaceKey: "file:/repo/b.ts", chatId: "b" }),
    );
    state = reducer(state, openTab("task:t1"));
    state = reducer(state, openTab("git:main"));
    state = reducer(
      state,
      restoreAppWorkspace({
        chatId: "a",
        root: appRoot([["app:bind"]]),
        focusedLeafId: "app-root:a:0",
      }),
    );
    expect(
      Object.fromEntries(
        Object.entries(state.groups).map(([key, group]) => [
          key,
          group ? shape(group.root) : group,
        ]),
      ),
    ).toEqual({
      "chat:b": ["chat:b", "file:/repo/b.ts", "task:t1", "git:main"],
      "chat:a": { row: [["chat:a"], ["app:bind"]] },
    });
    expect(state.contextChatByTab?.["file:/repo/b.ts"]).toBe("b");
  });

  it("carries only the chat's own context surfaces out of its bound group", () => {
    let state = reducer(undefined, openTab("chat:a"));
    state = reducer(state, showBrowserTabForChat({ chatId: "a", tabId: "t1" }));
    state = reducer(state, openTab("file:/repo/a.ts"));
    state = reducer(state, openTab("file:/repo/b.ts"));
    state = reducer(
      state,
      bindSurfaceToChat({ surfaceKey: "file:/repo/b.ts", chatId: "b" }),
    );
    const group = state.groups["chat:a"];
    const fileLeaf = group && findLeafByTab(group.root, "file:/repo/a.ts");
    if (!group || !fileLeaf) throw new Error("missing file leaf");
    for (const surface of ["task:t1", "buddy:home"])
      state = reducer(
        state,
        addSurfaceToPane({
          tabId: "chat:a",
          leafId: fileLeaf.id,
          surfaceKey: surface,
        }),
      );
    state = reducer(
      state,
      restoreAppWorkspace({
        chatId: "a",
        root: appRoot([["app:bind"]]),
        focusedLeafId: "app-root:a:0",
      }),
    );
    const restored = state.groups["chat:a"];
    if (!restored) throw new Error("missing group");
    const surfaces = collectTabIds(restored.root);
    expect(surfaces).toContain("file:/repo/a.ts");
    expect(surfaces).toContain("browser:t1");
    for (const foreign of ["file:/repo/b.ts", "task:t1", "buddy:home"])
      expect(surfaces).not.toContain(foreign);
  });

  it("keeps carried files out of app panes when a restore is at the pane cap", () => {
    let state = reducer(undefined, openTab("chat:a"));
    state = reducer(state, showBrowserTabForChat({ chatId: "a", tabId: "t1" }));
    state = reducer(state, openTab("file:/repo/a.ts"));
    state = reducer(
      state,
      restoreAppWorkspace({
        chatId: "a",
        root: appRoot(
          Array.from({ length: MAX_GROUP_LEAVES - 1 }, (_, index) => [
            `app:b${index}`,
          ]),
        ),
        focusedLeafId: "app-root:a:0",
      }),
    );
    const group = state.groups["chat:a"];
    if (!group) throw new Error("missing group");
    expect(findLeafByTab(group.root, "file:/repo/a.ts")?.tabIds).toContain(
      "chat:a",
    );
    expect(findLeafByTab(group.root, "chat:a")?.activeTabId).toBe("chat:a");
    for (let index = 0; index < MAX_GROUP_LEAVES - 1; index += 1)
      expect(
        findLeafByTab(group.root, `app:b${index}`)?.tabIds.some((surface) =>
          surface.startsWith("file:"),
        ),
      ).toBe(false);
  });
});
