import { RootState } from "../../../app/store";
import { createSelector } from "@reduxjs/toolkit";
import {
  isAssistantMessage,
  isDiffMessage,
  isToolMessage,
  isUserMessage,
  ChatMessages,
  ToolResult,
} from "../../../services/refact/types";
import { takeFromLast } from "../../../utils/takeFromLast";
import {
  ChatThreadRuntime,
  QueuedItem,
  ThreadConfirmation,
  ImageFile,
} from "./types";
import type { SessionState } from "../../../utils/sessionStatus";

const EMPTY_MESSAGES: ChatMessages = [];
const EMPTY_QUEUED: QueuedItem[] = [];
const EMPTY_PAUSE_REASONS: ThreadConfirmation["pause_reasons"] = [];
const EMPTY_IMAGES: ImageFile[] = [];
const DEFAULT_NEW_CHAT_SUGGESTED = { wasSuggested: false } as const;
const DEFAULT_CONFIRMATION: ThreadConfirmation = {
  pause: false,
  pause_reasons: [],
  status: { wasInteracted: false, confirmationStatus: true },
};
const DEFAULT_CONFIRMATION_STATUS = {
  wasInteracted: false,
  confirmationStatus: true,
} as const;

function deriveSessionStateFromRuntime(
  rt: ChatThreadRuntime | undefined,
): SessionState | undefined {
  if (!rt) return undefined;
  if (rt.error) return "error";
  if (rt.confirmation.pause) return "paused";
  if (rt.streaming) return "generating";
  if (rt.waiting_for_response) return "executing_tools";
  return "idle";
}

export const selectCurrentThreadId = (state: RootState) =>
  state.chat.current_thread_id;
export const selectOpenThreadIds = (state: RootState) =>
  state.chat.open_thread_ids;
export const selectAllThreads = (state: RootState) => state.chat.threads;

export type TabDisplayData = {
  id: string;
  title: string;
  session_state?: string;
};

export const selectTabsDisplayData = createSelector(
  [
    selectOpenThreadIds,
    selectAllThreads,
    (state: RootState) => state.history.chats,
  ],
  (openIds, threads, historyChats): TabDisplayData[] =>
    openIds.map((id) => {
      const runtime = threads[id];
      const historyItem = historyChats[id] as
        | (typeof historyChats)[string]
        | undefined;
      const liveSessionState = deriveSessionStateFromRuntime(runtime);
      return {
        id,
        title: runtime?.thread.title ?? historyItem?.title ?? "New Chat",
        session_state: liveSessionState ?? historyItem?.session_state,
      };
    }),
);

export const selectRuntimeById = (
  state: RootState,
  chatId: string,
): ChatThreadRuntime | null => {
  return state.chat.threads[chatId] ?? null;
};

export const selectCurrentRuntime = (
  state: RootState,
): ChatThreadRuntime | null =>
  state.chat.threads[state.chat.current_thread_id] ?? null;

export const selectThreadById = (state: RootState, chatId: string) =>
  state.chat.threads[chatId]?.thread ?? null;

export const selectThread = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.thread ?? null;

export const selectThreadTitle = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.thread.title;

export const selectChatId = (state: RootState) => state.chat.current_thread_id;

export const selectModel = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.thread.model ?? "";

export const selectMessages = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.thread.messages ??
  EMPTY_MESSAGES;

export const selectMessagesById = (state: RootState, chatId: string) =>
  state.chat.threads[chatId]?.thread.messages ?? EMPTY_MESSAGES;

export const selectToolUse = (state: RootState) => state.chat.tool_use;

export const selectThreadToolUse = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.thread.tool_use;

export const selectAutomaticPatch = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.thread.automatic_patch;

export const selectCheckpointsEnabled = (state: RootState) =>
  state.chat.checkpoints_enabled;

export const selectThreadBoostReasoning = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.thread.boost_reasoning;

export const selectIncludeProjectInfo = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.thread.include_project_info;

export const selectContextTokensCap = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.thread.context_tokens_cap;

export const selectThreadNewChatSuggested = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.thread.new_chat_suggested ??
  DEFAULT_NEW_CHAT_SUGGESTED;

export const selectThreadMaximumTokens = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.thread
    .currentMaximumContextTokens;

export const selectThreadCurrentMessageTokens = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.thread
    .currentMessageContextTokens;

export const selectIsWaiting = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.waiting_for_response ??
  false;

export const selectIsWaitingById = (state: RootState, chatId: string) =>
  state.chat.threads[chatId]?.waiting_for_response ?? false;

export const selectAreFollowUpsEnabled = (state: RootState) =>
  state.chat.follow_ups_enabled;

export const selectIsStreaming = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.streaming ?? false;

export const selectIsStreamingById = (state: RootState, chatId: string) =>
  state.chat.threads[chatId]?.streaming ?? false;

export const selectSnapshotReceived = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.snapshot_received ?? false;

export const selectSnapshotReceivedById = (state: RootState, chatId: string) =>
  state.chat.threads[chatId]?.snapshot_received ?? false;

export const selectPreventSend = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.prevent_send ?? false;

export const selectPreventSendById = (state: RootState, chatId: string) =>
  state.chat.threads[chatId]?.prevent_send ?? false;

export const selectChatError = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.error ?? null;

export const selectChatErrorById = (state: RootState, chatId: string) =>
  state.chat.threads[chatId]?.error ?? null;

export const selectSendImmediately = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.send_immediately ?? false;

export const getSelectedSystemPrompt = (state: RootState) =>
  state.chat.system_prompt;

