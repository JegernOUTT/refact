import { RootState } from "../../../app/store";
import { createSelector } from "@reduxjs/toolkit";
import {
  isAssistantMessage,
  isDiffMessage,
  isToolMessage,
  isUserMessage,
  ChatMessages,
  DiffMessage,
  ToolResult,
  ToolMessage,
} from "../../../services/refact/types";
import { takeFromLast } from "../../../utils/takeFromLast";
import {
  ChatThreadRuntime,
  QueuedItem,
  ThreadConfirmation,
  ImageFile,
  TodoItem,
  TodoStatus,
} from "./types";
import type { SessionState } from "../../../utils/sessionStatus";
import type { WorktreeMeta } from "../../../services/refact/worktrees";

const EMPTY_MESSAGES: ChatMessages = [];
const EMPTY_QUEUED: QueuedItem[] = [];
const EMPTY_PAUSE_REASONS: ThreadConfirmation["pause_reasons"] = [];
const EMPTY_IMAGES: ImageFile[] = [];
const EMPTY_TOOL_RESULTS: ToolResult[] = [];
const EMPTY_TOOL_RESULTS_BY_ID: ReadonlyMap<string, ToolResult> = new Map();
const EMPTY_DIFF_MESSAGES_BY_ID: ReadonlyMap<string, DiffMessage[]> = new Map();
const EMPTY_DIFF_MESSAGES: DiffMessage[] = [];
const EMPTY_TASKS: TodoItem[] = [];
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

function sameRefArray<T>(left: T[], right: T[]): boolean {
  if (left === right) return true;
  if (left.length !== right.length) return false;
  for (let i = 0; i < left.length; i++) {
    if (left[i] !== right[i]) return false;
  }
  return true;
}

function sameTodoItems(left: TodoItem[], right: TodoItem[]): boolean {
  if (left === right) return true;
  if (left.length !== right.length) return false;
  for (let i = 0; i < left.length; i++) {
    if (left[i] === right[i]) continue;
    if (
      left[i].id !== right[i].id ||
      left[i].content !== right[i].content ||
      left[i].status !== right[i].status
    ) {
      return false;
    }
  }
  return true;
}

type TaskProgress = { done: number; total: number; activeTitle?: string };

function sameTaskProgress(left: TaskProgress, right: TaskProgress): boolean {
  return (
    left.done === right.done &&
    left.total === right.total &&
    left.activeTitle === right.activeTitle
  );
}

