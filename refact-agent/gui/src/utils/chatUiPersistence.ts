import { collectTabIds, type PaneNode } from "../features/ChatPanes/panesTree";
import {
  MAX_GROUP_LEAVES,
  MAX_WORKSPACE_TABS,
  reconcileWorkspaceState,
  type PaneGroup,
  type WorkspaceGroups,
  type WorkspaceState,
} from "../features/Workspace/workspaceSlice";
import {
  isChatSurface,
  parseSurfaceKey,
  type SurfaceKey,
} from "../features/Workspace/surfaceKey";

type JsonRecord = Record<string, unknown>;

const CHAT_TABS_STORAGE_KEY = "refact:chat-ui:tabs:v1";
const ACTIVE_TAB_STORAGE_KEY = "refact:chat-ui:active-tab:v1";
const WORKSPACE_STORAGE_KEY = "refact:chat-ui:workspace:v1";
const TASKS_UI_STORAGE_KEY = "refact:chat-ui:tasks-ui:v1";
const ASK_QUESTIONS_STORAGE_KEY = "refact:chat-ui:ask-questions:v1";
const TASK_WORKSPACE_TABS_STORAGE_KEY = "refact:chat-ui:task-workspace-tabs:v1";
const PROJECT_STORAGE_NAMESPACE_SESSION_KEY =
  "refact:chat-ui:project-storage-namespace:v1";

let projectStorageNamespace: string | null = null;
let projectStorageNamespaceTrusted = false;

const MAX_OPEN_CHAT_TABS = 50;
const MAX_WORKSPACE_TREE_NODES = MAX_WORKSPACE_TABS * (MAX_GROUP_LEAVES * 2);
const MAX_OPEN_TASKS = 25;
const MAX_PLANNER_CHATS_PER_TASK = 50;
const MAX_ASK_QUESTIONS_DRAFTS = 100;
const ASK_QUESTIONS_DRAFT_TTL_MS = 7 * 24 * 60 * 60 * 1000;

export type PersistedChatTab = {
  id: string;
  title?: string;
  mode?: string;
  tool_use?: "quick" | "explore" | "agent";
  session_state?: string;
  is_buddy_chat?: boolean;
  is_task_chat?: boolean;
};

export type PersistedChatTabsState = {
  openThreadIds: string[];
  currentThreadId: string;
  tabs: PersistedChatTab[];
};

export type PersistedActiveTab =
  | { type: "dashboard" }
  | { type: "chat"; id: string }
  | { type: "task"; taskId: string }
  | { type: "buddy" };

export type PersistedTaskActiveChat =
  | { type: "planner"; chatId: string }
  | { type: "agent"; cardId: string; chatId: string }
  | null;

export interface PersistedPlannerInfo {
  id: string;
  title: string;
  createdAt: string;
  updatedAt: string;
  sessionState?: string;
  waitingForCardIds?: string[];
}

export interface PersistedOpenTask {
  id: string;
  name: string;
  plannerChats: PersistedPlannerInfo[];
  activeChat: PersistedTaskActiveChat;
}

export interface PersistedTasksUIState {
  openTasks: PersistedOpenTask[];
}

export type AskQuestionsDraftValue = string | string[];

export type AskQuestionsDraft = {
  answers: Record<string, AskQuestionsDraftValue>;
  additionalText: string;
  updatedAt: number;
};

export type TaskWorkspaceTab = "board" | "chat" | "memories" | "documents";

const TASK_WORKSPACE_TABS = ["board", "chat", "memories", "documents"] as const;

function getStorage(): Storage | null {
  try {
    if (typeof localStorage === "undefined") return null;
    return localStorage;
  } catch {
    return null;
  }
}

function getSessionStorage(): Storage | null {
  try {
    if (typeof sessionStorage === "undefined") return null;
    return sessionStorage;
  } catch {
    return null;
  }
}

function normalizeProjectStorageNamespace(value: string | undefined): string {
  return value?.trim() ?? "";
}

function normalizeWorkspaceIdentityPart(value: string): string {
  const normalized = value.trim().replace(/\\/g, "/").replace(/\/+$/u, "");
  return normalized || value.trim();
}

