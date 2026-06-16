import { createSlice, PayloadAction } from "@reduxjs/toolkit";
import {
  closeLeaf,
  collectLeafIds,
  findLeaf,
  findLeafByTab,
  moveTab,
  normalizeSizes,
  splitLeaf,
  type LeafPane,
  type PaneNode,
  type SplitNode,
} from "./panesTree";

export type PanesState = {
  root: PaneNode;
  focusedLeafId: string;
};

export const INITIAL_PANE_LEAF_ID = "root";

const createInitialLeaf = (): LeafPane => ({
  kind: "leaf",
  id: INITIAL_PANE_LEAF_ID,
  tabIds: [],
  activeTabId: null,
});

const createInitialState = (): PanesState => ({
  root: createInitialLeaf(),
  focusedLeafId: INITIAL_PANE_LEAF_ID,
});

const unique = (ids: string[]): string[] => {
  const seen = new Set<string>();
  return ids.filter((id) => {
    if (seen.has(id)) return false;
    seen.add(id);
    return true;
  });
};

const normalizeLeaf = (leaf: LeafPane): LeafPane => {
  const tabIds = unique(leaf.tabIds);
  const activeTabId =
    leaf.activeTabId && tabIds.includes(leaf.activeTabId)
      ? leaf.activeTabId
      : tabIds[0] ?? null;

  return {
    ...leaf,
    tabIds,
    activeTabId,
  };
};

const normalizePaneTree = (node: PaneNode): PaneNode => {
  if (node.kind === "leaf") {
    return normalizeLeaf(node);
  }

  return normalizeSizes({
    ...node,
    children: node.children.map((child) => normalizePaneTree(child)),
  });
};

const normalizePaneRoot = (node: PaneNode): PaneNode => {
  const root = normalizePaneTree(node);
  return collectLeafIds(root).length === 0 ? createInitialLeaf() : root;
};

const mapLeaves = (
  node: PaneNode,
  update: (leaf: LeafPane) => LeafPane,
): PaneNode => {
  if (node.kind === "leaf") {
    return normalizeLeaf(update(node));
  }

  return normalizePaneTree({
    ...node,
    children: node.children.map((child) => mapLeaves(child, update)),
  });
};

const ensureFocusedLeaf = (
  state: PanesState,
  preferredLeafId?: string | null,
): void => {
  const leafIds = collectLeafIds(state.root);
  if (leafIds.length === 0) {
    const initial = createInitialState();
    state.root = initial.root;
    state.focusedLeafId = initial.focusedLeafId;
    return;
  }

  if (preferredLeafId && leafIds.includes(preferredLeafId)) {
    state.focusedLeafId = preferredLeafId;
    return;
  }

  if (!leafIds.includes(state.focusedLeafId)) {
    state.focusedLeafId = leafIds[0];
  }
};

const activeTabIds = (node: PaneNode): string[] => {
  if (node.kind === "leaf") {
    return node.activeTabId ? [node.activeTabId] : [];
  }

  return node.children.flatMap((child) => activeTabIds(child));
};

const removeTabFromTree = (root: PaneNode, tabId: string): PaneNode =>
  mapLeaves(root, (leaf) => {
    const tabIds = leaf.tabIds.filter((id) => id !== tabId);
    const activeTabId = tabIds.includes(leaf.activeTabId ?? "")
      ? leaf.activeTabId
      : tabIds[0] ?? null;

    return {
      ...leaf,
      tabIds,
      activeTabId,
    };
  });

const removeTabFromState = (state: PanesState, tabId: string): void => {
  state.root = removeTabFromTree(state.root, tabId);
  ensureFocusedLeaf(state);
};

type PruneResult = {
  node: PaneNode;
  changed: boolean;
};

const prunePaneNodeToOpenThreads = (
  node: PaneNode,
  openThreadIds: ReadonlySet<string>,
): PruneResult => {
  if (node.kind === "leaf") {
    const tabIds = node.tabIds.filter((id) => openThreadIds.has(id));
    const activeTabId = tabIds.includes(node.activeTabId ?? "")
      ? node.activeTabId
      : tabIds[0] ?? null;
    const changed =
      tabIds.length !== node.tabIds.length || activeTabId !== node.activeTabId;

    return {
      node: changed ? { ...node, tabIds, activeTabId } : node,
      changed,
    };
  }

  const children = node.children.map((child) =>
    prunePaneNodeToOpenThreads(child, openThreadIds),
  );
  const changed = children.some((child) => child.changed);

  return {
    node: changed
      ? normalizeSizes({
          ...node,
          children: children.map((child) => child.node),
        })
      : node,
    changed,
  };
};

export const reconcilePanesWithOpenThreads = (
  state: PanesState,
  openThreadIds: string[],
): PanesState => {
  const result = prunePaneNodeToOpenThreads(state.root, new Set(openThreadIds));

  if (!result.changed && findLeaf(state.root, state.focusedLeafId)) {
    return state;
  }

  const nextState: PanesState = {
    root: result.node,
    focusedLeafId: state.focusedLeafId,
  };
  ensureFocusedLeaf(nextState);
  return nextState;
};

const setLeafActiveTab = (
  root: PaneNode,
  leafId: string,
  tabId: string,
): PaneNode =>
  mapLeaves(root, (leaf) => {
    if (leaf.id !== leafId || !leaf.tabIds.includes(tabId)) {
      return leaf;
    }

    return {
      ...leaf,
      activeTabId: tabId,
    };
  });

