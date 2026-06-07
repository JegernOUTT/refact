import { UnknownAction } from 'redux';
import { ThunkDispatch } from 'redux-thunk';
import { ActionCreatorWithOptionalPayload, AsyncThunk, ActionCreatorWithPayload, ActionCreatorWithoutPayload } from '@reduxjs/toolkit';
import { ManualPreviewItem, type PayloadWithIdAndTitle, type ChatThread, type PayloadWithId, type ToolUse, type ImageFile, type TextFile, IntegrationMeta, PayloadWithChatAndMessageId, PayloadWithChatAndBoolean } from './types';
import { EventEnvelope, ToolConfirmationPauseReason } from '../../../services/refact';
import { type ChatMessages } from "../../../services/refact/types";
import type { WorktreeMeta } from "../../../services/refact/worktrees";
import type { AppDispatch, RootState } from "../../../app/store";
import { type SystemPrompts } from "../../../services/refact/prompts";
import { ChatHistoryItem } from "../../History/historySlice";
import type { DiagnosticContext, BuddyConversationEntry } from "../../Buddy/types";
import { type BuddyInvestigationSource } from "../../Buddy/investigation";
declare function buildThreadParamsPatch(thread: ChatThread, isNewChat: boolean): Record<string, unknown>;
declare function buildThreadScopePatch(thread: ChatThread): Record<string, unknown>;
export { buildThreadParamsPatch, buildThreadScopePatch };
export declare const newChatAction: ActionCreatorWithOptionalPayload<Partial<ChatThread> | undefined, string>;
export interface TaskMeta {
    task_id: string;
    role: string;
    agent_id?: string;
    card_id?: string;
    planner_chat_id?: string;
}
export declare const sendIdeMessagesToCurrentChat: AsyncThunk<void, {
    messages: ChatMessages;
    priority?: boolean;
}, {
    state?: unknown;
    dispatch?: ThunkDispatch<unknown, unknown, UnknownAction>;
    extra?: unknown;
    rejectValue?: unknown;
    serializedErrorType?: unknown;
    pendingMeta?: unknown;
    fulfilledMeta?: unknown;
    rejectedMeta?: unknown;
}>;
export declare const createChatWithId: ActionCreatorWithPayload<{
    id: string;
    title?: string;
    isTaskChat?: boolean;
    mode?: string;
    taskMeta?: TaskMeta;
    model?: string;
    parentId?: string;
    linkType?: string;
    worktree?: WorktreeMeta | null;
}, string>;
export declare const openChatInModeAndStart: AsyncThunk<undefined, {
    mode: string;
    initialMessage?: string;
}, {
    dispatch: AppDispatch;
    state: RootState;
    extra?: unknown;
    rejectValue?: unknown;
    serializedErrorType?: unknown;
    pendingMeta?: unknown;
    fulfilledMeta?: unknown;
    rejectedMeta?: unknown;
}>;
export declare const newChatWithInitialMessages: AsyncThunk<void, {
    title?: string;
    messages: ChatMessages;
    priority?: boolean;
}, {
    state?: unknown;
    dispatch?: ThunkDispatch<unknown, unknown, UnknownAction>;
    extra?: unknown;
    rejectValue?: unknown;
    serializedErrorType?: unknown;
    pendingMeta?: unknown;
    fulfilledMeta?: unknown;
    rejectedMeta?: unknown;
}>;
export declare const newIntegrationChat: ActionCreatorWithPayload<{
    integration: IntegrationMeta;
    messages: ChatMessages;
    request_attempt_id: string;
}, string>;
export declare const setLastUserMessageId: ActionCreatorWithPayload<PayloadWithChatAndMessageId, string>;
export declare const setIsNewChatSuggested: ActionCreatorWithPayload<PayloadWithChatAndBoolean, string>;
export declare const setIsNewChatSuggestionRejected: ActionCreatorWithPayload<PayloadWithChatAndBoolean, string>;
export declare const backUpMessages: ActionCreatorWithPayload<PayloadWithId & {
    messages: ChatThread["messages"];
}, string>;
export type SetChatModelPayload = {
    model: string;
    modelMaxContextTokens?: number;
    previousModelMaxContextTokens?: number;
};
export declare const setChatModel: ActionCreatorWithPayload<SetChatModelPayload, string>;
export declare const getSelectedChatModel: (state: RootState) => string;
export declare const setSystemPrompt: ActionCreatorWithPayload<SystemPrompts, string>;
export declare const removeChatFromCache: ActionCreatorWithPayload<PayloadWithId, string>;
export declare const restoreChat: ActionCreatorWithPayload<ChatHistoryItem, string>;
export declare const updateOpenThread: ActionCreatorWithPayload<{
    id: string;
    thread: Partial<ChatThread>;
}, string>;
export declare const updateChatRuntimeFromSessionState: ActionCreatorWithPayload<{
    id: string;
    session_state: "idle" | "generating" | "executing_tools" | "paused" | "waiting_ide" | "waiting_user_input" | "completed" | "error";
    error?: string;
}, string>;
export declare const switchToThread: ActionCreatorWithPayload<PayloadWithId & {
    openTab?: boolean;
}, string>;
export declare const closeThread: ActionCreatorWithPayload<PayloadWithId & {
    force?: boolean;
}, string>;
export declare const setThreadPauseReasons: ActionCreatorWithPayload<{
    id: string;
    pauseReasons: ToolConfirmationPauseReason[];
}, string>;
export declare const clearThreadPauseReasons: ActionCreatorWithPayload<PayloadWithId, string>;
export declare const setThreadConfirmationStatus: ActionCreatorWithPayload<{
    id: string;
    wasInteracted: boolean;
    confirmationStatus: boolean;
}, string>;
export declare const addThreadImage: ActionCreatorWithPayload<{
    id: string;
    image: ImageFile;
}, string>;
export declare const removeThreadImageByIndex: ActionCreatorWithPayload<{
    id: string;
    index: number;
}, string>;
export declare const resetThreadImages: ActionCreatorWithPayload<PayloadWithId, string>;
export declare const addThreadTextFile: ActionCreatorWithPayload<{
    id: string;
    file: TextFile;
}, string>;
export declare const removeThreadTextFileByIndex: ActionCreatorWithPayload<{
    id: string;
    index: number;
}, string>;
export declare const resetThreadTextFiles: ActionCreatorWithPayload<PayloadWithId, string>;
export declare const clearChatError: ActionCreatorWithPayload<PayloadWithId, string>;
export declare const enableSend: ActionCreatorWithPayload<PayloadWithId, string>;
export declare const setPreventSend: ActionCreatorWithPayload<PayloadWithId, string>;
export declare const setAreFollowUpsEnabled: ActionCreatorWithPayload<boolean, string>;
export declare const setToolUse: ActionCreatorWithPayload<ToolUse, string>;
export declare const setThreadMode: ActionCreatorWithPayload<{
    chatId: string;
    mode: string;
    threadDefaults?: {
        include_project_info?: boolean;
        checkpoints_enabled?: boolean;
        auto_approve_editing_tools?: boolean;
        auto_approve_dangerous_commands?: boolean;
    };
}, string>;
export declare const setThreadWorktree: ActionCreatorWithPayload<{
    chatId: string;
    worktree: WorktreeMeta | null;
}, string>;
export declare const setEnabledCheckpoints: ActionCreatorWithPayload<boolean, string>;
export declare const setBoostReasoning: ActionCreatorWithPayload<PayloadWithChatAndBoolean, string>;
export declare const setAutoApproveEditingTools: ActionCreatorWithPayload<PayloadWithChatAndBoolean, string>;
export declare const setAutoApproveDangerousCommands: ActionCreatorWithPayload<PayloadWithChatAndBoolean, string>;
export declare const saveTitle: ActionCreatorWithPayload<PayloadWithIdAndTitle, string>;
export declare const setSendImmediately: ActionCreatorWithPayload<boolean, string>;
export declare const setChatMode: ActionCreatorWithPayload<string, string>;
export declare const setIntegrationData: ActionCreatorWithPayload<Partial<IntegrationMeta> | null, string>;
export declare const setIsWaitingForResponse: ActionCreatorWithPayload<{
    id: string;
    value: boolean;
}, string>;
export declare const markThreadSseError: ActionCreatorWithPayload<{
    id: string;
    error: string;
}, string>;
export declare const setMaxNewTokens: ActionCreatorWithPayload<number, string>;
export declare const fixBrokenToolMessages: ActionCreatorWithPayload<PayloadWithId, string>;
export declare const upsertToolCall: ActionCreatorWithPayload<{
    toolCallId: string;
    chatId: string;
    accepted: boolean | "indeterminate";
} & {
    replaceOnly?: boolean;
}, string>;
export declare const setIncreaseMaxTokens: ActionCreatorWithPayload<boolean, string>;
export declare const setIncludeProjectInfo: ActionCreatorWithPayload<PayloadWithChatAndBoolean, string>;
export declare const setReasoningEffort: ActionCreatorWithPayload<{
    chatId: string;
    value: "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | null;
}, string>;
export declare const setThinkingBudget: ActionCreatorWithPayload<{
    chatId: string;
    value: number | null;
}, string>;
export declare const setTemperature: ActionCreatorWithPayload<{
    chatId: string;
    value: number | null;
}, string>;
export declare const setFrequencyPenalty: ActionCreatorWithPayload<{
    chatId: string;
    value: number | null;
}, string>;
export declare const setMaxTokens: ActionCreatorWithPayload<{
    chatId: string;
    value: number | null;
}, string>;
export declare const setParallelToolCalls: ActionCreatorWithPayload<{
    chatId: string;
    value: boolean | null;
}, string>;
export declare const restoreChatFromBackend: AsyncThunk<undefined, {
    id: string;
    fallback: ChatHistoryItem;
}, {
    dispatch: AppDispatch;
    state: RootState;
    extra?: unknown;
    rejectValue?: unknown;
    serializedErrorType?: unknown;
    pendingMeta?: unknown;
    fulfilledMeta?: unknown;
    rejectedMeta?: unknown;
}>;
export declare const applyChatEvent: ActionCreatorWithPayload<EventEnvelope, string>;
export type IdeToolRequiredPayload = {
    chatId: string;
    toolCallId: string;
    toolName: string;
    args: unknown;
};
export declare const ideToolRequired: ActionCreatorWithPayload<IdeToolRequiredPayload, string>;
export declare const hydratePersistedChatTabs: ActionCreatorWithoutPayload<"chatThread/hydratePersistedChatTabs">;
export declare const requestSseRefresh: ActionCreatorWithPayload<{
    chatId: string;
}, string>;
export declare const setAutoEnrichmentEnabled: ActionCreatorWithPayload<PayloadWithChatAndBoolean, string>;
export declare const setAutoCompactEnabled: ActionCreatorWithPayload<PayloadWithChatAndBoolean, string>;
export declare const markMemoryEnrichmentUserTouched: ActionCreatorWithPayload<{
    chatId: string;
}, string>;
export declare const setManualPreviewItems: ActionCreatorWithPayload<{
    chatId: string;
    items: ManualPreviewItem[];
}, string>;
export declare const removeManualPreviewItem: ActionCreatorWithPayload<{
    chatId: string;
    index: number;
}, string>;
export declare const clearManualPreviewItems: ActionCreatorWithPayload<{
    chatId: string;
}, string>;
export declare const clearSseRefreshRequest: ActionCreatorWithoutPayload<"chatThread/clearSseRefreshRequest">;
export declare const setTaskWidgetExpanded: ActionCreatorWithPayload<{
    id: string;
    expanded: boolean;
}, string>;
export declare const openBuddyChat: ActionCreatorWithPayload<{
    chat_id: string;
    title?: string;
}, string>;
export declare const newBuddyChatAction: ActionCreatorWithPayload<{
    chat_id: string;
}, string>;
export declare const openExistingBuddyChat: AsyncThunk<undefined, BuddyConversationEntry, {
    dispatch: AppDispatch;
    state: RootState;
    extra?: unknown;
    rejectValue?: unknown;
    serializedErrorType?: unknown;
    pendingMeta?: unknown;
    fulfilledMeta?: unknown;
    rejectedMeta?: unknown;
}>;
export declare const startBuddyInvestigation: AsyncThunk<{
    chat_id: string;
    title: string;
} | undefined, {
    triggerText: string;
    triggerSource: BuddyInvestigationSource;
    sourceChatId?: string;
    diagnostic?: DiagnosticContext | null;
}, {
    dispatch: AppDispatch;
    state: RootState;
    extra?: unknown;
    rejectValue?: unknown;
    serializedErrorType?: unknown;
    pendingMeta?: unknown;
    fulfilledMeta?: unknown;
    rejectedMeta?: unknown;
}>;
