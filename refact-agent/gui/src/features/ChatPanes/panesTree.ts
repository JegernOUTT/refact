export type LeafPane = {
  kind: "leaf";
  id: string;
  tabIds: string[];
  activeTabId: string | null;
};

export type SplitNode = {
  kind: "split";
  id: string;
  dir: "row" | "col";
  children: PaneNode[];
  sizes: number[];
};

export type PaneNode = LeafPane | SplitNode;

export type SplitPlacement = "before" | "after";

const makeSiblingLeafId = (leafId: string, tabId: string): string =>
  `${leafId}:sibling:${tabId}`;

const makeSplitId = (leafId: string, dir: SplitNode["dir"]): string =>
  `${leafId}:split:${dir}`;

const evenSizes = (count: number): number[] => {
  if (count === 0) {
    return [];
  }

  return Array.from({ length: count }, () => 1 / count);
};

const normalizedSizeValues = (sizes: number[], count: number): number[] => {
  if (count === 0) {
    return [];
  }

  if (sizes.length !== count) {
    return evenSizes(count);
  }

  const positiveSizes = sizes.map((size) =>
    Number.isFinite(size) && size > 0 ? size : 0,
  );
  const sum = positiveSizes.reduce((total, size) => total + size, 0);

  if (sum === 0) {
    return evenSizes(count);
  }

  return positiveSizes.map((size) => size / sum);
};

const cloneLeaf = (leaf: LeafPane): LeafPane => ({
  ...leaf,
  tabIds: [...leaf.tabIds],
});

const leafWithoutTab = (leaf: LeafPane, tabId: string): LeafPane => {
  const tabIds = leaf.tabIds.filter((id) => id !== tabId);
  const activeTabId = tabIds.includes(leaf.activeTabId ?? "")
    ? leaf.activeTabId
    : tabIds[0] ?? null;

  return {
    ...leaf,
    tabIds,
    activeTabId,
  };
};

const addTabToLeaf = (leaf: LeafPane, tabId: string): LeafPane => {
  const tabIds = leaf.tabIds.includes(tabId)
    ? [...leaf.tabIds]
    : [...leaf.tabIds, tabId];

  return {
    ...leaf,
    tabIds,
    activeTabId: tabId,
  };
};

export function normalizeSizes(node: PaneNode): PaneNode {
  if (node.kind === "leaf") {
    return cloneLeaf(node);
  }

  const children = node.children.map((child) => normalizeSizes(child));

  return {
    ...node,
    children,
    sizes: normalizedSizeValues(node.sizes, children.length),
  };
}

export function findLeaf(tree: PaneNode, leafId: string): LeafPane | null {
  if (tree.kind === "leaf") {
    return tree.id === leafId ? tree : null;
  }

  for (const child of tree.children) {
    const found = findLeaf(child, leafId);
    if (found) {
      return found;
    }
  }

  return null;
}

export function findLeafByTab(tree: PaneNode, tabId: string): LeafPane | null {
  if (tree.kind === "leaf") {
    return tree.tabIds.includes(tabId) ? tree : null;
  }

  for (const child of tree.children) {
    const found = findLeafByTab(child, tabId);
    if (found) {
      return found;
    }
  }

  return null;
}

export function collectTabIds(tree: PaneNode): string[] {
  if (tree.kind === "leaf") {
    return [...tree.tabIds];
  }

  return tree.children.flatMap((child) => collectTabIds(child));
}

export function collectLeafIds(tree: PaneNode): string[] {
  if (tree.kind === "leaf") {
    return [tree.id];
  }

  return tree.children.flatMap((child) => collectLeafIds(child));
}

export function splitLeaf(
  tree: PaneNode,
  leafId: string,
  dir: SplitNode["dir"],
  movedTabId: string,
  placement: SplitPlacement = "after",
): PaneNode {
  if (tree.kind === "leaf") {
    if (tree.id !== leafId) {
      return cloneLeaf(tree);
    }

    const originalLeaf = leafWithoutTab(tree, movedTabId);
    const siblingLeaf: LeafPane = {
      kind: "leaf",
      id: makeSiblingLeafId(leafId, movedTabId),
      tabIds: [movedTabId],
      activeTabId: movedTabId,
    };

    return {
      kind: "split",
      id: makeSplitId(leafId, dir),
      dir,
      children:
        placement === "before"
          ? [siblingLeaf, originalLeaf]
          : [originalLeaf, siblingLeaf],
      sizes: evenSizes(2),
    };
  }

  const children = tree.children.map((child) =>
    splitLeaf(child, leafId, dir, movedTabId, placement),
  );

  return {
    ...tree,
    children,
    sizes: normalizedSizeValues(tree.sizes, children.length),
  };
}

