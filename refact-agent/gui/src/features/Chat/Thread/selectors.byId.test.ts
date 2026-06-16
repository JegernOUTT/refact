import { describe, expect, it } from "vitest";
import type { RootState } from "../../../app/store";
import type {
  DiffMessage,
  ToolMessage,
} from "../../../services/refact/types";
import type { WorktreeMeta } from "../../../services/refact/worktrees";
import type { ChatThreadRuntime, ThreadConfirmation } from "./types";
import {
  selectAutoApproveDangerousCommands,
  selectAutoApproveDangerousCommandsById,
  selectAutoApproveEditingTools,
  selectAutoApproveEditingToolsById,
  selectAutoCompactEnabled,
  selectAutoCompactEnabledById,
  selectAutoEnrichmentEnabled,
  selectAutoEnrichmentEnabledById,
  selectChatError,
  selectChatErrorById,
  selectCheckpointsEnabled,
  selectCheckpointsEnabledById,
  selectContextTokensCap,
  selectContextTokensCapById,
  selectCurrentRuntime,
  selectCurrentTasks,
  selectEffectiveMaxContextTokens,
  selectEffectiveMaxContextTokensById,
  selectFrequencyPenalty,
  selectFrequencyPenaltyById,
  selectIncludeProjectInfo,
  selectIncludeProjectInfoById,
  selectIntegration,
  selectIntegrationById,
  selectIsStreaming,
  selectIsStreamingById,
  selectIsWaiting,
  selectIsWaitingById,
  selectManualPreviewItems,
  selectManualPreviewItemsById,
  selectManualPreviewRan,
  selectManualPreviewRanById,
  selectManyDiffMessageByIds,
  selectManyDiffMessageByThreadAndIds,
  selectManyToolResultsByIds,
  selectManyToolResultsByThreadAndIds,
  selectMaxTokens,
  selectMaxTokensById,
  selectMemoryEnrichmentUserTouched,
  selectMemoryEnrichmentUserTouchedById,
  selectMessages,
  selectMessagesById,
  selectModel,
  selectModelById,
  selectParallelToolCalls,
  selectParallelToolCallsById,
  selectPreventSend,
  selectPreventSendById,
  selectQueuedItems,
  selectQueuedItemsById,
  selectReasoningEffort,
  selectReasoningEffortById,
  selectSendImmediately,
  selectSendImmediatelyById,
  selectSnapshotReceived,
  selectSnapshotReceivedById,
  selectTemperature,
  selectTemperatureById,
  selectTaskWidgetExpanded,
  selectTaskWidgetExpandedById,
  selectThinkingBudget,
  selectThinkingBudgetById,
  selectThread,
  selectThreadById,
  selectThreadBoostReasoning,
  selectThreadBoostReasoningById,
  selectThreadConfirmation,
  selectThreadConfirmationById,
  selectThreadConfirmationStatus,
  selectThreadConfirmationStatusById,
  selectThreadCurrentMessageTokens,
  selectThreadCurrentMessageTokensById,
  selectThreadImages,
  selectThreadImagesById,
  selectThreadMaximumTokens,
  selectThreadMaximumTokensById,
  selectThreadMode,
  selectThreadModeById,
  selectThreadNewChatSuggested,
  selectThreadNewChatSuggestedById,
  selectThreadPause,
  selectThreadPauseById,
  selectThreadPauseReasons,
  selectThreadPauseReasonsById,
  selectThreadTextFiles,
  selectThreadTextFilesById,
  selectThreadTitle,
  selectThreadTitleById,
  selectThreadToolUse,
  selectThreadToolUseById,
  selectThreadWorktree,
  selectThreadWorktreeById,
  selectToolResultById,
  selectToolResultByThreadAndId,
  selectDiffMessageById,
  selectDiffMessageByThreadAndId,
} from "./selectors";

const toolMessageA: ToolMessage = {
  role: "tool",
  tool_call_id: "tool-A",
  content: "result A",
};

const toolMessageB: ToolMessage = {
  role: "tool",
  tool_call_id: "tool-B",
  content: "result B",
};

