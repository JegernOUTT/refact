import {
  AssistantMessage,
  ChatContextFile,
  ChatMessages,
  DiffChunk,
  DiffMessage,
  ErrorMessage,
  EventMessage,
  eventContentAttachments,
  eventContentText,
  getAssistantCompressionMetadata,
  getCompressionReportMetadata,
  isChatContextFileMessage,
  isDiffMessage,
  isCompressedAssistantMessage,
  isAssistantMessage,
  isCompressionReportMessage,
  isEventMessage,
  isErrorMessage,
  isSystemMessage,
  isToolMessage,
  isSummarizationMessage,
  isVisibleCompressionFailureEvent,
  isBuddyPulsePayload,
  normalizeEventMessageMetadata,
  SummarizationMessage,
  syntheticCompressionReportMessage,
  syntheticSummarizationMessage,
  UserMessage,
  ToolMessage,
  ChatContextFileMessage,
  isToolContent,
  ToolCall,
} from "../../services/refact";
import { normalizeToolName, isToolName } from "../../utils/toolNameAliases";
import { isSkillReportContent, parseSkillReport } from "./skillReportUtils";
import { foldedEventIndexFor } from "./foldedEventIndex";
import {
  parseToolJobReceipt,
  type ToolJobReceipt,
} from "../../services/refact/toolJobs";

const BUDDY_PULSE_TOOL_CALL_ID = "buddy_project_memory_pulse";

export type NativeToolJobResult = {
  toolName: string;
  message: ToolMessage | ChatContextFileMessage;
  toolCall?: ToolCall;
  jobId?: string;
};

export type ToolJobResultsByToolId = Readonly<
  Record<string, readonly NativeToolJobResult[] | undefined>
>;

export const EMPTY_TOOL_JOB_RESULTS: ToolJobResultsByToolId = Object.freeze(
  Object.create(null) as ToolJobResultsByToolId,
);

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

const nativeToolJobResultCache = new WeakMap<
  ChatMessages[number],
  NativeToolJobResult | null
>();

export function nativeToolJobResult(
  message: ChatMessages[number],
): NativeToolJobResult | null {
  const cached = nativeToolJobResultCache.get(message);
  if (cached !== undefined) return cached;
  const parsed = parseNativeToolJobResult(message);
  nativeToolJobResultCache.set(message, parsed);
  return parsed;
}

// This is presentation validation, not authority. The owning transcript and the
// backend's fresh delivery clearance determine which chat may receive output.
function parseNativeToolJobResult(
  message: ChatMessages[number],
): NativeToolJobResult | null {
  const meta = record(
    message.extra?.tool_job_result ??
      ("tool_job_result" in message ? message.tool_job_result : undefined),
  );
  const delivery = record(
    message.extra?.delivery ??
      ("delivery" in message ? message.delivery : undefined),
  );
  if (
    !meta ||
    meta.actor !== "tool_completion" ||
    !nonEmptyString(meta.job_id) ||
    !nonEmptyString(meta.tool_name) ||
    typeof meta.tool_call_id !== "string" ||
    typeof meta.revision !== "number" ||
    !Number.isSafeInteger(meta.revision) ||
    meta.revision < 1 ||
    delivery?.source !== "tool_job.completed" ||
    delivery.id !== `tool_job:${meta.job_id}:${meta.revision}` ||
    !message.message_id?.startsWith(`${delivery.id}:`) ||
    !/^\d+$/.test(message.message_id.slice(delivery.id.length + 1))
  )
    return null;

  if (message.role === "event") {
    const event = normalizeEventMessageMetadata(message);
    const payload = record(event.payload);
    if (
      event.source !== "tool_job.completed" ||
      event.subkind !== "system_notice" ||
      payload?.actor !== "tool_completion" ||
      payload.job_id !== meta.job_id
    )
      return null;
    const content: unknown = message.content;
    if (meta.original_role === "tool" && isToolContent(content)) {
      return {
        toolName: meta.tool_name,
        jobId: meta.job_id,
        message: {
          ...message,
          role: "tool",
          content,
          tool_call_id: meta.tool_call_id,
          extra: flattenedWireExtra(message),
        },
      };
    }
  }
  if (
    meta.original_role === "context_file" &&
    isChatContextFileMessage(message)
  ) {
    return { toolName: meta.tool_name, jobId: meta.job_id, message };
  }
  return null;
}

const FLATTENED_WIRE_EXTRA_KEYS = [
  "tool_job_result",
  "delivery",
  "privacy_shell",
] as const;

function flattenedWireExtra(
  message: ChatMessages[number],
): Record<string, unknown> | undefined {
  const lifted = Object.fromEntries(
    Object.entries(message).filter(
      ([key, value]) =>
        (FLATTENED_WIRE_EXTRA_KEYS as readonly string[]).includes(key) &&
        value !== undefined,
    ),
  );
  if (Object.keys(lifted).length === 0) return message.extra;
  return { ...lifted, ...message.extra };
}

const toolCallResultCache = new WeakMap<
  NativeToolJobResult,
  { call: ToolCall; value: NativeToolJobResult }
>();

function withToolCall(
  result: NativeToolJobResult,
  call: ToolCall,
): NativeToolJobResult {
  const cached = toolCallResultCache.get(result);
  if (cached?.call === call) return cached.value;
  const value = { ...result, toolCall: call };
  toolCallResultCache.set(result, { call, value });
  return value;
}

type ToolJobIndex = {
  length: number;
  last: ChatMessages[number] | undefined;
  resultByMessageIndex: ReadonlyMap<number, NativeToolJobResult>;
  attachedMessageIndices: ReadonlySet<number>;
  byAssistantIndex: ReadonlyMap<number, ToolJobResultsByToolId>;
};