export const selectAnyThreadStreaming = createSelector(
  [selectAllThreads],
  (threads) => Object.values(threads).some((rt) => rt?.streaming),
);

export const selectStreamingThreadIds = createSelector(
  [selectAllThreads],
  (threads) =>
    Object.entries(threads)
      .filter(([, rt]) => rt?.streaming)
      .map(([id]) => id),
);

export const toolMessagesSelector = createSelector(selectMessages, (messages) =>
  messages.filter(isToolMessage),
);

export const selectToolResultById = createSelector(
  [toolMessagesSelector, (_, id?: string) => id],
  (messages, id) => {
    if (!id) return undefined;
    for (let i = messages.length - 1; i >= 0; i--) {
      const m = messages[i];
      if (m.tool_call_id === id) {
        return {
          tool_call_id: m.tool_call_id,
          content: m.content,
          tool_failed: m.tool_failed,
        } as ToolResult;
      }
    }
    return undefined;
  },
);
export const selectManyToolResultsByIds = (ids: string[]) =>
  createSelector(toolMessagesSelector, (messages) =>
    messages
      .filter((message) => ids.includes(message.tool_call_id))
      .map(
        (msg) =>
          ({
            tool_call_id: msg.tool_call_id,
            content: msg.content,
            tool_failed: msg.tool_failed,
          }) as ToolResult,
      ),
  );

const selectDiffMessages = createSelector(selectMessages, (messages) =>
  messages.filter(isDiffMessage),
);

export const selectDiffMessageById = createSelector(
  [selectDiffMessages, (_, id?: string) => id],
  (messages, id) => messages.find((message) => message.tool_call_id === id),
);

export const selectManyDiffMessageByIds = (ids: string[]) =>
  createSelector(selectDiffMessages, (diffs) =>
    diffs.filter((message) => ids.includes(message.tool_call_id)),
  );

export const getSelectedToolUse = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.thread.tool_use;

export const selectIntegration = createSelector(
  selectThread,
  (thread) => thread?.integration,
);

export const selectThreadMode = createSelector(
  selectThread,
  (thread) => thread?.mode,
);

export const selectQueuedItems = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.queued_items ??
  EMPTY_QUEUED;

export const selectQueuedItemsById = (state: RootState, chatId: string) =>
  state.chat.threads[chatId]?.queued_items ?? EMPTY_QUEUED;

export const selectQueuedItemsCount = createSelector(
  selectQueuedItems,
  (queued) => queued.length,
);

export const selectHasQueuedItems = createSelector(
  selectQueuedItems,
  (queued) => queued.length > 0,
);

function hasUncalledToolsInMessages(
  messages: ReturnType<typeof selectMessages>,
): boolean {
  if (messages.length === 0) return false;
  const tailMessages = takeFromLast(messages, isUserMessage);

  const toolCalls = tailMessages.reduce<string[]>((acc, cur) => {
    if (!isAssistantMessage(cur)) return acc;
    if (!cur.tool_calls || cur.tool_calls.length === 0) return acc;
    const curToolCallIds = cur.tool_calls
      .map((toolCall) => toolCall.id)
      .filter(
        (id): id is string => id !== undefined && !id.startsWith("srvtoolu_"),
      );
    return [...acc, ...curToolCallIds];
  }, []);

  if (toolCalls.length === 0) return false;

  const toolMessages = tailMessages
    .map((msg) => {
      if (isToolMessage(msg)) return msg.tool_call_id;
      if ("tool_call_id" in msg && typeof msg.tool_call_id === "string")
        return msg.tool_call_id;
      return undefined;
    })
    .filter((id): id is string => typeof id === "string");

  return toolCalls.some((toolCallId) => !toolMessages.includes(toolCallId));
}

export const selectHasUncalledToolsById = (
  state: RootState,
  chatId: string,
): boolean => hasUncalledToolsInMessages(selectMessagesById(state, chatId));

export const selectHasUncalledTools = createSelector(
  selectMessages,
  hasUncalledToolsInMessages,
);

export const selectThreadConfirmation = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.confirmation ??
  DEFAULT_CONFIRMATION;

export const selectThreadConfirmationById = (
  state: RootState,
  chatId: string,
) => state.chat.threads[chatId]?.confirmation ?? DEFAULT_CONFIRMATION;

export const selectThreadPauseReasons = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.confirmation
    .pause_reasons ?? EMPTY_PAUSE_REASONS;

export const selectThreadPause = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.confirmation.pause ?? false;

export const selectThreadPauseById = (state: RootState, chatId: string) =>
  state.chat.threads[chatId]?.confirmation.pause ?? false;

export const selectThreadConfirmationStatus = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.confirmation.status ??
  DEFAULT_CONFIRMATION_STATUS;

export const selectThreadImages = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.attached_images ??
  EMPTY_IMAGES;

export const selectThreadImagesById = (state: RootState, chatId: string) =>
  state.chat.threads[chatId]?.attached_images ?? EMPTY_IMAGES;

const EMPTY_TEXT_FILES: import("./types").TextFile[] = [];

export const selectThreadTextFiles = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.attached_text_files ??
  EMPTY_TEXT_FILES;

export const selectThreadTextFilesById = (state: RootState, chatId: string) =>
  state.chat.threads[chatId]?.attached_text_files ?? EMPTY_TEXT_FILES;

export const selectSseRefreshRequested = (state: RootState) =>
  state.chat.sse_refresh_requested;

export const selectStreamVersion = (state: RootState): number =>
  state.chat.stream_version;
