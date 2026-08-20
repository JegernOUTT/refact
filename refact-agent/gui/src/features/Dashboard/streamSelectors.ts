import { createSelector } from "@reduxjs/toolkit";
import type { RootState } from "../../app/store";
import {
  buildHistoryTree,
  type ChatHistoryItem,
  type HistoryTreeNode,
} from "../History/historySlice";
import type { TaskMeta } from "../../services/refact/tasks";
import { getDateGroup } from "./dateUtils";

export type StreamKind = "chat" | "task";

export type StreamStatus =
  | "streaming"
  | "working"
  | "idle"
  | "done"
  | "failed"
  | "paused"
  | "planning";

export type StreamItem = {
  kind: StreamKind;
  id: string;
  title: string;
  updatedAtMs: number;
  createdAtMs: number | null;
  status: StreamStatus;
  mode: string | null;
  model: string | null;
  messageCount: number | null;
  diff: { adds: number; dels: number } | null;
  branch: string | null;
  costUsd: number | null;
  totalTokens: number | null;
  cacheReadTokens: number | null;
  familyChildCount: number;
  cardsDone: number | null;
  cardsTotal: number | null;
  cardsFailed: number | null;
  agentsActive: number | null;
  linkedChats: number | null;
};

export type StreamGroup = { label: string; items: StreamItem[] };

export type FamilyChild = {
  id: string;
  title: string;
  linkType: string | null;
  status: StreamStatus;
  updatedAtMs: number;
  messageCount: number | null;
  depth: number;
};

export type StreamFilter = { kind: "all" | "chat" | "task"; query: string };

const EMPTY_CHATS: Record<string, ChatHistoryItem> = {};
const EMPTY_TASKS: TaskMeta[] = [];
const EMPTY_QUERIES: Record<string, unknown> = {};
const EMPTY_ITEMS: StreamItem[] = [];
const EMPTY_GROUPS: StreamGroup[] = [];
const EMPTY_FAMILY_CHILDREN: FamilyChild[] = [];

const ACTIVE_NOW_LABEL = "Active now";

/** Most-active-wins ordering used when a whole family collapses into one row. */
const STATUS_PRIORITY: Record<StreamStatus, number> = {
  streaming: 0,
  working: 1,
  failed: 2,
  paused: 3,
  planning: 4,
  idle: 5,
  done: 6,
};

function toMs(value?: string | null): number {
  if (!value) return 0;
  const parsed = Date.parse(value);
  return Number.isNaN(parsed) ? 0 : parsed;
}

function toMsOrNull(value?: string | null): number | null {
  if (!value) return null;
  const parsed = Date.parse(value);
  return Number.isNaN(parsed) ? null : parsed;
}

function numOrNull(value?: number | null): number | null {
  return typeof value === "number" && !Number.isNaN(value) ? value : null;
}

function sumOrNull(values: (number | null | undefined)[]): number | null {
  let total = 0;
  let seen = false;
  for (const value of values) {
    if (typeof value === "number" && !Number.isNaN(value)) {
      total += value;
      seen = true;
    }
  }
  return seen ? total : null;
}

/**
 * Chat session states as produced by the engine (see ChatHistoryItem["session_state"]
 * and utils/sessionStatus.ts). Mapped exhaustively, unknown values fall back to idle.
 */
export function chatSessionStateToStatus(
  sessionState?: string | null,
): StreamStatus {
  switch (sessionState) {
    case "generating":
      return "streaming";
    case "executing_tools":
      return "working";
    case "paused":
    case "waiting_ide":
    case "waiting_user_input":
      return "paused";
    case "error":
      return "failed";
    case "completed":
      return "done";
    case "idle":
      return "idle";
    default:
      return "idle";
  }
}

export function taskToStatus(task: TaskMeta): StreamStatus {
  const plannerState = task.planner_session_state;
  const agentsActive = numOrNull(task.agents_active) ?? 0;
  if (plannerState === "error") return "failed";
  if (task.status === "abandoned") return "failed";
  if (task.status === "completed") return "done";
  if (task.status === "paused" || plannerState === "paused") return "paused";
  if (plannerState === "generating") return "streaming";
  if (plannerState === "executing_tools") return "working";
  if (agentsActive > 0) return "working";
  if (task.status === "planning") return "planning";
  if (plannerState === "waiting_ide" || plannerState === "waiting_user_input") {
    return "paused";
  }
  return "idle";
}

export function isActiveStatus(status: StreamStatus): boolean {
  return status === "streaming" || status === "working";
}

function isActiveItem(item: StreamItem): boolean {
  return isActiveStatus(item.status) || (item.agentsActive ?? 0) > 0;
}

function collectFamily(node: HistoryTreeNode, out: HistoryTreeNode[]): void {
  out.push(node);
  for (const child of node.children) collectFamily(child, out);
  for (const child of node.bubbleChildren) collectFamily(child, out);
}

function branchOf(node: HistoryTreeNode): string | null {
  const branch = node.worktree?.branch?.trim();
  if (branch !== undefined && branch.length > 0) return branch;
  return null;
}

