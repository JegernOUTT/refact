import {
  createSlice,
  PayloadAction,
  createListenerMiddleware,
} from "@reduxjs/toolkit";
import {
  backUpMessages,
  ChatThread,
  normalizeLegacyMode,
  maybeAppendToolCallResultFromIdeToMessages,
  setChatMode,
  SuggestedChat,
  applyChatEvent,
  newChatAction,
  createChatWithId,
  restoreChat,
  switchToThread,
} from "../Chat/Thread";
import {
  trajectoriesApi,
  TrajectoryData,
  TrajectoryMeta,
  trajectoryDataToChatThread,
} from "../../services/refact";
import type { WorktreeMeta } from "../../services/refact/worktrees";
import { AppDispatch, RootState } from "../../app/store";
import { ideToolCallResponse } from "../../hooks/useEventBusForIDE";

export type ChatHistoryItem = Omit<ChatThread, "new_chat_suggested"> & {
  createdAt: string;
  updatedAt: string;
  title: string;
  isTitleGenerated?: boolean;
  new_chat_suggested?: SuggestedChat;
  parent_id?: string;
  link_type?: string;
  task_id?: string;
  task_role?: string;
  agent_id?: string;
  card_id?: string;
  worktree?: WorktreeMeta | null;
  session_state?:
    | "idle"
    | "generating"
    | "executing_tools"
    | "paused"
    | "waiting_ide"
    | "waiting_user_input"
    | "completed"
    | "error";
  message_count?: number;
  root_chat_id?: string;
  total_lines_added?: number;
  total_lines_removed?: number;
  tasks_total?: number;
  tasks_done?: number;
  tasks_failed?: number;
  total_prompt_tokens?: number;
  total_completion_tokens?: number;
  total_tokens?: number;
  total_cache_read_tokens?: number;
  total_cache_creation_tokens?: number;
  total_cost_usd?: number;
};

export function isTaskChatLike(
  x: Partial<Pick<ChatHistoryItem, "mode">>,
): boolean {
  const mode = x.mode?.toLowerCase();
  return mode === "task_agent" || mode === "task_planner";
}

export function isBuddyChatLike(
  x: Partial<Pick<ChatHistoryItem, "buddy_meta">>,
): boolean {
  return Boolean(x.buddy_meta?.is_buddy_chat);
}

const MAIN_CHAT_LINK_TYPES = new Set(["handoff", "mode_transition", "branch"]);

export function isSubagenticChatLike(
  x: Partial<Pick<ChatHistoryItem, "parent_id" | "link_type">>,
): boolean {
  return Boolean(
    x.parent_id && x.link_type && !MAIN_CHAT_LINK_TYPES.has(x.link_type),
  );
}

export type HistoryMeta = Pick<
  ChatHistoryItem,
  "id" | "title" | "createdAt" | "model" | "updatedAt"
> & { userMessageCount: number };

export type HistoryState = {
  chats: Record<string, ChatHistoryItem>;
  isLoading: boolean;
  loadError: string | null;
  pagination: {
    cursor: string | null;
    hasMore: boolean;
    totalCount: number | null;
    generation: number;
  };
};

export type TrajectoryWithMeta = TrajectoryData & {
  parent_id?: string;
  link_type?: string;
  task_id?: string;
  task_role?: string;
  agent_id?: string;
  card_id?: string;
};

export type HistoryTreeNode = ChatHistoryItem & {
  children: HistoryTreeNode[];
  bubbleChildren: HistoryTreeNode[];
};

