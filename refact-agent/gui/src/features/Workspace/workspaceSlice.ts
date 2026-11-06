import { createSelector, createSlice, PayloadAction } from "@reduxjs/toolkit";
import {
  closeLeaf,
  collectLeafIds,
  collectTabIds,
  findLeaf,
  findLeafByTab,
  normalizeSizes,
  type LeafPane,
  type PaneNode,
  type SplitPlacement,
  type SplitNode,
} from "../ChatPanes/panesTree";
import {
  isChatSurface,
  isSplittableSurface,
  isDesignSurface,
  isFileSurface,
  isFilesSurface,
  isMainSurface,
  isTerminalSurface,
  isWorkspaceSurface,
  isWorkspaceSurfaceEnabled,
  type WorkspaceCapabilities,
  type SurfaceKey,
} from "./surfaceKey";

export type PaneGroup = {
  root: PaneNode;
  focusedLeafId: string;
};

export type WorkspaceGroups = Record<SurfaceKey, PaneGroup | undefined>;

export type WorkspaceState = {
  tabs: SurfaceKey[];
  activeTabId: SurfaceKey | null;
  groups: WorkspaceGroups;
  contextChatByTab?: Record<SurfaceKey, string>;
  panelsForced?: boolean;
  liveEditsByChat?: Record<string, boolean>;
  /** Sticky default for chats without an explicit toggle; last manual choice wins. */
  liveEditsDefault?: boolean;
  dock?: WorkspaceDockState;
  drawer?: WorkspaceDrawerState;
};

export type WorkspaceDockSection = "files" | "git" | "agents" | "activity";

export const normalizeWorkspaceDockSection = (
  section: unknown,
): WorkspaceDockSection | undefined => {
  if (
    section === "files" ||
    section === "git" ||
    section === "agents" ||
    section === "activity"
  ) {
    return section;
  }
  return section === "tasks" ? "activity" : undefined;
};

export type WorkspaceDockState = {
  open: boolean;
  width: number;
  section: WorkspaceDockSection;
};

export type WorkspaceDrawerState = {
  open: boolean;
  height: number;
};

export type WorkspaceHydrationState = Pick<
  WorkspaceState,
  "tabs" | "activeTabId" | "groups"
> &
  Partial<
    Pick<
      WorkspaceState,
      | "contextChatByTab"
      | "panelsForced"
      | "liveEditsByChat"
      | "liveEditsDefault"
      | "dock"
      | "drawer"
    >
  >;

export const INITIAL_WORKSPACE_LEAF_ID = "root";
export const MAX_WORKSPACE_TABS = 30;
export const MAX_GROUP_LEAVES = 6;
export const MAX_WORKSPACE_SURFACES = MAX_WORKSPACE_TABS * MAX_WORKSPACE_TABS;
export const DOCK_MIN_WIDTH = 240;
export const DOCK_MAX_WIDTH = 640;
export const DOCK_DEFAULT_WIDTH = 280;
export const DRAWER_MIN_HEIGHT = 160;
export const DRAWER_DEFAULT_HEIGHT = 220;

export const DEFAULT_WORKSPACE_DOCK: WorkspaceDockState = {
  open: true,
  width: DOCK_DEFAULT_WIDTH,
  section: "files",
};

export const DEFAULT_WORKSPACE_DRAWER: WorkspaceDrawerState = {
  open: false,
  height: DRAWER_DEFAULT_HEIGHT,
};

export const normalizeDockWidth = (value: number): number =>
  Number.isFinite(value)
    ? Math.min(DOCK_MAX_WIDTH, Math.max(DOCK_MIN_WIDTH, value))
    : DOCK_DEFAULT_WIDTH;

export const normalizeDrawerHeight = (value: number): number =>
  Number.isFinite(value)
    ? Math.max(DRAWER_MIN_HEIGHT, value)
    : DRAWER_DEFAULT_HEIGHT;

export const clampDrawerHeight = (
  value: number,
  viewportHeight: number,
): number => {
  const maxHeight = Number.isFinite(viewportHeight)
    ? Math.max(DRAWER_MIN_HEIGHT, viewportHeight * 0.6)
    : DRAWER_DEFAULT_HEIGHT;
  return Math.min(maxHeight, normalizeDrawerHeight(value));
};

export const normalizeWorkspaceDock = (
  dock?:
    | (Partial<Omit<WorkspaceDockState, "section">> & {
        section?: unknown;
      })
    | null,
): WorkspaceDockState => ({
  open:
    typeof dock?.open === "boolean" ? dock.open : DEFAULT_WORKSPACE_DOCK.open,
  width: normalizeDockWidth(dock?.width ?? DOCK_DEFAULT_WIDTH),
  section:
    normalizeWorkspaceDockSection(dock?.section) ??
    DEFAULT_WORKSPACE_DOCK.section,
});

export const normalizeWorkspaceDrawer = (
  drawer?: Partial<WorkspaceDrawerState> | null,
): WorkspaceDrawerState => ({
  open:
    typeof drawer?.open === "boolean"
      ? drawer.open
      : DEFAULT_WORKSPACE_DRAWER.open,
  height: normalizeDrawerHeight(drawer?.height ?? DRAWER_DEFAULT_HEIGHT),
});

const initialState: WorkspaceState = {
  tabs: [],
  activeTabId: null,
  groups: {},
  panelsForced: false,
  liveEditsByChat: undefined,
  liveEditsDefault: undefined,
  dock: { ...DEFAULT_WORKSPACE_DOCK },
  drawer: { ...DEFAULT_WORKSPACE_DRAWER },
};

const DEFAULT_WORKSPACE_CAPABILITIES: WorkspaceCapabilities = {
  filesPanel: true,
  gitPanel: true,
  terminalPanel: true,
};

const effectiveWorkspaceCapabilities = (
  state: WorkspaceState,
  workspaceCapabilities: WorkspaceCapabilities,
): WorkspaceCapabilities =>
  state.panelsForced ? DEFAULT_WORKSPACE_CAPABILITIES : workspaceCapabilities;

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

const focusedSurfaceKey = (group: PaneGroup): SurfaceKey | null => {
  const focusedLeaf = findLeaf(group.root, group.focusedLeafId);
  if (focusedLeaf?.activeTabId) return focusedLeaf.activeTabId;
  if (focusedLeaf?.tabIds[0]) return focusedLeaf.tabIds[0];
  return firstSurfaceKey(group.root);
};

const chatIdFromSurface = (surfaceKey: SurfaceKey | null): string | null =>
  surfaceKey && isChatSurface(surfaceKey)
    ? surfaceKey.slice("chat:".length)
    : null;

const contextTabForSurface = (
  state: WorkspaceState,
  surfaceKey: SurfaceKey,
): SurfaceKey => {
  for (const [tabId, group] of Object.entries(state.groups)) {
    if (group && collectTabIds(group.root).includes(surfaceKey)) return tabId;
  }
  return surfaceKey;
};

const isContextChatSurface = (key: SurfaceKey): boolean =>
  isChatSurface(key) ||
  isFileSurface(key) ||
  isDesignSurface(key) ||
  isMainSurface(key);

const setContextChatForTab = (
  state: WorkspaceState,
  tabId: SurfaceKey,
  chatId: string | null,
): void => {
  if (!chatId || !isContextChatSurface(tabId)) return;
  state.contextChatByTab = {
    ...state.contextChatByTab,
    [tabId]: chatId,
  };
};

