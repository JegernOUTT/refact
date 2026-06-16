import { describe, expect, test } from "vitest";

import { findLeaf, type LeafPane, type PaneNode } from "./panesTree";
import {
  hydratePaneLayout,
  moveTabToPane,
  panesSlice,
  splitPane,
} from "./panesSlice";

const paneReducer = panesSlice.reducer;

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

const twoPaneRoot = (): PaneNode => ({
  kind: "split",
  id: "root-split",
  dir: "row",
  children: [leaf("left", ["chat-a"]), leaf("right", ["chat-b"])],
  sizes: [0.5, 0.5],
});

describe("pane drag split reducers", () => {
  test("right edge split creates a row split and removes the tab from its source leaf", () => {
    let state = paneReducer(
      undefined,
      hydratePaneLayout({ root: twoPaneRoot(), focusedLeafId: "right" }),
    );

    state = paneReducer(
      state,
      splitPane({ leafId: "right", dir: "row", tabId: "chat-a" }),
    );

    expect(findLeaf(state.root, "left")).toEqual(leaf("left", [], null));
    expect(findLeaf(state.root, "right")).toEqual(
      leaf("right", ["chat-b"], "chat-b"),
    );
    expect(findLeaf(state.root, "right:sibling:chat-a")).toEqual(
      leaf("right:sibling:chat-a", ["chat-a"], "chat-a"),
    );
    expect(state.focusedLeafId).toBe("right:sibling:chat-a");

    if (
      state.root.kind !== "split" ||
      state.root.children[1].kind !== "split"
    ) {
      throw new Error("expected nested split");
    }
    expect(state.root.children[1].dir).toBe("row");
  });

  test("bottom edge split creates a column split", () => {
    let state = paneReducer(
      undefined,
      hydratePaneLayout({
        root: leaf("root", ["chat-a", "chat-b"], "chat-a"),
        focusedLeafId: "root",
      }),
    );

    state = paneReducer(
      state,
      splitPane({ leafId: "root", dir: "col", tabId: "chat-b" }),
    );

    if (state.root.kind !== "split") {
      throw new Error("expected split root");
    }
    expect(state.root.dir).toBe("col");
    expect(findLeaf(state.root, "root")).toEqual(
      leaf("root", ["chat-a"], "chat-a"),
    );
    expect(findLeaf(state.root, "root:sibling:chat-b")).toEqual(
      leaf("root:sibling:chat-b", ["chat-b"], "chat-b"),
    );
  });

  test("strip drop moveTabToPane removes from source and appends to destination", () => {
    let state = paneReducer(
      undefined,
      hydratePaneLayout({ root: twoPaneRoot(), focusedLeafId: "left" }),
    );

    state = paneReducer(
      state,
      moveTabToPane({
        fromLeafId: "left",
        toLeafId: "right",
        tabId: "chat-a",
      }),
    );

    expect(findLeaf(state.root, "left")).toEqual(leaf("left", [], null));
    expect(findLeaf(state.root, "right")).toEqual(
      leaf("right", ["chat-b", "chat-a"], "chat-a"),
    );
    expect(state.focusedLeafId).toBe("right");
  });
});
