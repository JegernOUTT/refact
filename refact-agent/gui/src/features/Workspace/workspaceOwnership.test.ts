import { describe, expect, it } from "vitest";
import { collectLeafIds, collectTabIds } from "../ChatPanes/panesTree";
import reducer, {
  bindSurfaceToChat,
  reconcileWorkspace,
  selectWorkspaceFocusedChatId,
  activateWorkspaceLayout,
  addSurfaceToPane,
  openSurfaceInNewTab,
  splitPaneWithSurface,
  clearWorkspaceChatState,
  consolidateWorkspaceTabs,
  openTab,
  setActiveTab,
  selectSurfaceChatId,
  hydrateWorkspace,
  type WorkspaceState,
} from "./workspaceSlice";
import { makeGitDiffSurfaceKey } from "./surfaceKey";
import {
  loadPersistedWorkspace,
  savePersistedChatTabs,
  savePersistedWorkspace,
  setProjectStorageNamespace,
} from "../../utils/chatUiPersistence";

const contextualSurfaces = [
  "git:main",
  "design:preview",
  makeGitDiffSurfaceKey({
    root: "/repo",
    path: "x.ts",
    old_path: null,
    left: { rev: "HEAD", path: "x.ts" },
    right: { rev: "WORKTREE", path: "x.ts" },
    kind: "working",
  }),
];

const fileOnly = (): WorkspaceState => ({
  tabs: ["file:/x"],
  activeTabId: "file:/x",
  groups: {
    "file:/x": {
      root: {
        kind: "leaf",
        id: "root",
        tabIds: ["file:/x"],
        activeTabId: "file:/x",
      },
      focusedLeafId: "root",
    },
  },
  contextChatByTab: { "file:/x": "a" },
});
describe("workspace file ownership isolation", () => {
  it("migrates active file roots without aliasing focus to ownership", () => {
    const legacy = fileOnly();
    legacy.tabs = ["chat:a", "chat:b", "file:/x"];
    let state = reducer(legacy, consolidateWorkspaceTabs());
    state = reducer(state, setActiveTab("chat:b"));
    state = reducer(state, clearWorkspaceChatState("b"));
    expect(
      Object.values(state.groups).flatMap((group) =>
        group ? collectTabIds(group.root) : [],
      ),
    ).toContain("file:/x");
    expect(selectSurfaceChatId({ workspace: state }, "file:/x")).toBe("a");
  });
  it("does not rebind a file-only root when a chat joins", () => {
    let state = reducer(fileOnly(), openTab("chat:b"));
    state = reducer(state, clearWorkspaceChatState("b"));
    expect(selectSurfaceChatId({ workspace: state }, "file:/x")).toBe("a");
    expect(
      Object.values(state.groups).flatMap((group) =>
        group ? collectTabIds(group.root) : [],
      ),
    ).toContain("file:/x");
  });
  it("keeps truly unbound files unbound after subsequent chat focus", () => {
    let state = reducer(undefined, openTab("file:/unbound"));
    state = reducer(state, openTab("chat:b"));
    state = reducer(state, setActiveTab("chat:b"));
    expect(
      selectSurfaceChatId({ workspace: state }, "file:/unbound"),
    ).toBeNull();
    state = reducer(state, clearWorkspaceChatState("b"));
    expect(
      Object.values(state.groups).flatMap((group) =>
        group ? collectTabIds(group.root) : [],
      ),
    ).toContain("file:/unbound");
  });
  it("returns to Editor without replacing the selected file with the root chat", () => {
    let state = reducer(undefined, openTab("chat:a"));
    state = reducer(state, openTab("file:/x"));
    const root = state.groups["chat:a"];
    state = reducer(state, openTab("main:settings"));
    state = reducer(state, activateWorkspaceLayout());
    expect(state.activeTabId).toBe("chat:a");
    expect(state.groups["chat:a"]).toEqual(root);
  });
  it.each(["new-tab", "pane", "split"])(
    "records file ownership at the %s creation path",
    (path) => {
      let state = reducer(undefined, openTab("chat:a"));
      state = reducer(
        state,
        path === "new-tab"
          ? openSurfaceInNewTab("file:/x")
          : path === "pane"
            ? addSurfaceToPane({
                tabId: "chat:a",
                leafId: "root",
                surfaceKey: "file:/x",
              })
            : splitPaneWithSurface({
                tabId: "chat:a",
                leafId: "root",
                surfaceKey: "file:/x",
                dir: "row",
              }),
      );
      state = reducer(state, openTab("chat:b"));
      expect(selectSurfaceChatId({ workspace: state }, "file:/x")).toBe("a");
      state = reducer(state, setActiveTab("file:/x"));
      expect(selectSurfaceChatId({ workspace: state }, "file:/x")).toBe("a");
    },
  );
  it("keeps the target pane selected file when another tab is edge-split beside it", () => {
    let state = reducer(undefined, openTab("chat:a"));
    state = reducer(state, openTab("file:/a"));
    state = reducer(state, openTab("file:/b"));
    state = reducer(state, setActiveTab("file:/a"));
    state = reducer(
      state,
      splitPaneWithSurface({
        tabId: "chat:a",
        leafId: "root",
        surfaceKey: "file:/b",
        dir: "row",
      }),
    );
    const root = state.groups["chat:a"]?.root;
    expect(root?.kind).toBe("split");
    if (root?.kind !== "split") throw new Error("missing split");
    expect(root.children[0]).toMatchObject({ activeTabId: "file:/a" });
    expect(root.children[1]).toMatchObject({ activeTabId: "file:/b" });
  });
  it("collapses only leaves emptied by invalid surface pruning", () => {
    const state = reducer(
      undefined,
      hydrateWorkspace({
        tabs: ["chat:a"],
        activeTabId: "chat:a",
        groups: {
          "chat:a": {
            focusedLeafId: "invalid",
            root: {
              kind: "split",
              id: "split",
              dir: "row",
              sizes: [1, 1, 1],
              children: [
                {
                  kind: "leaf",
                  id: "valid",
                  tabIds: ["chat:a"],
                  activeTabId: "chat:a",
                },
                {
                  kind: "leaf",
                  id: "invalid",
                  tabIds: ["main:tasks"],
                  activeTabId: "main:tasks",
                },
                {
                  kind: "leaf",
                  id: "user-empty",
                  tabIds: [],
                  activeTabId: null,
                },
              ],
            },
          },
        },
        openThreadIds: ["a"],
      }),
    );
    expect(
      collectLeafIds(
        state.groups["chat:a"]?.root ?? {
          kind: "leaf",
          id: "missing",
          tabIds: [],
          activeTabId: null,
        },
      ),
    ).toEqual(["valid", "user-empty"]);
  });
});

