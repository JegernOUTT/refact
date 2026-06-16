import { configureStore } from "@reduxjs/toolkit";
import type { UnknownAction } from "@reduxjs/toolkit";
import { describe, expect, test } from "vitest";
import { chatReducer } from "../Chat/Thread/reducer";
import { closeThread, createChatWithId } from "../Chat/Thread/actions";
import {
  collectTabIds,
  findLeaf,
  type LeafPane,
  type PaneNode,
} from "./panesTree";
import {
  INITIAL_PANE_LEAF_ID,
  addTabToFocusedPane,
  closePane,
  focusPane,
  hydratePaneLayout,
  moveTabToPane,
  panesSlice,
  reconcilePanesWithOpenThreads,
  removeTabEverywhere,
  resizeSplit,
  selectFocusedActiveTabId,
  selectFocusedLeafId,
  selectLeafForTab,
  selectPaneRoot,
  selectVisibleThreadIds,
  setPaneActiveTab,
  splitPane,
} from "./panesSlice";

const leaf = (
  id: string,
  tabIds: string[] = [],
  activeTabId: string | null = tabIds[0] ?? null,
): LeafPane => ({
  kind: "leaf",
  id,
  tabIds,
  activeTabId,
});

const rootState = (root: PaneNode, focusedLeafId: string) => ({
  panes: { root, focusedLeafId },
});

const paneReducer = panesSlice.reducer;

function expectTabSubset(root: PaneNode, openThreadIds: string[]) {
  const openThreads = new Set(openThreadIds);
  expect(collectTabIds(root).every((tabId) => openThreads.has(tabId))).toBe(
    true,
  );
}

function reduceWithPaneInvariant(
  state:
    | {
        chat: ReturnType<typeof chatReducer>;
        panes: ReturnType<typeof paneReducer>;
      }
    | undefined,
  action: UnknownAction,
) {
  const nextState = {
    chat: chatReducer(state?.chat, action),
    panes: paneReducer(state?.panes, action),
  };

  return {
    ...nextState,
    panes: reconcilePanesWithOpenThreads(
      nextState.panes,
      nextState.chat.open_thread_ids,
      nextState.chat.current_thread_id,
    ),
  };
}