const toolJobIndexCache = new WeakMap<ChatMessages, ToolJobIndex>();

function sameToolName(left: string | undefined, right: string | undefined) {
  return normalizeToolName(left) === normalizeToolName(right);
}

function toolJobIndexFor(messages: ChatMessages): ToolJobIndex {
  const cached = toolJobIndexCache.get(messages);
  if (
    cached?.length === messages.length &&
    cached.last === messages[messages.length - 1]
  )
    return cached;
  const calls = new Map<string, { call: ToolCall; assistantIndex: number }>();
  const receipts = new Map<string, ToolJobReceipt>();
  const resultByMessageIndex = new Map<number, NativeToolJobResult>();
  const attachedMessageIndices = new Set<number>();
  const byAssistantIndex = new Map<
    number,
    Record<string, NativeToolJobResult[] | undefined>
  >();
  for (let i = 0; i < messages.length; i++) {
    const message = messages[i];
    if (isAssistantMessage(message)) {
      if (assistantDisplayClass(message) !== "normal") continue;
      for (const call of message.tool_calls ?? []) {
        if (call.id) {
          calls.set(call.id, { call, assistantIndex: i });
          receipts.delete(call.id);
        }
      }
      continue;
    }
    const base = nativeToolJobResult(message);
    if (base) {
      const callId =
        ("tool_call_id" in base.message ? base.message.tool_call_id : "") ?? "";
      const owner = callId ? calls.get(callId) : undefined;
      const matched =
        owner && sameToolName(owner.call.function.name, base.toolName)
          ? owner
          : undefined;
      if (!matched) {
        resultByMessageIndex.set(i, base);
        continue;
      }
      const result = withToolCall(base, matched.call);
      resultByMessageIndex.set(i, result);
      const receipt = receipts.get(callId);
      if (
        receipt &&
        base.jobId === receipt.job_id &&
        sameToolName(receipt.tool_name, base.toolName)
      ) {
        attachedMessageIndices.add(i);
        const byToolId =
          byAssistantIndex.get(matched.assistantIndex) ??
          (Object.create(null) as Record<
            string,
            NativeToolJobResult[] | undefined
          >);
        byAssistantIndex.set(matched.assistantIndex, byToolId);
        (byToolId[callId] ??= []).push(result);
      }
      continue;
    }
    if (isToolMessage(message) && message.tool_call_id) {
      const receipt = parseToolJobReceipt(message.extra?.tool_job);
      if (
        receipt?.status === "accepted" &&
        receipt.tool_call_id === message.tool_call_id
      ) {
        receipts.set(message.tool_call_id, receipt);
      }
    }
  }
  const index: ToolJobIndex = {
    length: messages.length,
    last: messages[messages.length - 1],
    resultByMessageIndex,
    attachedMessageIndices,
    byAssistantIndex,
  };
  toolJobIndexCache.set(messages, index);
  return index;
}

function sameToolJobResults(
  left: ToolJobResultsByToolId,
  right: ToolJobResultsByToolId,
): boolean {
  if (left === right) return true;
  const leftKeys = Object.keys(left);
  if (leftKeys.length !== Object.keys(right).length) return false;
  return leftKeys.every((key) => {
    const a = left[key];
    const b = right[key] as readonly NativeToolJobResult[] | undefined;
    return (
      b !== undefined &&
      a !== undefined &&
      a.length === b.length &&
      a.every((value, index) => value === b[index])
    );
  });
}

function withToolJobAttachments(
  items: DisplayItem[],
  messages: ChatMessages,
): DisplayItem[] {
  const index = toolJobIndexFor(messages);
  let patched: DisplayItem[] | null = null;
  for (let k = 0; k < items.length; k++) {
    const item = items[k];
    if (item.type === "assistant") {
      const next =
        index.byAssistantIndex.get(item.index) ?? EMPTY_TOOL_JOB_RESULTS;
      if (sameToolJobResults(item.jobResultsByToolId, next)) continue;
      patched ??= [...items];
      patched[k] = { ...item, jobResultsByToolId: next };
    } else if (item.type === "tool_job_result") {
      const next = index.resultByMessageIndex.get(item.messageIndex);
      if (!next || next === item.result) continue;
      patched ??= [...items];
      patched[k] = { ...item, result: next };
    }
  }
  return patched ?? items;
}

type DisplayItemToolJobResult = {
  type: "tool_job_result";
  key: string;
  messageIndex: number;
  result: NativeToolJobResult;
};

function getMessageKey(message: ChatMessages[number], index: number): string {
  if (message.message_id) return message.message_id;
  if ("tool_call_id" in message && message.tool_call_id) {
    return `${message.role}-${message.tool_call_id}-${index}`;
  }
  return `${message.role}-${index}`;
}

// Keep this set in sync with engine summarization is_read_like_tool_name;
// engine tests guard the shared matrix.
const READ_TOOLS = new Set([
  "cat",
  "tree",
  "glob",
  "search_pattern",
  "search_semantic",
  "search_symbol_definition",
  "web",
  "web_search",
  "knowledge",
  "search_trajectories",
  "get_trajectory_context",
  // CC OAuth mode aliases
  "t_cat",
  "t_tree",
  "t_glob",
  "t_search_pattern",
  "t_search_semantic",
  "t_search_symbol_definition",
  "t_web",
  "t_web_search",
  "t_knowledge",
  "t_hist_search",
  "t_hist_get",
]);