export function buildHistoryTree(
  chats: Record<string, ChatHistoryItem>,
): HistoryTreeNode[] {
  const nodes = Object.values(chats)
    .filter((x) => !isTaskChatLike(x) && !isBuddyChatLike(x))
    .map((x) => ({
      ...x,
      children: [] as HistoryTreeNode[],
      bubbleChildren: [] as HistoryTreeNode[],
    }));

  const byId = new Map(nodes.map((n) => [n.id, n]));
  const parentByChild = new Map<string, string>();
  const bubbleParentByChild = new Map<string, string>();

  const ordered = [...nodes].sort((a, b) =>
    b.updatedAt.localeCompare(a.updatedAt),
  );

  const wouldCycle = (parentId: string, childId: string): boolean => {
    let cur: string | undefined = parentId;
    while (cur) {
      if (cur === childId) return true;
      cur = parentByChild.get(cur);
    }
    return false;
  };

  const attach = (parentId: string, childId: string) => {
    if (parentByChild.has(childId)) return;
    if (wouldCycle(parentId, childId)) return;
    const parent = byId.get(parentId);
    const child = byId.get(childId);
    if (!parent || !child) return;
    parentByChild.set(childId, parentId);
    parent.children.push(child);
  };

  const attachBubble = (parentId: string, childId: string) => {
    if (bubbleParentByChild.has(childId)) return;
    const parent = byId.get(parentId);
    const child = byId.get(childId);
    if (!parent || !child || parent.id === child.id) return;
    bubbleParentByChild.set(childId, parentId);
    parent.bubbleChildren.push(child);
  };

  for (const node of ordered) {
    const pid = node.parent_id;
    if (!pid || !byId.has(pid)) continue;
    if (isSubagenticChatLike(node)) {
      attachBubble(pid, node.id);
    } else if (MAIN_CHAT_LINK_TYPES.has(node.link_type ?? "")) {
      attach(node.id, pid);
    } else {
      attach(pid, node.id);
    }
  }

  const sortTree = (xs: HistoryTreeNode[]) => {
    xs.sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
    for (const x of xs) {
      sortTree(x.children);
      sortTree(x.bubbleChildren);
    }
  };

  const roots = nodes.filter(
    (n) => !parentByChild.has(n.id) && !isSubagenticChatLike(n),
  );
  sortTree(roots);
  return roots;
}

const initialState: HistoryState = {
  chats: {},
  isLoading: true,
  loadError: null,
  pagination: {
    cursor: null,
    hasMore: true,
    totalCount: null,
    generation: 0,
  },
};

function getFirstUserContentFromChat(messages: ChatThread["messages"]): string {
  const message = messages.find(
    (msg): msg is ChatThread["messages"][number] & { role: "user" } =>
      msg.role === "user",
  );
  if (!message) return "New Chat";
  if (typeof message.content === "string") {
    return message.content.replace(/^\s+/, "").slice(0, 100);
  }

  const firstUserInput = message.content.find((item) => {
    if ("m_type" in item && item.m_type === "text") {
      return true;
    }
    if ("type" in item && item.type === "text") {
      return true;
    }
    return false;
  });
  if (!firstUserInput) return "New Chat";
  const text =
    "m_content" in firstUserInput
      ? firstUserInput.m_content
      : "text" in firstUserInput
        ? firstUserInput.text
        : "New Chat";

  return text.replace(/^\s+/, "").slice(0, 100);
}

function chatThreadToHistoryItem(thread: ChatThread): ChatHistoryItem {
  const now = new Date().toISOString();
  const updatedMode = normalizeLegacyMode(thread.mode);

  return {
    ...thread,
    title: thread.title ?? getFirstUserContentFromChat(thread.messages),
    createdAt: thread.createdAt ?? now,
    updatedAt: now,
    integration: thread.integration,
    currentMaximumContextTokens: thread.currentMaximumContextTokens,
    isTitleGenerated: thread.isTitleGenerated,
    mode: updatedMode,
    task_id: thread.task_meta?.task_id,
  };
}