type CloseResult = {
  node: PaneNode;
  size: number;
} | null;

function closeLeafNode(node: PaneNode, leafId: string, size = 1): CloseResult {
  if (node.kind === "leaf") {
    if (node.id !== leafId) {
      return { node: normalizeSizes(node), size };
    }

    return null;
  }

  const childSizes = normalizedSizeValues(node.sizes, node.children.length);
  const children = node.children.flatMap((child, index) => {
    const nextChild = closeLeafNode(child, leafId, childSizes[index]);
    return nextChild ? [nextChild] : [];
  });

  if (children.length === 0) {
    return null;
  }

  if (children.length === 1) {
    return { node: children[0].node, size };
  }

  return {
    node: {
      ...node,
      children: children.map((child) => child.node),
      sizes: normalizedSizeValues(
        children.map((child) => child.size),
        children.length,
      ),
    },
    size,
  };
}

export function closeLeaf(tree: PaneNode, leafId: string): PaneNode {
  if (tree.kind === "leaf" && tree.id === leafId) {
    return {
      ...tree,
      tabIds: [],
      activeTabId: null,
    };
  }

  const targetExists = findLeaf(tree, leafId) !== null;
  const nextTree = closeLeafNode(tree, leafId);

  if (nextTree) {
    return nextTree.node;
  }

  if (targetExists) {
    return {
      kind: "leaf",
      id: leafId,
      tabIds: [],
      activeTabId: null,
    };
  }

  return normalizeSizes(tree);
}

export function moveTab(
  tree: PaneNode,
  fromLeafId: string,
  toLeafId: string,
  tabId: string,
): PaneNode {
  const fromLeaf = findLeaf(tree, fromLeafId);
  const toLeaf = findLeaf(tree, toLeafId);

  if (!fromLeaf || !toLeaf || !fromLeaf.tabIds.includes(tabId)) {
    return normalizeSizes(tree);
  }

  const visit = (node: PaneNode): PaneNode => {
    if (node.kind === "leaf") {
      if (node.id === fromLeafId && node.id === toLeafId) {
        return addTabToLeaf(leafWithoutTab(node, tabId), tabId);
      }

      if (node.id === fromLeafId) {
        return leafWithoutTab(node, tabId);
      }

      if (node.id === toLeafId) {
        return addTabToLeaf(node, tabId);
      }

      return cloneLeaf(node);
    }

    const children = node.children.map((child) => visit(child));

    return {
      ...node,
      children,
      sizes: normalizedSizeValues(node.sizes, children.length),
    };
  };

  return visit(tree);
}

export function reorderTabInLeaf(
  tree: PaneNode,
  leafId: string,
  sourceTabId: string,
  targetTabId: string,
): PaneNode {
  if (tree.kind === "leaf") {
    if (tree.id !== leafId) {
      return tree;
    }

    const sourceIndex = tree.tabIds.indexOf(sourceTabId);
    const targetIndex = tree.tabIds.indexOf(targetTabId);

    if (
      sourceIndex === -1 ||
      targetIndex === -1 ||
      sourceIndex === targetIndex
    ) {
      return tree;
    }

    const tabIds = [...tree.tabIds];
    const [source] = tabIds.splice(sourceIndex, 1);
    tabIds.splice(targetIndex, 0, source);

    return {
      ...tree,
      tabIds,
      activeTabId: tree.activeTabId,
    };
  }

  const children = tree.children.map((child) =>
    reorderTabInLeaf(child, leafId, sourceTabId, targetTabId),
  );

  if (children.every((child, index) => child === tree.children[index])) {
    return tree;
  }

  return {
    ...tree,
    children,
    sizes: [...tree.sizes],
  };
}