const bindContextChatForSurface = (
  state: WorkspaceState,
  surfaceKey: SurfaceKey,
  chatId: string | null,
): void => {
  if (!chatId) return;
  if (!isContextChatSurface(surfaceKey)) return;
  if (!isChatSurface(surfaceKey)) {
    setContextChatForTab(state, surfaceKey, chatId);
    return;
  }
  const tabId = contextTabForSurface(state, surfaceKey);
  if (
    !isChatSurface(tabId) ||
    (tabId === surfaceKey && isChatSurface(surfaceKey))
  )
    return;
  const group = state.groups[tabId];
  if (group && !collectTabIds(group.root).includes(`chat:${chatId}`)) return;
  setContextChatForTab(state, tabId, chatId);
};

const removeContextChatForTab = (
  state: WorkspaceState,
  tabId: SurfaceKey,
): void => {
  if (!state.contextChatByTab?.[tabId]) return;
  const { [tabId]: _removed, ...rest } = state.contextChatByTab;
  state.contextChatByTab = Object.keys(rest).length > 0 ? rest : undefined;
};

const setContextChatForFocusedLeaf = (
  state: WorkspaceState,
  tabId: SurfaceKey,
  leafId: string,
): void => {
  const group = state.groups[tabId];
  const leaf = group ? findLeaf(group.root, leafId) : null;
  const surface = leaf?.activeTabId ?? leaf?.tabIds[0] ?? null;
  const chatId =
    chatIdFromSurface(surface) ??
    (surface ? state.contextChatByTab?.[surface] ?? null : null);
  if (isChatSurface(tabId)) setContextChatForTab(state, tabId, chatId);
};

const sanitizeContextChatByTab = (
  contextChatByTab: WorkspaceState["contextChatByTab"],
  tabs: SurfaceKey[],
  groups: WorkspaceGroups,
  openThreadIds?: ReadonlySet<string>,
): WorkspaceState["contextChatByTab"] => {
  if (!contextChatByTab) return undefined;
  const entries = Object.entries(contextChatByTab).filter(([tabId, chatId]) => {
    if (
      !tabs.includes(tabId) &&
      !Object.values(groups).some(
        (group) => group && collectTabIds(group.root).includes(tabId),
      )
    )
      return false;
    if (!isContextChatSurface(tabId)) return false;
    if (!chatId.trim()) return isFileSurface(tabId);
    if (openThreadIds && !openThreadIds.has(chatId)) return false;
    return true;
  });
  return entries.length > 0 ? Object.fromEntries(entries) : undefined;
};

const focusedWorkspaceChatId = (state: WorkspaceState): string | null => {
  const activeTabId = state.activeTabId;
  if (!activeTabId) return null;
  const group = state.groups[activeTabId];
  const focusedChatId = chatIdFromSurface(
    group ? focusedSurfaceKey(group) : activeTabId,
  );
  if (focusedChatId) return focusedChatId;
  const surface = group ? focusedSurfaceKey(group) : activeTabId;
  if (surface && !isChatSurface(surface) && isContextChatSurface(surface))
    return state.contextChatByTab?.[surface]?.trim()
      ? state.contextChatByTab[surface]
      : null;
  return null;
};