function trajectoryToHistoryItem(
  data: TrajectoryData,
  meta?: {
    parent_id?: string;
    link_type?: string;
    task_id?: string;
    task_role?: string;
    agent_id?: string;
    card_id?: string;
  },
): ChatHistoryItem {
  const thread = trajectoryDataToChatThread(data);
  return {
    ...thread,
    createdAt: data.created_at,
    updatedAt: data.updated_at,
    title: data.title,
    isTitleGenerated: data.isTitleGenerated,
    parent_id: meta?.parent_id,
    link_type: meta?.link_type,
    task_id: meta?.task_id,
    task_role: meta?.task_role,
    agent_id: meta?.agent_id,
    card_id: meta?.card_id,
  };
}

function trajectoryMetaToHistoryItem(meta: TrajectoryMeta): ChatHistoryItem {
  return {
    id: meta.id,
    title: meta.title,
    model: meta.model,
    mode: meta.mode as ChatHistoryItem["mode"],
    tool_use: "agent",
    messages: [],
    boost_reasoning: false,
    context_tokens_cap: undefined,
    include_project_info: true,
    increase_max_tokens: false,
    project_name: undefined,
    isTitleGenerated: false,
    createdAt: meta.created_at,
    last_user_message_id: "",
    updatedAt: meta.updated_at,
    parent_id: meta.parent_id,
    link_type: meta.link_type,
    task_id: meta.task_id,
    task_role: meta.task_role,
    agent_id: meta.agent_id,
    card_id: meta.card_id,
    session_state: meta.session_state,
    message_count: meta.message_count,
    root_chat_id: meta.root_chat_id,
    total_lines_added: meta.total_lines_added,
    total_lines_removed: meta.total_lines_removed,
    tasks_total: meta.tasks_total,
    tasks_done: meta.tasks_done,
    tasks_failed: meta.tasks_failed,
    total_prompt_tokens: meta.total_prompt_tokens,
    total_completion_tokens: meta.total_completion_tokens,
    total_tokens: meta.total_tokens,
    total_cache_read_tokens: meta.total_cache_read_tokens,
    total_cache_creation_tokens: meta.total_cache_creation_tokens,
    total_cost_usd: meta.total_cost_usd,
    worktree: meta.worktree,
  };
}

