import {
  createSlice,
  PayloadAction,
  createListenerMiddleware,
} from "@reduxjs/toolkit";
import {
  backUpMessages,
  ChatThread,
  isLspChatMode,
  maybeAppendToolCallResultFromIdeToMessages,
  restoreChat,
  setChatMode,
  SuggestedChat,
  applyChatEvent,
} from "../Chat/Thread";
import {
  trajectoriesApi,
  TrajectoryData,
  trajectoryDataToChatThread,
} from "../../services/refact";
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
};

export type HistoryMeta = Pick<
  ChatHistoryItem,
  "id" | "title" | "createdAt" | "model" | "updatedAt"
> & { userMessageCount: number };

export type HistoryState = {
  chats: Record<string, ChatHistoryItem>;
  isLoading: boolean;
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
};

const initialState: HistoryState = {
  chats: {},
  isLoading: true,
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
  const updatedMode =
    thread.mode && !isLspChatMode(thread.mode) ? "AGENT" : thread.mode;

  return {
    ...thread,
    title: thread.title ?? getFirstUserContentFromChat(thread.messages),
    createdAt: thread.createdAt ?? now,
    updatedAt: now,
    integration: thread.integration,
    currentMaximumContextTokens: thread.currentMaximumContextTokens,
    isTitleGenerated: thread.isTitleGenerated,
    automatic_patch: thread.automatic_patch,
    mode: updatedMode,
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

export const historySlice = createSlice({
  name: "history",
  initialState,
  reducers: {
    setHistoryLoading: (state, action: PayloadAction<boolean>) => {
      state.isLoading = action.payload;
    },

    saveChat: (state, action: PayloadAction<ChatThread>) => {
      if (action.payload.messages.length === 0) return;
      const chat = chatThreadToHistoryItem(action.payload);
      if (chat.id in state.chats) {
        const existing = state.chats[chat.id];
        if (
          existing.isTitleGenerated === true &&
          chat.isTitleGenerated !== true
        ) {
          chat.title = existing.title;
          chat.isTitleGenerated = true;
        }
        chat.parent_id = existing.parent_id;
        chat.link_type = existing.link_type;
        chat.task_id = existing.task_id;
        chat.task_role = existing.task_role;
        chat.agent_id = existing.agent_id;
        chat.card_id = existing.card_id;
      }
      state.chats[chat.id] = chat;

      const allChats = Object.values(state.chats);
      if (allChats.length > 100) {
        const sorted = allChats.sort((a, b) =>
          b.updatedAt.localeCompare(a.updatedAt),
        );
        const idsToKeep = new Set(sorted.slice(0, 100).map((c) => c.id));
        const idsToRemove = Object.keys(state.chats).filter(
          (id) => !idsToKeep.has(id),
        );
        for (const id of idsToRemove) {
          // eslint-disable-next-line @typescript-eslint/no-dynamic-delete
          delete state.chats[id];
        }
      }
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

    markChatAsUnread: (state, action: PayloadAction<string>) => {
      if (action.payload in state.chats) {
        state.chats[action.payload].read = false;
      }
    },

    markChatAsRead: (state, action: PayloadAction<string>) => {
      if (action.payload in state.chats) {
        state.chats[action.payload].read = true;
      }
    },

    deleteChatById: (state, action: PayloadAction<string>) => {
      // eslint-disable-next-line @typescript-eslint/no-dynamic-delete
      delete state.chats[action.payload];
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
      action: PayloadAction<{ id: string; title?: string; updatedAt?: string }>,
    ) => {
      if (!(action.payload.id in state.chats)) return;
      const chat = state.chats[action.payload.id];
      if (action.payload.title !== undefined) {
        chat.title = action.payload.title;
        chat.isTitleGenerated = true;
      }
      if (action.payload.updatedAt !== undefined) {
        chat.updatedAt = action.payload.updatedAt;
      }
    },

    clearHistory: () => {
      return { chats: {}, isLoading: false };
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
        .filter((item) => !item.task_id)
        .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt)),

    getHistoryTree: (state): HistoryTreeNode[] => {
      const items = Object.values(state.chats).filter((item) => !item.task_id);
      const itemMap = new Map<string, HistoryTreeNode>();
      const roots: HistoryTreeNode[] = [];

      for (const item of items) {
        itemMap.set(item.id, { ...item, children: [] });
      }

      const assignedAsChild = new Set<string>();
      const handoffParentIds = new Set<string>();

      for (const item of items) {
        if (
          item.link_type === "handoff" &&
          item.parent_id &&
          itemMap.has(item.parent_id)
        ) {
          handoffParentIds.add(item.parent_id);
        }
      }

      for (const item of items) {
        const node = itemMap.get(item.id);
        if (!node) continue;

        if (handoffParentIds.has(item.id)) {
          continue;
        }

        if (item.parent_id && itemMap.has(item.parent_id)) {
          if (assignedAsChild.has(item.id)) {
            roots.push(node);
            continue;
          }
          const parent = itemMap.get(item.parent_id);
          if (!parent || parent.parent_id === item.id) {
            roots.push(node);
            continue;
          }

          if (item.link_type === "handoff") {
            const parentNode = itemMap.get(item.parent_id);
            if (parentNode) {
              node.children.push(parentNode);
              assignedAsChild.add(item.parent_id);
              roots.push(node);
            }
          } else {
            const parentNode = itemMap.get(item.parent_id);
            if (parentNode) {
              parentNode.children.push(node);
              assignedAsChild.add(item.id);
            }
          }
        } else {
          roots.push(node);
        }
      }

      const sortByUpdated = (a: HistoryTreeNode, b: HistoryTreeNode) =>
        b.updatedAt.localeCompare(a.updatedAt);

      const sortTree = (nodes: HistoryTreeNode[]) => {
        nodes.sort(sortByUpdated);
        for (const node of nodes) {
          if (node.children.length > 0) {
            sortTree(node.children);
          }
        }
      };

      sortTree(roots);
      return roots;
    },
  },
});

export const {
  setHistoryLoading,
  saveChat,
  hydrateHistory,
  deleteChatById,
  markChatAsUnread,
  markChatAsRead,
  updateChatTitleById,
  updateChatMetaById,
  clearHistory,
  upsertToolCallIntoHistory,
} = historySlice.actions;
export const { selectHistoryIsLoading, getChatById, getHistory, getHistoryTree } =
  historySlice.selectors;

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
  actionCreator: restoreChat,
  effect: (action, listenerApi) => {
    const chat = listenerApi.getState().chat;
    const runtime = chat.threads[action.payload.id];
    if (!runtime || runtime.streaming) return;
    listenerApi.dispatch(markChatAsRead(action.payload.id));
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