type FamilyStream = {
  item: StreamItem;
  /** Root title + every descendant title, lowercased, for query matching. */
  searchText: string;
};

function familyToStream(root: HistoryTreeNode): FamilyStream {
  const family: HistoryTreeNode[] = [];
  collectFamily(root, family);

  let updatedAtMs = toMs(root.updatedAt);
  let status = chatSessionStateToStatus(root.session_state);
  let branch = branchOf(root);
  let adds = 0;
  let dels = 0;
  const titles: string[] = [];

  for (const node of family) {
    updatedAtMs = Math.max(updatedAtMs, toMs(node.updatedAt));
    const nodeStatus = chatSessionStateToStatus(node.session_state);
    if (STATUS_PRIORITY[nodeStatus] < STATUS_PRIORITY[status]) {
      status = nodeStatus;
    }
    if (branch === null) branch = branchOf(node);
    adds += node.total_lines_added ?? 0;
    dels += node.total_lines_removed ?? 0;
    titles.push(node.title);
  }

  const item: StreamItem = {
    kind: "chat",
    id: root.id,
    title: root.title.length > 0 ? root.title : "New Chat",
    updatedAtMs,
    createdAtMs: toMsOrNull(root.createdAt),
    status,
    mode: root.mode ?? null,
    model: root.model.length > 0 ? root.model : null,
    messageCount: sumOrNull(family.map((node) => node.message_count)),
    diff: adds > 0 || dels > 0 ? { adds, dels } : null,
    branch,
    costUsd: sumOrNull(family.map((node) => node.total_cost_usd)),
    totalTokens: sumOrNull(family.map((node) => node.total_tokens)),
    cacheReadTokens: sumOrNull(
      family.map((node) => node.total_cache_read_tokens),
    ),
    familyChildCount: family.length - 1,
    cardsDone: null,
    cardsTotal: null,
    cardsFailed: null,
    agentsActive: null,
    linkedChats: null,
  };

  return { item, searchText: titles.join("\n").toLowerCase() };
}

function taskToStream(
  task: TaskMeta,
  linkedChatsByTask: Record<string, number>,
): FamilyStream {
  const linked = linkedChatsByTask[task.id];
  const item: StreamItem = {
    kind: "task",
    id: task.id,
    title: task.name.length > 0 ? task.name : "Untitled task",
    updatedAtMs: toMs(task.updated_at),
    createdAtMs: toMsOrNull(task.created_at),
    status: taskToStatus(task),
    mode: task.status,
    model:
      task.default_agent_model !== undefined &&
      task.default_agent_model.length > 0
        ? task.default_agent_model
        : null,
    messageCount: null,
    diff: null,
    branch:
      task.base_branch !== undefined && task.base_branch.length > 0
        ? task.base_branch
        : null,
    costUsd: null,
    totalTokens: null,
    cacheReadTokens: null,
    familyChildCount: 0,
    cardsDone: numOrNull(task.cards_done),
    cardsTotal: numOrNull(task.cards_total),
    cardsFailed: numOrNull(task.cards_failed),
    agentsActive: numOrNull(task.agents_active),
    linkedChats: typeof linked === "number" ? linked : null,
  };
  return { item, searchText: item.title.toLowerCase() };
}

// --- base state slices -------------------------------------------------------

export const selectHistoryChats = (
  state: RootState,
): Record<string, ChatHistoryItem> => {
  const history = (
    state as unknown as {
      history?: { chats?: Record<string, ChatHistoryItem> };
    }
  ).history;
  return history?.chats ?? EMPTY_CHATS;
};

type LooseQueryEntry = { endpointName?: string; data?: unknown } | undefined;

const selectTasksApiQueries = (state: RootState): Record<string, unknown> => {
  const tasksApiState = (
    state as unknown as {
      tasksApi?: { queries?: Record<string, unknown> };
    }
  ).tasksApi;
  return tasksApiState?.queries ?? EMPTY_QUERIES;
};

/** Reads the same `listTasks` RTK Query cache entry the Dashboard TasksSection reads. */
export const selectTaskMetas = createSelector(
  [selectTasksApiQueries],
  (queries): TaskMeta[] => {
    for (const value of Object.values(queries)) {
      const entry = value as LooseQueryEntry;
      if (entry?.endpointName === "listTasks" && Array.isArray(entry.data)) {
        return entry.data as TaskMeta[];
      }
    }
    return EMPTY_TASKS;
  },
);

export const selectHistoryTree = createSelector(
  [selectHistoryChats],
  (chats): HistoryTreeNode[] => buildHistoryTree(chats),
);

const selectLinkedChatsByTask = createSelector(
  [selectHistoryChats],
  (chats): Record<string, number> => {
    const counts = new Map<string, number>();
    for (const chat of Object.values(chats)) {
      const taskId = chat.task_id;
      if (!taskId) continue;
      if (!chat.card_id && !chat.agent_id) continue;
      counts.set(taskId, (counts.get(taskId) ?? 0) + 1);
    }
    return Object.fromEntries(counts);
  },
);