export const historySlice = createSlice({
  name: "history",
  initialState,
  reducers: {
    setHistoryLoading: (state, action: PayloadAction<boolean>) => {
      state.isLoading = action.payload;
      if (action.payload) {
        state.loadError = null;
      }
    },

    setHistoryLoadError: (state, action: PayloadAction<string | null>) => {
      state.loadError = action.payload;
      state.isLoading = false;
    },

    saveChat: (state, action: PayloadAction<ChatThread>) => {
      if (action.payload.messages.length === 0) return;
      if (isTaskChatLike(action.payload)) return;
      if (isBuddyChatLike(action.payload)) return;
      const chat = chatThreadToHistoryItem(action.payload);
      chat.message_count = action.payload.messages.length;
      chat.messages = [];
      if (chat.id in state.chats) {
        const existing = state.chats[chat.id];
        if (
          existing.isTitleGenerated === true &&
          chat.isTitleGenerated !== true
        ) {
          chat.title = existing.title;
          chat.isTitleGenerated = true;
        }
        chat.parent_id = chat.parent_id ?? existing.parent_id;
        chat.link_type = chat.link_type ?? existing.link_type;
        chat.task_id = chat.task_id ?? existing.task_id;
        chat.task_role = chat.task_role ?? existing.task_role;
        chat.agent_id = chat.agent_id ?? existing.agent_id;
        chat.card_id = chat.card_id ?? existing.card_id;
      }
      state.chats[chat.id] = chat;
    },

    hydrateHistory: (state, action: PayloadAction<TrajectoryWithMeta[]>) => {
      for (const data of action.payload) {
        state.chats[data.id] = trajectoryToHistoryItem(data, {
          parent_id: data.parent_id,
          link_type: data.link_type,
          task_id: data.task_id,
          task_role: data.task_role,
          agent_id: data.agent_id,
          card_id: data.card_id,
        });
      }
    },

    hydrateHistoryFromMeta: (
      state,
      action: PayloadAction<TrajectoryMeta[]>,
    ) => {
      for (const meta of action.payload) {
        if (!(meta.id in state.chats)) {
          state.chats[meta.id] = trajectoryMetaToHistoryItem(meta);
        } else {
          const existing = state.chats[meta.id];
          existing.title = meta.title;
          existing.updatedAt = meta.updated_at;
          existing.model = meta.model;
          existing.mode = meta.mode as ChatHistoryItem["mode"];
          existing.parent_id = meta.parent_id;
          existing.link_type = meta.link_type;
          existing.task_id = meta.task_id;
          existing.task_role = meta.task_role;
          existing.agent_id = meta.agent_id;
          existing.card_id = meta.card_id;
          existing.worktree = meta.worktree;
          existing.session_state = meta.session_state;
          existing.message_count = meta.message_count;
          existing.root_chat_id = meta.root_chat_id;
          existing.total_lines_added = meta.total_lines_added;
          existing.total_lines_removed = meta.total_lines_removed;
          existing.tasks_total = meta.tasks_total;
          existing.tasks_done = meta.tasks_done;
          existing.tasks_failed = meta.tasks_failed;
          existing.total_prompt_tokens = meta.total_prompt_tokens;
          existing.total_completion_tokens = meta.total_completion_tokens;
          existing.total_tokens = meta.total_tokens;
          existing.total_cache_read_tokens = meta.total_cache_read_tokens;
          existing.total_cache_creation_tokens =
            meta.total_cache_creation_tokens;
          existing.total_cost_usd = meta.total_cost_usd;
        }
      }
    },

    replaceSnapshotHistory: (
      state,
      action: PayloadAction<{
        items: TrajectoryMeta[];
        append?: boolean;
        pagination?: {
          cursor: string | null;
          hasMore: boolean;
          totalCount?: number | null;
        };
      }>,
    ) => {
      const { items, append = false, pagination } = action.payload;
      const snapshotIds = new Set(items.map((m) => m.id));
      if (!append) {
        state.chats = Object.fromEntries(
          Object.entries(state.chats).filter(([id]) => snapshotIds.has(id)),
        );
      }
      for (const meta of items) {
        if (!(meta.id in state.chats)) {
          state.chats[meta.id] = trajectoryMetaToHistoryItem(meta);
        } else {
          const existing = state.chats[meta.id];
          existing.title = meta.title;
          existing.updatedAt = meta.updated_at;
          existing.model = meta.model;
          existing.mode = meta.mode as ChatHistoryItem["mode"];
          existing.parent_id = meta.parent_id;
          existing.link_type = meta.link_type;
          existing.task_id = meta.task_id;
          existing.task_role = meta.task_role;
          existing.agent_id = meta.agent_id;
          existing.card_id = meta.card_id;
          existing.worktree = meta.worktree;
          existing.session_state = meta.session_state;
          existing.message_count = meta.message_count;
          existing.root_chat_id = meta.root_chat_id;
          existing.total_lines_added = meta.total_lines_added;
          existing.total_lines_removed = meta.total_lines_removed;
          existing.tasks_total = meta.tasks_total;
          existing.tasks_done = meta.tasks_done;
          existing.tasks_failed = meta.tasks_failed;
          existing.total_prompt_tokens = meta.total_prompt_tokens;
          existing.total_completion_tokens = meta.total_completion_tokens;
          existing.total_tokens = meta.total_tokens;
          existing.total_cache_read_tokens = meta.total_cache_read_tokens;
          existing.total_cache_creation_tokens =
            meta.total_cache_creation_tokens;
          existing.total_cost_usd = meta.total_cost_usd;
        }
      }
      if (pagination) {
        state.pagination.cursor = pagination.cursor;
        state.pagination.hasMore =
          pagination.hasMore && pagination.cursor !== null;
        state.pagination.totalCount = pagination.totalCount ?? null;
        if (!append) {
          state.pagination.generation += 1;
        }
      } else if (!append) {
        state.pagination.cursor = null;
        state.pagination.hasMore = false;
        state.pagination.totalCount = items.length;
        state.pagination.generation += 1;
      }
    },

    setPagination: (
      state,
      action: PayloadAction<{
        cursor: string | null;
        hasMore: boolean;
        totalCount?: number | null;
      }>,
    ) => {
      state.pagination.cursor = action.payload.cursor;
      state.pagination.hasMore =
        action.payload.hasMore && action.payload.cursor !== null;
      if ("totalCount" in action.payload) {
        state.pagination.totalCount = action.payload.totalCount ?? null;
      }
    },

    deleteChatById: (state, action: PayloadAction<string>) => {
      const { [action.payload]: _, ...rest } = state.chats;
      state.chats = rest;
    },

    upsertChatStub: (
      state,
      action: PayloadAction<{
        id: string;
        title?: string;
        model?: string;
        session_state?: ChatHistoryItem["session_state"];
        parent_id?: string;
        link_type?: string;
      }>,
    ) => {
      const { id, title, model, session_state, parent_id, link_type } =
        action.payload;
      if (id in state.chats) {
        if (title) state.chats[id].title = title;
        if (model) state.chats[id].model = model;
        if (session_state) state.chats[id].session_state = session_state;
        if (parent_id) state.chats[id].parent_id = parent_id;
        if (link_type) state.chats[id].link_type = link_type;
        return;
      }
      const now = new Date().toISOString();
      state.chats[id] = {
        id,
        title: title ?? "New Chat",
        model: model ?? "",
        mode: "AGENT",
        tool_use: "agent",
        messages: [],
        boost_reasoning: false,
        context_tokens_cap: undefined,
        include_project_info: true,
        increase_max_tokens: false,
        project_name: undefined,
        isTitleGenerated: false,
        createdAt: now,
        last_user_message_id: "",
        updatedAt: now,
        session_state: session_state ?? "idle",
        message_count: 0,
        parent_id,
        link_type,
      };
    },

    updateChatTitleById: (
      state,
      action: PayloadAction<{ chatId: string; newTitle: string }>,
    ) => {
      if (action.payload.chatId in state.chats) {
        state.chats[action.payload.chatId].title = action.payload.newTitle;
      }
    },

    updateChatMetaById: (
      state,
      action: PayloadAction<{
        id: string;
        title?: string;
        isTitleGenerated?: boolean;
        updatedAt?: string;
        session_state?: ChatHistoryItem["session_state"];
        message_count?: number;
        parent_id?: string;
        link_type?: string;
        root_chat_id?: string;
        total_lines_added?: number;
        total_lines_removed?: number;
        worktree?: WorktreeMeta | null;
        model?: string;
        mode?: string;
        tasks_total?: number;
        tasks_done?: number;
        tasks_failed?: number;
        task_id?: string;
        task_role?: string;
        agent_id?: string;
        card_id?: string;
        total_prompt_tokens?: number;
        total_completion_tokens?: number;
        total_tokens?: number;
        total_cache_read_tokens?: number;
        total_cache_creation_tokens?: number;
        total_cost_usd?: number;
      }>,
    ) => {
      if (!(action.payload.id in state.chats)) return;
      const chat = state.chats[action.payload.id];
      if (action.payload.title !== undefined) {
        chat.title = action.payload.title;
      }
      if (action.payload.isTitleGenerated !== undefined) {
        chat.isTitleGenerated = action.payload.isTitleGenerated;
      }
      if (action.payload.updatedAt !== undefined) {
        chat.updatedAt = action.payload.updatedAt;
      }
      if (action.payload.session_state !== undefined) {
        chat.session_state = action.payload.session_state;
      }
      if (action.payload.message_count !== undefined) {
        chat.message_count = action.payload.message_count;
      }
      if (action.payload.parent_id !== undefined) {
        chat.parent_id = action.payload.parent_id;
      }
      if (action.payload.link_type !== undefined) {
        chat.link_type = action.payload.link_type;
      }
      if (action.payload.root_chat_id !== undefined) {
        chat.root_chat_id = action.payload.root_chat_id;
      }
      if (action.payload.total_lines_added !== undefined) {
        chat.total_lines_added = action.payload.total_lines_added;
      }
      if (action.payload.total_lines_removed !== undefined) {
        chat.total_lines_removed = action.payload.total_lines_removed;
      }
      if (action.payload.worktree !== undefined) {
        chat.worktree = action.payload.worktree;
      }
      if (action.payload.model !== undefined) {
        chat.model = action.payload.model;
      }
      if (action.payload.mode !== undefined) {
        chat.mode = action.payload.mode as ChatHistoryItem["mode"];
      }
      if (action.payload.tasks_total !== undefined) {
        chat.tasks_total = action.payload.tasks_total;
      }
      if (action.payload.tasks_done !== undefined) {
        chat.tasks_done = action.payload.tasks_done;
      }
      if (action.payload.tasks_failed !== undefined) {
        chat.tasks_failed = action.payload.tasks_failed;
      }
      if (action.payload.task_id !== undefined) {
        chat.task_id = action.payload.task_id;
      }
      if (action.payload.task_role !== undefined) {
        chat.task_role = action.payload.task_role;
      }
      if (action.payload.agent_id !== undefined) {
        chat.agent_id = action.payload.agent_id;
      }
      if (action.payload.card_id !== undefined) {
        chat.card_id = action.payload.card_id;
      }
      if (action.payload.total_prompt_tokens !== undefined) {
        chat.total_prompt_tokens = action.payload.total_prompt_tokens;
      }
      if (action.payload.total_completion_tokens !== undefined) {
        chat.total_completion_tokens = action.payload.total_completion_tokens;
      }
      if (action.payload.total_tokens !== undefined) {
        chat.total_tokens = action.payload.total_tokens;
      }
      if (action.payload.total_cache_read_tokens !== undefined) {
        chat.total_cache_read_tokens = action.payload.total_cache_read_tokens;
      }
      if (action.payload.total_cache_creation_tokens !== undefined) {
        chat.total_cache_creation_tokens =
          action.payload.total_cache_creation_tokens;
      }
      if (action.payload.total_cost_usd !== undefined) {
        chat.total_cost_usd = action.payload.total_cost_usd;
      }
    },

    clearHistory: () => {
      return {
        chats: {},
        isLoading: false,
        loadError: null,
        pagination: {
          cursor: null,
          hasMore: true,
          totalCount: null,
          generation: 0,
        },
      };
    },

    upsertToolCallIntoHistory: (
      state,
      action: PayloadAction<
        Parameters<typeof ideToolCallResponse>[0] & {
          replaceOnly?: boolean;
        }
      >,
    ) => {
      if (!(action.payload.chatId in state.chats)) return;
      maybeAppendToolCallResultFromIdeToMessages(
        state.chats[action.payload.chatId].messages,
        action.payload.toolCallId,
        action.payload.accepted,
        action.payload.replaceOnly,
      );
    },
  },
  selectors: {
    selectHistoryIsLoading: (state): boolean => state.isLoading,

    getChatById: (state, id: string): ChatHistoryItem | null => {
      if (!(id in state.chats)) return null;
      return state.chats[id];
    },

    getHistory: (state): ChatHistoryItem[] =>
      Object.values(state.chats)
        .filter((item) => !isTaskChatLike(item) && !isBuddyChatLike(item))
        .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt)),

    getHistoryTree: (state): HistoryTreeNode[] => buildHistoryTree(state.chats),
  },
});