const EDIT_TOOLS = new Set([
  "create_textdoc",
  "update_textdoc",
  "replace_textdoc",
  "update_textdoc_regex",
  "update_textdoc_by_lines",
  "update_textdoc_anchored",
  "apply_patch",
  "undo_textdoc",
  "rm",
  // CC OAuth mode aliases
  "t_write",
  "t_patch",
  "t_patch_re",
  "t_patch_ln",
  "t_patch_at",
  "t_undo",
  "t_rm",
]);

type DisplayItemAssistant = {
  type: "assistant";
  key: string;
  index: number;
  messageIndex: number;
  message: AssistantMessage;
  contextFilesByToolId: Record<string, ChatContextFile[]>;
  diffsByToolId: Record<string, DiffChunk[]>;
  jobResultsByToolId: ToolJobResultsByToolId;
  isStreaming: boolean;
};

type DisplayItemUser = {
  type: "user";
  key: string;
  index: number;
  messageIndex: number;
  message: UserMessage;
  isLastUser: boolean;
};

type DisplayItemContextFiles = {
  type: "context_files";
  key: string;
  messageIndex: number;
  files: ChatContextFile[];
  toolCallId?: string;
  rawExtra?: unknown;
};

type DisplayItemDiffGroup = {
  type: "diff_group";
  key: string;
  messageIndex: number;
  diffs: DiffMessage[];
};

type DisplayItemSystem = {
  type: "system";
  key: string;
  messageIndex: number;
  content: string;
};

type DisplayItemPlainText = {
  type: "plain_text";
  key: string;
  messageIndex: number;
  content: string;
};

type DisplayItemError = {
  type: "error";
  key: string;
  messageIndex: number;
  errors: ErrorMessage[];
};

type DisplayItemSkillActivated = {
  type: "skill_activated";
  key: string;
  messageIndex: number;
  name: string;
  body: string;
  allowedTools: string[];
  modelOverride: string | null;
};

type DisplayItemSkillReport = {
  type: "skill_report";
  key: string;
  messageIndex: number;
  skillName: string;
  report: string;
};

type DisplayItemSummarization = {
  type: "summarization";
  key: string;
  messageIndex: number;
  message: SummarizationMessage;
};

// Rails connect adjacent rendered events, including across hidden messages.
export type EventRunPosition = "single" | "start" | "middle" | "end";

type DisplayItemEvent = {
  type: "event";
  key: string;
  messageIndex: number;
  event: EventMessage;
  run: EventRunPosition;
};

function syntheticCompressionFailureError(
  message: ChatMessages[number],
): ErrorMessage {
  return {
    role: "error",
    content: typeof message.content === "string" ? message.content : "",
    message_id: message.message_id,
    extra: message.extra,
  };
}

function buildCompressionDisplayItem(
  message: ChatMessages[number],
  index: number,
  pairedSummaryContent?: string,
): DisplayItemError | DisplayItemSummarization | null {
  if (isVisibleCompressionFailureEvent(message)) {
    return {
      type: "error",
      key: getMessageKey(message, index),
      messageIndex: index,
      errors: [syntheticCompressionFailureError(message)],
    };
  }

  if (isCompressedAssistantMessage(message)) {
    return {
      type: "summarization",
      key: getMessageKey(message, index),
      messageIndex: index,
      message: syntheticSummarizationMessage(message),
    };
  }

  if (isCompressionReportMessage(message)) {
    const syntheticMessage = syntheticCompressionReportMessage(message);
    const trimmedSummary = pairedSummaryContent?.trim();
    return {
      type: "summarization",
      key: getMessageKey(message, index),
      messageIndex: index,
      message: trimmedSummary
        ? {
            ...syntheticMessage,
            paired_summary_content: trimmedSummary,
          }
        : syntheticMessage,
    };
  }

  return null;
}

// Adjacent matching report+summary pairs render as one report card: source-preserving
// hides only the internal model summary, and legacy replacement-style reports keep
// their historical collapse behavior.
function isCompressedAssistantPairedWithReport(
  messages: ChatMessages,
  index: number,
): boolean {
  const message = messages[index];
  return (
    isCompressedAssistantMessage(message) &&
    index > 0 &&
    isMatchingLlmSegmentCompressionReport(messages[index - 1], message)
  );
}

// The engine inserts the report immediately followed by its summary, so
// pairing only inspects the adjacent next message; a non-adjacent summary
// renders as its own card and the report simply omits the model summary.
function pairedSummaryContentForReport(
  messages: ChatMessages,
  index: number,
): string | undefined {
  const nextMessage = messages.at(index + 1);
  const message = messages[index];
  if (!nextMessage || !isAssistantMessage(nextMessage)) return undefined;
  if (!isMatchingLlmSegmentCompressionReport(message, nextMessage)) {
    return undefined;
  }
  const content =
    typeof nextMessage.content === "string" ? nextMessage.content : "";
  const trimmedContent = content.trim();
  return trimmedContent.length > 0 ? trimmedContent : undefined;
}

type CompressionIdentityMetadata = {
  source_hash?: unknown;
  source_message_ids?: unknown;
};

function nonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.length > 0;
}

function nonEmptyStringArray(value: unknown): string[] | null {
  if (!Array.isArray(value) || value.length === 0) return null;
  return value.every(nonEmptyString) ? value : null;
}