const selectStreams = createSelector(
  [selectHistoryTree, selectTaskMetas, selectLinkedChatsByTask],
  (roots, tasks, linkedChatsByTask): FamilyStream[] => {
    const streams: FamilyStream[] = roots.map((root) => familyToStream(root));
    for (const task of tasks) {
      streams.push(taskToStream(task, linkedChatsByTask));
    }
    streams.sort((a, b) => b.item.updatedAtMs - a.item.updatedAtMs);
    return streams;
  },
);

export const selectStreamItems = createSelector(
  [selectStreams],
  (streams): StreamItem[] =>
    streams.length === 0 ? EMPTY_ITEMS : streams.map((stream) => stream.item),
);

// --- public selectors --------------------------------------------------------

const selectFilterKind = (_state: RootState, filter: StreamFilter) =>
  filter.kind;
const selectFilterQuery = (_state: RootState, filter: StreamFilter) =>
  filter.query;

export const selectStreamGroups = createSelector(
  [selectStreams, selectFilterKind, selectFilterQuery],
  (streams, kind, query): StreamGroup[] => {
    const needle = query.trim().toLowerCase();
    const matched = streams.filter((stream) => {
      if (kind !== "all" && stream.item.kind !== kind) return false;
      if (needle.length === 0) return true;
      return stream.searchText.includes(needle);
    });

    if (matched.length === 0) return EMPTY_GROUPS;

    const buckets = new Map<string, StreamItem[]>();
    const push = (label: string, item: StreamItem) => {
      const existing = buckets.get(label);
      if (existing) existing.push(item);
      else buckets.set(label, [item]);
    };

    for (const { item } of matched) {
      if (isActiveItem(item)) {
        push(ACTIVE_NOW_LABEL, item);
        continue;
      }
      push(getDateGroup(new Date(item.updatedAtMs).toISOString()), item);
    }

    const order = [ACTIVE_NOW_LABEL, "Today", "Yesterday", "Earlier"];
    const groups: StreamGroup[] = [];
    for (const label of order) {
      const items = buckets.get(label);
      if (!items || items.length === 0) continue;
      items.sort((a, b) => b.updatedAtMs - a.updatedAtMs);
      groups.push({ label, items });
    }
    return groups;
  },
);

export const selectContinueItems = createSelector(
  [selectStreamItems],
  (items): StreamItem[] => {
    if (items.length === 0) return EMPTY_ITEMS;
    const active = items
      .filter((item) => isActiveItem(item))
      .sort((a, b) => b.updatedAtMs - a.updatedAtMs);
    const rest = items
      .filter((item) => !isActiveItem(item))
      .sort((a, b) => b.updatedAtMs - a.updatedAtMs);
    return [...active, ...rest].slice(0, 3);
  },
);

export const selectAttentionItems = createSelector(
  [selectStreamItems],
  (items): StreamItem[] => {
    const picked = items.filter(
      (item) =>
        (item.kind === "task" && (item.cardsFailed ?? 0) > 0) ||
        item.status === "failed" ||
        item.status === "paused",
    );
    if (picked.length === 0) return EMPTY_ITEMS;
    return [...picked].sort((a, b) => b.updatedAtMs - a.updatedAtMs);
  },
);

const selectRootId = (_state: RootState, rootId: string) => rootId;

export const selectFamilyChildren = createSelector(
  [selectHistoryTree, selectRootId],
  (roots, rootId): FamilyChild[] => {
    const findRoot = (nodes: HistoryTreeNode[]): HistoryTreeNode | null => {
      for (const node of nodes) {
        if (node.id === rootId) return node;
        const inMain = findRoot(node.children);
        if (inMain) return inMain;
        const inBubble = findRoot(node.bubbleChildren);
        if (inBubble) return inBubble;
      }
      return null;
    };

    const root = findRoot(roots);
    if (!root) return EMPTY_FAMILY_CHILDREN;

    const out: FamilyChild[] = [];
    const walk = (node: HistoryTreeNode, depth: number) => {
      for (const child of [...node.children, ...node.bubbleChildren]) {
        out.push({
          id: child.id,
          title: child.title.length > 0 ? child.title : "New Chat",
          linkType: child.link_type ?? null,
          status: chatSessionStateToStatus(child.session_state),
          updatedAtMs: toMs(child.updatedAt),
          messageCount: numOrNull(child.message_count),
          depth,
        });
        walk(child, depth + 1);
      }
    };
    walk(root, 1);

    return out.length === 0 ? EMPTY_FAMILY_CHILDREN : out;
  },
);

export type TodayAggregate = {
  chats: number;
  costUsd: number;
  tokens: number;
};

export const selectTodayAggregate = createSelector(
  [selectHistoryChats],
  (chats): TodayAggregate => {
    let count = 0;
    let costUsd = 0;
    let tokens = 0;
    for (const chat of Object.values(chats)) {
      if (chat.updatedAt.length === 0) continue;
      if (getDateGroup(chat.updatedAt) !== "Today") continue;
      count += 1;
      costUsd += chat.total_cost_usd ?? 0;
      tokens += chat.total_tokens ?? 0;
    }
    return { chats: count, costUsd, tokens };
  },
);