describe("panesSlice", () => {
  test("starts with a single empty focused leaf", () => {
    const state = paneReducer(undefined, { type: "@@INIT" });

    expect(state).toEqual({
      root: leaf(INITIAL_PANE_LEAF_ID, [], null),
      focusedLeafId: INITIAL_PANE_LEAF_ID,
    });
    expect(selectPaneRoot({ panes: state })).toEqual(
      leaf(INITIAL_PANE_LEAF_ID, [], null),
    );
    expect(selectFocusedLeafId({ panes: state })).toBe(INITIAL_PANE_LEAF_ID);
    expect(selectFocusedActiveTabId({ panes: state })).toBeNull();
  });

  test("splitPane creates and focuses a sibling leaf", () => {
    let state = paneReducer(undefined, addTabToFocusedPane("chat-a"));

    state = paneReducer(
      state,
      splitPane({ leafId: INITIAL_PANE_LEAF_ID, dir: "row", tabId: "chat-a" }),
    );

    expect(state.focusedLeafId).toBe("root:sibling:chat-a");
    expect(findLeaf(state.root, INITIAL_PANE_LEAF_ID)).toEqual(
      leaf(INITIAL_PANE_LEAF_ID, [], null),
    );
    expect(findLeaf(state.root, "root:sibling:chat-a")).toEqual(
      leaf("root:sibling:chat-a", ["chat-a"], "chat-a"),
    );
    expect(selectFocusedActiveTabId({ panes: state })).toBe("chat-a");
  });

  test("addTabToFocusedPane moves a tab out of its previous pane", () => {
    let state = paneReducer(undefined, addTabToFocusedPane("chat-a"));
    state = paneReducer(
      state,
      splitPane({ leafId: INITIAL_PANE_LEAF_ID, dir: "row", tabId: "chat-a" }),
    );
    state = paneReducer(state, focusPane(INITIAL_PANE_LEAF_ID));
    state = paneReducer(state, addTabToFocusedPane("chat-a"));

    expect(findLeaf(state.root, INITIAL_PANE_LEAF_ID)).toEqual(
      leaf(INITIAL_PANE_LEAF_ID, ["chat-a"], "chat-a"),
    );
    expect(findLeaf(state.root, "root:sibling:chat-a")).toEqual(
      leaf("root:sibling:chat-a", [], null),
    );
  });
  test("updates active tabs, focus, closing, moving, adding, and removing tabs", () => {
    let state = paneReducer(undefined, addTabToFocusedPane("chat-a"));
    state = paneReducer(
      state,
      splitPane({ leafId: INITIAL_PANE_LEAF_ID, dir: "row", tabId: "chat-a" }),
    );
    state = paneReducer(state, addTabToFocusedPane("chat-b"));
    state = paneReducer(state, addTabToFocusedPane("chat-b"));

    expect(findLeaf(state.root, "root:sibling:chat-a")?.tabIds).toEqual([
      "chat-a",
      "chat-b",
    ]);

    state = paneReducer(
      state,
      setPaneActiveTab({ leafId: "root:sibling:chat-a", tabId: "chat-a" }),
    );
    expect(findLeaf(state.root, "root:sibling:chat-a")?.activeTabId).toBe(
      "chat-a",
    );

    state = paneReducer(state, focusPane(INITIAL_PANE_LEAF_ID));
    expect(state.focusedLeafId).toBe(INITIAL_PANE_LEAF_ID);

    state = paneReducer(
      state,
      moveTabToPane({
        fromLeafId: "root:sibling:chat-a",
        toLeafId: INITIAL_PANE_LEAF_ID,
        tabId: "chat-b",
      }),
    );
    expect(findLeaf(state.root, INITIAL_PANE_LEAF_ID)).toEqual(
      leaf(INITIAL_PANE_LEAF_ID, ["chat-b"], "chat-b"),
    );
    expect(findLeaf(state.root, "root:sibling:chat-a")).toEqual(
      leaf("root:sibling:chat-a", ["chat-a"], "chat-a"),
    );
    expect(state.focusedLeafId).toBe(INITIAL_PANE_LEAF_ID);

    state = paneReducer(state, removeTabEverywhere("chat-b"));
    expect(state.root).toEqual(
      leaf("root:sibling:chat-a", ["chat-a"], "chat-a"),
    );
    expect(state.focusedLeafId).toBe("root:sibling:chat-a");

    state = paneReducer(state, closePane("root:sibling:chat-a"));
    expect(state.root).toEqual(leaf("root:sibling:chat-a", [], null));
    expect(state.focusedLeafId).toBe("root:sibling:chat-a");
  });

  test("resizeSplit normalizes split sizes", () => {
    let state = paneReducer(undefined, addTabToFocusedPane("chat-a"));
    state = paneReducer(
      state,
      splitPane({ leafId: INITIAL_PANE_LEAF_ID, dir: "col", tabId: "chat-b" }),
    );

    state = paneReducer(
      state,
      resizeSplit({ splitId: "root:split:col", sizes: [2, 1] }),
    );

    if (state.root.kind !== "split") {
      throw new Error("expected split root");
    }
    expect(state.root.sizes[0]).toBeCloseTo(2 / 3);
    expect(state.root.sizes[1]).toBeCloseTo(1 / 3);
  });

  test("hydratePaneLayout normalizes layout and falls back to an existing focused leaf", () => {
    const root: PaneNode = {
      kind: "split",
      id: "split-a",
      dir: "row",
      children: [
        leaf("left", ["chat-a", "chat-a"], "missing"),
        leaf("right", ["chat-b"], "chat-b"),
      ],
      sizes: [0, 0],
    };

    const state = paneReducer(
      undefined,
      hydratePaneLayout({ root, focusedLeafId: "missing" }),
    );

    expect(state.focusedLeafId).toBe("left");
    expect(findLeaf(state.root, "left")).toEqual(
      leaf("left", ["chat-a"], "chat-a"),
    );
    if (state.root.kind !== "split") {
      throw new Error("expected split root");
    }
    expect(state.root.sizes).toEqual([0.5, 0.5]);
  });

  test("selectors expose visible active tabs and leaf lookup", () => {
    const root: PaneNode = {
      kind: "split",
      id: "root-split",
      dir: "row",
      children: [
        leaf("left", ["chat-a", "chat-c"], "chat-a"),
        leaf("right", ["chat-b", "chat-a"], "chat-b"),
        leaf("third", ["chat-a"], "chat-a"),
      ],
      sizes: [1, 1, 1],
    };
    const state = rootState(root, "right");

    expect(selectVisibleThreadIds(state)).toEqual(["chat-a", "chat-b"]);
    expect(selectLeafForTab(state, "chat-b")).toEqual(
      leaf("right", ["chat-b", "chat-a"], "chat-b"),
    );
    expect(selectFocusedActiveTabId(state)).toBe("chat-b");
  });

  test("pane tab ids stay within canonical open thread ids when chats close", () => {
    const store = configureStore({
      reducer: reduceWithPaneInvariant,
    });

    store.dispatch(createChatWithId({ id: "chat-a" }));
    store.dispatch(createChatWithId({ id: "chat-b" }));
    store.dispatch(addTabToFocusedPane("chat-a"));
    store.dispatch(
      splitPane({ leafId: INITIAL_PANE_LEAF_ID, dir: "row", tabId: "chat-a" }),
    );
    store.dispatch(addTabToFocusedPane("chat-b"));

    store.dispatch(closeThread({ id: "chat-a", force: true }));

    const afterCloseA = store.getState();
    expect(afterCloseA.chat.open_thread_ids).toEqual(["chat-b"]);
    expectTabSubset(afterCloseA.panes.root, afterCloseA.chat.open_thread_ids);
    expect(collectTabIds(afterCloseA.panes.root)).toEqual(["chat-b"]);

    store.dispatch(closeThread({ id: "chat-b", force: true }));

    const afterCloseB = store.getState();
    expect(afterCloseB.chat.open_thread_ids).toEqual([]);
    expectTabSubset(afterCloseB.panes.root, afterCloseB.chat.open_thread_ids);
    expect(collectTabIds(afterCloseB.panes.root)).toEqual([]);
  });
  test("reconcilePanesWithOpenThreads prunes hydrated tabs outside open_thread_ids", () => {
    const root: PaneNode = {
      kind: "split",
      id: "root-split",
      dir: "row",
      children: [
        leaf("left", ["chat-a", "closed"], "closed"),
        leaf("right", ["chat-b", "also-closed"], "chat-b"),
      ],
      sizes: [3, 1],
    };

    const state = reconcilePanesWithOpenThreads(
      { root, focusedLeafId: "missing" },
      ["chat-a", "chat-b"],
    );

    expect(findLeaf(state.root, "left")).toEqual(
      leaf("left", ["chat-a"], "chat-a"),
    );
    expect(findLeaf(state.root, "right")).toEqual(
      leaf("right", ["chat-b"], "chat-b"),
    );
    expect(state.focusedLeafId).toBe("left");
    expectTabSubset(state.root, ["chat-a", "chat-b"]);
    if (state.root.kind !== "split") {
      throw new Error("expected split root");
    }
    expect(state.root.sizes).toEqual([0.75, 0.25]);
  });
});