const pruneContextChatByTab = (
  state: WorkspaceState,
  openThreadIds?: ReadonlySet<string>,
): void => {
  const contextChatByTab = sanitizeContextChatByTab(
    state.contextChatByTab,
    state.tabs,
    state.groups,
    openThreadIds,
  );
  if (contextChatByTab) state.contextChatByTab = contextChatByTab;
  else delete state.contextChatByTab;
  for (const [tabId, group] of Object.entries(state.groups)) {
    if (!group || !isChatSurface(tabId) || state.contextChatByTab?.[tabId])
      continue;
    const groupSurfaces = collectTabIds(group.root);
    if (
      !groupSurfaces.some((key) => isFileSurface(key) || isDesignSurface(key))
    )
      continue;
    const chatId =
      chatIdFromSurface(focusedSurfaceKey(group)) ??
      chatIdFromSurface(groupSurfaces.find(isChatSurface) ?? null);
    if (chatId && (!openThreadIds || openThreadIds.has(chatId))) {
      setContextChatForTab(state, tabId, chatId);
    }
  }
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

const findFirstFileLeaf = (node: PaneNode): LeafPane | null => {
  if (node.kind === "leaf") {
    return node.tabIds.some(isFileSurface) ? node : null;
  }

  for (const child of node.children) {
    const found = findFirstFileLeaf(child);
    if (found) return found;
  }

  return null;
};

const findFirstChatLeaf = (
  node: PaneNode,
  chatKey: SurfaceKey,
): LeafPane | null => {
  if (node.kind === "leaf") {
    return node.tabIds.includes(chatKey) ? node : null;
  }

  for (const child of node.children) {
    const found = findFirstChatLeaf(child, chatKey);
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
  _dropEmptyTab = false,
): void => {
  const contextChatId = state.contextChatByTab?.[tabId] ?? null;
  const normalizedGroup = normalizeGroup({
    root: collapseEmpty ? collapseEmptyLeaves(group.root) : group.root,
    focusedLeafId: group.focusedLeafId,
  });
  const leafCount = collectLeafIds(normalizedGroup.root).length;
  const surfaceCount = collectTabIds(normalizedGroup.root).length;
  const tabIndex = tabIndexOrEnd(state.tabs, tabId);

  if (surfaceCount === 0) {
    state.groups = withoutGroup(state.groups, tabId);
    removeContextChatForTab(state, tabId);
    removeTopLevelTab(state, tabId);
    return;
  }

  if (leafCount > MAX_GROUP_LEAVES) {
    state.groups = withoutGroup(state.groups, tabId);
    removeContextChatForTab(state, tabId);
    return;
  }

  if (
    !state.tabs.includes(tabId) ||
    !collectTabIds(normalizedGroup.root).includes(tabId)
  ) {
    removeTopLevelTab(state, tabId);
    const replacementKey =
      collectTabIds(normalizedGroup.root).find(isChatSurface) ??
      firstSurfaceKey(normalizedGroup.root);
    if (!replacementKey) {
      state.groups = withoutGroup(state.groups, tabId);
      return;
    }

    state.groups = withoutGroup(state.groups, tabId);
    if (
      isChatSurface(tabId) ||
      !collectTabIds(normalizedGroup.root).includes(tabId)
    )
      removeContextChatForTab(state, tabId);
    state.tabs = insertTabAt(state.tabs, replacementKey, tabIndex);
    state.groups[replacementKey] = normalizedGroup;
    if (isChatSurface(replacementKey) && isChatSurface(tabId))
      setContextChatForTab(state, replacementKey, contextChatId);
    if (state.activeTabId === tabId) {
      state.activeTabId = replacementKey;
    }
    return;
  }

  state.groups[tabId] = normalizedGroup;
  const groupSurfaces = collectTabIds(normalizedGroup.root);
  if (
    isChatSurface(tabId) &&
    groupSurfaces.some((key) => isFileSurface(key) || isDesignSurface(key)) &&
    (!contextChatId || !groupSurfaces.includes(`chat:${contextChatId}`))
  ) {
    setContextChatForTab(
      state,
      tabId,
      chatIdFromSurface(focusedSurfaceKey(normalizedGroup)) ??
        chatIdFromSurface(groupSurfaces.find(isChatSurface) ?? null),
    );
  }
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

const closeWorkspaceSurface = (
  state: WorkspaceState,
  surfaceKey: SurfaceKey,
): void => {
  const neighbor = removeTopLevelTab(state, surfaceKey);
  if (state.groups[surfaceKey])
    state.tabs = insertTabAt(state.tabs, surfaceKey, 0);
  removeContextChatForTab(state, surfaceKey);
  removeSurfaceFromGroups(state, surfaceKey);

  if (state.activeTabId === surfaceKey) {
    state.activeTabId =
      neighbor && state.tabs.includes(neighbor)
        ? neighbor
        : state.tabs[0] ?? null;
  } else if (state.activeTabId && !state.tabs.includes(state.activeTabId)) {
    state.activeTabId = state.tabs[0] ?? null;
  }
};

const closeContextSurfacesForChat = (
  state: WorkspaceState,
  chatId: string,
): void => {
  Object.assign(state, ensureSingleWorkspaceLayout(state));
  const ownedSurfaceKeys = Object.entries(state.contextChatByTab ?? {})
    .filter(([key, owner]) => owner === chatId && isFileSurface(key))
    .map(([key]) => key);
  for (const key of unique(ownedSurfaceKeys)) closeWorkspaceSurface(state, key);
  pruneContextChatByTab(state);
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
  isChatSurface(key) && openThreadIds.has(key.slice("chat:".length));

const pruneNodeToOpenThreads = (
  node: PaneNode,
  openThreadIds: ReadonlySet<string>,
  capabilities: WorkspaceCapabilities,
): { node: PaneNode; changed: boolean } => {
  if (node.kind === "leaf") {
    const tabIds = node.tabIds.filter(
      (key) =>
        openChatSurface(key, openThreadIds) ||
        (!isChatSurface(key) && isWorkspaceSurfaceEnabled(key, capabilities)),
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
    pruneNodeToOpenThreads(child, openThreadIds, capabilities),
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
  const emptied: string[] = [];
  let root = mapLeaves(group.root, (leaf) => {
    const tabIds = leaf.tabIds.filter(isSplittableSurface);
    if (leaf.tabIds.length > 0 && tabIds.length === 0) emptied.push(leaf.id);
    return { ...leaf, tabIds };
  });
  for (const id of emptied)
    if (root.kind === "split") root = closeLeaf(root, id);
  const normalized = normalizeGroup({ ...group, root });
  const leafCount = collectLeafIds(normalized.root).length;
  const surfaces = collectTabIds(normalized.root);
  const surfaceCount = surfaces.length;

  if (
    leafCount > MAX_GROUP_LEAVES ||
    surfaceCount === 0 ||
    surfaceCount > MAX_WORKSPACE_SURFACES ||
    surfaces.some((key) => !isSplittableSurface(key))
  ) {
    return null;
  }

  return normalized;
};

const paneNodeEquals = (left: PaneNode, right: PaneNode): boolean => {
  if (left.kind !== right.kind || left.id !== right.id) return false;

  if (left.kind === "leaf" && right.kind === "leaf") {
    return (
      left.activeTabId === right.activeTabId &&
      left.tabIds.length === right.tabIds.length &&
      left.tabIds.every((tabId, index) => tabId === right.tabIds[index])
    );
  }

  if (left.kind === "split" && right.kind === "split") {
    return (
      left.dir === right.dir &&
      left.sizes.length === right.sizes.length &&
      left.sizes.every((size, index) => size === right.sizes[index]) &&
      left.children.length === right.children.length &&
      left.children.every((child, index) =>
        paneNodeEquals(child, right.children[index]),
      )
    );
  }

  return false;
};

const groupsEqual = (
  left: WorkspaceGroups,
  right: WorkspaceGroups,
): boolean => {
  const leftKeys = Object.keys(left).filter((key) => left[key]);
  const rightKeys = Object.keys(right).filter((key) => right[key]);
  if (leftKeys.length !== rightKeys.length) return false;

  return leftKeys.every((key) => {
    const leftGroup = left[key];
    const rightGroup = right[key];
    if (!leftGroup || !rightGroup) return false;
    return (
      leftGroup.focusedLeafId === rightGroup.focusedLeafId &&
      paneNodeEquals(leftGroup.root, rightGroup.root)
    );
  });
};

const workspaceStatesEqual = (
  left: WorkspaceState,
  right: WorkspaceState,
): boolean => {
  const leftDock = normalizeWorkspaceDock(left.dock);
  const rightDock = normalizeWorkspaceDock(right.dock);
  const leftDrawer = normalizeWorkspaceDrawer(left.drawer);
  const rightDrawer = normalizeWorkspaceDrawer(right.drawer);
  const leftLiveEdits = left.liveEditsByChat ?? {};
  const rightLiveEdits = right.liveEditsByChat ?? {};
  const leftLiveEditKeys = Object.keys(leftLiveEdits);
  const rightLiveEditKeys = Object.keys(rightLiveEdits);
  const leftContextChats = left.contextChatByTab ?? {};
  const rightContextChats = right.contextChatByTab ?? {};
  const leftContextKeys = Object.keys(leftContextChats);
  const rightContextKeys = Object.keys(rightContextChats);
  return (
    left.activeTabId === right.activeTabId &&
    Boolean(left.panelsForced) === Boolean(right.panelsForced) &&
    Boolean(left.liveEditsDefault) === Boolean(right.liveEditsDefault) &&
    leftDock.open === rightDock.open &&
    leftDock.width === rightDock.width &&
    leftDock.section === rightDock.section &&
    leftDrawer.open === rightDrawer.open &&
    leftDrawer.height === rightDrawer.height &&
    leftLiveEditKeys.length === rightLiveEditKeys.length &&
    leftLiveEditKeys.every(
      (chatId) => leftLiveEdits[chatId] === rightLiveEdits[chatId],
    ) &&
    leftContextKeys.length === rightContextKeys.length &&
    leftContextKeys.every(
      (tabId) => leftContextChats[tabId] === rightContextChats[tabId],
    ) &&
    left.tabs.length === right.tabs.length &&
    left.tabs.every((tabId, index) => tabId === right.tabs[index]) &&
    groupsEqual(left.groups, right.groups)
  );
};

export const ensureSingleWorkspaceLayout = (
  state: WorkspaceState,
): WorkspaceState => {
  const candidates: SurfaceKey[] = state.tabs.filter(isSplittableSurface);
  if (candidates.length === 0) return state;
  const primary = candidates.includes(state.activeTabId ?? "")
    ? state.activeTabId ?? candidates[0]
    : candidates[0];
  const order = [primary, ...candidates.filter((key) => key !== primary)];
  const seen = new Set<string>();
  const owners = { ...state.contextChatByTab };
  const clean = (node: PaneNode, owner?: string): PaneNode =>
    mapLeaves(node, (leaf) => ({
      ...leaf,
      tabIds: leaf.tabIds.filter((key) => {
        if (seen.has(key)) return false;
        seen.add(key);
        if (isFileSurface(key) && owner && !(key in owners))
          owners[key] = owner;
        return true;
      }),
    }));
  const base = state.groups[primary] ?? {
    root: createLeaf(INITIAL_WORKSPACE_LEAF_ID, [primary]),
    focusedLeafId: INITIAL_WORKSPACE_LEAF_ID,
  };
  const group = ensureGroupFocus({
    ...base,
    root: clean(base.root, owners[primary]),
  });
  for (const key of order.slice(1)) {
    const other = state.groups[key];
    if (!other) {
      if (!seen.has(key)) {
        seen.add(key);
        const focused = findLeaf(group.root, group.focusedLeafId);
        group.root = mapLeaves(group.root, (leaf) =>
          leaf.id === focused?.id
            ? { ...leaf, tabIds: [...leaf.tabIds, key] }
            : leaf,
        );
      }
      continue;
    }
    let root = collapseEmptyLeaves(clean(other.root, owners[key]));
    if (collectTabIds(root).length === 0) continue;
    if (
      root.kind === "split" &&
      collectLeafIds(root).length + collectLeafIds(group.root).length <=
        MAX_GROUP_LEAVES
    ) {
      const rename = (node: PaneNode): PaneNode => {
        const id = uniquePaneNodeId(group.root, `merged:${key}:${node.id}`);
        return node.kind === "leaf"
          ? { ...node, id }
          : { ...node, id, children: node.children.map(rename) };
      };
      root = rename(root);
      group.root = {
        kind: "split",
        id: uniquePaneNodeId(group.root, `merged:${key}`),
        dir: "row",
        children: [group.root, root],
        sizes: [0.5, 0.5],
      };
    } else {
      const added = collectTabIds(root);
      group.root = mapLeaves(group.root, (leaf) =>
        leaf.id === group.focusedLeafId
          ? { ...leaf, tabIds: [...leaf.tabIds, ...added] }
          : leaf,
      );
    }
  }
  const groups = Object.fromEntries(
    Object.entries(state.groups).filter(([key]) => !candidates.includes(key)),
  );
  const rootKey =
    isChatSurface(primary) && collectTabIds(group.root).includes(primary)
      ? primary
      : collectTabIds(group.root).find(isChatSurface) ?? primary;
  groups[rootKey] = group;
  const tabs = state.tabs
    .filter((key) => key === primary || !candidates.includes(key))
    .map((key) => (key === primary ? rootKey : key));
  const next = {
    ...state,
    tabs,
    groups,
    contextChatByTab: owners,
    activeTabId: candidates.includes(state.activeTabId ?? "")
      ? rootKey
      : state.activeTabId,
  };
  return workspaceStatesEqual(state, next) ? state : next;
};

export const sanitizeWorkspaceSurfaceUniqueness = (
  state: WorkspaceState,
  workspaceCapabilities: WorkspaceCapabilities = DEFAULT_WORKSPACE_CAPABILITIES,
): WorkspaceState => {
  const capabilities = effectiveWorkspaceCapabilities(
    state,
    workspaceCapabilities,
  );
  const tabs = unique(state.tabs)
    .filter(
      (key) =>
        !isFilesSurface(key) &&
        (isWorkspaceSurfaceEnabled(key, capabilities) ||
          (isSplittableSurface(key) && !!state.groups[key])),
    )
    .slice(0, MAX_WORKSPACE_TABS);
  const groups: WorkspaceGroups = {};
  for (const key of tabs) {
    const group = state.groups[key];
    if (!group || !isSplittableSurface(key)) continue;
    const normalized = normalizeHydratedGroup(group);
    if (normalized) {
      const emptied: string[] = [];
      let root = mapLeaves(normalized.root, (leaf) => {
        const tabIds = leaf.tabIds.filter((surface) =>
          isWorkspaceSurfaceEnabled(surface, capabilities),
        );
        if (leaf.tabIds.length > 0 && tabIds.length === 0)
          emptied.push(leaf.id);
        return { ...leaf, tabIds };
      });
      for (const id of emptied)
        if (root.kind === "split") root = closeLeaf(root, id);
      groups[key] = ensureGroupFocus({ ...normalized, root });
    }
  }
  const prepared = {
    ...state,
    tabs,
    groups,
    activeTabId: tabs.includes(state.activeTabId ?? "")
      ? state.activeTabId
      : tabs[0] ?? null,
  };
  for (const [key, group] of Object.entries(groups))
    if (
      group &&
      (!isWorkspaceSurfaceEnabled(key, capabilities) ||
        !collectTabIds(group.root).includes(key))
    )
      writeGroup(prepared, key, group, false);
  const next = ensureSingleWorkspaceLayout(prepared);
  const mutable = { ...next, contextChatByTab: { ...next.contextChatByTab } };
  pruneContextChatByTab(mutable);
  return workspaceStatesEqual(state, mutable) ? state : mutable;
};

const openLocalSurface = (state: WorkspaceState, key: SurfaceKey): void => {
  Object.assign(state, ensureSingleWorkspaceLayout(state));
  if (isFileSurface(key) && !(key in (state.contextChatByTab ?? {})))
    state.contextChatByTab = {
      ...state.contextChatByTab,
      [key]: focusedWorkspaceChatId(state) ?? "",
    };
  const tabId = state.tabs.find(isSplittableSurface);
  const group = tabId ? state.groups[tabId] : undefined;
  if (!tabId || !group) {
    state.tabs.push(key);
    state.activeTabId = key;
    state.groups[key] = {
      root: createLeaf(INITIAL_WORKSPACE_LEAF_ID, [key]),
      focusedLeafId: INITIAL_WORKSPACE_LEAF_ID,
    };
    return;
  }
  const existing = findLeafByTab(group.root, key);
  if (
    !existing &&
    isFileSurface(key) &&
    collectTabIds(group.root).filter(isFileSurface).length >= MAX_WORKSPACE_TABS
  )
    return;
  if (!existing && collectTabIds(group.root).length >= MAX_WORKSPACE_SURFACES)
    return;
  const leafId = existing?.id ?? group.focusedLeafId;
  state.groups[tabId] = {
    root: existing
      ? setLeafActiveSurface(group.root, leafId, key)
      : addSurfaceToLeaf(group.root, leafId, key),
    focusedLeafId: leafId,
  };
  state.activeTabId = tabId;
  setContextChatForFocusedLeaf(state, tabId, leafId);
  Object.assign(state, ensureSingleWorkspaceLayout(state));
};

export const reconcileWorkspaceState = (
  state: WorkspaceState,
  openThreadIds: string[],
  workspaceCapabilities: WorkspaceCapabilities = DEFAULT_WORKSPACE_CAPABILITIES,
  contextChatIds: readonly string[] = [],
): WorkspaceState => {
  const effectiveCapabilities = effectiveWorkspaceCapabilities(
    state,
    workspaceCapabilities,
  );
  const openThreads = new Set(openThreadIds);
  const contextOwners = new Set([...openThreadIds, ...contextChatIds]);
  const contextOwnerIsOpen = (tabId: SurfaceKey): boolean => {
    const chatId = state.contextChatByTab?.[tabId]?.trim();
    return !chatId || contextOwners.has(chatId);
  };
  const nextState: WorkspaceState = {
    tabs: unique(state.tabs)
      .filter(
        (key) =>
          (isWorkspaceSurfaceEnabled(key, effectiveCapabilities) ||
            !!state.groups[key]) &&
          (isChatSurface(key)
            ? openChatSurface(key, openThreads) || !!state.groups[key]
            : !isFileSurface(key) ||
              !!state.groups[key] ||
              contextOwnerIsOpen(key)),
      )
      .slice(0, MAX_WORKSPACE_TABS),
    activeTabId: state.activeTabId,
    groups: {},
    ...(state.contextChatByTab
      ? { contextChatByTab: state.contextChatByTab }
      : {}),
    panelsForced: state.panelsForced === true ? true : undefined,
    liveEditsByChat: state.liveEditsByChat,
    liveEditsDefault: state.liveEditsDefault,
    dock: normalizeWorkspaceDock(state.dock),
    drawer: normalizeWorkspaceDrawer(state.drawer),
  };

  for (const [tabId, group] of Object.entries(state.groups)) {
    if (!group) continue;
    if (!nextState.tabs.includes(tabId) || !isSplittableSurface(tabId))
      continue;

    const pruned = pruneNodeToOpenThreads(
      group.root,
      openThreads,
      effectiveCapabilities,
    );
    const nextGroup = {
      root: mapLeaves(pruned.node, (leaf) => ({
        ...leaf,
        tabIds: leaf.tabIds.filter(
          (key) => !isFileSurface(key) || contextOwnerIsOpen(key),
        ),
      })),
      focusedLeafId: group.focusedLeafId,
    };
    nextState.groups[tabId] = nextGroup;
    const ownerPruned =
      collectTabIds(nextGroup.root).length !==
      collectTabIds(pruned.node).length;
    writeGroup(
      nextState,
      tabId,
      nextGroup,
      pruned.changed || ownerPruned,
      true,
    );
  }

  if (
    !nextState.activeTabId ||
    !nextState.tabs.includes(nextState.activeTabId)
  ) {
    nextState.activeTabId = nextState.tabs[0] ?? null;
  }
  pruneContextChatByTab(nextState, contextOwners);

  const sanitizedState = sanitizeWorkspaceSurfaceUniqueness(
    nextState,
    effectiveCapabilities,
  );

  return workspaceStatesEqual(state, sanitizedState) ? state : sanitizedState;
};

export const workspaceSlice = createSlice({
  name: "workspace",
  reducerPath: "workspace",
  initialState,
  reducers: {
    consolidateWorkspaceTabs: (state) => ensureSingleWorkspaceLayout(state),
    setPanelsForced: (state, action: PayloadAction<boolean>) => {
      state.panelsForced = action.payload;
    },
    setLiveEditsForChat: (
      state,
      action: PayloadAction<{ chatId: string; enabled: boolean }>,
    ) => {
      state.liveEditsByChat = {
        ...state.liveEditsByChat,
        [action.payload.chatId]: action.payload.enabled,
      };
      state.liveEditsDefault = action.payload.enabled;
    },
    clearWorkspaceChatState: (state, action: PayloadAction<string>) => {
      closeContextSurfacesForChat(state, action.payload);
      if (state.liveEditsByChat) {
        const { [action.payload]: _liveEdits, ...otherLiveEdits } =
          state.liveEditsByChat;
        state.liveEditsByChat =
          Object.keys(otherLiveEdits).length > 0 ? otherLiveEdits : undefined;
      }
      if (state.contextChatByTab) {
        const otherContextChats = Object.fromEntries(
          Object.entries(state.contextChatByTab).filter(
            ([, chatId]) => chatId !== action.payload,
          ),
        );
        state.contextChatByTab =
          Object.keys(otherContextChats).length > 0
            ? otherContextChats
            : undefined;
      }
    },
    setDockOpen: (state, action: PayloadAction<boolean>) => {
      state.dock = {
        ...normalizeWorkspaceDock(state.dock),
        open: action.payload,
      };
    },
    toggleDock: (state) => {
      const dock = normalizeWorkspaceDock(state.dock);
      state.dock = { ...dock, open: !dock.open };
    },
    setDockWidth: (state, action: PayloadAction<number>) => {
      state.dock = {
        ...normalizeWorkspaceDock(state.dock),
        width: normalizeDockWidth(action.payload),
      };
    },
    setDockSection: (state, action: PayloadAction<WorkspaceDockSection>) => {
      state.dock = {
        ...normalizeWorkspaceDock(state.dock),
        section: action.payload,
      };
    },
    setDrawerOpen: (state, action: PayloadAction<boolean>) => {
      state.drawer = {
        ...normalizeWorkspaceDrawer(state.drawer),
        open: action.payload,
      };
    },
    toggleDrawer: (state) => {
      const drawer = normalizeWorkspaceDrawer(state.drawer);
      state.drawer = { ...drawer, open: !drawer.open };
    },
    setDrawerHeight: (state, action: PayloadAction<number>) => {
      state.drawer = {
        ...normalizeWorkspaceDrawer(state.drawer),
        height: normalizeDrawerHeight(action.payload),
      };
    },
    bindSurfaceToChat: (
      state,
      action: PayloadAction<{ surfaceKey: SurfaceKey; chatId: string }>,
    ) => {
      if (!isWorkspaceSurface(action.payload.surfaceKey)) return;
      bindContextChatForSurface(
        state,
        action.payload.surfaceKey,
        action.payload.chatId,
      );
    },
    openTab: (state, action: PayloadAction<SurfaceKey>) => {
      if (!isWorkspaceSurface(action.payload)) return;
      openLocalSurface(state, action.payload);
    },
    openSurfaceInNewTab: (state, action: PayloadAction<SurfaceKey>) => {
      if (isSplittableSurface(action.payload))
        openLocalSurface(state, action.payload);
    },
    closeTab: (state, action: PayloadAction<SurfaceKey>) => {
      closeWorkspaceSurface(state, action.payload);
      pruneContextChatByTab(state);
    },
    activateWorkspaceLayout: (state) => {
      Object.assign(state, ensureSingleWorkspaceLayout(state));
      const root = state.tabs.find(isSplittableSurface);
      if (root) state.activeTabId = root;
    },
    setActiveTab: (state, action: PayloadAction<SurfaceKey>) => {
      for (const [key, group] of Object.entries(state.groups)) {
        const leaf = group ? findLeafByTab(group.root, action.payload) : null;
        if (!leaf || !group) continue;
        state.groups[key] = {
          root: setLeafActiveSurface(group.root, leaf.id, action.payload),
          focusedLeafId: leaf.id,
        };
        state.activeTabId = key;
        setContextChatForFocusedLeaf(state, key, leaf.id);
        return;
      }
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
        surfaceKey?: SurfaceKey;
      }>,
    ) => {
      const {
        tabId,
        dir,
        placement,
        surfaceKey: draggedSurfaceKey,
      } = action.payload;
      if (!isSplittableSurface(tabId)) return;
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
      if (surfaceKey && !isSplittableSurface(surfaceKey)) return;
      if (!focusedLeaf || !surfaceKey) return;
      if (draggedSurfaceKey) {
        if (!isSplittableSurface(draggedSurfaceKey)) return;
        if (draggedSurfaceKey === surfaceKey) return;
        if (
          !collectTabIds(currentGroup.root).includes(draggedSurfaceKey) &&
          (collectTabIds(currentGroup.root).length >= MAX_WORKSPACE_SURFACES ||
            (isFileSurface(draggedSurfaceKey) &&
              collectTabIds(currentGroup.root).filter(isFileSurface).length >=
                MAX_WORKSPACE_TABS))
        )
          return;
      }

      const splitResult = splitLeafWithEmptySibling(
        currentGroup.root,
        currentGroup.focusedLeafId,
        dir,
        surfaceKey,
        placement,
      );
      if (!splitResult) return;

      const nextRoot = draggedSurfaceKey
        ? removeSurfaceFromTree(splitResult.root, draggedSurfaceKey)
        : splitResult.root;
      const nextLeafIds = collectLeafIds(nextRoot);
      if (nextLeafIds.length === leafCount) return;

      if (draggedSurfaceKey) {
        detachSurfaceForGroup(state, tabId, draggedSurfaceKey);
      }

      state.groups[tabId] = ensureGroupFocus(
        {
          root: draggedSurfaceKey
            ? addSurfaceToLeaf(
                nextRoot,
                splitResult.siblingLeafId,
                draggedSurfaceKey,
              )
            : nextRoot,
          focusedLeafId: currentGroup.focusedLeafId,
        },
        splitResult.siblingLeafId,
      );
      setContextChatForFocusedLeaf(state, tabId, splitResult.siblingLeafId);
      state.activeTabId = tabId;
    },
    ensureLiveEditSplit: (
      state,
      action: PayloadAction<{ chatId: string; filePath: string }>,
    ) => {
      const chatKey = `chat:${action.payload.chatId}`;
      const fileKey = `file:${action.payload.filePath}`;
      const tabId = state.tabs.find((key) => {
        if (key === chatKey) return true;
        const group = state.groups[key];
        return group ? collectTabIds(group.root).includes(chatKey) : false;
      });
      if (!tabId || !isSplittableSurface(tabId)) return;

      const currentGroup =
        state.groups[tabId] ??
        normalizeGroup({
          root: createLeaf(INITIAL_WORKSPACE_LEAF_ID, [chatKey], chatKey),
          focusedLeafId: INITIAL_WORKSPACE_LEAF_ID,
        });
      const existingSurfaces = collectTabIds(currentGroup.root);
      if (
        !existingSurfaces.includes(fileKey) &&
        (existingSurfaces.length >= MAX_WORKSPACE_SURFACES ||
          existingSurfaces.filter(isFileSurface).length >= MAX_WORKSPACE_TABS)
      )
        return;
      const fileLeaf = findFirstFileLeaf(currentGroup.root);

      if (fileLeaf) {
        detachSurfaceForGroup(state, tabId, fileKey);
        const nextRoot = mapLeaves(currentGroup.root, (leaf) => {
          const tabIds = leaf.tabIds.filter((key) => key !== fileKey);
          if (leaf.id !== fileLeaf.id) {
            return {
              ...leaf,
              tabIds,
              activeTabId: tabIds.includes(leaf.activeTabId ?? "")
                ? leaf.activeTabId
                : tabIds[0] ?? null,
            };
          }
          return {
            ...leaf,
            tabIds: [...tabIds, fileKey],
            activeTabId: fileKey,
          };
        });
        state.groups[tabId] = ensureGroupFocus(
          { root: nextRoot, focusedLeafId: currentGroup.focusedLeafId },
          fileLeaf.id,
        );
        setContextChatForTab(state, fileKey, action.payload.chatId);
        if (isChatSurface(tabId) || isFileSurface(tabId))
          setContextChatForTab(state, tabId, action.payload.chatId);
        state.activeTabId = tabId;
        return;
      }

      const chatLeaf = findFirstChatLeaf(currentGroup.root, chatKey);
      if (
        !chatLeaf ||
        collectLeafIds(currentGroup.root).length >= MAX_GROUP_LEAVES
      ) {
        return;
      }
      const splitResult = splitLeafWithEmptySibling(
        currentGroup.root,
        chatLeaf.id,
        "row",
        chatKey,
        "after",
      );
      if (!splitResult) return;
      detachSurfaceForGroup(state, tabId, fileKey);
      state.groups[tabId] = ensureGroupFocus(
        {
          root: addSurfaceToLeaf(
            splitResult.root,
            splitResult.siblingLeafId,
            fileKey,
          ),
          focusedLeafId: currentGroup.focusedLeafId,
        },
        splitResult.siblingLeafId,
      );
      setContextChatForTab(state, fileKey, action.payload.chatId);
      if (isChatSurface(tabId) || isFileSurface(tabId))
        setContextChatForTab(state, tabId, action.payload.chatId);
      state.activeTabId = tabId;
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
      pruneContextChatByTab(state);
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
      setContextChatForFocusedLeaf(state, tabId, leafId);
    },
    reorderPaneTabs: (
      state,
      action: PayloadAction<{
        tabId: SurfaceKey;
        leafId: string;
        fromIndex: number;
        toIndex: number;
      }>,
    ) => {
      const { tabId, leafId, fromIndex, toIndex } = action.payload;
      const group = state.groups[tabId];
      const leaf = group ? findLeaf(group.root, leafId) : null;
      if (
        !group ||
        !leaf ||
        !Number.isInteger(fromIndex) ||
        !Number.isInteger(toIndex) ||
        fromIndex < 0 ||
        toIndex < 0 ||
        fromIndex >= leaf.tabIds.length ||
        toIndex >= leaf.tabIds.length
      )
        return;
      state.groups[tabId] = {
        ...group,
        root: mapLeaves(group.root, (pane) =>
          pane.id === leafId
            ? {
                ...pane,
                tabIds: reorderItems(
                  pane.tabIds,
                  pane.tabIds[fromIndex],
                  pane.tabIds[toIndex],
                ),
              }
            : pane,
        ),
      };
    },
    closePaneTab: (
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
      writeGroup(
        state,
        tabId,
        {
          root: removeSurfaceFromTree(group.root, surfaceKey),
          focusedLeafId: group.focusedLeafId,
        },
        true,
        true,
      );
      if (state.activeTabId && !state.tabs.includes(state.activeTabId))
        state.activeTabId = state.tabs[0] ?? null;
      pruneContextChatByTab(state);
    },
    focusPane: (
      state,
      action: PayloadAction<{ tabId: SurfaceKey; leafId: string }>,
    ) => {
      const group = state.groups[action.payload.tabId];
      if (!group || !findLeaf(group.root, action.payload.leafId)) return;
      if (group.focusedLeafId === action.payload.leafId) return;
      state.groups[action.payload.tabId] = ensureGroupFocus(
        group,
        action.payload.leafId,
      );
      setContextChatForFocusedLeaf(
        state,
        action.payload.tabId,
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
      if (!isSplittableSurface(tabId) || !isSplittableSurface(surfaceKey))
        return;
      if (!state.tabs.includes(tabId)) return;
      const group = state.groups[tabId] ?? {
        root: createLeaf(INITIAL_WORKSPACE_LEAF_ID, [tabId], tabId),
        focusedLeafId: INITIAL_WORKSPACE_LEAF_ID,
      };
      if (!findLeaf(group.root, leafId)) return;
      if (
        !collectTabIds(group.root).includes(surfaceKey) &&
        (collectTabIds(group.root).length >= MAX_WORKSPACE_SURFACES ||
          (isFileSurface(surfaceKey) &&
            collectTabIds(group.root).filter(isFileSurface).length >=
              MAX_WORKSPACE_TABS))
      )
        return;
      if (
        isFileSurface(surfaceKey) &&
        !collectTabIds(group.root).includes(surfaceKey) &&
        collectTabIds(group.root).filter(isFileSurface).length >=
          MAX_WORKSPACE_TABS
      )
        return;
      if (
        isFileSurface(surfaceKey) &&
        !(surfaceKey in (state.contextChatByTab ?? {}))
      )
        state.contextChatByTab = {
          ...state.contextChatByTab,
          [surfaceKey]: focusedWorkspaceChatId(state) ?? "",
        };
      state.groups = { ...state.groups, [tabId]: group };

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
        setContextChatForFocusedLeaf(state, tabId, leafId);
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
      if (!isSplittableSurface(tabId) || !isSplittableSurface(surfaceKey))
        return;
      const group = state.groups[tabId];
      if (!group || !findLeaf(group.root, leafId)) return;

      const leafCount = collectLeafIds(group.root).length;
      if (leafCount >= MAX_GROUP_LEAVES) return;
      if (
        !collectTabIds(group.root).includes(surfaceKey) &&
        (collectTabIds(group.root).length >= MAX_WORKSPACE_SURFACES ||
          (isFileSurface(surfaceKey) &&
            collectTabIds(group.root).filter(isFileSurface).length >=
              MAX_WORKSPACE_TABS))
      )
        return;
      if (
        isFileSurface(surfaceKey) &&
        !(surfaceKey in (state.contextChatByTab ?? {}))
      )
        state.contextChatByTab = {
          ...state.contextChatByTab,
          [surfaceKey]: focusedWorkspaceChatId(state) ?? "",
        };

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

      let splitRoot = addSurfaceToLeaf(
        splitResult.root,
        splitResult.siblingLeafId,
        surfaceKey,
      );
      const previousSurface = findLeaf(nextGroup.root, leafId)?.activeTabId;
      if (previousSurface && previousSurface !== surfaceKey)
        splitRoot = setLeafActiveSurface(splitRoot, leafId, previousSurface);
      writeGroup(
        state,
        tabId,
        { root: splitRoot, focusedLeafId: nextGroup.focusedLeafId },
        true,
      );

      const writtenGroup = state.groups[tabId];
      if (writtenGroup) {
        state.groups[tabId] = ensureGroupFocus(
          writtenGroup,
          splitResult.siblingLeafId,
        );
        setContextChatForFocusedLeaf(state, tabId, splitResult.siblingLeafId);
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
      action: PayloadAction<{
        openThreadIds: string[];
        workspaceCapabilities?: WorkspaceCapabilities;
        contextChatIds?: string[];
      }>,
    ) =>
      reconcileWorkspaceState(
        state,
        action.payload.openThreadIds,
        action.payload.workspaceCapabilities,
        action.payload.contextChatIds,
      ),
    hydrateWorkspace: (
      _state,
      action: PayloadAction<
        WorkspaceHydrationState & {
          workspaceCapabilities?: WorkspaceCapabilities;
          openThreadIds?: string[];
          contextChatIds?: string[];
        }
      >,
    ) => {
      const panelsForced = action.payload.panelsForced === true;
      const baseWorkspaceCapabilities =
        action.payload.workspaceCapabilities ?? DEFAULT_WORKSPACE_CAPABILITIES;
      const workspaceCapabilities = panelsForced
        ? DEFAULT_WORKSPACE_CAPABILITIES
        : baseWorkspaceCapabilities;
      const hydrationCapabilities = action.payload.openThreadIds
        ? { ...workspaceCapabilities, filesPanel: true }
        : workspaceCapabilities;
      const legacyTerminalTab = action.payload.tabs.some(isTerminalSurface);
      const tabs = unique(action.payload.tabs)
        .filter(
          (key) =>
            !isFilesSurface(key) &&
            (isWorkspaceSurfaceEnabled(key, hydrationCapabilities) ||
              (isSplittableSurface(key) && !!action.payload.groups[key])),
        )
        .slice(0, MAX_WORKSPACE_TABS);
      const groups: Record<SurfaceKey, PaneGroup> = {};

      for (const [tabId, group] of Object.entries(action.payload.groups)) {
        if (!isSplittableSurface(tabId)) continue;
        if (!tabs.includes(tabId) || !group) continue;
        const normalized = normalizeHydratedGroup(group);
        if (!normalized) continue;
        groups[tabId] = normalized;
      }

      const activeTabId = action.payload.activeTabId;
      const persistedContextChatByTab = sanitizeContextChatByTab(
        action.payload.contextChatByTab,
        tabs,
        {},
      );

      const hydratedState = sanitizeWorkspaceSurfaceUniqueness(
        {
          tabs,
          activeTabId:
            activeTabId &&
            isSplittableSurface(activeTabId) &&
            tabs.includes(activeTabId)
              ? activeTabId
              : tabs[0] ?? null,
          groups,
          ...(sanitizeContextChatByTab(
            action.payload.contextChatByTab,
            tabs,
            groups,
          )
            ? {
                contextChatByTab: sanitizeContextChatByTab(
                  action.payload.contextChatByTab,
                  tabs,
                  groups,
                ),
              }
            : {}),
          panelsForced: panelsForced || undefined,
          liveEditsByChat: action.payload.liveEditsByChat,
          liveEditsDefault: action.payload.liveEditsDefault,
          dock: normalizeWorkspaceDock(action.payload.dock),
          drawer: {
            ...normalizeWorkspaceDrawer(action.payload.drawer),
            open:
              legacyTerminalTab ||
              normalizeWorkspaceDrawer(action.payload.drawer).open,
          },
        },
        hydrationCapabilities,
      );
      const restoredActiveTabId = hydratedState.activeTabId;
      const restoredGroup = restoredActiveTabId
        ? hydratedState.groups[restoredActiveTabId]
        : undefined;
      const restoredLeafId = restoredGroup?.focusedLeafId;
      const restoredSurface = restoredGroup
        ? focusedSurfaceKey(restoredGroup)
        : null;
      for (const id of action.payload.openThreadIds ?? []) {
        const key = `chat:${id}`;
        if (
          !hydratedState.tabs.includes(key) &&
          !Object.values(hydratedState.groups).some(
            (group) => group && collectTabIds(group.root).includes(key),
          )
        )
          openLocalSurface(hydratedState, key);
      }
      if (restoredActiveTabId) {
        hydratedState.activeTabId = restoredActiveTabId;
        const group = hydratedState.groups[restoredActiveTabId];
        if (group && restoredLeafId && restoredSurface)
          hydratedState.groups[restoredActiveTabId] = {
            root: setLeafActiveSurface(
              group.root,
              restoredLeafId,
              restoredSurface,
            ),
            focusedLeafId: restoredLeafId,
          };
      }
      return action.payload.openThreadIds
        ? reconcileWorkspaceState(
            persistedContextChatByTab
              ? {
                  ...hydratedState,
                  contextChatByTab: {
                    ...persistedContextChatByTab,
                    ...hydratedState.contextChatByTab,
                  },
                }
              : hydratedState,
            action.payload.openThreadIds,
            workspaceCapabilities,
            action.payload.contextChatIds,
          )
        : hydratedState;
    },
  },
});

export const {
  clearWorkspaceChatState,
  consolidateWorkspaceTabs,
  setPanelsForced,
  setLiveEditsForChat,
  setDockOpen,
  toggleDock,
  setDockWidth,
  setDockSection,
  setDrawerOpen,
  toggleDrawer,
  setDrawerHeight,
  bindSurfaceToChat,
  openTab,
  closeTab,
  setActiveTab,
  activateWorkspaceLayout,
  reorderTabs,
  splitTab,
  ensureLiveEditSplit,
  closePane,
  closePaneTab,
  reorderPaneTabs,
  openSurfaceInNewTab,
  setPaneActive,
  focusPane,
  addSurfaceToPane,
  splitPaneWithSurface,
  resizeGroupSplit,
  reconcileWorkspace,
  hydrateWorkspace,
} = workspaceSlice.actions;

type WorkspacePageState = {
  name: string;
  taskId?: string;
  chatId?: string;
};

type WorkspaceTaskState = {
  id: string;
  activeChat: { chatId: string } | null;
  plannerChats?: readonly { id: string }[];
};

type WorkspaceRootState = {
  workspace: WorkspaceState;
  pages?: readonly WorkspacePageState[];
  tasksUI?: { openTasks: readonly WorkspaceTaskState[] };
};

type FocusedChatWorkspaceRootState = WorkspaceRootState & {
  chat: {
    threads: Record<
      string,
      | {
          thread: {
            worktree?: {
              root: string;
              source_workspace_root: string;
            } | null;
          };
        }
      | undefined
    >;
  };
  current_project: {
    workspaceRoots?: string[];
  };
  config?: {
    host?: "web" | "ide" | "vscode" | "jetbrains";
  };
};

const EMPTY_WORKSPACE_ROOTS: string[] = [];

export const selectTabs = (state: WorkspaceRootState) => state.workspace.tabs;

export const selectPanelsForced = (state: WorkspaceRootState) =>
  state.workspace.panelsForced ?? false;

export const selectWorkspaceDock = (state: WorkspaceRootState) =>
  state.workspace.dock ?? DEFAULT_WORKSPACE_DOCK;

export const selectWorkspaceDrawer = (state: WorkspaceRootState) =>
  state.workspace.drawer ?? DEFAULT_WORKSPACE_DRAWER;

export const selectActiveTabId = (state: WorkspaceRootState) =>
  state.workspace.activeTabId;

export const selectGroupForTab = (
  state: WorkspaceRootState,
  tabId: SurfaceKey,
) => state.workspace.groups[tabId] ?? null;

export const selectWorkspaceGroups = (state: WorkspaceRootState) =>
  state.workspace.groups;

export const selectLiveEditsForChat = (
  state: FocusedChatWorkspaceRootState,
  chatId: string,
): boolean =>
  state.workspace.liveEditsByChat?.[chatId] ??
  state.workspace.liveEditsDefault ??
  false;

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

export const selectFocusedWorkspaceSurfaceKey = (
  state: WorkspaceRootState,
): SurfaceKey | null => {
  const activeTabId = state.workspace.activeTabId;
  if (!activeTabId) return null;

  const group = state.workspace.groups[activeTabId];
  return group ? focusedSurfaceKey(group) : activeTabId;
};

export const selectWorkspaceFocusedChatId = (
  state: WorkspaceRootState,
): string | null => focusedWorkspaceChatId(state.workspace);

export const selectTaskPageChatId = (
  state: WorkspaceRootState,
): string | null | undefined => {
  const page = state.pages?.[state.pages.length - 1];
  if (page?.name === "task agent") return page.chatId ?? null;
  if (page?.name !== "task workspace") return undefined;
  const task = state.tasksUI?.openTasks.find(({ id }) => id === page.taskId);
  return task?.activeChat?.chatId ?? null;
};

export const selectFocusedWorkspaceChatId = (
  state: WorkspaceRootState,
): string | null => {
  const surface = selectFocusedWorkspaceSurfaceKey(state);
  if (surface?.startsWith("task:"))
    return (
      state.tasksUI?.openTasks.find(({ id }) => id === surface.slice(5))
        ?.activeChat?.chatId ?? null
    );
  const taskChatId = selectTaskPageChatId(state);
  return taskChatId === undefined
    ? selectWorkspaceFocusedChatId(state)
    : taskChatId;
};

type WorkspaceContextRootState = WorkspaceRootState & {
  chat?: {
    threads: Record<
      string,
      { thread: { task_meta?: { task_id: string } | null } } | undefined
    >;
  };
};

export const selectWorkspaceContextChatIds = (
  state: WorkspaceContextRootState,
): string[] => {
  const chatIds = new Set<string>();
  const openTaskIds = new Set<string>();
  for (const task of state.tasksUI?.openTasks ?? []) {
    openTaskIds.add(task.id);
    if (task.activeChat?.chatId) chatIds.add(task.activeChat.chatId);
    for (const planner of task.plannerChats ?? []) chatIds.add(planner.id);
  }
  for (const [chatId, runtime] of Object.entries(state.chat?.threads ?? {})) {
    const taskId = runtime?.thread.task_meta?.task_id;
    if (taskId && openTaskIds.has(taskId)) chatIds.add(chatId);
  }
  const taskChatId = selectTaskPageChatId(state);
  if (taskChatId) chatIds.add(taskChatId);
  return [...chatIds];
};

export const selectSurfaceChatId = (
  state: WorkspaceRootState,
  surfaceKey: SurfaceKey,
): string | null => {
  if (!isContextChatSurface(surfaceKey)) return null;
  return (
    chatIdFromSurface(surfaceKey) ??
    (state.workspace.contextChatByTab?.[surfaceKey]?.trim()
      ? state.workspace.contextChatByTab[surfaceKey]
      : null)
  );
};

export const selectFocusedChatLiveEdits = (
  state: FocusedChatWorkspaceRootState,
): boolean => {
  const chatId = selectFocusedWorkspaceChatId(state);
  return chatId ? selectLiveEditsForChat(state, chatId) : false;
};

export const selectFocusedChatWorktreeRoot = (
  state: FocusedChatWorkspaceRootState,
): string | null => {
  const chatId = selectFocusedWorkspaceChatId(state);
  return chatId
    ? state.chat.threads[chatId]?.thread.worktree?.root ?? null
    : null;
};

export const selectFocusedChatWorktreeSourceRoot = (
  state: FocusedChatWorkspaceRootState,
): string | null => {
  const chatId = selectFocusedWorkspaceChatId(state);
  return chatId
    ? state.chat.threads[chatId]?.thread.worktree?.source_workspace_root ?? null
    : null;
};

const selectConfiguredWorkspaceRoots = (
  state: FocusedChatWorkspaceRootState,
): string[] => state.current_project.workspaceRoots ?? EMPTY_WORKSPACE_ROOTS;

export const selectFocusedChatWorkspaceRoot = createSelector(
  [selectFocusedChatWorktreeRoot, selectConfiguredWorkspaceRoots],
  (worktreeRoot, workspaceRoots): string => {
    if (worktreeRoot) return worktreeRoot;
    return workspaceRoots.length > 0 ? workspaceRoots[0] : "";
  },
);

export const selectChatWorkspaceRoot = (
  state: FocusedChatWorkspaceRootState,
  chatId: string,
): string => {
  const worktreeRoot = state.chat.threads[chatId]?.thread.worktree?.root;
  if (worktreeRoot) return worktreeRoot;
  const workspaceRoots = selectConfiguredWorkspaceRoots(state);
  return workspaceRoots.length > 0 ? workspaceRoots[0] : "";
};

export const selectFocusedChatWorkspaceRoots = createSelector(
  [selectFocusedChatWorktreeRoot, selectConfiguredWorkspaceRoots],
  (worktreeRoot, workspaceRoots): string[] =>
    worktreeRoot ? [worktreeRoot] : workspaceRoots,
);

export default workspaceSlice.reducer;