describe("explicit workspace surface chat context", () => {
  it.each(contextualSurfaces)(
    "keeps %s context on its exact surface key",
    (key) => {
      let state = reducer(undefined, openTab("chat:a"));
      state = reducer(state, openTab(key));
      expect(selectSurfaceChatId({ workspace: state }, key)).toBeNull();
      state = reducer(
        state,
        bindSurfaceToChat({ surfaceKey: key, chatId: "b" }),
      );
      state = reducer(state, setActiveTab("chat:a"));
      state = reducer(state, setActiveTab(key));
      expect(selectSurfaceChatId({ workspace: state }, key)).toBe("b");
      expect(selectWorkspaceFocusedChatId({ workspace: state })).toBe("b");
      state = reducer(state, openTab("file:/from-context"));
      expect(
        selectSurfaceChatId({ workspace: state }, "file:/from-context"),
      ).toBe("b");
      expect(selectSurfaceChatId({ workspace: state }, key)).toBe("b");
      expect(state.tabs).toEqual(["chat:a"]);
      state = reducer(state, clearWorkspaceChatState("b"));
      expect(selectSurfaceChatId({ workspace: state }, key)).toBeNull();
    },
  );

  it.each(contextualSurfaces)(
    "does not replace %s root context when a chat joins",
    (key) => {
      let state = reducer(undefined, openTab(key));
      state = reducer(
        state,
        bindSurfaceToChat({ surfaceKey: key, chatId: "b" }),
      );
      state = reducer(state, openTab("chat:a"));
      state = reducer(state, setActiveTab(key));
      expect(selectWorkspaceFocusedChatId({ workspace: state })).toBe("b");
      expect(state.contextChatByTab?.[key]).toBe("b");
    },
  );

  it.each(["task:t", "buddy:home"])(
    "does not bind unrelated %s context",
    (key) => {
      let state = reducer(undefined, openTab("chat:a"));
      state = reducer(state, openTab(key));
      state = reducer(
        state,
        bindSurfaceToChat({ surfaceKey: key, chatId: "a" }),
      );
      expect(selectSurfaceChatId({ workspace: state }, key)).toBeNull();
      expect(selectWorkspaceFocusedChatId({ workspace: state })).toBeNull();
      expect(state.contextChatByTab?.[key]).toBeUndefined();
    },
  );

  it("roundtrips explicit contexts only in a trusted project and drops closed owners", () => {
    setProjectStorageNamespace("explicit-context-test");
    try {
      savePersistedChatTabs({
        openThreadIds: ["a", "b"],
        currentThreadId: "a",
        tabs: [{ id: "a" }, { id: "b" }],
      });
      let state = reducer(undefined, openTab("chat:a"));
      for (const key of contextualSurfaces) {
        state = reducer(state, openTab(key));
        state = reducer(
          state,
          bindSurfaceToChat({ surfaceKey: key, chatId: "b" }),
        );
      }
      savePersistedWorkspace(state);
      const restored = loadPersistedWorkspace();
      for (const key of contextualSurfaces)
        expect(selectSurfaceChatId({ workspace: restored }, key)).toBe("b");
      const closed = reducer(
        restored,
        reconcileWorkspace({ openThreadIds: ["a"] }),
      );
      for (const key of contextualSurfaces)
        expect(selectSurfaceChatId({ workspace: closed }, key)).toBeNull();
      setProjectStorageNamespace(undefined);
      expect(loadPersistedWorkspace().contextChatByTab).toBeUndefined();
    } finally {
      setProjectStorageNamespace(undefined);
    }
  });
});