const addTabToLeaf = (
  root: PaneNode,
  leafId: string,
  tabId: string,
): PaneNode =>
  mapLeaves(root, (leaf) => {
    if (leaf.id !== leafId) {
      if (!leaf.tabIds.includes(tabId)) {
        return leaf;
      }

      const tabIds = leaf.tabIds.filter((id) => id !== tabId);
      const activeTabId = tabIds.includes(leaf.activeTabId ?? "")
        ? leaf.activeTabId
        : tabIds[0] ?? null;

      return {
        ...leaf,
        tabIds,
        activeTabId,
      };
    }

    const tabIds = leaf.tabIds.includes(tabId)
      ? leaf.tabIds
      : [...leaf.tabIds, tabId];

    return {
      ...leaf,
      tabIds,
      activeTabId: tabId,
    };
  });

const resizeSplitNode = (
  node: PaneNode,
  splitId: string,
  sizes: number[],
): PaneNode => {
  if (node.kind === "leaf") {
    return normalizeLeaf(node);
  }

  const children = node.children.map((child) =>
    resizeSplitNode(child, splitId, sizes),
  );

  return normalizePaneTree({
    ...node,
    children,
    sizes: node.id === splitId ? sizes : node.sizes,
  });
};

export const panesSlice = createSlice({
  name: "panes",
  reducerPath: "panes",
  initialState: createInitialState(),
  reducers: {
    splitPane: (
      state,
      action: PayloadAction<{
        leafId: string;
        dir: SplitNode["dir"];
        tabId: string;
      }>,
    ) => {
      const { leafId, dir, tabId } = action.payload;
      if (!findLeaf(state.root, leafId)) return;

      const previousLeafIds = collectLeafIds(state.root);
      state.root = normalizePaneRoot(splitLeaf(state.root, leafId, dir, tabId));
      const nextLeafId = collectLeafIds(state.root).find(
        (id) => !previousLeafIds.includes(id),
      );
      ensureFocusedLeaf(state, nextLeafId ?? leafId);
    },
    setPaneActiveTab: (
      state,
      action: PayloadAction<{ leafId: string; tabId: string }>,
    ) => {
      const { leafId, tabId } = action.payload;
      state.root = setLeafActiveTab(state.root, leafId, tabId);
      ensureFocusedLeaf(state);
    },
    focusPane: (state, action: PayloadAction<string>) => {
      ensureFocusedLeaf(state, action.payload);
    },
    closePane: (state, action: PayloadAction<string>) => {
      if (!findLeaf(state.root, action.payload)) return;
      state.root = normalizePaneRoot(closeLeaf(state.root, action.payload));
      ensureFocusedLeaf(state);
    },
    moveTabToPane: (
      state,
      action: PayloadAction<{
        fromLeafId: string;
        toLeafId: string;
        tabId: string;
      }>,
    ) => {
      const { fromLeafId, toLeafId, tabId } = action.payload;
      const fromLeaf = findLeaf(state.root, fromLeafId);
      const toLeaf = findLeaf(state.root, toLeafId);
      if (!fromLeaf || !toLeaf || !fromLeaf.tabIds.includes(tabId)) return;

      state.root = normalizePaneRoot(
        moveTab(state.root, fromLeafId, toLeafId, tabId),
      );
      ensureFocusedLeaf(state, toLeafId);
    },
    addTabToFocusedPane: (state, action: PayloadAction<string>) => {
      ensureFocusedLeaf(state);
      state.root = addTabToLeaf(
        state.root,
        state.focusedLeafId,
        action.payload,
      );
      ensureFocusedLeaf(state, state.focusedLeafId);
    },
    removeTabEverywhere: (state, action: PayloadAction<string>) => {
      removeTabFromState(state, action.payload);
    },
    resizeSplit: (
      state,
      action: PayloadAction<{ splitId: string; sizes: number[] }>,
    ) => {
      state.root = normalizePaneRoot(
        resizeSplitNode(
          state.root,
          action.payload.splitId,
          action.payload.sizes,
        ),
      );
      ensureFocusedLeaf(state);
    },
    hydratePaneLayout: (
      state,
      action: PayloadAction<{ root: PaneNode; focusedLeafId: string }>,
    ) => {
      state.root = normalizePaneRoot(action.payload.root);
      state.focusedLeafId = action.payload.focusedLeafId;
      ensureFocusedLeaf(state);
    },
  },
});

export const {
  splitPane,
  setPaneActiveTab,
  focusPane,
  closePane,
  moveTabToPane,
  addTabToFocusedPane,
  removeTabEverywhere,
  resizeSplit,
  hydratePaneLayout,
} = panesSlice.actions;

type PanesRootState = {
  panes: PanesState;
};

export const selectPaneRoot = (state: PanesRootState) => state.panes.root;

export const selectFocusedLeafId = (state: PanesRootState) =>
  state.panes.focusedLeafId;

export const selectFocusedActiveTabId = (state: PanesRootState) =>
  findLeaf(state.panes.root, state.panes.focusedLeafId)?.activeTabId ?? null;

export const selectVisibleThreadIds = (state: PanesRootState) =>
  unique(activeTabIds(state.panes.root));

export const selectLeafForTab = (state: PanesRootState, tabId: string) =>
  findLeafByTab(state.panes.root, tabId);

export default panesSlice.reducer;