function compressionIdentityMatches(
  reportMetadata: CompressionIdentityMetadata,
  assistantMetadata: CompressionIdentityMetadata,
): boolean {
  if (
    nonEmptyString(reportMetadata.source_hash) &&
    nonEmptyString(assistantMetadata.source_hash)
  ) {
    return reportMetadata.source_hash === assistantMetadata.source_hash;
  }

  const reportSourceIds = nonEmptyStringArray(
    reportMetadata.source_message_ids,
  );
  const assistantSourceIds = nonEmptyStringArray(
    assistantMetadata.source_message_ids,
  );
  if (!reportSourceIds || !assistantSourceIds) return false;
  if (reportSourceIds.length !== assistantSourceIds.length) return false;
  return reportSourceIds.every((id, index) => id === assistantSourceIds[index]);
}

function isMatchingLlmSegmentCompressionReport(
  reportMessage: ChatMessages[number],
  assistantMessage: AssistantMessage,
): boolean {
  if (!isCompressionReportMessage(reportMessage)) return false;
  const reportMetadata = getCompressionReportMetadata(reportMessage);
  if (reportMetadata?.compression_kind !== "llm_segment_summary") {
    return false;
  }

  const assistantMetadata = getAssistantCompressionMetadata(assistantMessage);
  if (assistantMetadata?.kind !== "llm_segment_summary") return false;
  return compressionIdentityMatches(reportMetadata, assistantMetadata);
}

type AssistantDisplayClass =
  | "compressed"
  | "hidden_activate_skill_only"
  | "empty"
  | "normal";

function assistantHasVisibleContent(message: AssistantMessage): boolean {
  return Boolean(message.content && String(message.content).trim());
}

function hasItems(value: readonly unknown[] | null | undefined): boolean {
  return Array.isArray(value) && value.length > 0;
}

function hasThinkingText(message: AssistantMessage): boolean {
  return (message.thinking_blocks ?? []).some(
    (block) =>
      Boolean(block.thinking?.trim()) ||
      (block.summary ?? []).some(
        (entry) =>
          entry?.type === "summary_text" &&
          typeof entry.text === "string" &&
          entry.text.trim().length > 0,
      ),
  );
}

function isEmptyAssistant(message: AssistantMessage): boolean {
  return (
    !assistantHasVisibleContent(message) &&
    !message.reasoning_content?.trim() &&
    !hasThinkingText(message) &&
    !hasItems(message.tool_calls) &&
    !hasItems(message.server_executed_tools) &&
    !hasItems(message.server_content_blocks) &&
    !hasItems(message.citations)
  );
}

function isHiddenActivateSkillOnlyAssistant(
  message: AssistantMessage,
): boolean {
  const toolCalls = message.tool_calls ?? [];
  return (
    toolCalls.length > 0 &&
    toolCalls.every((tc) => isToolName(tc.function.name, "activate_skill")) &&
    !assistantHasVisibleContent(message)
  );
}

function assistantDisplayClass(
  message: ChatMessages[number],
): AssistantDisplayClass | null {
  if (!isAssistantMessage(message)) return null;
  if (isCompressedAssistantMessage(message)) return "compressed";
  if (isHiddenActivateSkillOnlyAssistant(message)) {
    return "hidden_activate_skill_only";
  }
  if (isEmptyAssistant(message)) return "empty";
  return "normal";
}

function isHiddenDisplayMessage(message: ChatMessages[number]): boolean {
  return (
    message.role === "plan" ||
    (isEventMessage(message) &&
      normalizeEventMessageMetadata(message).source === "review.comment")
  );
}

function isBlankText(value: unknown): boolean {
  return typeof value !== "string" || value.trim().length === 0;
}

function hasEventPayload(event: EventMessage): boolean {
  const payload = event.payload;
  if (payload === undefined || payload === null) return false;
  if (typeof payload === "string") return payload.trim().length > 0;
  if (Array.isArray(payload)) return payload.length > 0;
  if (typeof payload === "object") return Object.keys(payload).length > 0;
  return true;
}

function isEmptyEvent(event: EventMessage): boolean {
  return (
    isBlankText(eventContentText(event.content)) &&
    eventContentAttachments(event.content).length === 0 &&
    !hasEventPayload(event)
  );
}

function contextFilesRenderNothing(
  files: ChatContextFile[],
  toolCallId: string | undefined,
  rawExtra: unknown,
): boolean {
  if (toolCallId === BUDDY_PULSE_TOOL_CALL_ID) {
    const extra =
      typeof rawExtra === "object" && rawExtra !== null
        ? (rawExtra as Record<string, unknown>)
        : null;
    return !isBuddyPulsePayload(extra?.buddy_pulse_payload);
  }
  return !Array.isArray(files) || files.length === 0;
}

function hasDiffChunks(diffs: DiffMessage[]): boolean {
  return diffs.some((diff) => hasItems(diff.content));
}

function isEmptyUserMessage(message: UserMessage): boolean {
  if (hasItems(message.checkpoints)) return false;
  if (typeof message.content === "string") return isBlankText(message.content);
  return !hasItems(message.content);
}

export type DisplayItem =
  | DisplayItemToolJobResult
  | DisplayItemAssistant
  | DisplayItemUser
  | DisplayItemContextFiles
  | DisplayItemDiffGroup
  | DisplayItemSystem
  | DisplayItemPlainText
  | DisplayItemError
  | DisplayItemSkillActivated
  | DisplayItemSkillReport
  | DisplayItemSummarization
  | DisplayItemEvent;

function runPositionFor(
  hasPrevious: boolean,
  hasNext: boolean,
): EventRunPosition {
  if (hasPrevious && hasNext) return "middle";
  if (hasPrevious) return "end";
  if (hasNext) return "start";
  return "single";
}

