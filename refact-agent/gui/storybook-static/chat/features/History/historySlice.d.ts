import { UnknownAction } from 'redux';
import { ThunkDispatch } from 'redux-thunk';
import { Selector } from 'reselect';
import { WritableDraft } from 'immer';
import { Slice, ActionCreatorWithPayload, ActionCreatorWithoutPayload, ListenerMiddlewareInstance, PayloadAction } from '@reduxjs/toolkit';
import { ChatThread, SuggestedChat } from "../Chat/Thread";
import { TrajectoryData, TrajectoryMeta } from "../../services/refact";
import type { WorktreeMeta } from "../../services/refact/worktrees";
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
    session_state?: "idle" | "generating" | "executing_tools" | "paused" | "waiting_ide" | "waiting_user_input" | "completed" | "error";
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
export declare function isTaskChatLike(x: Partial<Pick<ChatHistoryItem, "mode">>): boolean;
export declare function isBuddyChatLike(x: Partial<Pick<ChatHistoryItem, "buddy_meta">>): boolean;
export declare function isSubagenticChatLike(x: Partial<Pick<ChatHistoryItem, "parent_id" | "link_type">>): boolean;
export type HistoryMeta = Pick<ChatHistoryItem, "id" | "title" | "createdAt" | "model" | "updatedAt"> & {
    userMessageCount: number;
};
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
export declare function buildHistoryTree(chats: Record<string, ChatHistoryItem>): HistoryTreeNode[];
export declare const historySlice: Slice<HistoryState, {
    setHistoryLoading: (state: WritableDraft<HistoryState>, action: PayloadAction<boolean>) => void;
    setHistoryLoadError: (state: WritableDraft<HistoryState>, action: PayloadAction<string | null>) => void;
    saveChat: (state: WritableDraft<HistoryState>, action: PayloadAction<ChatThread>) => void;
    hydrateHistory: (state: WritableDraft<HistoryState>, action: PayloadAction<TrajectoryWithMeta[]>) => void;
    hydrateHistoryFromMeta: (state: WritableDraft<HistoryState>, action: PayloadAction<TrajectoryMeta[]>) => void;
    replaceSnapshotHistory: (state: WritableDraft<HistoryState>, action: PayloadAction<{
        items: TrajectoryMeta[];
        append?: boolean;
        pagination?: {
            cursor: string | null;
            hasMore: boolean;
            totalCount?: number | null;
        };
    }>) => void;
    setPagination: (state: WritableDraft<HistoryState>, action: PayloadAction<{
        cursor: string | null;
        hasMore: boolean;
        totalCount?: number | null;
    }>) => void;
    deleteChatById: (state: WritableDraft<HistoryState>, action: PayloadAction<string>) => void;
    upsertChatStub: (state: WritableDraft<HistoryState>, action: PayloadAction<{
        id: string;
        title?: string;
        model?: string;
        session_state?: ChatHistoryItem["session_state"];
        parent_id?: string;
        link_type?: string;
    }>) => void;
    updateChatTitleById: (state: WritableDraft<HistoryState>, action: PayloadAction<{
        chatId: string;
        newTitle: string;
    }>) => void;
    updateChatMetaById: (state: WritableDraft<HistoryState>, action: PayloadAction<{
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
    }>) => void;
    clearHistory: () => {
        chats: {};
        isLoading: boolean;
        loadError: null;
        pagination: {
            cursor: null;
            hasMore: boolean;
            totalCount: null;
            generation: number;
        };
    };
    upsertToolCallIntoHistory: (state: WritableDraft<HistoryState>, action: PayloadAction<Parameters<typeof ideToolCallResponse>[0] & {
        replaceOnly?: boolean;
    }>) => void;
}, "history", "history", {
    selectHistoryIsLoading: (state: HistoryState) => boolean;
    getChatById: (state: HistoryState, id: string) => ChatHistoryItem | null;
    getHistory: (state: HistoryState) => ChatHistoryItem[];
    getHistoryTree: (state: HistoryState) => HistoryTreeNode[];
}>;
export declare const setHistoryLoading: ActionCreatorWithPayload<boolean, "history/setHistoryLoading">, setHistoryLoadError: ActionCreatorWithPayload<string | null, "history/setHistoryLoadError">, saveChat: ActionCreatorWithPayload<ChatThread, "history/saveChat">, hydrateHistory: ActionCreatorWithPayload<TrajectoryWithMeta[], "history/hydrateHistory">, hydrateHistoryFromMeta: ActionCreatorWithPayload<TrajectoryMeta[], "history/hydrateHistoryFromMeta">, replaceSnapshotHistory: ActionCreatorWithPayload<{
    items: TrajectoryMeta[];
    append?: boolean;
    pagination?: {
        cursor: string | null;
        hasMore: boolean;
        totalCount?: number | null;
    };
}, "history/replaceSnapshotHistory">, setPagination: ActionCreatorWithPayload<{
    cursor: string | null;
    hasMore: boolean;
    totalCount?: number | null;
}, "history/setPagination">, deleteChatById: ActionCreatorWithPayload<string, "history/deleteChatById">, upsertChatStub: ActionCreatorWithPayload<{
    id: string;
    title?: string;
    model?: string;
    session_state?: ChatHistoryItem["session_state"];
    parent_id?: string;
    link_type?: string;
}, "history/upsertChatStub">, updateChatTitleById: ActionCreatorWithPayload<{
    chatId: string;
    newTitle: string;
}, "history/updateChatTitleById">, updateChatMetaById: ActionCreatorWithPayload<{
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
}, "history/updateChatMetaById">, clearHistory: ActionCreatorWithoutPayload<"history/clearHistory">, upsertToolCallIntoHistory: ActionCreatorWithPayload<{
    toolCallId: string;
    chatId: string;
    accepted: boolean | "indeterminate";
} & {
    replaceOnly?: boolean;
}, "history/upsertToolCallIntoHistory">;
export declare const selectHistoryIsLoading: Selector<{
    history: HistoryState;
}, boolean, []> & {
    unwrapped: (state: HistoryState) => boolean;
}, getChatById: Selector<{
    history: HistoryState;
}, ChatHistoryItem | null, [id: string]> & {
    unwrapped: (state: HistoryState, id: string) => ChatHistoryItem | null;
}, getHistory: Selector<{
    history: HistoryState;
}, ChatHistoryItem[], []> & {
    unwrapped: (state: HistoryState) => ChatHistoryItem[];
}, getHistoryTree: Selector<{
    history: HistoryState;
}, HistoryTreeNode[], []> & {
    unwrapped: (state: HistoryState) => HistoryTreeNode[];
};
export declare const historyMiddleware: ListenerMiddlewareInstance<unknown, ThunkDispatch<unknown, unknown, UnknownAction>, unknown>;
