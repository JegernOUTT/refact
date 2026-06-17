import { describe, expect, test } from "vitest";
import {
  closeLeaf,
  collectLeafIds,
  collectTabIds,
  findLeaf,
  findLeafByTab,
  moveTab,
  normalizeSizes,
  reorderTabInLeaf,
  splitLeaf,
  type LeafPane,
  type PaneNode,
  type SplitNode,
} from "./panesTree";

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

const split = (
  id: string,
  dir: SplitNode["dir"],
  children: PaneNode[],
  sizes: number[],
): SplitNode => ({
  kind: "split",
  id,
  dir,
  children,
  sizes,
});

const sum = (sizes: number[]): number =>
  sizes.reduce((total, size) => total + size, 0);

describe("panesTree", () => {
  test("splitLeaf replaces a nested leaf with an even split and moved tab sibling", () => {
    const tree = split(
      "root",
      "row",
      [leaf("left", ["a"], "a"), leaf("right", ["b", "c"], "c")],
      [0.3, 0.7],
    );

    const next = splitLeaf(tree, "right", "col", "c");

    expect(next).not.toBe(tree);
    expect(findLeaf(next, "right")).toEqual({
      kind: "leaf",
      id: "right",
      tabIds: ["b"],
      activeTabId: "b",
    });
    expect(findLeaf(next, "right:sibling:c")).toEqual({
      kind: "leaf",
      id: "right:sibling:c",
      tabIds: ["c"],
      activeTabId: "c",
    });
    expect(findLeafByTab(next, "c")?.id).toBe("right:sibling:c");

    if (next.kind !== "split") {
      throw new Error("expected root split");
    }
    expect(next.sizes).toEqual([0.3, 0.7]);

    const nested = next.children[1];
    if (nested.kind !== "split") {
      throw new Error("expected nested split");
    }
    expect(nested.dir).toBe("col");
    expect(nested.sizes).toEqual([0.5, 0.5]);
    expect(sum(nested.sizes)).toBeCloseTo(1);
    expect(tree.children[1]).toEqual(leaf("right", ["b", "c"], "c"));
  });

  test("closeLeaf removes nested leaves, collapses one-child splits, and renormalizes sibling sizes", () => {
    const tree = split(
      "root",
      "row",
      [
        leaf("a", ["a-tab"]),
        split(
          "nested",
          "col",
          [leaf("b", ["b-tab"]), leaf("c", ["c-tab"])],
          [0.25, 0.75],
        ),
        leaf("d", ["d-tab"]),
      ],
      [0.2, 0.3, 0.5],
    );

    const withoutC = closeLeaf(tree, "c");

    if (withoutC.kind !== "split") {
      throw new Error("expected root split");
    }
    expect(withoutC.children).toEqual([
      leaf("a", ["a-tab"]),
      leaf("b", ["b-tab"]),
      leaf("d", ["d-tab"]),
    ]);
    expect(withoutC.sizes).toEqual([0.2, 0.3, 0.5]);
    expect(sum(withoutC.sizes)).toBeCloseTo(1);

    const withoutD = closeLeaf(withoutC, "d");

    if (withoutD.kind !== "split") {
      throw new Error("expected root split");
    }
    expect(withoutD.children).toEqual([
      leaf("a", ["a-tab"]),
      leaf("b", ["b-tab"]),
    ]);
    expect(withoutD.sizes[0]).toBeCloseTo(0.4);
    expect(withoutD.sizes[1]).toBeCloseTo(0.6);
    expect(sum(withoutD.sizes)).toBeCloseTo(1);

    const collapsed = closeLeaf(withoutD, "a");
    expect(collapsed).toEqual(leaf("b", ["b-tab"]));
  });

  test("moveTab moves a tab between leaves without mutating the input", () => {
    const tree = split(
      "root",
      "row",
      [leaf("left", ["a", "b"], "b"), leaf("right", ["c"], "c")],
      [2, 1],
    );

    const next = moveTab(tree, "left", "right", "b");

    expect(findLeaf(next, "left")).toEqual({
      kind: "leaf",
      id: "left",
      tabIds: ["a"],
      activeTabId: "a",
    });
    expect(findLeaf(next, "right")).toEqual({
      kind: "leaf",
      id: "right",
      tabIds: ["c", "b"],
      activeTabId: "b",
    });
    expect(findLeafByTab(next, "b")?.id).toBe("right");

    if (next.kind !== "split") {
      throw new Error("expected root split");
    }
    expect(next.sizes[0]).toBeCloseTo(2 / 3);
    expect(next.sizes[1]).toBeCloseTo(1 / 3);
    expect(tree.children).toEqual([
      leaf("left", ["a", "b"], "b"),
      leaf("right", ["c"], "c"),
    ]);
  });

  test("collect helpers return leaf and tab ids in tree order", () => {
    const tree = split(
      "root",
      "row",
      [
        leaf("a", ["a1", "a2"]),
        split("nested", "col", [leaf("b", []), leaf("c", ["c1"])], [1, 1]),
      ],
      [1, 1],
    );

    expect(collectLeafIds(tree)).toEqual(["a", "b", "c"]);
    expect(collectTabIds(tree)).toEqual(["a1", "a2", "c1"]);
  });

  test("reorderTabInLeaf moves tabs within a leaf without changing active tab", () => {
    const tree = split(
      "root",
      "row",
      [leaf("left", ["a", "b"], "b"), leaf("right", ["c", "d", "e"], "d")],
      [0.4, 0.6],
    );

    const next = reorderTabInLeaf(tree, "right", "e", "c");

    expect(next).not.toBe(tree);
    expect(findLeaf(next, "right")).toEqual(
      leaf("right", ["e", "c", "d"], "d"),
    );
    expect(findLeaf(next, "left")).toBe(tree.children[0]);
    expect(findLeaf(next, "left")).toEqual(leaf("left", ["a", "b"], "b"));
    expect(tree.children[1]).toEqual(leaf("right", ["c", "d", "e"], "d"));
  });

  test("reorderTabInLeaf is a no-op when source or target is missing", () => {
    const tree = split(
      "root",
      "row",
      [leaf("left", ["a", "b"]), leaf("right", ["c", "d"])],
      [1, 1],
    );

    expect(reorderTabInLeaf(tree, "right", "missing", "c")).toBe(tree);
    expect(reorderTabInLeaf(tree, "right", "c", "missing")).toBe(tree);
    expect(reorderTabInLeaf(tree, "missing", "c", "d")).toBe(tree);
  });

  test("normalizeSizes clones leaves and normalizes invalid split sizes recursively", () => {
    const tree = split(
      "root",
      "row",
      [split("nested", "col", [leaf("a"), leaf("b")], [0, 0]), leaf("c")],
      [2, -1],
    );

    const next = normalizeSizes(tree);

    expect(next).not.toBe(tree);
    if (next.kind !== "split") {
      throw new Error("expected root split");
    }
    expect(next.sizes).toEqual([1, 0]);
    expect(sum(next.sizes)).toBeCloseTo(1);

    const nested = next.children[0];
    if (nested.kind !== "split") {
      throw new Error("expected nested split");
    }
    expect(nested.sizes).toEqual([0.5, 0.5]);
    expect(nested.children[0]).not.toBe(tree.children[0]);
  });

  test("root leaf stays as a single empty leaf when closed", () => {
    const root = leaf("root", ["a"], "a");

    const emptyRoot = closeLeaf(root, "root");

    expect(emptyRoot).toEqual({
      kind: "leaf",
      id: "root",
      tabIds: [],
      activeTabId: null,
    });
    expect(root).toEqual(leaf("root", ["a"], "a"));

    const splitRoot = split(
      "split-root",
      "row",
      [leaf("left", ["a"]), leaf("right", ["b"])],
      [0.5, 0.5],
    );
    const collapsed = closeLeaf(splitRoot, "left");
    const emptyCollapsedRoot = closeLeaf(collapsed, "right");

    expect(emptyCollapsedRoot).toEqual({
      kind: "leaf",
      id: "right",
      tabIds: [],
      activeTabId: null,
    });
  });

  test("unknown close target leaves the tree intact except for normalized sizes", () => {
    const tree = split(
      "root",
      "row",
      [leaf("a", ["a-tab"]), leaf("b", ["b-tab"])],
      [4, 1],
    );

    const next = closeLeaf(tree, "missing");

    expect(next).toEqual(
      split(
        "root",
        "row",
        [leaf("a", ["a-tab"]), leaf("b", ["b-tab"])],
        [0.8, 0.2],
      ),
    );
  });
});