const diffMessageA: DiffMessage = {
  role: "diff",
  tool_call_id: "tool-A",
  content: [
    {
      file_name: "a.ts",
      file_action: "edit",
      line1: 1,
      line2: 1,
      lines_remove: "old A",
      lines_add: "new A",
    },
  ],
};

const diffMessageB: DiffMessage = {
  role: "diff",
  tool_call_id: "tool-B",
  content: [
    {
      file_name: "b.ts",
      file_action: "edit",
      line1: 2,
      line2: 2,
      lines_remove: "old B",
      lines_add: "new B",
    },
  ],
};

const messagesA = [toolMessageA, diffMessageA];
const messagesB = [toolMessageB, diffMessageB];
const queuedA = [
  {
    client_request_id: "queued-A",
    priority: true,
    command_type: "user_message",
    preview: "queued A",
  },
];
const queuedB = [
  {
    client_request_id: "queued-B",
    priority: false,
    command_type: "set_params",
    preview: "queued B",
  },
];
const imagesA = [{ name: "a.png", content: null, type: "image/png" }];
const imagesB = [{ name: "b.png", content: null, type: "image/png" }];
const textFilesA = [{ name: "a.txt", content: "text A" }];
const textFilesB = [{ name: "b.txt", content: "text B" }];
const manualPreviewItemsA: ChatThreadRuntime["manual_preview_items"] = [
  {
    kind: "memory",
    label: "memory A",
    context_file: {
      file_name: "memory-a.md",
      file_content: "A",
      line1: 1,
      line2: 1,
      usefulness: 1,
    },
  },
];
const manualPreviewItemsB: ChatThreadRuntime["manual_preview_items"] = [
  {
    kind: "file",
    label: "file B",
    context_file: {
      file_name: "file-b.md",
      file_content: "B",
      line1: 2,
      line2: 2,
      usefulness: 2,
    },
  },
];
const pauseReasonsA: ThreadConfirmation["pause_reasons"] = [
  {
    type: "confirmation",
    tool_name: "tool-A",
    command: "edit",
    rule: "ask",
    tool_call_id: "tool-A",
    integr_config_path: null,
  },
];
const pauseReasonsB: ThreadConfirmation["pause_reasons"] = [
  {
    type: "denial",
    tool_name: "tool-B",
    command: "shell",
    rule: "deny",
    tool_call_id: "tool-B",
    integr_config_path: null,
  },
];
const confirmationStatusA = {
  wasInteracted: true,
  confirmationStatus: false,
};
const confirmationStatusB = {
  wasInteracted: false,
  confirmationStatus: true,
};
const integrationA = { name: "github", project: "alpha" };
const integrationB = { name: "gitlab", project: "beta" };
const newChatSuggestedA = { wasSuggested: true, wasRejectedByUser: false };
const newChatSuggestedB = { wasSuggested: true, wasRejectedByUser: true };
const worktreeA: WorktreeMeta = {
  id: "worktree-A",
  kind: "task",
  root: "/tmp/a",
  source_workspace_root: "/tmp/source",
  repo_root: "/tmp/a",
  enforce: true,
};
const worktreeB: WorktreeMeta = {
  id: "worktree-B",
  kind: "task",
  root: "/tmp/b",
  source_workspace_root: "/tmp/source",
  repo_root: "/tmp/b",
  enforce: true,
};