export const {
  setHistoryLoading,
  setHistoryLoadError,
  saveChat,
  hydrateHistory,
  hydrateHistoryFromMeta,
  replaceSnapshotHistory,
  setPagination,
  deleteChatById,
  upsertChatStub,
  updateChatTitleById,
  updateChatMetaById,
  clearHistory,
  upsertToolCallIntoHistory,
} = historySlice.actions;
export const {
  selectHistoryIsLoading,
  getChatById,
  getHistory,
  getHistoryTree,
} = historySlice.selectors;

export const historyMiddleware = createListenerMiddleware();
const startHistoryListening = historyMiddleware.startListening.withTypes<
  RootState,
  AppDispatch
>();

startHistoryListening({
  actionCreator: applyChatEvent,
  effect: (action, listenerApi) => {
    const event = action.payload;
    if (event.type !== "stream_finished") return;
    if (event.finish_reason === "abort" || event.finish_reason === "error")
      return;

    const state = listenerApi.getState();
    const runtime = state.chat.threads[event.chat_id];
    if (!runtime) return;
    const thread = runtime.thread;

    listenerApi.dispatch(saveChat(thread));
  },
});

startHistoryListening({
  actionCreator: backUpMessages,
  effect: (action, listenerApi) => {
    const state = listenerApi.getState();
    const runtime = state.chat.threads[action.payload.id];
    if (!runtime) return;
    const thread = runtime.thread;

    const toSave = {
      ...thread,
      messages: action.payload.messages,
      project_name: thread.project_name ?? state.current_project.name,
    };
    listenerApi.dispatch(saveChat(toSave));
  },
});