function withEventRunPositions(items: DisplayItem[]): DisplayItem[] {
  let patched: DisplayItem[] | null = null;

  for (let i = 0; i < items.length; i++) {
    const item = items[i];
    if (item.type !== "event") continue;
    const run = runPositionFor(
      items[i - 1]?.type === "event",
      items[i + 1]?.type === "event",
    );
    if (run === item.run) continue;
    patched ??= [...items];
    patched[i] = { ...item, run };
  }

  return patched ?? items;
}

function updateAssistantStreamingFlags(
  items: DisplayItem[],
  isStreaming: boolean,
  lastAssistantIdx: number,
): DisplayItem[] {
  let needsPatch = false;
  for (const item of items) {
    if (item.type !== "assistant") continue;
    const shouldStream = isStreaming && item.index === lastAssistantIdx;
    if (shouldStream !== item.isStreaming) {
      needsPatch = true;
      break;
    }
  }

  if (!needsPatch) return items;

  return items.map((item) => {
    if (item.type !== "assistant") return item;
    const shouldStream = isStreaming && item.index === lastAssistantIdx;
    return shouldStream === item.isStreaming
      ? item
      : { ...item, isStreaming: shouldStream };
  });
}

function findLastAssistantMessageIndex(messages: ChatMessages): number {
  for (let i = messages.length - 1; i >= 0; i--) {
    if (messages[i].role === "assistant") return i;
  }
  return -1;
}

function findRebuildStartIndex(messages: ChatMessages, index: number): number {
  if (isCompressedAssistantPairedWithReport(messages, index)) {
    return index - 1;
  }

  let rebuildStart = index;
  let sawSupplemental = false;

  for (let i = index - 1; i >= 0; i--) {
    const msg = messages[i];

    if (msg.role === "plan") {
      rebuildStart = i;
      continue;
    }

    if (
      isToolMessage(msg) ||
      isChatContextFileMessage(msg) ||
      isDiffMessage(msg)
    ) {
      sawSupplemental = true;
      rebuildStart = i;
      continue;
    }

    if (msg.role === "assistant") {
      return i;
    }

    return sawSupplemental ? rebuildStart : index;
  }

  return sawSupplemental ? rebuildStart : index;
}

function rebuildDisplayItemsFromStart(
  previousItems: DisplayItem[],
  nextMessages: ChatMessages,
  isStreaming: boolean,
  rebuildStart: number,
): DisplayItem[] {
  const hidden = computeHiddenQaMessageIndices(nextMessages);
  const tailItems = buildDisplayItemsFromIndex(
    nextMessages,
    isStreaming,
    hidden,
    rebuildStart,
  );
  const prefixItems = previousItems.filter(
    (item) => item.messageIndex < rebuildStart,
  );

  return withEventRunPositions(
    updateAssistantStreamingFlags(
      withToolJobAttachments([...prefixItems, ...tailItems], nextMessages),
      isStreaming,
      findLastAssistantMessageIndex(nextMessages),
    ),
  );
}

function assistantGroupingSignature(message: AssistantMessage): string {
  return (message.tool_calls ?? [])
    .map((toolCall) => {
      const name = normalizeToolName(toolCall.function.name) ?? "";
      const id = toolCall.id ?? "";
      const args =
        name === "shell_service" ? `:${toolCall.function.arguments}` : "";
      return `${id}:${name}${args}`;
    })
    .join("|");
}

function sameIndices(
  left: ReadonlySet<number>,
  right: ReadonlySet<number>,
): boolean {
  if (left === right) return true;
  if (left.size !== right.size) return false;
  for (const value of left) if (!right.has(value)) return false;
  return true;
}