function makeRuntime(
  id: "thread-A" | "thread-B",
): ChatThreadRuntime {
  const isA = id === "thread-A";

  return {
    thread: {
      id,
      messages: isA ? messagesA : messagesB,
      title: isA ? "Thread A" : "Thread B",
      model: isA ? "model-A" : "model-B",
      tool_use: isA ? "quick" : "agent",
      new_chat_suggested: isA ? newChatSuggestedA : newChatSuggestedB,
      boost_reasoning: isA,
      reasoning_effort: isA ? "low" : "high",
      thinking_budget: isA ? 111 : 222,
      temperature: isA ? 0.1 : 0.2,
      frequency_penalty: isA ? 0.3 : 0.4,
      max_tokens: isA ? 1000 : 2000,
      parallel_tool_calls: isA,
      integration: isA ? integrationA : integrationB,
      mode: isA ? "agent" : "task_agent",
      auto_approve_editing_tools: isA,
      auto_approve_dangerous_commands: !isA,
      checkpoints_enabled: !isA,
      currentMaximumContextTokens: isA ? 8000 : 16000,
      currentMessageContextTokens: isA ? 512 : 1024,
      include_project_info: isA,
      context_tokens_cap: isA ? 4000 : 32000,
      auto_enrichment_enabled: !isA,
      auto_compact_enabled: isA,
      worktree: isA ? worktreeA : worktreeB,
    },
    streaming: isA,
    waiting_for_response: !isA,
    prevent_send: !isA,
    error: isA ? null : "thread B error",
    queued_items: isA ? queuedA : queuedB,
    send_immediately: isA,
    attached_images: isA ? imagesA : imagesB,
    attached_text_files: isA ? textFilesA : textFilesB,
    background_agents: {},
    confirmation: {
      pause: !isA,
      pause_reasons: isA ? pauseReasonsA : pauseReasonsB,
      status: isA ? confirmationStatusA : confirmationStatusB,
    },
    snapshot_received: isA,
    task_widget_expanded: !isA,
    memory_enrichment_user_touched: !isA,
    manual_preview_items: isA ? manualPreviewItemsA : manualPreviewItemsB,
    manual_preview_ran: !isA,
  };
}

const runtimeA = makeRuntime("thread-A");
const runtimeB = makeRuntime("thread-B");
const state = {
  chat: {
    current_thread_id: "thread-A",
    open_thread_ids: ["thread-A", "thread-B"],
    threads: {
      "thread-A": runtimeA,
      "thread-B": runtimeB,
    },
    system_prompt: {},
    tool_use: "explore",
    checkpoints_enabled: true,
    follow_ups_enabled: true,
    sse_refresh_requested: null,
    stream_version: 0,
  },
} as unknown as RootState;

type CurrentSelector = (state: RootState) => unknown;
type ByIdSelector = (state: RootState, chatId: string) => unknown;

