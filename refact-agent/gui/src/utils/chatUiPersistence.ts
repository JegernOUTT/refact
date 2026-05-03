type JsonRecord = Record<string, unknown>;

const CHAT_TABS_STORAGE_KEY = "refact:chat-ui:tabs:v1";
const ACTIVE_TAB_STORAGE_KEY = "refact:chat-ui:active-tab:v1";
const TASKS_UI_STORAGE_KEY = "refact:chat-ui:tasks-ui:v1";
const ASK_QUESTIONS_STORAGE_KEY = "refact:chat-ui:ask-questions:v1";
const TASK_WORKSPACE_LAYOUT_STORAGE_KEY =
  "refact:chat-ui:task-workspace-layouts:v1";

const MAX_OPEN_CHAT_TABS = 50;
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
};

export type PersistedChatTabsState = {
  openThreadIds: string[];
  currentThreadId: string;
  tabs: PersistedChatTab[];
};

export type PersistedActiveTab =
  | { type: "dashboard" }
  | { type: "chat"; id: string }
  | { type: "task"; taskId: string };

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

export type TaskWorkspaceLayout = {
  chatExpanded: boolean;
  panelsExpanded: boolean;
  boardHeightPx: number;
};

function getStorage(): Storage | null {
  try {
    if (typeof localStorage === "undefined") return null;
    return localStorage;
  } catch {
    return null;
  }
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
  };
}

export function loadPersistedChatTabs(): PersistedChatTabsState {
  const record = readRecord(CHAT_TABS_STORAGE_KEY);
  const rawOpenThreadIds = dedupeStrings(
    stringArrayOrEmpty(record?.openThreadIds).slice(-MAX_OPEN_CHAT_TABS),
  );
  const rawTabs = Array.isArray(record?.tabs) ? record.tabs : [];
  const tabsById = new Map<string, PersistedChatTab>();

  for (const rawTab of rawTabs) {
    const tab = normalizeChatTab(rawTab);
    if (tab && !tab.is_buddy_chat) tabsById.set(tab.id, tab);
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
  const existing = loadPersistedChatTabs();
  const tabsById = new Map<string, PersistedChatTab>();

  for (const tab of input.tabs) {
    if (!tab.is_buddy_chat) tabsById.set(tab.id, tab);
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

  writeRecord(CHAT_TABS_STORAGE_KEY, {
    version: 1,
    openThreadIds,
    currentThreadId,
    tabs: openThreadIds.map((id) => tabsById.get(id) ?? { id }),
    updatedAt: Date.now(),
  });
}

function normalizeActiveTab(value: unknown): PersistedActiveTab | null {
  if (!isRecord(value)) return null;
  if (value.type === "dashboard") return { type: "dashboard" };

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
  return normalizeActiveTab(readRecord(ACTIVE_TAB_STORAGE_KEY)?.activeTab);
}

export function savePersistedActiveTab(activeTab: PersistedActiveTab): void {
  writeRecord(ACTIVE_TAB_STORAGE_KEY, {
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

  return {
    id,
    title: stringOrUndefined(value.title) ?? "",
    createdAt: stringOrUndefined(value.createdAt) ?? "",
    updatedAt: stringOrUndefined(value.updatedAt) ?? "",
    sessionState: stringOrUndefined(value.sessionState),
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
  const record = readRecord(TASKS_UI_STORAGE_KEY);
  const rawOpenTasks = Array.isArray(record?.openTasks) ? record.openTasks : [];
  const openTasks = rawOpenTasks
    .map(normalizeOpenTask)
    .filter((task): task is PersistedOpenTask => task !== null)
    .slice(-MAX_OPEN_TASKS);

  return { openTasks };
}

export function savePersistedTasksUIState(state: PersistedTasksUIState): void {
  writeRecord(TASKS_UI_STORAGE_KEY, {
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

function loadTaskWorkspaceLayouts(): Record<string, TaskWorkspaceLayout> {
  const record = readRecord(TASK_WORKSPACE_LAYOUT_STORAGE_KEY);
  const layoutsRecord = isRecord(record?.layouts) ? record.layouts : {};
  const result: Record<string, TaskWorkspaceLayout> = {};

  for (const [taskId, value] of Object.entries(layoutsRecord)) {
    if (!isRecord(value)) continue;
    const boardHeightPx = numberOrUndefined(value.boardHeightPx);
    if (boardHeightPx === undefined) continue;
    result[taskId] = {
      chatExpanded: booleanOrUndefined(value.chatExpanded) ?? false,
      panelsExpanded: booleanOrUndefined(value.panelsExpanded) ?? false,
      boardHeightPx,
    };
  }

  return result;
}

export function loadTaskWorkspaceLayout(
  taskId: string,
  defaults: TaskWorkspaceLayout,
): TaskWorkspaceLayout {
  const layouts = loadTaskWorkspaceLayouts() as Record<
    string,
    TaskWorkspaceLayout | undefined
  >;
  const layout = layouts[taskId];
  return layout ? { ...defaults, ...layout } : defaults;
}

export function saveTaskWorkspaceLayout(
  taskId: string,
  layout: TaskWorkspaceLayout,
): void {
  if (!taskId.trim()) return;
  const layouts = loadTaskWorkspaceLayouts();
  layouts[taskId] = layout;
  writeRecord(TASK_WORKSPACE_LAYOUT_STORAGE_KEY, {
    version: 1,
    layouts,
    updatedAt: Date.now(),
  });
}