function buildDisplayItemsFromIndex(
  messages: ChatMessages,
  isStreaming: boolean,
  hiddenQaIndices: Set<number>,
  startIndex: number,
): DisplayItem[] {
  const items: DisplayItem[] = [];
  if (startIndex >= messages.length) return items;
  const foldedEvents = foldedEventIndexFor(messages).foldedMessageIndices;
  const toolJobs = toolJobIndexFor(messages);

  let lastUserIdx = -1;
  let lastAssistantIdx = -1;
  for (let i = messages.length - 1; i >= 0; i--) {
    const msg = messages[i];
    if (msg.role === "user" && !hiddenQaIndices.has(i) && lastUserIdx === -1) {
      lastUserIdx = i;
    }
    if (msg.role === "assistant" && lastAssistantIdx === -1) {
      lastAssistantIdx = i;
    }
    if (lastUserIdx !== -1 && lastAssistantIdx !== -1) break;
  }

  for (let i = startIndex; i < messages.length; i++) {
    const head = messages[i];

    const jobResult = toolJobs.resultByMessageIndex.get(i);
    if (jobResult) {
      if (toolJobs.attachedMessageIndices.has(i)) continue;
      items.push({
        type: "tool_job_result",
        key: getMessageKey(head, i),
        messageIndex: i,
        result: jobResult,
      });
      continue;
    }

    if (isToolMessage(head)) continue;

    if (isCompressedAssistantPairedWithReport(messages, i)) continue;

    const compressionItem = buildCompressionDisplayItem(
      head,
      i,
      pairedSummaryContentForReport(messages, i),
    );
    if (compressionItem) {
      items.push(compressionItem);
      continue;
    }

    if (isHiddenDisplayMessage(head)) continue;

    if (isEventMessage(head)) {
      if (foldedEvents.has(i)) continue;
      const event = normalizeEventMessageMetadata(head);
      if (isEmptyEvent(event)) continue;
      items.push({
        type: "event",
        key: getMessageKey(head, i),
        messageIndex: i,
        event,
        run: "single",
      });
      continue;
    }

    if (isErrorMessage(head)) {
      const errors = [head];
      let j = i + 1;
      while (j < messages.length) {
        const candidate = messages[j];
        if (!isErrorMessage(candidate)) break;
        errors.push(candidate);
        j++;
      }
      items.push({
        type: "error",
        key: getMessageKey(head, i),
        messageIndex: i,
        errors,
      });
      i = j - 1;
      continue;
    }

    if (head.role === "plain_text") {
      if (
        typeof head.content === "string" &&
        isSkillReportContent(head.content)
      ) {
        const parsed = parseSkillReport(head.content);
        if (parsed) {
          items.push({
            type: "skill_report",
            key: getMessageKey(head, i),
            messageIndex: i,
            skillName: parsed.skillName,
            report: parsed.report,
          });
          continue;
        }
      }
      if (isBlankText(head.content)) continue;
      items.push({
        type: "plain_text",
        key: getMessageKey(head, i),
        messageIndex: i,
        content: head.content,
      });
      continue;
    }

    if (head.role === "assistant") {
      const toolCalls = "tool_calls" in head ? head.tool_calls ?? [] : [];
      const displayClass = assistantDisplayClass(head);
      if (
        displayClass === "hidden_activate_skill_only" ||
        displayClass === "empty"
      ) {
        continue;
      }

      const key = getMessageKey(head, i);
      const contextFilesAfter: DisplayItemContextFiles[] = [];
      const diffMessagesAfter: DiffMessage[] = [];
      let diffGroupStartIndex: number | null = null;
      const contextFilesByToolId: Partial<Record<string, ChatContextFile[]>> =
        {};
      const diffsByToolId: Partial<Record<string, DiffChunk[]>> = {};

      const eligibleToolCalls = toolCalls.filter(
        (tc) =>
          tc.id &&
          tc.function.name &&
          READ_TOOLS.has(normalizeToolName(tc.function.name) ?? ""),
      );
      const eligibleToolIds = new Set(
        eligibleToolCalls
          .map((tc) => tc.id)
          .filter((id): id is string => Boolean(id)),
      );
      const lastEligibleToolId =
        eligibleToolCalls.length > 0
          ? eligibleToolCalls[eligibleToolCalls.length - 1].id
          : null;

      const editToolCalls = toolCalls.filter(
        (tc) =>
          tc.id &&
          tc.function.name &&
          EDIT_TOOLS.has(normalizeToolName(tc.function.name) ?? ""),
      );
      const editToolIds = new Set(
        editToolCalls
          .map((tc) => tc.id)
          .filter((id): id is string => Boolean(id)),
      );

      let j = i + 1;
      while (j < messages.length) {
        const nextMsg = messages[j];

        if (nativeToolJobResult(nextMsg)) break;

        if (isToolMessage(nextMsg)) {
          j++;
          continue;
        }

        if (isChatContextFileMessage(nextMsg)) {
          if (
            nextMsg.tool_call_id === "knowledge_enrichment" ||
            nextMsg.tool_call_id === "project_context"
          ) {
            break;
          }

          let targetToolId: string | null = null;

          if (
            nextMsg.tool_call_id &&
            eligibleToolIds.has(nextMsg.tool_call_id)
          ) {
            targetToolId = nextMsg.tool_call_id;
          } else if (!nextMsg.tool_call_id && lastEligibleToolId) {
            targetToolId = lastEligibleToolId;
          }

          if (targetToolId) {
            const prev = contextFilesByToolId[targetToolId] ?? [];
            contextFilesByToolId[targetToolId] = [...prev, ...nextMsg.content];
          } else if (
            !contextFilesRenderNothing(
              nextMsg.content,
              nextMsg.tool_call_id,
              (nextMsg as { extra?: unknown }).extra,
            )
          ) {
            contextFilesAfter.push({
              type: "context_files",
              key: getMessageKey(nextMsg, j),
              messageIndex: j,
              files: nextMsg.content,
              toolCallId: nextMsg.tool_call_id,
              rawExtra: (nextMsg as { extra?: unknown }).extra,
            });
          }
          j++;
          continue;
        }

        if (isDiffMessage(nextMsg)) {
          if (nextMsg.tool_call_id && editToolIds.has(nextMsg.tool_call_id)) {
            const prevDiffs = diffsByToolId[nextMsg.tool_call_id] ?? [];
            diffsByToolId[nextMsg.tool_call_id] = [
              ...prevDiffs,
              ...nextMsg.content,
            ];
          } else {
            if (diffGroupStartIndex === null) diffGroupStartIndex = j;
            diffMessagesAfter.push(nextMsg);
          }
          j++;
          continue;
        }

        break;
      }

      items.push({
        type: "assistant",
        key,
        index: i,
        messageIndex: i,
        message: head,
        contextFilesByToolId: contextFilesByToolId as Record<
          string,
          ChatContextFile[]
        >,
        diffsByToolId: diffsByToolId as Record<string, DiffChunk[]>,
        jobResultsByToolId:
          toolJobs.byAssistantIndex.get(i) ?? EMPTY_TOOL_JOB_RESULTS,
        isStreaming: isStreaming && i === lastAssistantIdx,
      });

      for (const ctxItem of contextFilesAfter) {
        items.push(ctxItem);
      }

      if (hasDiffChunks(diffMessagesAfter)) {
        items.push({
          type: "diff_group",
          key: `diffs-${key}`,
          messageIndex: diffGroupStartIndex ?? i,
          diffs: diffMessagesAfter,
        });
      }

      i = j - 1;
      continue;
    }

    if (head.role === "user") {
      if (hiddenQaIndices.has(i) || isEmptyUserMessage(head)) {
        continue;
      }

      items.push({
        type: "user",
        key: getMessageKey(head, i),
        index: i,
        messageIndex: i,
        message: head,
        isLastUser: i === lastUserIdx,
      });
      continue;
    }

    if (head.role === "cd_instruction" && typeof head.content === "string") {
      const parsed = tryParseSkillActivated(head.content);
      if (parsed) {
        items.push({
          type: "skill_activated",
          key: getMessageKey(head, i),
          messageIndex: i,
          ...parsed,
        });
      }
      continue;
    }

    if (isSummarizationMessage(head)) {
      items.push({
        type: "summarization",
        key: getMessageKey(head, i),
        messageIndex: i,
        message: head,
      });
      continue;
    }

    if (isChatContextFileMessage(head)) {
      const rawExtra = (head as { extra?: unknown }).extra;
      if (
        contextFilesRenderNothing(head.content, head.tool_call_id, rawExtra)
      ) {
        continue;
      }
      items.push({
        type: "context_files",
        key: getMessageKey(head, i),
        messageIndex: i,
        files: head.content,
        toolCallId: head.tool_call_id,
        rawExtra,
      });
      continue;
    }

    if (isSystemMessage(head)) {
      if (isBlankText(head.content)) continue;
      items.push({
        type: "system",
        key: getMessageKey(head, i),
        messageIndex: i,
        content: head.content,
      });
      continue;
    }

    if (isDiffMessage(head)) {
      const key = getMessageKey(head, i);
      const diffs: DiffMessage[] = [head];
      let j = i + 1;
      while (j < messages.length) {
        const m = messages[j];
        if (isToolMessage(m)) {
          j++;
          continue;
        }
        if (isDiffMessage(m)) {
          diffs.push(m);
          j++;
          continue;
        }
        break;
      }

      if (!hasDiffChunks(diffs)) {
        i = j - 1;
        continue;
      }
      items.push({
        type: "diff_group",
        key: `diffs-${key}`,
        messageIndex: i,
        diffs,
      });
      i = j - 1;
      continue;
    }
  }

  return items;
}