const selectorCases: {
  name: string;
  select: CurrentSelector;
  selectById: ByIdSelector;
  expectedThreadB: unknown;
}[] = [
  {
    name: "selectCurrentRuntime",
    select: selectCurrentRuntime,
    selectById: selectCurrentRuntimeById,
    expectedThreadB: runtimeB,
  },
  {
    name: "selectThread",
    select: selectThread,
    selectById: selectThreadById,
    expectedThreadB: runtimeB.thread,
  },
  {
    name: "selectThreadTitle",
    select: selectThreadTitle,
    selectById: selectThreadTitleById,
    expectedThreadB: "Thread B",
  },
  {
    name: "selectModel",
    select: selectModel,
    selectById: selectModelById,
    expectedThreadB: "model-B",
  },
  {
    name: "selectMessages",
    select: selectMessages,
    selectById: selectMessagesById,
    expectedThreadB: messagesB,
  },
  {
    name: "selectThreadToolUse",
    select: selectThreadToolUse,
    selectById: selectThreadToolUseById,
    expectedThreadB: "agent",
  },
  {
    name: "selectAutoApproveEditingTools",
    select: selectAutoApproveEditingTools,
    selectById: selectAutoApproveEditingToolsById,
    expectedThreadB: false,
  },
  {
    name: "selectAutoApproveDangerousCommands",
    select: selectAutoApproveDangerousCommands,
    selectById: selectAutoApproveDangerousCommandsById,
    expectedThreadB: true,
  },
  {
    name: "selectCheckpointsEnabled",
    select: selectCheckpointsEnabled,
    selectById: selectCheckpointsEnabledById,
    expectedThreadB: true,
  },
  {
    name: "selectThreadBoostReasoning",
    select: selectThreadBoostReasoning,
    selectById: selectThreadBoostReasoningById,
    expectedThreadB: false,
  },
  {
    name: "selectIncludeProjectInfo",
    select: selectIncludeProjectInfo,
    selectById: selectIncludeProjectInfoById,
    expectedThreadB: false,
  },
  {
    name: "selectContextTokensCap",
    select: selectContextTokensCap,
    selectById: selectContextTokensCapById,
    expectedThreadB: 32000,
  },
  {
    name: "selectReasoningEffort",
    select: selectReasoningEffort,
    selectById: selectReasoningEffortById,
    expectedThreadB: "high",
  },
  {
    name: "selectThinkingBudget",
    select: selectThinkingBudget,
    selectById: selectThinkingBudgetById,
    expectedThreadB: 222,
  },
  {
    name: "selectTemperature",
    select: selectTemperature,
    selectById: selectTemperatureById,
    expectedThreadB: 0.2,
  },
  {
    name: "selectFrequencyPenalty",
    select: selectFrequencyPenalty,
    selectById: selectFrequencyPenaltyById,
    expectedThreadB: 0.4,
  },
  {
    name: "selectMaxTokens",
    select: selectMaxTokens,
    selectById: selectMaxTokensById,
    expectedThreadB: 2000,
  },
  {
    name: "selectParallelToolCalls",
    select: selectParallelToolCalls,
    selectById: selectParallelToolCallsById,
    expectedThreadB: false,
  },
  {
    name: "selectThreadNewChatSuggested",
    select: selectThreadNewChatSuggested,
    selectById: selectThreadNewChatSuggestedById,
    expectedThreadB: newChatSuggestedB,
  },
  {
    name: "selectThreadMaximumTokens",
    select: selectThreadMaximumTokens,
    selectById: selectThreadMaximumTokensById,
    expectedThreadB: 16000,
  },
  {
    name: "selectEffectiveMaxContextTokens",
    select: selectEffectiveMaxContextTokens,
    selectById: selectEffectiveMaxContextTokensById,
    expectedThreadB: 16000,
  },
  {
    name: "selectThreadCurrentMessageTokens",
    select: selectThreadCurrentMessageTokens,
    selectById: selectThreadCurrentMessageTokensById,
    expectedThreadB: 1024,
  },
  {
    name: "selectIsWaiting",
    select: selectIsWaiting,
    selectById: selectIsWaitingById,
    expectedThreadB: true,
  },
  {
    name: "selectIsStreaming",
    select: selectIsStreaming,
    selectById: selectIsStreamingById,
    expectedThreadB: false,
  },
  {
    name: "selectSnapshotReceived",
    select: selectSnapshotReceived,
    selectById: selectSnapshotReceivedById,
    expectedThreadB: false,
  },
  {
    name: "selectPreventSend",
    select: selectPreventSend,
    selectById: selectPreventSendById,
    expectedThreadB: true,
  },
  {
    name: "selectChatError",
    select: selectChatError,
    selectById: selectChatErrorById,
    expectedThreadB: "thread B error",
  },
  {
    name: "selectSendImmediately",
    select: selectSendImmediately,
    selectById: selectSendImmediatelyById,
    expectedThreadB: false,
  },
  {
    name: "selectIntegration",
    select: selectIntegration,
    selectById: selectIntegrationById,
    expectedThreadB: integrationB,
  },
  {
    name: "selectThreadMode",
    select: selectThreadMode,
    selectById: selectThreadModeById,
    expectedThreadB: "task_agent",
  },
  {
    name: "selectQueuedItems",
    select: selectQueuedItems,
    selectById: selectQueuedItemsById,
    expectedThreadB: queuedB,
  },
  {
    name: "selectThreadConfirmation",
    select: selectThreadConfirmation,
    selectById: selectThreadConfirmationById,
    expectedThreadB: runtimeB.confirmation,
  },
  {
    name: "selectThreadPauseReasons",
    select: selectThreadPauseReasons,
    selectById: selectThreadPauseReasonsById,
    expectedThreadB: pauseReasonsB,
  },
  {
    name: "selectThreadPause",
    select: selectThreadPause,
    selectById: selectThreadPauseById,
    expectedThreadB: true,
  },
  {
    name: "selectThreadConfirmationStatus",
    select: selectThreadConfirmationStatus,
    selectById: selectThreadConfirmationStatusById,
    expectedThreadB: confirmationStatusB,
  },
  {
    name: "selectThreadImages",
    select: selectThreadImages,
    selectById: selectThreadImagesById,
    expectedThreadB: imagesB,
  },
  {
    name: "selectThreadTextFiles",
    select: selectThreadTextFiles,
    selectById: selectThreadTextFilesById,
    expectedThreadB: textFilesB,
  },
  {
    name: "selectTaskWidgetExpanded",
    select: selectTaskWidgetExpanded,
    selectById: selectTaskWidgetExpandedById,
    expectedThreadB: true,
  },
  {
    name: "selectAutoEnrichmentEnabled",
    select: selectAutoEnrichmentEnabled,
    selectById: selectAutoEnrichmentEnabledById,
    expectedThreadB: true,
  },
  {
    name: "selectAutoCompactEnabled",
    select: selectAutoCompactEnabled,
    selectById: selectAutoCompactEnabledById,
    expectedThreadB: false,
  },
  {
    name: "selectMemoryEnrichmentUserTouched",
    select: selectMemoryEnrichmentUserTouched,
    selectById: selectMemoryEnrichmentUserTouchedById,
    expectedThreadB: true,
  },
  {
    name: "selectManualPreviewItems",
    select: selectManualPreviewItems,
    selectById: selectManualPreviewItemsById,
    expectedThreadB: manualPreviewItemsB,
  },
  {
    name: "selectManualPreviewRan",
    select: selectManualPreviewRan,
    selectById: selectManualPreviewRanById,
    expectedThreadB: true,
  },
  {
    name: "selectThreadWorktree",
    select: selectThreadWorktree,
    selectById: selectThreadWorktreeById,
    expectedThreadB: worktreeB,
  },
];

