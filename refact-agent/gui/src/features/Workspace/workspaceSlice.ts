import { createSlice, PayloadAction } from "@reduxjs/toolkit";
import {
  closeLeaf,
  collectLeafIds,
  collectTabIds,
  findLeaf,
  normalizeSizes,
  type LeafPane,
  type PaneNode,
  type SplitPlacement,
  type SplitNode,
} from "../ChatPanes/panesTree";
import { isChatSurface, type SurfaceKey } from "./surfaceKey";

export type PaneGroup = {
  root: PaneNode;
  focusedLeafId: string;
};

export type WorkspaceGroups = Record<SurfaceKey, PaneGroup | undefined>;

export type WorkspaceState = {
  tabs: SurfaceKey[];
  activeTabId: SurfaceKey | null;
  groups: WorkspaceGroups;
};

export const INITIAL_WORKSPACE_LEAF_ID = "root";
export const MAX_WORKSPACE_TABS = 30;
export const MAX_GROUP_LEAVES = 6;

const initialState: WorkspaceState = {
  tabs: [],
  activeTabId: null,
  groups: {},
};

const createLeaf = (
  id: string,
  tabIds: SurfaceKey[] = [],
  activeTabId: SurfaceKey | null = tabIds[0] ?? null,
): LeafPane => ({
  kind: "leaf",
  id,
  tabIds,
  activeTabId,
});