function patchTailDisplayItems(
  previousMessages: ChatMessages,
  nextMessages: ChatMessages,
  previousItems: DisplayItem[],
  isStreaming: boolean,
): DisplayItem[] | null {
  if (previousMessages.length >= nextMessages.length) return null;

  const sharedLen = previousMessages.length;
  for (let i = 0; i < sharedLen; i++) {
    if (previousMessages[i] !== nextMessages[i]) return null;
  }

  const rebuildStart = findRebuildStartIndex(nextMessages, sharedLen);
  const nextItems = rebuildDisplayItemsFromStart(
    previousItems,
    nextMessages,
    isStreaming,
    rebuildStart,
  );

  return nextItems;
}

function findReportAndSummaryReplacementIndex(
  previousMessages: ChatMessages,
  nextMessages: ChatMessages,
): number | null {
  if (nextMessages.length !== previousMessages.length + 1) return null;

  const changedIndex = previousMessages.findIndex(
    (message, index) => message !== nextMessages[index],
  );
  if (changedIndex === -1) return null;

  if (!isAssistantMessage(previousMessages[changedIndex])) return null;
  if (!isCompressionReportMessage(nextMessages[changedIndex])) return null;
  if (!isCompressedAssistantMessage(nextMessages[changedIndex + 1])) {
    return null;
  }

  for (let i = changedIndex + 1; i < previousMessages.length; i++) {
    if (previousMessages[i] !== nextMessages[i + 1]) return null;
  }

  return changedIndex;
}

function findReportAndSummaryInsertionIndex(
  previousMessages: ChatMessages,
  nextMessages: ChatMessages,
): number | null {
  if (nextMessages.length !== previousMessages.length + 2) return null;

  const changedIndex = previousMessages.findIndex(
    (message, index) => message !== nextMessages[index],
  );
  if (changedIndex === -1) return null;

  if (!isCompressionReportMessage(nextMessages[changedIndex])) return null;
  if (!isCompressedAssistantMessage(nextMessages[changedIndex + 1])) {
    return null;
  }

  for (let i = changedIndex; i < previousMessages.length; i++) {
    if (previousMessages[i] !== nextMessages[i + 2]) return null;
  }

  return changedIndex;
}

function tryParseSkillActivated(
  content: string,
): Omit<DisplayItemSkillActivated, "type" | "key" | "messageIndex"> | null {
  const prefix = "💿 SKILL_ACTIVATED ";
  const firstNewline = content.indexOf("\n");
  const headerLine =
    firstNewline === -1 ? content : content.slice(0, firstNewline);
  if (!headerLine.startsWith(prefix)) return null;
  try {
    const meta = JSON.parse(headerLine.slice(prefix.length)) as {
      name?: string;
      allowed_tools?: string[];
      model_override?: string | null;
    };
    const body =
      firstNewline === -1 ? "" : content.slice(firstNewline + 1).trimStart();
    return {
      name: meta.name ?? "",
      body,
      allowedTools: meta.allowed_tools ?? [],
      modelOverride: meta.model_override ?? null,
    };
  } catch {
    return null;
  }
}

export function buildDisplayItems(
  messages: ChatMessages,
  isStreaming: boolean,
): DisplayItem[] {
  if (messages.length === 0) return [];
  return withEventRunPositions(
    buildDisplayItemsFromIndex(
      messages,
      isStreaming,
      computeHiddenQaMessageIndices(messages),
      0,
    ),
  );
}