function selectCurrentRuntimeById(state: RootState, chatId: string) {
  return state.chat.threads[chatId] ?? null;
}

describe("thread-scoped selectors", () => {
  it.each(selectorCases)(
    "$name delegates to current id and reads requested background id",
    ({ select, selectById, expectedThreadB }) => {
      expect(select(state)).toBe(selectById(state, "thread-A"));
      expect(selectById(state, "thread-B")).toBe(expectedThreadB);
    },
  );

  it("keeps memoized current-thread selectors referentially stable", () => {
    const selectToolResults = selectManyToolResultsByIds(["tool-A"]);
    const selectDiffMessages = selectManyDiffMessageByIds(["tool-A"]);

    expect(selectCurrentTasks(state)).toBe(selectCurrentTasks(state));
    expect(selectToolResults(state)).toBe(selectToolResults(state));
    expect(selectDiffMessages(state)).toBe(selectDiffMessages(state));
  });

  it("reads tool and diff lookups from the requested thread", () => {
    const selectBackgroundToolResults = selectManyToolResultsByThreadAndIds(
      "thread-B",
      ["tool-A", "tool-B"],
    );
    const selectBackgroundDiffMessages = selectManyDiffMessageByThreadAndIds(
      "thread-B",
      ["tool-A", "tool-B"],
    );

    expect(selectToolResultById(state, "tool-A")).toBe(toolMessageA);
    expect(selectToolResultByThreadAndId(state, "thread-B", "tool-B")).toBe(
      toolMessageB,
    );
    expect(selectDiffMessageById(state, "tool-A")).toBe(diffMessageA);
    expect(selectDiffMessageByThreadAndId(state, "thread-B", "tool-B")).toBe(
      diffMessageB,
    );
    expect(selectBackgroundToolResults(state)).toEqual([toolMessageB]);
    expect(selectBackgroundToolResults(state)).toBe(
      selectBackgroundToolResults(state),
    );
    expect(selectBackgroundDiffMessages(state)).toEqual([diffMessageB]);
    expect(selectBackgroundDiffMessages(state)).toBe(
      selectBackgroundDiffMessages(state),
    );
  });
});