function hashStorageIdentity(value: string): string {
  let hash = 0x811c9dc5;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0).toString(36);
}

function readSessionProjectStorageNamespace(): string | null {
  const storage = getSessionStorage();
  if (!storage) return null;

  try {
    const namespace = normalizeProjectStorageNamespace(
      storage.getItem(PROJECT_STORAGE_NAMESPACE_SESSION_KEY) ?? undefined,
    );
    return namespace || null;
  } catch {
    return null;
  }
}

function writeSessionProjectStorageNamespace(value: string | null): void {
  const storage = getSessionStorage();
  if (!storage) return;

  try {
    if (value) {
      storage.setItem(PROJECT_STORAGE_NAMESPACE_SESSION_KEY, value);
    } else {
      storage.removeItem(PROJECT_STORAGE_NAMESPACE_SESSION_KEY);
    }
  } catch {
    return;
  }
}

function trustedProjectScopedStorageKey(baseKey: string): string | null {
  if (!projectStorageNamespaceTrusted || !projectStorageNamespace) return null;
  return `refact:project:${projectStorageNamespace}:${baseKey}`;
}

export function getProjectStorageNamespace(): string | null {
  return projectStorageNamespace ?? readSessionProjectStorageNamespace();
}

export function isProjectStorageNamespaceTrusted(): boolean {
  return projectStorageNamespaceTrusted;
}

export function setProjectStorageNamespace(value: string | undefined): void {
  const next = normalizeProjectStorageNamespace(value);
  projectStorageNamespace = next ? next : null;
  projectStorageNamespaceTrusted = Boolean(projectStorageNamespace);
  writeSessionProjectStorageNamespace(projectStorageNamespace);
}

function firstNonEmpty(values: (string | undefined)[]): string | undefined {
  return values.map((value) => value?.trim()).find(Boolean);
}

export function setProjectStorageNamespaceFromProjectInfo(input: {
  workspaceRoots?: string[];
  projectName?: string;
  workspaceName?: string;
}): void {
  const roots = dedupeStrings(
    (input.workspaceRoots ?? []).map(normalizeWorkspaceIdentityPart),
  ).sort();
  const fallback = firstNonEmpty([input.projectName, input.workspaceName]);
  const identityParts = roots.length > 0 ? roots : fallback ? [fallback] : [];
  const identity = identityParts.join("\n");
  setProjectStorageNamespace(
    identity ? `v2:${hashStorageIdentity(identity)}` : undefined,
  );
}