export function tryIncrementalDisplayItemsUpdate(
  previousMessages: ChatMessages | null,
  nextMessages: ChatMessages,
  previousItems: DisplayItem[] | null,
  isStreaming: boolean,
): DisplayItem[] | null {
  if (!previousMessages || !previousItems) return null;
  if (
    !sameIndices(
      foldedEventIndexFor(previousMessages).foldedMessageIndices,
      foldedEventIndexFor(nextMessages).foldedMessageIndices,
    )
  ) {
    return null;
  }
  if (previousMessages.length !== nextMessages.length) {
    const replacementIndex = findReportAndSummaryReplacementIndex(
      previousMessages,
      nextMessages,
    );
    if (replacementIndex !== null) {
      return rebuildDisplayItemsFromStart(
        previousItems,
        nextMessages,
        isStreaming,
        replacementIndex,
      );
    }

    const insertionIndex = findReportAndSummaryInsertionIndex(
      previousMessages,
      nextMessages,
    );
    if (insertionIndex !== null) {
      return rebuildDisplayItemsFromStart(
        previousItems,
        nextMessages,
        isStreaming,
        insertionIndex,
      );
    }

    return patchTailDisplayItems(
      previousMessages,
      nextMessages,
      previousItems,
      isStreaming,
    );
  }

  let changedIndex = -1;
  for (let i = 0; i < nextMessages.length; i++) {
    if (previousMessages[i] !== nextMessages[i]) {
      if (changedIndex !== -1) return null;
      changedIndex = i;
    }
  }

  let lastAssistantIdx = -1;
  for (let i = nextMessages.length - 1; i >= 0; i--) {
    if (nextMessages[i].role === "assistant") {
      lastAssistantIdx = i;
      break;
    }
  }

  if (changedIndex === -1) {
    return updateAssistantStreamingFlags(
      previousItems,
      isStreaming,
      lastAssistantIdx,
    );
  }

  const changedMessage = nextMessages[changedIndex];
  if (changedMessage.role !== "assistant") return null;
  if (!isAssistantMessage(changedMessage)) return null;

  const previousMessage = previousMessages[changedIndex];
  const previousDisplayClass = assistantDisplayClass(previousMessage);
  const changedDisplayClass = assistantDisplayClass(changedMessage);
  if (previousDisplayClass !== changedDisplayClass) {
    return rebuildDisplayItemsFromStart(
      previousItems,
      nextMessages,
      isStreaming,
      changedIndex,
    );
  }

  if (changedDisplayClass === "compressed") {
    return rebuildDisplayItemsFromStart(
      previousItems,
      nextMessages,
      isStreaming,
      isCompressedAssistantPairedWithReport(nextMessages, changedIndex)
        ? changedIndex - 1
        : changedIndex,
    );
  }

  if (
    !isAssistantMessage(previousMessage) ||
    assistantGroupingSignature(previousMessage) !==
      assistantGroupingSignature(changedMessage)
  ) {
    return rebuildDisplayItemsFromStart(
      previousItems,
      nextMessages,
      isStreaming,
      changedIndex,
    );
  }

  const nextItems = previousItems.map((item) => {
    if (item.type !== "assistant") return item;
    if (item.index !== changedIndex) {
      const shouldStream = isStreaming && item.index === lastAssistantIdx;
      return shouldStream === item.isStreaming
        ? item
        : {
            ...item,
            isStreaming: shouldStream,
          };
    }

    return {
      ...item,
      message: changedMessage,
      isStreaming: isStreaming && item.index === lastAssistantIdx,
    };
  });

  return withToolJobAttachments(nextItems, nextMessages);
}

function extractUserMessageText(content: UserMessage["content"]): string {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  for (const item of content) {
    if ("type" in item && item.type === "text" && "text" in item) {
      return item.text;
    }
    if ("m_type" in item && item.m_type === "text" && "m_content" in item) {
      return String(item.m_content);
    }
  }
  return "";
}

function isAskQuestionsAnswerText(
  contentStr: string,
  toolCallId: string,
): boolean {
  if (contentStr.startsWith(`[QA:${toolCallId}]`)) return true;
  const trimmed = contentStr.trim();
  if (!trimmed.startsWith("{")) return false;
  try {
    const parsed: unknown = JSON.parse(trimmed);
    if (typeof parsed !== "object" || parsed === null) return false;
    const record = parsed as Record<string, unknown>;
    return (
      record.type === "ask_questions_answer" &&
      record.tool_call_id === toolCallId
    );
  } catch {
    return false;
  }
}

function computeHiddenQaMessageIndices(messages: ChatMessages): Set<number> {
  const hiddenIndices = new Set<number>();
  const askQuestionsToolIds = new Map<string, number>();

  for (let i = 0; i < messages.length; i++) {
    const msg = messages[i];
    if (msg.role === "assistant" && "tool_calls" in msg && msg.tool_calls) {
      for (const tc of msg.tool_calls) {
        if (isToolName(tc.function.name, "ask_questions") && tc.id) {
          askQuestionsToolIds.set(tc.id, i);
        }
      }
    }
  }

  for (const [toolCallId, assistantIdx] of askQuestionsToolIds) {
    let foundToolResult = false;
    for (let j = assistantIdx + 1; j < messages.length; j++) {
      const msg = messages[j];
      if (isToolMessage(msg) && msg.tool_call_id === toolCallId) {
        foundToolResult = true;
        continue;
      }
      if (foundToolResult && msg.role === "user") {
        const contentStr = extractUserMessageText(msg.content);
        if (isAskQuestionsAnswerText(contentStr, toolCallId)) {
          hiddenIndices.add(j);
        }
        break;
      }
    }
  }

  return hiddenIndices;
}