function deriveSessionStateFromRuntime(
  rt: ChatThreadRuntime | undefined,
): SessionState | undefined {
  if (!rt) return undefined;
  // Use stored session_state if available (for waiting_user_input, completed, etc.)
  if (rt.session_state) {
    return rt.session_state as SessionState;
  }
  // Fallback to derived state from booleans
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
export const selectAllThreads = (
  state: RootState,
): Record<string, ChatThreadRuntime | undefined> => state.chat.threads;

export type TabDisplayData = {
  id: string;
  title: string;
  session_state?: string;
  mode?: string;
  is_buddy_chat?: boolean;
};

export const selectTabsDisplayData = createSelector(
  [
    selectOpenThreadIds,
    selectAllThreads,
    (state: RootState) => state.history.chats,
  ],
  (openIds, threads, historyChats): TabDisplayData[] =>
    openIds.flatMap((id) => {
      const runtime = threads[id];
      if (runtime?.thread.buddy_meta?.is_buddy_chat) return [];
      const historyItem = historyChats[id] as
        | (typeof historyChats)[string]
        | undefined;
      const liveSessionState = deriveSessionStateFromRuntime(runtime);
      return [
        {
          id,
          title: runtime?.thread.title ?? historyItem?.title ?? "New Chat",
          session_state: liveSessionState ?? historyItem?.session_state,
          mode: runtime?.thread.mode ?? historyItem?.mode,
          is_buddy_chat: false,
        },
      ];
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

export const selectAutoApproveEditingTools = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.thread
    .auto_approve_editing_tools ?? false;

export const selectAutoApproveDangerousCommands = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.thread
    .auto_approve_dangerous_commands ?? false;

export const selectCheckpointsEnabled = (state: RootState) =>
  state.chat.checkpoints_enabled;

export const selectThreadBoostReasoning = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.thread.boost_reasoning;

export const selectIncludeProjectInfo = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.thread.include_project_info;

export const selectContextTokensCap = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.thread.context_tokens_cap;

export const selectReasoningEffort = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.thread.reasoning_effort;

export const selectThinkingBudget = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.thread.thinking_budget;

export const selectTemperature = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.thread.temperature;

export const selectFrequencyPenalty = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.thread.frequency_penalty;

export const selectMaxTokens = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.thread.max_tokens;

export const selectParallelToolCalls = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.thread.parallel_tool_calls;

export const selectThreadNewChatSuggested = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.thread.new_chat_suggested ??
  DEFAULT_NEW_CHAT_SUGGESTED;

export const selectThreadMaximumTokens = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.thread
    .currentMaximumContextTokens;

export const selectEffectiveMaxContextTokens = (state: RootState) => {
  const thread = state.chat.threads[state.chat.current_thread_id]?.thread;
  if (!thread) return undefined;
  const modelMax = thread.currentMaximumContextTokens;
  const cap = thread.context_tokens_cap;
  if (cap && cap > 0) {
    return modelMax && modelMax > 0 ? Math.min(cap, modelMax) : cap;
  }
  return modelMax;
};

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

export const toolResultsByIdSelector = (() => {
  let prevMessages: ToolMessage[] = [];
  let prevMap: ReadonlyMap<string, ToolResult> = EMPTY_TOOL_RESULTS_BY_ID;

  return createSelector(toolMessagesSelector, (messages) => {
    if (messages.length === 0) {
      prevMessages = [];
      prevMap = EMPTY_TOOL_RESULTS_BY_ID;
      return prevMap;
    }

    if (sameRefArray(prevMessages, messages)) {
      return prevMap;
    }

    const nextMap = new Map<string, ToolResult>();
    for (let i = 0; i < messages.length; i++) {
      const msg = messages[i];
      nextMap.set(msg.tool_call_id, msg as unknown as ToolResult);
    }

    prevMessages = messages;
    prevMap = nextMap;
    return nextMap;
  });
})();

export const selectToolResultById = createSelector(
  [toolResultsByIdSelector, (_, id?: string) => id],
  (messagesById, id) => (id ? messagesById.get(id) : undefined),
);
export const selectManyToolResultsByIds = (ids: string[]) => {
  let prev = EMPTY_TOOL_RESULTS;

  return createSelector(toolResultsByIdSelector, (messagesById) => {
    if (ids.length === 0 || messagesById.size === 0) {
      prev = EMPTY_TOOL_RESULTS;
      return prev;
    }

    const next: ToolResult[] = [];
    for (const id of ids) {
      const msg = messagesById.get(id);
      if (msg) next.push(msg);
    }

    if (sameRefArray(prev, next)) {
      return prev;
    }

    prev = next;
    return next;
  });
};

const selectDiffMessages = createSelector(selectMessages, (messages) =>
  messages.filter(isDiffMessage),
);

export const diffMessagesByIdSelector = (() => {
  let prevDiffs: DiffMessage[] = [];
  let prevMap: ReadonlyMap<string, DiffMessage[]> = EMPTY_DIFF_MESSAGES_BY_ID;

  return createSelector(selectDiffMessages, (diffs) => {
    if (diffs.length === 0) {
      prevDiffs = [];
      prevMap = EMPTY_DIFF_MESSAGES_BY_ID;
      return prevMap;
    }

    if (sameRefArray(prevDiffs, diffs)) {
      return prevMap;
    }

    const nextMap = new Map<string, DiffMessage[]>();
    for (let i = 0; i < diffs.length; i++) {
      const diff = diffs[i];
      const existing = nextMap.get(diff.tool_call_id);
      if (existing) {
        existing.push(diff);
      } else {
        nextMap.set(diff.tool_call_id, [diff]);
      }
    }

    prevDiffs = diffs;
    prevMap = nextMap;
    return nextMap;
  });
})();

export const selectDiffMessageById = createSelector(
  [diffMessagesByIdSelector, (_, id?: string) => id],
  (messagesById, id) => {
    if (!id) return undefined;
    const messages = messagesById.get(id);
    return messages ? messages[messages.length - 1] : undefined;
  },
);
export const selectManyDiffMessageByIds = (ids: string[]) => {
  let prev = EMPTY_DIFF_MESSAGES;

  return createSelector(diffMessagesByIdSelector, (diffsById) => {
    if (ids.length === 0 || diffsById.size === 0) {
      prev = EMPTY_DIFF_MESSAGES;
      return prev;
    }

    const next: DiffMessage[] = [];
    for (const id of ids) {
      const messages = diffsById.get(id);
      if (messages) next.push(...messages);
    }

    if (sameRefArray(prev, next)) {
      return prev;
    }

    prev = next;
    return next;
  });
};

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

// Task Progress Widget selectors

export const selectTaskWidgetExpanded = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.task_widget_expanded ??
  false;

export const selectTaskWidgetExpandedById = (
  state: RootState,
  chatId: string,
) => state.chat.threads[chatId]?.task_widget_expanded ?? false;

function normalizeTaskStatus(status: unknown): TodoStatus | null {
  if (typeof status !== "string") return null;
  switch (status.toLowerCase()) {
    case "pending":
      return "pending";
    case "in_progress":
    case "in-progress":
    case "inprogress":
      return "in_progress";
    case "completed":
    case "done":
    case "complete":
      return "completed";
    case "failed":
    case "error":
      return "failed";
    default:
      return null;
  }
}

function sanitizeText(text: string, maxLen: number): string {
  return (
    text
      // eslint-disable-next-line no-control-regex
      .replace(/[\x00-\x1F\x7F]/g, "")
      .trim()
      .slice(0, maxLen)
  );
}

function parseTasksFromArgs(argsStr: string): TodoItem[] | null {
  try {
    const args = JSON.parse(argsStr) as unknown;
    if (!args || typeof args !== "object") return null;
    const tasksArray = (args as Record<string, unknown>).tasks;
    if (!Array.isArray(tasksArray)) return null;

    if (tasksArray.length === 0) return [];

    const result: TodoItem[] = [];
    const seenIds = new Set<string>();

    for (const item of tasksArray) {
      if (!item || typeof item !== "object") continue;
      const t = item as Record<string, unknown>;

      const rawId =
        typeof t.id === "string"
          ? t.id
          : typeof t.id === "number"
            ? String(t.id)
            : null;
      if (!rawId) continue;

      const id = sanitizeText(rawId, 50);
      if (!id || seenIds.has(id)) continue;
      seenIds.add(id);

      const rawContent = typeof t.content === "string" ? t.content : null;
      if (!rawContent) continue;

      const content = sanitizeText(rawContent, 500);
      if (!content) continue;

      const status = normalizeTaskStatus(t.status);
      if (!status) continue;

      result.push({ id, content, status });
    }
    return result.length > 0 ? result : null;
  } catch {
    return null;
  }
}

export function deriveTasksFromMessages(
  messages: ChatMessages,
  toolMessages: ToolMessage[],
): TodoItem[] {
  const successfulToolIds = new Set(
    toolMessages.filter((m) => !m.tool_failed).map((m) => m.tool_call_id),
  );

  for (let i = messages.length - 1; i >= 0; i--) {
    const msg = messages[i];
    if (!isAssistantMessage(msg) || !msg.tool_calls) continue;

    for (let j = msg.tool_calls.length - 1; j >= 0; j--) {
      const tc = msg.tool_calls[j];
      if (tc.function.name !== "tasks_set" || !tc.id) continue;
      if (!successfulToolIds.has(tc.id)) continue;

      const parsed = parseTasksFromArgs(tc.function.arguments);
      if (parsed !== null) return parsed;
    }
  }

  return [];
}

export const selectCurrentTasks = (() => {
  let prev = EMPTY_TASKS;

  return createSelector(
    [selectMessages, toolMessagesSelector],
    (messages, toolMessages): TodoItem[] => {
      const next = deriveTasksFromMessages(messages, toolMessages);
      if (sameTodoItems(prev, next)) {
        return prev;
      }

      prev = next;
      return next;
    },
  );
})();

export const selectCurrentTasksById = (state: RootState, chatId: string) => {
  const messages = selectMessagesById(state, chatId);
  const toolMessages = messages.filter(isToolMessage);
  return deriveTasksFromMessages(messages, toolMessages);
};

export const selectHasTasks = createSelector(
  [selectCurrentTasks],
  (tasks) => tasks.length > 0,
);

export const selectTasksEverUsed = createSelector(
  [selectMessages, toolMessagesSelector],
  (messages, toolMessages): boolean => {
    const successfulToolIds = new Set(
      toolMessages.filter((m) => !m.tool_failed).map((m) => m.tool_call_id),
    );

    for (const msg of messages) {
      if (!isAssistantMessage(msg) || !msg.tool_calls) continue;
      for (const tc of msg.tool_calls) {
        if (
          tc.function.name === "tasks_set" &&
          tc.id &&
          successfulToolIds.has(tc.id)
        ) {
          return true;
        }
      }
    }
    return false;
  },
);

export const selectTaskProgress = (() => {
  let prev: TaskProgress = { done: 0, total: 0, activeTitle: undefined };

  return createSelector([selectCurrentTasks], (tasks): TaskProgress => {
    const done = tasks.filter((t) => t.status === "completed").length;
    const active = tasks.find((t) => t.status === "in_progress");
    const next = {
      done,
      total: tasks.length,
      activeTitle: active?.content,
    };

    if (sameTaskProgress(prev, next)) {
      return prev;
    }

    prev = next;
    return next;
  });
})();

export type TaskProgressInfo = {
  done: number;
  total: number;
  failed: number;
};

/**
 * Compute task progress from messages array.
 * Useful for history items that have messages but aren't in Redux state.
 */
export function getTaskProgressFromMessages(
  messages: ChatMessages,
): TaskProgressInfo | null {
  const toolMessages = messages.filter(isToolMessage);
  const tasks = deriveTasksFromMessages(messages, toolMessages);

  if (tasks.length === 0) return null;

  return {
    done: tasks.filter((t) => t.status === "completed").length,
    total: tasks.length,
    failed: tasks.filter((t) => t.status === "failed").length,
  };
}

export const selectAutoEnrichmentEnabled = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.thread
    .auto_enrichment_enabled ?? false;

export const selectAutoEnrichmentEnabledById = (
  state: RootState,
  chatId: string,
) => state.chat.threads[chatId]?.thread.auto_enrichment_enabled ?? false;

export const selectMemoryEnrichmentUserTouched = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]
    ?.memory_enrichment_user_touched ?? false;

export const selectManualPreviewItems = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.manual_preview_items ?? [];

export const selectManualPreviewItemsById = (
  state: RootState,
  chatId: string,
) => state.chat.threads[chatId]?.manual_preview_items ?? [];

export const selectManualPreviewRan = (state: RootState) =>
  state.chat.threads[state.chat.current_thread_id]?.manual_preview_ran ?? false;

export const selectThreadWorktree = (state: RootState): WorktreeMeta | null =>
  state.chat.threads[state.chat.current_thread_id]?.thread.worktree ?? null;

export const selectThreadWorktreeById = (
  state: RootState,
  chatId: string,
): WorktreeMeta | null => state.chat.threads[chatId]?.thread.worktree ?? null;

export const selectIsBuddyChat = (state: RootState, chatId: string): boolean =>
  !!state.chat.threads[chatId]?.thread.buddy_meta?.is_buddy_chat;