function isRecord(value: unknown): value is JsonRecord {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function readRecord(key: string): JsonRecord | null {
  const storage = getStorage();
  if (!storage) return null;

  try {
    const raw = storage.getItem(key);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as unknown;
    return isRecord(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

function writeRecord(key: string, value: JsonRecord): void {
  const storage = getStorage();
  if (!storage) return;

  try {
    storage.setItem(key, JSON.stringify(value));
  } catch {
    return;
  }
}

function removeRecord(key: string): void {
  const storage = getStorage();
  if (!storage) return;

  try {
    storage.removeItem(key);
  } catch {
    return;
  }
}

function stringOrUndefined(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function booleanOrUndefined(value: unknown): boolean | undefined {
  return typeof value === "boolean" ? value : undefined;
}

function numberOrUndefined(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value)
    ? value
    : undefined;
}

function stringArrayOrEmpty(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.filter((item): item is string => typeof item === "string");
}

function dedupeStrings(values: string[]): string[] {
  const seen = new Set<string>();
  const result: string[] = [];

  for (const value of values) {
    const trimmed = value.trim();
    if (!trimmed || seen.has(trimmed)) continue;
    seen.add(trimmed);
    result.push(trimmed);
  }

  return result;
}

function normalizeToolUse(value: unknown): PersistedChatTab["tool_use"] {
  return value === "quick" || value === "explore" || value === "agent"
    ? value
    : undefined;
}

function normalizeChatTab(
  value: unknown,
  fallbackId?: string,
): PersistedChatTab | null {
  if (!isRecord(value)) {
    return fallbackId ? { id: fallbackId } : null;
  }

  const id = stringOrUndefined(value.id) ?? fallbackId;
  if (!id?.trim()) return null;

  return {
    id: id.trim(),
    title: stringOrUndefined(value.title),
    mode: stringOrUndefined(value.mode),
    tool_use: normalizeToolUse(value.tool_use),
    session_state: stringOrUndefined(value.session_state),
    is_buddy_chat: booleanOrUndefined(value.is_buddy_chat),
    is_task_chat: booleanOrUndefined(value.is_task_chat),
  };
}

export function loadPersistedChatTabs(): PersistedChatTabsState {
  const trustedKey = trustedProjectScopedStorageKey(CHAT_TABS_STORAGE_KEY);
  const record = trustedKey ? readRecord(trustedKey) : null;
  const rawOpenThreadIds = dedupeStrings(
    stringArrayOrEmpty(record?.openThreadIds).slice(-MAX_OPEN_CHAT_TABS),
  );
  const rawTabs = Array.isArray(record?.tabs) ? record.tabs : [];
  const tabsById = new Map<string, PersistedChatTab>();

  for (const rawTab of rawTabs) {
    const tab = normalizeChatTab(rawTab);
    if (tab) tabsById.set(tab.id, tab);
  }

  const openThreadIds = rawOpenThreadIds.filter((id) => tabsById.has(id));
  const tabs = openThreadIds.map(
    (id) => tabsById.get(id) ?? ({ id } satisfies PersistedChatTab),
  );
  const rawCurrentThreadId = stringOrUndefined(record?.currentThreadId) ?? "";
  const currentThreadId = openThreadIds.includes(rawCurrentThreadId)
    ? rawCurrentThreadId
    : openThreadIds[openThreadIds.length - 1] ?? "";

  return { openThreadIds, currentThreadId, tabs };
}

export function savePersistedChatTabs(input: PersistedChatTabsState): void {
  const storageKey = trustedProjectScopedStorageKey(CHAT_TABS_STORAGE_KEY);
  if (!storageKey) return;

  const existing = loadPersistedChatTabs();
  const tabsById = new Map<string, PersistedChatTab>();

  for (const tab of input.tabs) {
    tabsById.set(tab.id, tab);
  }

  const openThreadIds = dedupeStrings(
    input.openThreadIds
      .filter((id) => tabsById.has(id))
      .slice(-MAX_OPEN_CHAT_TABS),
  );
  const currentThreadId = openThreadIds.includes(input.currentThreadId)
    ? input.currentThreadId
    : openThreadIds.includes(existing.currentThreadId)
      ? existing.currentThreadId
      : openThreadIds[openThreadIds.length - 1] ?? "";

  writeRecord(storageKey, {
    version: 1,
    openThreadIds,
    currentThreadId,
    tabs: openThreadIds.map((id) => tabsById.get(id) ?? { id }),
    updatedAt: Date.now(),
  });
}

function collectPaneLeafIds(node: PaneNode): string[] {
  if (node.kind === "leaf") return [node.id];
  return node.children.flatMap((child) => collectPaneLeafIds(child));
}

function createFallbackWorkspace(): WorkspaceState {
  const persistedTabs = loadPersistedChatTabs();
  const fallbackThreadId = persistedTabs.currentThreadId
    ? persistedTabs.currentThreadId
    : persistedTabs.openThreadIds.at(-1) ?? null;
  const fallbackTabId = fallbackThreadId ? `chat:${fallbackThreadId}` : null;

  return {
    tabs: fallbackTabId ? [fallbackTabId] : [],
    activeTabId: fallbackTabId,
    groups: {},
  };
}

function normalizeSurfaceKey(value: unknown): SurfaceKey | null {
  const key = stringOrUndefined(value)?.trim();
  if (!key) return null;

  try {
    parseSurfaceKey(key);
    return key;
  } catch {
    return null;
  }
}

function isOpenWorkspaceSurface(
  surfaceKey: SurfaceKey,
  openThreadIds: ReadonlySet<string>,
): boolean {
  return (
    !isChatSurface(surfaceKey) ||
    openThreadIds.has(surfaceKey.slice("chat:".length))
  );
}

type WorkspaceNodeValidationContext = {
  openThreadIds: ReadonlySet<string>;
  seenNodeIds: Set<string>;
  totalNodeCount: { value: number };
  surfacePlacementCount: { value: number };
};

function normalizePersistedWorkspaceNode(
  value: unknown,
  context: WorkspaceNodeValidationContext,
): PaneNode | null {
  if (!isRecord(value)) return null;

  const id = stringOrUndefined(value.id)?.trim();
  if (!id || context.seenNodeIds.has(id)) return null;

  context.totalNodeCount.value += 1;
  if (context.totalNodeCount.value > MAX_WORKSPACE_TREE_NODES) return null;
  context.seenNodeIds.add(id);

  if (value.kind === "leaf") {
    if (!Array.isArray(value.tabIds)) return null;
    if (
      value.activeTabId !== null &&
      value.activeTabId !== undefined &&
      typeof value.activeTabId !== "string"
    ) {
      return null;
    }

    const tabIds: SurfaceKey[] = [];
    for (const rawSurfaceKey of value.tabIds) {
      const surfaceKey = normalizeSurfaceKey(rawSurfaceKey);
      if (!surfaceKey) return null;
      context.surfacePlacementCount.value += 1;
      if (context.surfacePlacementCount.value > MAX_WORKSPACE_TABS) return null;
      if (tabIds.includes(surfaceKey)) continue;
      tabIds.push(surfaceKey);
    }

    const rawActiveTabId =
      value.activeTabId === null || value.activeTabId === undefined
        ? null
        : normalizeSurfaceKey(value.activeTabId);
    if (value.activeTabId && !rawActiveTabId) return null;
    const activeTabId =
      rawActiveTabId && tabIds.includes(rawActiveTabId)
        ? rawActiveTabId
        : tabIds[0] ?? null;

    return {
      kind: "leaf",
      id,
      tabIds,
      activeTabId,
    };
  }

  if (value.kind === "split") {
    const dir = value.dir === "row" || value.dir === "col" ? value.dir : null;
    if (!dir) return null;
    if (!Array.isArray(value.children) || value.children.length < 2) {
      return null;
    }
    if (
      !Array.isArray(value.sizes) ||
      value.sizes.length !== value.children.length
    ) {
      return null;
    }

    const sizes = value.sizes.filter(
      (size): size is number =>
        typeof size === "number" && Number.isFinite(size) && size > 0,
    );
    if (sizes.length !== value.sizes.length) return null;

    const children: PaneNode[] = [];
    for (const child of value.children) {
      const node = normalizePersistedWorkspaceNode(child, context);
      if (!node) return null;
      children.push(node);
    }

    const sizeSum = sizes.reduce((total, size) => total + size, 0);
    if (sizeSum <= 0) return null;

    return {
      kind: "split",
      id,
      dir,
      children,
      sizes: sizes.map((size) => size / sizeSum),
    };
  }

  return null;
}

function normalizePersistedWorkspaceGroup(
  value: unknown,
  openThreadIds: ReadonlySet<string>,
  totalNodeCount: { value: number },
): PaneGroup | null {
  if (!isRecord(value)) return null;

  const context: WorkspaceNodeValidationContext = {
    openThreadIds,
    seenNodeIds: new Set<string>(),
    totalNodeCount,
    surfacePlacementCount: { value: 0 },
  };
  const root = normalizePersistedWorkspaceNode(value.root, context);
  if (!root) return null;

  const leafIds = collectPaneLeafIds(root);
  const surfaceCount = collectTabIds(root).length;
  if (leafIds.length < 2 || leafIds.length > MAX_GROUP_LEAVES) return null;
  if (surfaceCount === 0 || surfaceCount > MAX_WORKSPACE_TABS) return null;

  const rawFocusedLeafId = stringOrUndefined(value.focusedLeafId)?.trim();
  const focusedLeafId =
    rawFocusedLeafId && leafIds.includes(rawFocusedLeafId)
      ? rawFocusedLeafId
      : leafIds[0];

  return { root, focusedLeafId };
}

export function loadPersistedWorkspace(): WorkspaceState {
  const fallback = createFallbackWorkspace();
  const trustedKey = trustedProjectScopedStorageKey(WORKSPACE_STORAGE_KEY);
  const record = trustedKey ? readRecord(trustedKey) : null;
  if (!record || record.version !== 2) return fallback;

  const persistedTabs = loadPersistedChatTabs();
  const openThreadIds = new Set(persistedTabs.openThreadIds);
  if (!Array.isArray(record.tabs) || record.tabs.length > MAX_WORKSPACE_TABS) {
    return fallback;
  }

  const tabs: SurfaceKey[] = [];
  for (const rawSurfaceKey of record.tabs) {
    const surfaceKey = normalizeSurfaceKey(rawSurfaceKey);
    if (!surfaceKey) return fallback;
    if (!isOpenWorkspaceSurface(surfaceKey, openThreadIds)) continue;
    if (tabs.includes(surfaceKey)) continue;
    tabs.push(surfaceKey);
  }

  if (tabs.length === 0 && persistedTabs.openThreadIds.length > 0) {
    return fallback;
  }

  if (!isRecord(record.groups)) return fallback;

  const totalNodeCount = { value: 0 };
  const groups: WorkspaceGroups = {};
  for (const [rawTabId, rawGroup] of Object.entries(record.groups)) {
    const tabId = normalizeSurfaceKey(rawTabId);
    if (!tabId) return fallback;
    if (!tabs.includes(tabId)) continue;
    if (rawGroup === null || rawGroup === undefined) continue;

    const group = normalizePersistedWorkspaceGroup(
      rawGroup,
      openThreadIds,
      totalNodeCount,
    );
    if (!group) return fallback;
    groups[tabId] = group;
  }

  if (
    record.activeTabId !== null &&
    record.activeTabId !== undefined &&
    typeof record.activeTabId !== "string"
  ) {
    return fallback;
  }
  const rawActiveTabId =
    record.activeTabId === null || record.activeTabId === undefined
      ? null
      : normalizeSurfaceKey(record.activeTabId);
  if (record.activeTabId && !rawActiveTabId) return fallback;

  return reconcileWorkspaceState(
    {
      tabs,
      activeTabId:
        rawActiveTabId && tabs.includes(rawActiveTabId)
          ? rawActiveTabId
          : tabs[0] ?? null,
      groups,
    },
    persistedTabs.openThreadIds,
  );
}

export function savePersistedWorkspace(workspace: WorkspaceState): void {
  const storageKey = trustedProjectScopedStorageKey(WORKSPACE_STORAGE_KEY);
  if (!storageKey) return;

  writeRecord(storageKey, {
    version: 2,
    tabs: workspace.tabs.slice(0, MAX_WORKSPACE_TABS),
    activeTabId: workspace.activeTabId,
    groups: workspace.groups,
    updatedAt: Date.now(),
  });
}

function normalizeActiveTab(value: unknown): PersistedActiveTab | null {
  if (!isRecord(value)) return null;
  if (value.type === "dashboard") return { type: "dashboard" };
  if (value.type === "buddy") return { type: "buddy" };

  if (value.type === "chat") {
    const id = stringOrUndefined(value.id)?.trim();
    return id ? { type: "chat", id } : null;
  }

  if (value.type === "task") {
    const taskId = stringOrUndefined(value.taskId)?.trim();
    return taskId ? { type: "task", taskId } : null;
  }

  return null;
}

export function loadPersistedActiveTab(): PersistedActiveTab | null {
  const trustedKey = trustedProjectScopedStorageKey(ACTIVE_TAB_STORAGE_KEY);
  const record = trustedKey ? readRecord(trustedKey) : null;
  return normalizeActiveTab(record?.activeTab);
}

export function savePersistedActiveTab(activeTab: PersistedActiveTab): void {
  const storageKey = trustedProjectScopedStorageKey(ACTIVE_TAB_STORAGE_KEY);
  if (!storageKey) return;

  writeRecord(storageKey, {
    version: 1,
    activeTab,
    updatedAt: Date.now(),
  });
}

function normalizeTaskActiveChat(value: unknown): PersistedTaskActiveChat {
  if (!isRecord(value)) return null;

  if (value.type === "planner") {
    const chatId = stringOrUndefined(value.chatId)?.trim();
    return chatId ? { type: "planner", chatId } : null;
  }

  if (value.type === "agent") {
    const cardId = stringOrUndefined(value.cardId)?.trim();
    const chatId = stringOrUndefined(value.chatId)?.trim();
    return cardId && chatId ? { type: "agent", cardId, chatId } : null;
  }

  return null;
}

function normalizePlannerInfo(value: unknown): PersistedPlannerInfo | null {
  if (!isRecord(value)) return null;
  const id = stringOrUndefined(value.id)?.trim();
  if (!id) return null;

  const rawWaiting = Array.isArray(value.waitingForCardIds)
    ? value.waitingForCardIds
        .filter((item): item is string => typeof item === "string")
        .slice(0, 50)
    : undefined;

  return {
    id,
    title: stringOrUndefined(value.title) ?? "",
    createdAt: stringOrUndefined(value.createdAt) ?? "",
    updatedAt: stringOrUndefined(value.updatedAt) ?? "",
    sessionState: stringOrUndefined(value.sessionState),
    waitingForCardIds: rawWaiting,
  };
}

function normalizeOpenTask(value: unknown): PersistedOpenTask | null {
  if (!isRecord(value)) return null;
  const id = stringOrUndefined(value.id)?.trim();
  if (!id) return null;

  const rawPlannerChats = Array.isArray(value.plannerChats)
    ? value.plannerChats
    : [];
  const plannerChats = rawPlannerChats
    .map(normalizePlannerInfo)
    .filter((planner): planner is PersistedPlannerInfo => planner !== null)
    .slice(-MAX_PLANNER_CHATS_PER_TASK);

  const name = stringOrUndefined(value.name)?.trim();

  return {
    id,
    name: name?.length ? name : "Task",
    plannerChats,
    activeChat: normalizeTaskActiveChat(value.activeChat),
  };
}

export function loadPersistedTasksUIState(): PersistedTasksUIState {
  const trustedKey = trustedProjectScopedStorageKey(TASKS_UI_STORAGE_KEY);
  const record = trustedKey ? readRecord(trustedKey) : null;
  const rawOpenTasks = Array.isArray(record?.openTasks) ? record.openTasks : [];
  const openTasks = rawOpenTasks
    .map(normalizeOpenTask)
    .filter((task): task is PersistedOpenTask => task !== null)
    .slice(-MAX_OPEN_TASKS);

  return { openTasks };
}

export function savePersistedTasksUIState(state: PersistedTasksUIState): void {
  const storageKey = trustedProjectScopedStorageKey(TASKS_UI_STORAGE_KEY);
  if (!storageKey) return;

  writeRecord(storageKey, {
    version: 1,
    openTasks: state.openTasks.slice(-MAX_OPEN_TASKS),
    updatedAt: Date.now(),
  });
}

function normalizeAskQuestionsAnswers(
  value: unknown,
): Record<string, AskQuestionsDraftValue> {
  if (!isRecord(value)) return {};

  const result: Record<string, AskQuestionsDraftValue> = {};
  for (const [key, rawAnswer] of Object.entries(value)) {
    if (!key.trim()) continue;
    if (typeof rawAnswer === "string") {
      result[key] = rawAnswer;
      continue;
    }
    if (Array.isArray(rawAnswer)) {
      const values = rawAnswer.filter(
        (item): item is string => typeof item === "string",
      );
      result[key] = values;
    }
  }

  return result;
}

function normalizeAskQuestionsDraft(value: unknown): AskQuestionsDraft | null {
  if (!isRecord(value)) return null;

  return {
    answers: normalizeAskQuestionsAnswers(value.answers),
    additionalText: stringOrUndefined(value.additionalText) ?? "",
    updatedAt: numberOrUndefined(value.updatedAt) ?? Date.now(),
  };
}

function loadAskQuestionsDrafts(): Record<string, AskQuestionsDraft> {
  const record = readRecord(ASK_QUESTIONS_STORAGE_KEY);
  const draftsRecord = isRecord(record?.drafts) ? record.drafts : {};
  const drafts: Record<string, AskQuestionsDraft> = {};
  const cutoff = Date.now() - ASK_QUESTIONS_DRAFT_TTL_MS;

  for (const [toolCallId, value] of Object.entries(draftsRecord)) {
    const draft = normalizeAskQuestionsDraft(value);
    if (!draft || draft.updatedAt < cutoff) continue;
    drafts[toolCallId] = draft;
  }

  return drafts;
}

function saveAskQuestionsDrafts(
  drafts: Record<string, AskQuestionsDraft>,
): void {
  const entries = Object.entries(drafts)
    .sort(([, left], [, right]) => right.updatedAt - left.updatedAt)
    .slice(0, MAX_ASK_QUESTIONS_DRAFTS);

  writeRecord(ASK_QUESTIONS_STORAGE_KEY, {
    version: 1,
    drafts: Object.fromEntries(entries),
    updatedAt: Date.now(),
  });
}

export function loadAskQuestionsDraft(
  toolCallId: string | undefined,
): AskQuestionsDraft | null {
  if (!toolCallId) return null;
  const drafts = loadAskQuestionsDrafts() as Record<
    string,
    AskQuestionsDraft | undefined
  >;
  return drafts[toolCallId] ?? null;
}

export function saveAskQuestionsDraft(
  toolCallId: string | undefined,
  answers: Record<string, AskQuestionsDraftValue>,
  additionalText: string,
): void {
  if (!toolCallId) return;
  const drafts = loadAskQuestionsDrafts();
  drafts[toolCallId] = {
    answers,
    additionalText,
    updatedAt: Date.now(),
  };
  saveAskQuestionsDrafts(drafts);
}

export function clearAskQuestionsDraft(toolCallId: string | undefined): void {
  if (!toolCallId) return;
  const drafts = loadAskQuestionsDrafts();
  const { [toolCallId]: _, ...rest } = drafts;

  if (Object.keys(rest).length === 0) {
    removeRecord(ASK_QUESTIONS_STORAGE_KEY);
    return;
  }

  saveAskQuestionsDrafts(rest);
}

function normalizeTaskWorkspaceTab(value: unknown): TaskWorkspaceTab | null {
  return typeof value === "string" &&
    (TASK_WORKSPACE_TABS as readonly string[]).includes(value)
    ? (value as TaskWorkspaceTab)
    : null;
}

function loadTaskWorkspaceTabs(): Record<string, TaskWorkspaceTab> {
  const trustedKey = trustedProjectScopedStorageKey(
    TASK_WORKSPACE_TABS_STORAGE_KEY,
  );
  const record = trustedKey ? readRecord(trustedKey) : null;
  const tabsRecord = isRecord(record?.tabs) ? record.tabs : {};
  const result: Record<string, TaskWorkspaceTab> = {};

  for (const [taskId, value] of Object.entries(tabsRecord)) {
    const tab = normalizeTaskWorkspaceTab(value);
    if (tab) result[taskId] = tab;
  }

  return result;
}

export function loadTaskWorkspaceTab(taskId: string): TaskWorkspaceTab | null {
  const tabs = loadTaskWorkspaceTabs() as Record<
    string,
    TaskWorkspaceTab | undefined
  >;
  return tabs[taskId] ?? null;
}

export function saveTaskWorkspaceTab(
  taskId: string,
  tab: TaskWorkspaceTab,
): void {
  if (!taskId.trim()) return;
  const storageKey = trustedProjectScopedStorageKey(
    TASK_WORKSPACE_TABS_STORAGE_KEY,
  );
  if (!storageKey) return;

  const tabs = loadTaskWorkspaceTabs();
  tabs[taskId] = tab;
  writeRecord(storageKey, {
    version: 1,
    tabs,
    updatedAt: Date.now(),
  });
}