startHistoryListening({
  actionCreator: setChatMode,
  effect: (action, listenerApi) => {
    const state = listenerApi.getState();
    const runtime = state.chat.threads[state.chat.current_thread_id];
    if (!runtime) return;
    const thread = runtime.thread;
    if (!(thread.id in state.history.chats)) return;

    const toSave = { ...thread, mode: action.payload };
    listenerApi.dispatch(saveChat(toSave));
  },
});

startHistoryListening({
  actionCreator: deleteChatById,
  effect: (action, listenerApi) => {
    void listenerApi.dispatch(
      trajectoriesApi.endpoints.deleteTrajectory.initiate(action.payload),
    );
  },
});

startHistoryListening({
  actionCreator: newChatAction,
  effect: (_, listenerApi) => {
    const state = listenerApi.getState();
    const id = state.chat.current_thread_id;
    const runtime = state.chat.threads[id];
    if (!runtime) return;
    if (isTaskChatLike(runtime.thread)) return;
    if (isBuddyChatLike(runtime.thread)) return;
    listenerApi.dispatch(
      upsertChatStub({
        id,
        title: runtime.thread.title ? runtime.thread.title : undefined,
        model: runtime.thread.model ? runtime.thread.model : undefined,
      }),
    );
  },
});

startHistoryListening({
  actionCreator: createChatWithId,
  effect: (action, listenerApi) => {
    if (action.payload.isTaskChat === true || action.payload.taskMeta?.task_id)
      return;
    listenerApi.dispatch(
      upsertChatStub({
        id: action.payload.id,
        title: action.payload.title,
        model: action.payload.model,
        parent_id: action.payload.parentId,
        link_type: action.payload.linkType,
      }),
    );
  },
});

startHistoryListening({
  actionCreator: restoreChat,
  effect: (action, listenerApi) => {
    if (isTaskChatLike(action.payload)) return;
    if (isBuddyChatLike(action.payload)) return;
    listenerApi.dispatch(
      upsertChatStub({
        id: action.payload.id,
        title: action.payload.title,
        model: action.payload.model,
      }),
    );
  },
});

startHistoryListening({
  actionCreator: switchToThread,
  effect: (action, listenerApi) => {
    const state = listenerApi.getState();
    const runtime = state.chat.threads[action.payload.id];
    if (!runtime) return;
    if (isTaskChatLike(runtime.thread)) return;
    if (isBuddyChatLike(runtime.thread)) return;
    listenerApi.dispatch(
      upsertChatStub({
        id: action.payload.id,
        title: runtime.thread.title ? runtime.thread.title : undefined,
        model: runtime.thread.model ? runtime.thread.model : undefined,
      }),
    );
  },
});