const unique = <T>(items: T[]): T[] => {
  const seen = new Set<T>();
  return items.filter((item) => {
    if (seen.has(item)) return false;
    seen.add(item);
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

const normalizePaneRoot = (node: PaneNode): PaneNode => normalizePaneTree(node);

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

const ensureGroupFocus = (
  group: PaneGroup,
  preferredLeafId?: string | null,
): PaneGroup => {
  const leafIds = collectLeafIds(group.root);
  const focusedLeafId =
    preferredLeafId && leafIds.includes(preferredLeafId)
      ? preferredLeafId
      : leafIds.includes(group.focusedLeafId)
        ? group.focusedLeafId
        : leafIds[0] ?? INITIAL_WORKSPACE_LEAF_ID;

  return {
    root: group.root,
    focusedLeafId,
  };
};

const normalizeGroup = (group: PaneGroup): PaneGroup =>
  ensureGroupFocus({
    root: normalizePaneRoot(group.root),
    focusedLeafId: group.focusedLeafId,
  });

const activeSurfaceKeys = (node: PaneNode): SurfaceKey[] => {
  if (node.kind === "leaf") {
    return node.activeTabId ? [node.activeTabId] : [];
  }

  return node.children.flatMap((child) => activeSurfaceKeys(child));
};

const allSurfaceKeys = (node: PaneNode): SurfaceKey[] =>
  unique(collectTabIds(node));

const withoutGroup = (
  groups: WorkspaceGroups,
  key: SurfaceKey,
): WorkspaceGroups => {
  const { [key]: _removed, ...rest } = groups;
  return rest;
};

const firstSurfaceKey = (node: PaneNode): SurfaceKey | null => {
  const [activeKey] = activeSurfaceKeys(node);
  if (activeKey) return activeKey;

  const surfaceKeys = allSurfaceKeys(node);
  return surfaceKeys.length > 0 ? surfaceKeys[0] : null;
};

const findFirstEmptyLeafId = (node: PaneNode): string | null => {
  if (node.kind === "leaf") {
    return node.tabIds.length === 0 ? node.id : null;
  }

  for (const child of node.children) {
    const found = findFirstEmptyLeafId(child);
    if (found) return found;
  }

  return null;
};

const collapseEmptyLeaves = (root: PaneNode): PaneNode => {
  if (collectTabIds(root).length === 0) {
    return normalizePaneRoot(root);
  }

  let next = normalizePaneRoot(root);

  while (next.kind === "split") {
    const emptyLeafId = findFirstEmptyLeafId(next);
    if (!emptyLeafId) return normalizePaneRoot(next);
    next = normalizePaneRoot(closeLeaf(next, emptyLeafId));
  }

  return normalizePaneRoot(next);
};

const setLeafActiveSurface = (
  root: PaneNode,
  leafId: string,
  surfaceKey: SurfaceKey,
): PaneNode =>
  mapLeaves(root, (leaf) => {
    if (leaf.id !== leafId || !leaf.tabIds.includes(surfaceKey)) {
      return leaf;
    }

    return {
      ...leaf,
      activeTabId: surfaceKey,
    };
  });

const removeSurfaceFromTree = (
  root: PaneNode,
  surfaceKey: SurfaceKey,
): PaneNode =>
  mapLeaves(root, (leaf) => {
    const tabIds = leaf.tabIds.filter((key) => key !== surfaceKey);
    const activeTabId = tabIds.includes(leaf.activeTabId ?? "")
      ? leaf.activeTabId
      : tabIds[0] ?? null;

    return {
      ...leaf,
      tabIds,
      activeTabId,
    };
  });

const addSurfaceToLeaf = (
  root: PaneNode,
  leafId: string,
  surfaceKey: SurfaceKey,
): PaneNode =>
  mapLeaves(root, (leaf) => {
    const tabIdsWithoutSurface = leaf.tabIds.filter(
      (key) => key !== surfaceKey,
    );

    if (leaf.id !== leafId) {
      return {
        ...leaf,
        tabIds: tabIdsWithoutSurface,
        activeTabId: tabIdsWithoutSurface.includes(leaf.activeTabId ?? "")
          ? leaf.activeTabId
          : tabIdsWithoutSurface[0] ?? null,
      };
    }

    return {
      ...leaf,
      tabIds: [...tabIdsWithoutSurface, surfaceKey],
      activeTabId: surfaceKey,
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

const makeSiblingLeafId = (leafId: string, surfaceKey: SurfaceKey): string =>
  `${leafId}:sibling:${surfaceKey}`;

const makeSplitId = (leafId: string, dir: SplitNode["dir"]): string =>
  `${leafId}:split:${dir}`;

const paneNodeIdExists = (node: PaneNode, id: string): boolean => {
  if (node.id === id) return true;
  if (node.kind === "leaf") return false;
  return node.children.some((child) => paneNodeIdExists(child, id));
};

const uniquePaneNodeId = (root: PaneNode, baseId: string): string => {
  if (!paneNodeIdExists(root, baseId)) return baseId;

  let index = 2;
  let nextId = `${baseId}:${index}`;
  while (paneNodeIdExists(root, nextId)) {
    index += 1;
    nextId = `${baseId}:${index}`;
  }

  return nextId;
};

const replaceLeafWithNode = (
  node: PaneNode,
  leafId: string,
  replacement: PaneNode,
): PaneNode => {
  if (node.kind === "leaf") {
    return node.id === leafId ? replacement : normalizeLeaf(node);
  }

  return normalizePaneTree({
    ...node,
    children: node.children.map((child) =>
      replaceLeafWithNode(child, leafId, replacement),
    ),
  });
};

const splitLeafWithEmptySibling = (
  root: PaneNode,
  leafId: string,
  dir: SplitNode["dir"],
  surfaceKey: SurfaceKey,
  placement?: SplitPlacement,
): { root: PaneNode; siblingLeafId: string } | null => {
  const targetLeaf = findLeaf(root, leafId);
  if (!targetLeaf || !targetLeaf.tabIds.includes(surfaceKey)) {
    return null;
  }

  const siblingLeafId = uniquePaneNodeId(
    root,
    makeSiblingLeafId(leafId, surfaceKey),
  );
  const splitId = uniquePaneNodeId(root, makeSplitId(leafId, dir));
  const keptLeaf = normalizeLeaf(targetLeaf);
  const siblingLeaf = createLeaf(siblingLeafId);
  const splitNode: SplitNode = {
    kind: "split",
    id: splitId,
    dir,
    children:
      placement === "before"
        ? [siblingLeaf, keptLeaf]
        : [keptLeaf, siblingLeaf],
    sizes: [0.5, 0.5],
  };

  return {
    root: normalizePaneRoot(replaceLeafWithNode(root, leafId, splitNode)),
    siblingLeafId,
  };
};

const reorderItems = <T>(items: T[], source: T, target: T): T[] => {
  const sourceIndex = items.indexOf(source);
  const targetIndex = items.indexOf(target);

  if (sourceIndex === -1 || targetIndex === -1 || sourceIndex === targetIndex) {
    return items;
  }

  const next = [...items];
  const [item] = next.splice(sourceIndex, 1);
  next.splice(targetIndex, 0, item);
  return next;
};

const tabIndexOrEnd = (tabs: SurfaceKey[], key: SurfaceKey): number => {
  const index = tabs.indexOf(key);
  return index === -1 ? tabs.length : index;
};

const insertTabAt = (
  tabs: SurfaceKey[],
  key: SurfaceKey,
  index: number,
): SurfaceKey[] => {
  if (tabs.includes(key)) {
    return tabs;
  }

  if (tabs.length >= MAX_WORKSPACE_TABS) {
    return tabs;
  }

  const next = [...tabs];
  next.splice(Math.min(index, next.length), 0, key);
  return next;
};

const replaceTabWithSurfaceKeys = (
  state: WorkspaceState,
  tabId: SurfaceKey,
  surfaceKeys: SurfaceKey[],
  preferredSurfaceKey: SurfaceKey | null,
): void => {
  const tabIndex = tabIndexOrEnd(state.tabs, tabId);
  const uniqueSurfaceKeys = unique(surfaceKeys);
  const nextTabs = state.tabs.filter(
    (key) => key !== tabId && !uniqueSurfaceKeys.includes(key),
  );
  const slotsAvailable = Math.max(0, MAX_WORKSPACE_TABS - nextTabs.length);
  const insertedKeys = uniqueSurfaceKeys.slice(0, slotsAvailable);

  nextTabs.splice(Math.min(tabIndex, nextTabs.length), 0, ...insertedKeys);
  state.tabs = nextTabs;

  if (
    state.activeTabId === tabId ||
    !state.activeTabId ||
    !state.tabs.includes(state.activeTabId)
  ) {
    state.activeTabId =
      preferredSurfaceKey && state.tabs.includes(preferredSurfaceKey)
        ? preferredSurfaceKey
        : insertedKeys[0] ?? state.tabs[0];
  }
};

const removeTopLevelTab = (
  state: WorkspaceState,
  key: SurfaceKey,
): SurfaceKey | null => {
  const index = state.tabs.indexOf(key);
  if (index === -1) return null;

  const nextTabs = state.tabs.filter((tab) => tab !== key);
  const neighbor =
    index < nextTabs.length
      ? nextTabs[index]
      : nextTabs.length > 0
        ? nextTabs[nextTabs.length - 1]
        : null;
  state.tabs = nextTabs;
  return neighbor;
};

const removeTopLevelTabKeepingGroup = (
  state: WorkspaceState,
  key: SurfaceKey,
): SurfaceKey | null => {
  const group = state.groups[key];
  const neighbor = removeTopLevelTab(state, key);

  if (group) {
    state.groups[key] = group;
  }

  return neighbor;
};

const writeGroup = (
  state: WorkspaceState,
  tabId: SurfaceKey,
  group: PaneGroup,
  collapseEmpty: boolean,
  dropEmptyTab = false,
): void => {
  const normalizedGroup = normalizeGroup({
    root: collapseEmpty ? collapseEmptyLeaves(group.root) : group.root,
    focusedLeafId: group.focusedLeafId,
  });
  const leafCount = collectLeafIds(normalizedGroup.root).length;
  const surfaceCount = collectTabIds(normalizedGroup.root).length;
  const tabIndex = tabIndexOrEnd(state.tabs, tabId);

  if (surfaceCount === 0) {
    state.groups = withoutGroup(state.groups, tabId);
    if (dropEmptyTab) {
      removeTopLevelTab(state, tabId);
    }
    return;
  }

  if (leafCount <= 1) {
    const remainingSurfaceKeys = allSurfaceKeys(normalizedGroup.root);
    const remainingSurface = firstSurfaceKey(normalizedGroup.root);
    state.groups = withoutGroup(state.groups, tabId);
    replaceTabWithSurfaceKeys(
      state,
      tabId,
      remainingSurfaceKeys,
      remainingSurface,
    );

    return;
  }

  if (leafCount > MAX_GROUP_LEAVES) {
    state.groups = withoutGroup(state.groups, tabId);
    return;
  }

  if (!state.tabs.includes(tabId)) {
    const replacementKey = firstSurfaceKey(normalizedGroup.root);
    if (!replacementKey) {
      state.groups = withoutGroup(state.groups, tabId);
      return;
    }

    state.groups = withoutGroup(state.groups, tabId);
    state.tabs = insertTabAt(state.tabs, replacementKey, tabIndex);
    state.groups[replacementKey] = normalizedGroup;
    if (state.activeTabId === tabId) {
      state.activeTabId = replacementKey;
    }
    return;
  }

  state.groups[tabId] = normalizedGroup;
};

const removeSurfaceFromGroups = (
  state: WorkspaceState,
  surfaceKey: SurfaceKey,
): void => {
  for (const tabId of Object.keys(state.groups)) {
    const group = state.groups[tabId];
    if (!group) continue;

    writeGroup(
      state,
      tabId,
      {
        root: removeSurfaceFromTree(group.root, surfaceKey),
        focusedLeafId: group.focusedLeafId,
      },
      true,
    );
  }
};

const detachSurfaceForGroup = (
  state: WorkspaceState,
  tabId: SurfaceKey,
  surfaceKey: SurfaceKey,
): void => {
  if (surfaceKey !== tabId) {
    const sourceGroup = state.groups[surfaceKey];
    if (sourceGroup) {
      removeTopLevelTabKeepingGroup(state, surfaceKey);
      writeGroup(
        state,
        surfaceKey,
        {
          root: removeSurfaceFromTree(sourceGroup.root, surfaceKey),
          focusedLeafId: sourceGroup.focusedLeafId,
        },
        true,
        true,
      );
    } else {
      removeTopLevelTab(state, surfaceKey);
    }
  }

  for (const groupTabId of Object.keys(state.groups)) {
    if (groupTabId === tabId) continue;
    const otherGroup = state.groups[groupTabId];
    if (!otherGroup) continue;
    writeGroup(
      state,
      groupTabId,
      {
        root: removeSurfaceFromTree(otherGroup.root, surfaceKey),
        focusedLeafId: otherGroup.focusedLeafId,
      },
      true,
    );
  }
};

const openChatSurface = (
  key: SurfaceKey,
  openThreadIds: ReadonlySet<string>,
): boolean =>
  !isChatSurface(key) || openThreadIds.has(key.slice("chat:".length));

const pruneNodeToOpenThreads = (
  node: PaneNode,
  openThreadIds: ReadonlySet<string>,
): { node: PaneNode; changed: boolean } => {
  if (node.kind === "leaf") {
    const tabIds = node.tabIds.filter((key) =>
      openChatSurface(key, openThreadIds),
    );
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
    pruneNodeToOpenThreads(child, openThreadIds),
  );
  const changed = children.some((child) => child.changed);

  return {
    node: changed
      ? normalizePaneTree({
          ...node,
          children: children.map((child) => child.node),
        })
      : node,
    changed,
  };
};

const normalizeHydratedGroup = (group: PaneGroup): PaneGroup | null => {
  const normalized = normalizeGroup(group);
  const leafCount = collectLeafIds(normalized.root).length;
  const surfaceCount = collectTabIds(normalized.root).length;

  if (
    leafCount < 2 ||
    leafCount > MAX_GROUP_LEAVES ||
    surfaceCount === 0 ||
    surfaceCount > MAX_WORKSPACE_TABS
  ) {
    return null;
  }

  return normalized;
};

export const reconcileWorkspaceState = (
  state: WorkspaceState,
  openThreadIds: string[],
): WorkspaceState => {
  const openThreads = new Set(openThreadIds);
  const nextState: WorkspaceState = {
    tabs: unique(state.tabs)
      .filter((key) => openChatSurface(key, openThreads))
      .slice(0, MAX_WORKSPACE_TABS),
    activeTabId: state.activeTabId,
    groups: {},
  };

  for (const [tabId, group] of Object.entries(state.groups)) {
    if (!group) continue;

    const pruned = pruneNodeToOpenThreads(group.root, openThreads);
    const nextGroup = {
      root: pruned.node,
      focusedLeafId: group.focusedLeafId,
    };
    nextState.groups[tabId] = nextGroup;
    writeGroup(nextState, tabId, nextGroup, pruned.changed, true);
  }

  if (
    !nextState.activeTabId ||
    !nextState.tabs.includes(nextState.activeTabId)
  ) {
    nextState.activeTabId = nextState.tabs[0] ?? null;
  }

  return nextState;
};

export const workspaceSlice = createSlice({
  name: "workspace",
  reducerPath: "workspace",
  initialState,
  reducers: {
    openTab: (state, action: PayloadAction<SurfaceKey>) => {
      if (!state.tabs.includes(action.payload)) {
        if (state.tabs.length >= MAX_WORKSPACE_TABS) return;
        state.tabs.push(action.payload);
      }

      state.activeTabId = action.payload;
    },
    closeTab: (state, action: PayloadAction<SurfaceKey>) => {
      const neighbor = removeTopLevelTab(state, action.payload);
      state.groups = withoutGroup(state.groups, action.payload);
      removeSurfaceFromGroups(state, action.payload);

      if (state.activeTabId === action.payload) {
        state.activeTabId =
          neighbor && state.tabs.includes(neighbor)
            ? neighbor
            : state.tabs[0] ?? null;
      } else if (state.activeTabId && !state.tabs.includes(state.activeTabId)) {
        state.activeTabId = state.tabs[0] ?? null;
      }
    },
    setActiveTab: (state, action: PayloadAction<SurfaceKey>) => {
      if (!state.tabs.includes(action.payload)) return;
      state.activeTabId = action.payload;
    },
    reorderTabs: (
      state,
      action: PayloadAction<{ sourceKey: SurfaceKey; targetKey: SurfaceKey }>,
    ) => {
      state.tabs = reorderItems(
        state.tabs,
        action.payload.sourceKey,
        action.payload.targetKey,
      );
    },
    splitTab: (
      state,
      action: PayloadAction<{
        tabId: SurfaceKey;
        dir: SplitNode["dir"];
        placement?: SplitPlacement;
      }>,
    ) => {
      const { tabId, dir, placement } = action.payload;
      if (!state.tabs.includes(tabId)) return;

      const currentGroup =
        state.groups[tabId] ??
        normalizeGroup({
          root: createLeaf(INITIAL_WORKSPACE_LEAF_ID, [tabId], tabId),
          focusedLeafId: INITIAL_WORKSPACE_LEAF_ID,
        });
      const leafCount = collectLeafIds(currentGroup.root).length;
      if (leafCount >= MAX_GROUP_LEAVES) return;

      const focusedLeaf = findLeaf(
        currentGroup.root,
        currentGroup.focusedLeafId,
      );
      const surfaceKey =
        focusedLeaf?.activeTabId ?? focusedLeaf?.tabIds[0] ?? null;
      if (!focusedLeaf || !surfaceKey) return;

      const splitResult = splitLeafWithEmptySibling(
        currentGroup.root,
        currentGroup.focusedLeafId,
        dir,
        surfaceKey,
        placement,
      );
      if (!splitResult) return;

      const nextRoot = splitResult.root;
      const nextLeafIds = collectLeafIds(nextRoot);
      if (nextLeafIds.length === leafCount) return;

      state.groups[tabId] = ensureGroupFocus(
        {
          root: nextRoot,
          focusedLeafId: currentGroup.focusedLeafId,
        },
        splitResult.siblingLeafId,
      );
    },
    closePane: (
      state,
      action: PayloadAction<{ tabId: SurfaceKey; leafId: string }>,
    ) => {
      const group = state.groups[action.payload.tabId];
      if (!group || !findLeaf(group.root, action.payload.leafId)) return;

      writeGroup(
        state,
        action.payload.tabId,
        {
          root: closeLeaf(group.root, action.payload.leafId),
          focusedLeafId: group.focusedLeafId,
        },
        true,
      );

      const nextGroup = state.groups[action.payload.tabId];
      if (nextGroup) {
        state.groups[action.payload.tabId] = ensureGroupFocus(nextGroup);
      }

      if (state.activeTabId && !state.tabs.includes(state.activeTabId)) {
        state.activeTabId = state.tabs[0] ?? null;
      }
    },
    setPaneActive: (
      state,
      action: PayloadAction<{
        tabId: SurfaceKey;
        leafId: string;
        surfaceKey: SurfaceKey;
      }>,
    ) => {
      const { tabId, leafId, surfaceKey } = action.payload;
      const group = state.groups[tabId];
      if (!group || !findLeaf(group.root, leafId)?.tabIds.includes(surfaceKey))
        return;

      state.groups[tabId] = ensureGroupFocus(
        {
          root: setLeafActiveSurface(group.root, leafId, surfaceKey),
          focusedLeafId: group.focusedLeafId,
        },
        leafId,
      );
    },
    focusPane: (
      state,
      action: PayloadAction<{ tabId: SurfaceKey; leafId: string }>,
    ) => {
      const group = state.groups[action.payload.tabId];
      if (!group || !findLeaf(group.root, action.payload.leafId)) return;
      state.groups[action.payload.tabId] = ensureGroupFocus(
        group,
        action.payload.leafId,
      );
    },
    addSurfaceToPane: (
      state,
      action: PayloadAction<{
        tabId: SurfaceKey;
        leafId: string;
        surfaceKey: SurfaceKey;
      }>,
    ) => {
      const { tabId, leafId, surfaceKey } = action.payload;
      const group = state.groups[tabId];
      if (!group || !findLeaf(group.root, leafId)) return;

      detachSurfaceForGroup(state, tabId, surfaceKey);

      const nextGroup = state.groups[tabId];
      if (!nextGroup) return;

      writeGroup(
        state,
        tabId,
        {
          root: addSurfaceToLeaf(nextGroup.root, leafId, surfaceKey),
          focusedLeafId: nextGroup.focusedLeafId,
        },
        true,
      );

      const writtenGroup = state.groups[tabId];
      if (writtenGroup) {
        state.groups[tabId] = ensureGroupFocus(writtenGroup, leafId);
        state.activeTabId = tabId;
      } else if (state.activeTabId && !state.tabs.includes(state.activeTabId)) {
        state.activeTabId = state.tabs[0] ?? null;
      }
    },
    splitPaneWithSurface: (
      state,
      action: PayloadAction<{
        tabId: SurfaceKey;
        leafId: string;
        surfaceKey: SurfaceKey;
        dir: SplitNode["dir"];
        placement?: SplitPlacement;
      }>,
    ) => {
      const { tabId, leafId, surfaceKey, dir, placement } = action.payload;
      const group = state.groups[tabId];
      if (!group || !findLeaf(group.root, leafId)) return;

      const leafCount = collectLeafIds(group.root).length;
      if (leafCount >= MAX_GROUP_LEAVES) return;

      detachSurfaceForGroup(state, tabId, surfaceKey);

      const nextGroup = state.groups[tabId];
      if (!nextGroup || !findLeaf(nextGroup.root, leafId)) return;
      const splitResult = splitLeafWithEmptySibling(
        addSurfaceToLeaf(nextGroup.root, leafId, surfaceKey),
        leafId,
        dir,
        surfaceKey,
        placement,
      );
      if (!splitResult) return;

      writeGroup(
        state,
        tabId,
        {
          root: addSurfaceToLeaf(
            splitResult.root,
            splitResult.siblingLeafId,
            surfaceKey,
          ),
          focusedLeafId: nextGroup.focusedLeafId,
        },
        true,
      );

      const writtenGroup = state.groups[tabId];
      if (writtenGroup) {
        state.groups[tabId] = ensureGroupFocus(
          writtenGroup,
          splitResult.siblingLeafId,
        );
      }
      state.activeTabId = tabId;
    },
    resizeGroupSplit: (
      state,
      action: PayloadAction<{
        tabId: SurfaceKey;
        splitId: string;
        sizes: number[];
      }>,
    ) => {
      const group = state.groups[action.payload.tabId];
      if (!group) return;

      state.groups[action.payload.tabId] = normalizeGroup({
        root: resizeSplitNode(
          group.root,
          action.payload.splitId,
          action.payload.sizes,
        ),
        focusedLeafId: group.focusedLeafId,
      });
    },
    reconcileWorkspace: (
      state,
      action: PayloadAction<{ openThreadIds: string[] }>,
    ) => reconcileWorkspaceState(state, action.payload.openThreadIds),
    hydrateWorkspace: (_state, action: PayloadAction<WorkspaceState>) => {
      const tabs = unique(action.payload.tabs).slice(0, MAX_WORKSPACE_TABS);
      const groups: Record<SurfaceKey, PaneGroup> = {};

      for (const [tabId, group] of Object.entries(action.payload.groups)) {
        if (!tabs.includes(tabId) || !group) continue;
        const normalized = normalizeHydratedGroup(group);
        if (!normalized) continue;
        groups[tabId] = normalized;
      }

      return {
        tabs,
        activeTabId:
          action.payload.activeTabId &&
          tabs.includes(action.payload.activeTabId)
            ? action.payload.activeTabId
            : tabs[0] ?? null,
        groups,
      };
    },
  },
});

export const {
  openTab,
  closeTab,
  setActiveTab,
  reorderTabs,
  splitTab,
  closePane,
  setPaneActive,
  focusPane,
  addSurfaceToPane,
  splitPaneWithSurface,
  resizeGroupSplit,
  reconcileWorkspace,
  hydrateWorkspace,
} = workspaceSlice.actions;

type WorkspaceRootState = {
  workspace: WorkspaceState;
};

export const selectTabs = (state: WorkspaceRootState) => state.workspace.tabs;

export const selectActiveTabId = (state: WorkspaceRootState) =>
  state.workspace.activeTabId;

export const selectGroupForTab = (
  state: WorkspaceRootState,
  tabId: SurfaceKey,
) => state.workspace.groups[tabId] ?? null;

export const selectWorkspaceGroups = (state: WorkspaceRootState) =>
  state.workspace.groups;

export const selectActiveGroup = (state: WorkspaceRootState) =>
  state.workspace.activeTabId
    ? selectGroupForTab(state, state.workspace.activeTabId)
    : null;

export const selectIsTabSplit = (
  state: WorkspaceRootState,
  tabId: SurfaceKey,
) => Boolean(state.workspace.groups[tabId]);

export const selectVisibleSurfaceKeys = (
  state: WorkspaceRootState,
): SurfaceKey[] => {
  const activeTabId = state.workspace.activeTabId;
  if (!activeTabId) return [];

  const group = state.workspace.groups[activeTabId];
  if (!group) return [activeTabId];

  return unique(activeSurfaceKeys(group.root));
};

export const selectVisibleThreadIds = (state: WorkspaceRootState): string[] =>
  selectVisibleSurfaceKeys(state)
    .filter(isChatSurface)
    .map((key) => key.slice("chat:".length));

export default workspaceSlice.reducer;
