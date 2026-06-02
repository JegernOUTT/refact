import { LspChatMode } from "../../features/Chat";
import { Checkpoint } from "../../features/Checkpoints/types";
import { Usage } from "./chat";
import { MCPArgs, MCPEnvs } from "./integrations";

export type ChatRole =
  | "user"
  | "assistant"
  | "error"
  | "context_file"
  | "system"
  | "tool"
  | "diff"
  | "plain_text"
  | "cd_instruction"
  | "compression_report"
  | "summarization"
  | "event"
  | "plan";

export type ChatContextFile = {
  file_name: string;
  file_content: string;
  line1: number;
  line2: number;
  cursor?: number;
  usefulness?: number;
  usefullness?: number;
};

export type ToolCall = {
  function: {
    arguments: string;
    name?: string;
  };
  index: number;
  type?: "function";
  id?: string;
  attached_files?: string[];
  subchat?: string;
  subchat_log?: string[];
};

export type ToolUsage = {
  functionName: string;
  amountOfCalls: number;
};

export type BackgroundAgentKind = "subagent" | "delegate";

export type BackgroundAgentStatus =
  | "queued"
  | "running"
  | "waiting_for_approval"
  | "completed"
  | "failed"
  | "cancelled"
  | "interrupted";

export interface BackgroundAgentToolFields {
  background_agent_id?: string;
  background_agent_kind?: BackgroundAgentKind;
  child_chat_id?: string;
  background_agent_status?: string;
  target_files?: string[];
}

export interface BackgroundAgentSummary {
  agent_id: string;
  parent_chat_id: string;
  child_chat_id: string | null;
  kind: BackgroundAgentKind;
  status: BackgroundAgentStatus;
  title: string;
  progress: string | null;
  step_count: number;
  last_activity: string | null;
  target_files: string[];
  edited_files: string[];
  diff_summary: string | null;
  conflict_summary: string | null;
  result_summary: string | null;
  error: string | null;
  started_at: string | null;
  finished_at: string | null;
  change_seq: number;
}

function isToolCall(call: unknown): call is ToolCall {
  if (!call) return false;
  if (typeof call !== "object") return false;
  if (!("function" in call)) return false;
  if (!("index" in call)) return false;
  return true;
}

export const validateToolCall = (toolCall: ToolCall) => {
  if (!isToolCall(toolCall)) return false;
  try {
    JSON.parse(toolCall.function.arguments);
    return true;
  } catch {
    return false;
  }
};

type ToolContent = string | MultiModalToolContent[];

export function isToolContent(json: unknown): json is ToolContent {
  if (!json) return false;
  if (typeof json === "string") return true;
  if (Array.isArray(json)) return json.every(isMultiModalToolContent);
  return false;
}
export interface BaseToolResult extends BackgroundAgentToolFields {
  tool_call_id: string;
  finish_reason?: string;
  content: ToolContent;
  compression_strength?: CompressionStrength;
  tool_failed?: boolean;
  extra?: Record<string, unknown>;
}

export interface SingleModelToolResult extends BaseToolResult {
  content: string;
}
export interface MultiModalToolResult extends BaseToolResult {
  content: MultiModalToolContent[];
}

export type ToolResult = SingleModelToolResult | MultiModalToolResult;

export type ExecProcessStatus =
  | "starting"
  | "running"
  | "running_in_background"
  | "exited"
  | "failed"
  | "killed"
  | "timed_out";

export type ExecOutputChunkMetadata = {
  process_id?: string;
  seq?: number;
  stream?: string;
  text?: string;
  timestamp_ms?: number;
};

export type ExecTranscriptMetadata = {
  process_id?: string;
  found?: boolean;
  since_seq?: number;
  next_seq?: number;
  latest_seq?: number;
  chunks?: ExecOutputChunkMetadata[];
  total_bytes_appended?: number;
  total_lines_appended?: number;
  persisted_output_path?: string;
  dropped_chunks?: number;
  dropped_bytes?: number;
  truncated_chunks?: number;
  current_bytes?: number;
  max_bytes?: number;
  chunk_count?: number;
  is_truncated?: boolean;
};

export type ExecProcessMetadata = {
  process_id?: string;
  status?: ExecProcessStatus;
  status_detail?: unknown;
  mode?: string;
  service_name?: string | null;
  chat_id?: string | null;
  tool_call_id?: string | null;
  workspace?: string | null;
  command?: string;
  cwd?: string | null;
  short_description?: string;
  created_at?: number;
  created_at_ms?: number;
  started_at?: number;
  started_at_ms?: number;
  ended_at?: number | null;
  ended_at_ms?: number | null;
  duration_ms?: number;
  timeout_secs?: number;
  exit_code?: number | null;
  stream?: string;
  persisted_output_path?: string;
  bytes_written?: number;
  chunks_returned?: number;
  tty?: boolean;
  transcript?: ExecTranscriptMetadata;
};

export type ExecSingleProcessMetadata = ExecProcessMetadata & {
  process_id: string;
  status: ExecProcessStatus;
  processes?: never;
};

export type ExecProcessListMetadata = ExecProcessMetadata & {
  count?: number;
  status_filter?: string;
  scope_filter?: string;
  processes: ExecSingleProcessMetadata[];
};

export type ExecToolMetadata =
  | ExecSingleProcessMetadata
  | ExecProcessListMetadata;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function isExecProcessStatus(
  value: unknown,
): value is ExecProcessStatus {
  return (
    value === "starting" ||
    value === "running" ||
    value === "running_in_background" ||
    value === "exited" ||
    value === "failed" ||
    value === "killed" ||
    value === "timed_out"
  );
}

export function isExecToolMetadata(value: unknown): value is ExecToolMetadata {
  if (!isRecord(value)) return false;
  if (
    typeof value.process_id === "string" &&
    isExecProcessStatus(value.status)
  ) {
    return true;
  }
  if (Array.isArray(value.processes)) {
    return value.processes.every(
      (process) =>
        isRecord(process) &&
        typeof process.process_id === "string" &&
        isExecProcessStatus(process.status),
    );
  }
  return false;
}

export function extractExecMetadata(
  extra: Record<string, unknown> | undefined,
): ExecToolMetadata | undefined {
  const exec = extra?.exec;
  return isExecToolMetadata(exec) ? exec : undefined;
}

export type MultiModalToolContent = {
  m_type: string;
  m_content: string;
};

export function isMultiModalToolContent(
  content: unknown,
): content is MultiModalToolContent {
  if (!content) return false;
  if (typeof content !== "object") return false;
  if (!("m_type" in content)) return false;
  if (typeof content.m_type !== "string") return false;
  if (!("m_content" in content)) return false;
  if (typeof content.m_content !== "string") return false;
  return true;
}

export function isMultiModalToolContentArray(content: ToolContent) {
  if (!Array.isArray(content)) return false;
  return content.every(isMultiModalToolContent);
}

export function isMultiModalToolResult(
  toolResult: ToolResult,
): toolResult is MultiModalToolResult {
  return isMultiModalToolContentArray(toolResult.content);
}

export function isSingleModelToolResult(toolResult: ToolResult) {
  return typeof toolResult.content === "string";
}

interface BaseMessage {
  role: ChatRole;
  message_id?: string;
  content:
    | string
    | ChatContextFile[]
    | MultiModalToolContent[]
    | DiffChunk[]
    | null
    | (UserMessageContentWithImage | ProcessedUserMessageContentWithImages)[];
  extra?: Record<string, unknown>;
}

type MessageEnvelope = Pick<BaseMessage, "message_id" | "extra">;

export interface ChatContextFileMessage extends BaseMessage {
  role: "context_file";
  content: ChatContextFile[];
  tool_call_id?: string;
}

export type UserImage = {
  type: "image_url";
  image_url: { url: string };
};

export type UserMessageContentWithImage =
  | {
      type: "text";
      text: string;
    }
  | UserImage;
export interface UserMessage extends BaseMessage {
  role: "user";
  content:
    | string
    | (UserMessageContentWithImage | ProcessedUserMessageContentWithImages)[];
  checkpoints?: Checkpoint[];
  compression_strength?: CompressionStrength;
}

export type ProcessedUserMessageContentWithImages = {
  m_type: string;
  m_content: string;
};
export type WebSearchCitation = {
  type: "web_search_result_location";
  cited_text: string;
  url: string;
  title: string;
  encrypted_index?: string;
};

export interface AssistantMessage extends BaseMessage, CostInfo {
  role: "assistant";
  content: string | null;
  reasoning_content?: string | null;
  tool_calls?: ToolCall[] | null;
  server_executed_tools?: ToolCall[] | null;
  server_content_blocks?: unknown[] | null;
  thinking_blocks?: ThinkingBlock[] | null;
  citations?: WebSearchCitation[] | null;
  finish_reason?: "stop" | "length" | "abort" | "tool_calls" | "error" | null;
  usage?: Usage | null;
  summarized_token_estimate?: number;
  summarization_tier?: string;
  compression?: LlmSegmentSummaryCompressionMetadata;
  extra?: Record<string, unknown>;
}

export type UserErrorCategory =
  | "ProviderTransient"
  | "ProviderRateLimit"
  | "ContextTooLarge"
  | "AuthenticationFailed"
  | "ModelUnavailable"
  | "BillingQuota"
  | "InvalidRequest"
  | "NetworkFailure"
  | "StreamCorrupted"
  | "ToolSchemaInvalid"
  | "ContentPolicy"
  | "Unknown";

export type UserErrorInfo = {
  category: UserErrorCategory;
  title: string;
  explanation: string;
  suggested_action: string;
  is_retryable: boolean;
  raw_error?: string;
};

export type RetryStatus = {
  attempt: number;
  max_attempts: number;
  delay_secs: number;
  in_progress: boolean;
};

export interface ErrorMessage extends BaseMessage {
  role: "error";
  content: string;
  error_info?: UserErrorInfo;
  retry_status?: RetryStatus;
}

export interface ToolCallMessage extends AssistantMessage {
  tool_calls: ToolCall[];
}

export interface SystemMessage extends BaseMessage {
  role: "system";
  content: string;
}

export interface ToolMessage extends BaseMessage, BackgroundAgentToolFields {
  role: "tool";
  content: string | MultiModalToolContent[];
  tool_call_id: string;
  tool_failed?: boolean;
  compression_strength?: CompressionStrength;
  extra?: Record<string, unknown>;
}

export type DiffChunk = {
  file_name: string;
  file_action: string;
  line1: number;
  line2: number;
  lines_remove: string;
  lines_add: string;
  lines_before?: string | null;
  lines_after?: string | null;
  file_name_rename?: string | null;
  application_details?: string;
};

export function isDiffChunk(json: unknown) {
  if (!json) {
    return false;
  }
  if (typeof json !== "object") {
    return false;
  }
  if (!("file_name" in json) || typeof json.file_name !== "string") {
    return false;
  }
  if (!("file_action" in json) || typeof json.file_action !== "string") {
    return false;
  }
  if (!("line1" in json) || typeof json.line1 !== "number") {
    return false;
  }
  if (!("line2" in json) || typeof json.line2 !== "number") {
    return false;
  }
  if (!("lines_remove" in json) || typeof json.lines_remove !== "string") {
    return false;
  }
  if (!("lines_add" in json) || typeof json.lines_add !== "string") {
    return false;
  }
  return true;
}
export interface DiffMessage extends BaseMessage {
  role: "diff";
  content: DiffChunk[];
  tool_call_id: string;
}

export function isUserMessage(message: ChatMessage): message is UserMessage {
  return message.role === "user";
}

export interface PlainTextMessage extends BaseMessage {
  role: "plain_text";
  content: string;
}

export interface CDInstructionMessage extends BaseMessage {
  role: "cd_instruction";
  content: string;
}

export type SummarizationTier =
  | "tier0_deterministic"
  | "tier1_llm"
  | "tier1_merged"
  | "tier2_reactive";

export type LlmSegmentSummaryCompressionMetadata = Record<string, unknown> & {
  kind: "llm_segment_summary";
  schema_version?: number;
  source_hash?: string;
  source_message_ids?: string[];
  created_at?: string;
  summary_model?: string;
};

function isLlmSegmentSummaryCompressionMetadata(
  value: unknown,
): value is LlmSegmentSummaryCompressionMetadata {
  return isRecord(value) && value.kind === "llm_segment_summary";
}

export function getAssistantCompressionMetadata(message: {
  extra?: Record<string, unknown>;
  compression?: unknown;
}): LlmSegmentSummaryCompressionMetadata | null {
  const nested = message.extra?.compression;
  if (isLlmSegmentSummaryCompressionMetadata(nested)) return nested;
  if (isLlmSegmentSummaryCompressionMetadata(message.compression)) {
    return message.compression;
  }
  return null;
}

export interface SummarizationMessage extends BaseMessage {
  role: "summarization";
  content: string;
  summarized_range?: [number, number];
  summarization_tier?: SummarizationTier;
  summarized_token_estimate?: number;
  compression?: LlmSegmentSummaryCompressionMetadata;
  compression_report?: ChatCompressionReportMetadata;
}

export type ChatCompressionReportMetadata = {
  kind: "chat_compression_report";
  compression_kind?: string;
  source_message_count?: number;
  source_message_ids?: string[];
  source_hash?: string;
  summary_model?: string;
  context_files_removed?: number;
  context_messages_dropped?: number;
  tool_results_truncated?: number;
  tokens_before?: number;
  tokens_after?: number;
  estimated_tokens_saved?: number;
  reduction_percent?: number;
};

const CHAT_COMPRESSION_REPORT_NUMBER_FIELDS = [
  "source_message_count",
  "context_files_removed",
  "context_messages_dropped",
  "tool_results_truncated",
  "tokens_before",
  "tokens_after",
  "estimated_tokens_saved",
  "reduction_percent",
] as const satisfies readonly (keyof ChatCompressionReportMetadata)[];

const CHAT_COMPRESSION_REPORT_STRING_FIELDS = [
  "compression_kind",
  "source_hash",
  "summary_model",
] as const satisfies readonly (keyof ChatCompressionReportMetadata)[];

function hasOptionalNumberField(
  record: Record<string, unknown>,
  key: (typeof CHAT_COMPRESSION_REPORT_NUMBER_FIELDS)[number],
): boolean {
  const value = record[key];
  return (
    value === undefined || (typeof value === "number" && Number.isFinite(value))
  );
}

function hasOptionalStringField(
  record: Record<string, unknown>,
  key: (typeof CHAT_COMPRESSION_REPORT_STRING_FIELDS)[number],
): boolean {
  const value = record[key];
  return value === undefined || typeof value === "string";
}

function hasOptionalStringArrayField(
  record: Record<string, unknown>,
  key: "source_message_ids",
): boolean {
  const value = record[key];
  return (
    value === undefined ||
    (Array.isArray(value) && value.every((item) => typeof item === "string"))
  );
}

function isChatCompressionReportMetadata(
  value: unknown,
): value is ChatCompressionReportMetadata {
  return (
    isRecord(value) &&
    value.kind === "chat_compression_report" &&
    CHAT_COMPRESSION_REPORT_NUMBER_FIELDS.every((key) =>
      hasOptionalNumberField(value, key),
    ) &&
    CHAT_COMPRESSION_REPORT_STRING_FIELDS.every((key) =>
      hasOptionalStringField(value, key),
    ) &&
    hasOptionalStringArrayField(value, "source_message_ids")
  );
}

export function getCompressionReportMetadata(message: {
  extra?: Record<string, unknown>;
  compression_report?: unknown;
}): ChatCompressionReportMetadata | null {
  const nested = message.extra?.compression_report;
  if (isChatCompressionReportMetadata(nested)) return nested;
  if (isChatCompressionReportMetadata(message.compression_report)) {
    return message.compression_report;
  }
  return null;
}

export type CompressionReportExtra = Record<string, unknown> & {
  compression_report?: ChatCompressionReportMetadata;
};

export interface CompressionReportMessage extends BaseMessage {
  role: "compression_report";
  content: string;
  summarization_tier?: SummarizationTier;
  summarized_token_estimate?: number;
  compression_report?: ChatCompressionReportMetadata;
  extra?: CompressionReportExtra;
}

export type EventSubkind =
  | "mode_switch"
  | "tool_decision"
  | "ide_callback"
  | "process_completed"
  | "cron_fire"
  | "tick"
  | "summarization_marker"
  | "verifier_report"
  | "cancellation_note"
  | "plan_delta"
  | "system_notice";

export type EventMetadata = {
  subkind: EventSubkind;
  source: string;
  payload?: unknown;
};

export type EventMessage = MessageEnvelope & {
  role: "event";
  content: string;
  subkind: EventSubkind;
  source: string;
  payload?: unknown;
};

export function isEventSubkind(value: unknown): value is EventSubkind {
  return (
    value === "mode_switch" ||
    value === "tool_decision" ||
    value === "ide_callback" ||
    value === "process_completed" ||
    value === "cron_fire" ||
    value === "tick" ||
    value === "summarization_marker" ||
    value === "verifier_report" ||
    value === "cancellation_note" ||
    value === "plan_delta" ||
    value === "system_notice"
  );
}

export function getEventMetadata(message: EventMessage): EventMetadata | null {
  const backendEvent = isRecord(message.extra?.event)
    ? message.extra.event
    : null;
  const subkind = isEventSubkind(backendEvent?.subkind)
    ? backendEvent.subkind
    : isEventSubkind(message.subkind)
      ? message.subkind
      : null;
  if (!subkind) return null;

  const source =
    typeof backendEvent?.source === "string"
      ? backendEvent.source
      : typeof message.source === "string"
        ? message.source
        : "";
  const payload = backendEvent?.payload ?? message.payload;
  return payload === undefined
    ? { subkind, source }
    : { subkind, source, payload };
}

export function normalizeEventMessageMetadata(
  message: EventMessage,
): EventMessage {
  const metadata = getEventMetadata(message);
  if (!metadata) return message;
  if (
    message.subkind === metadata.subkind &&
    message.source === metadata.source &&
    message.payload === metadata.payload
  ) {
    return message;
  }
  return { ...message, ...metadata };
}

export type PlanMetadata = {
  mode?: string;
  version?: number;
  created_at_ms?: number;
  supersedes?: string | null;
};

export type PlanMessage = Omit<MessageEnvelope, "extra"> & {
  role: "plan";
  content: string;
  extra?: Record<string, unknown> & { plan?: unknown };
};

export function getPlanMetadata(message: PlanMessage): PlanMetadata {
  const rawPlan = message.extra?.plan;
  if (!isRecord(rawPlan)) return {};

  const metadata: PlanMetadata = {};
  if (typeof rawPlan.mode === "string" && rawPlan.mode.length > 0) {
    metadata.mode = rawPlan.mode;
  }
  if (typeof rawPlan.version === "number" && Number.isFinite(rawPlan.version)) {
    metadata.version = rawPlan.version;
  }
  if (
    typeof rawPlan.created_at_ms === "number" &&
    Number.isFinite(rawPlan.created_at_ms)
  ) {
    metadata.created_at_ms = rawPlan.created_at_ms;
  }
  if (typeof rawPlan.supersedes === "string" || rawPlan.supersedes === null) {
    metadata.supersedes = rawPlan.supersedes;
  }
  return metadata;
}

export function isSummarizationMessage(
  message: ChatMessage,
): message is SummarizationMessage {
  return message.role === "summarization";
}

export function isCompressionReportMessage(
  message: ChatMessage,
): message is CompressionReportMessage {
  return message.role === "compression_report";
}

export function isCompressedAssistantMessage(
  message: ChatMessage,
): message is AssistantMessage {
  return (
    message.role === "assistant" &&
    getAssistantCompressionMetadata(message) !== null
  );
}

type NormalizedExtraMetadata =
  | LlmSegmentSummaryCompressionMetadata
  | ChatCompressionReportMetadata;

function withNormalizedExtraMetadata(
  extra: Record<string, unknown> | undefined,
  key: "compression" | "compression_report",
  metadata: NormalizedExtraMetadata | null,
): Record<string, unknown> | undefined {
  if (metadata === null) return extra;
  return { ...(extra ?? {}), [key]: metadata };
}

export function syntheticSummarizationMessage(
  msg: AssistantMessage,
): SummarizationMessage {
  const compression = getAssistantCompressionMetadata(msg);
  return {
    role: "summarization",
    content: typeof msg.content === "string" ? msg.content : "",
    message_id: msg.message_id,
    summarization_tier: "tier1_llm",
    summarized_token_estimate:
      typeof msg.summarized_token_estimate === "number"
        ? msg.summarized_token_estimate
        : undefined,
    compression: compression ?? undefined,
    extra: withNormalizedExtraMetadata(msg.extra, "compression", compression),
  };
}

export function syntheticCompressionReportMessage(
  msg: CompressionReportMessage,
): SummarizationMessage {
  const compressionReport = getCompressionReportMetadata(msg);
  return {
    role: "summarization",
    content: msg.content,
    message_id: msg.message_id,
    summarization_tier: msg.summarization_tier ?? "tier2_reactive",
    summarized_token_estimate:
      typeof msg.summarized_token_estimate === "number"
        ? msg.summarized_token_estimate
        : undefined,
    compression_report: compressionReport ?? undefined,
    extra: withNormalizedExtraMetadata(
      msg.extra,
      "compression_report",
      compressionReport,
    ),
  };
}

export function isEventMessage(message: ChatMessage): message is EventMessage {
  return message.role === "event" && getEventMetadata(message) !== null;
}

export function isVisibleCompressionFailureEvent(
  message: ChatMessage,
): message is EventMessage {
  if (!isEventMessage(message)) return false;
  const metadata = getEventMetadata(message);
  return (
    metadata?.subkind === "system_notice" &&
    metadata.source === "chat.summarizer" &&
    message.content.startsWith("Context compression failed:")
  );
}

export function isPlanMessage(message: ChatMessage): message is PlanMessage {
  return message.role === "plan";
}

export type ChatMessage =
  | UserMessage
  | AssistantMessage
  | ErrorMessage
  | ChatContextFileMessage
  | SystemMessage
  | ToolMessage
  | DiffMessage
  | PlainTextMessage
  | CDInstructionMessage
  | CompressionReportMessage
  | SummarizationMessage
  | EventMessage
  | PlanMessage;

export type ChatMessages = ChatMessage[];

export type ChatMeta = {
  current_config_file?: string | undefined;
  chat_id?: string | undefined;
  request_attempt_id?: string | undefined;
  chat_mode: LspChatMode;
};

export function isChatContextFileMessage(
  message: ChatMessage,
): message is ChatContextFileMessage {
  return message.role === "context_file" && Array.isArray(message.content);
}

export function isAssistantMessage(
  message: ChatMessage,
): message is AssistantMessage {
  return message.role === "assistant";
}

export function isErrorMessage(message: ChatMessage): message is ErrorMessage {
  return message.role === "error";
}

export function isToolMessage(message: ChatMessage): message is ToolMessage {
  return message.role === "tool";
}

export function isDiffMessage(message: ChatMessage): message is DiffMessage {
  return message.role === "diff" && Array.isArray(message.content);
}

export function isSystemMessage(
  message: ChatMessage,
): message is SystemMessage {
  return message.role === "system";
}

export function isToolCallMessage(
  message: ChatMessage,
): message is ToolCallMessage {
  if (!isAssistantMessage(message)) return false;
  const tool_calls = message.tool_calls;
  if (!tool_calls) return false;
  return tool_calls.every(isToolCall);
}

export function isPlainTextMessage(
  message: ChatMessage,
): message is PlainTextMessage {
  return message.role === "plain_text";
}

export function isCDInstructionMessage(
  message: ChatMessage,
): message is CDInstructionMessage {
  return message.role === "cd_instruction";
}

interface BaseDelta {
  role?: ChatRole | null;
  provider_specific_fields?: {
    citation?: WebSearchCitation;
    thinking_blocks?: ThinkingBlock[];
  } | null;
}

interface AssistantDelta extends BaseDelta {
  role?: "assistant" | null;
  content?: string | null;
  reasoning_content?: string | null;
  tool_calls?: ToolCall[] | null;
  thinking_blocks?: ThinkingBlock[] | null;
}

export function isAssistantDelta(delta: unknown): delta is AssistantDelta {
  if (!delta) return false;
  if (typeof delta !== "object") return false;
  if ("role" in delta) {
    if (delta.role === null) return true;
    if (delta.role !== "assistant") return false;
  }
  if (!("content" in delta)) return false;
  if ("reasoning_content" in delta) {
    // reasoning_content is optional, but if present, must be a string
    if (
      delta.reasoning_content !== null &&
      typeof delta.reasoning_content !== "string"
    )
      return false;
  }
  if (typeof delta.content !== "string") return false;
  return true;
}
interface ChatContextFileDelta extends BaseDelta {
  role: "context_file";
  content: ChatContextFile[];
}

export function isChatContextFileDelta(
  delta: unknown,
): delta is ChatContextFileDelta {
  if (!delta) return false;
  if (typeof delta !== "object") return false;
  if (!("role" in delta)) return false;
  return delta.role === "context_file";
}

interface ToolCallDelta extends BaseDelta {
  tool_calls: ToolCall[];
}

export function isToolCallDelta(delta: unknown): delta is ToolCallDelta {
  if (!delta) return false;
  if (typeof delta !== "object") return false;
  if (!("tool_calls" in delta)) return false;
  if (delta.tool_calls === null) return false;
  return Array.isArray(delta.tool_calls);
}

export type ThinkingBlock = {
  type?: "thinking";
  thinking: null | string;
  signature: null | string;
};

interface ThinkingBlocksDelta extends BaseDelta {
  thinking_blocks?: ThinkingBlock[];
  reasoning_content?: string | null;
}

export function isThinkingBlocksDelta(
  delta: unknown,
): delta is ThinkingBlocksDelta {
  if (!delta) return false;
  if (typeof delta !== "object") return false;
  if ("reasoning_content" in delta) {
    // reasoning_content is optional, but if present, must be a string
    if (
      delta.reasoning_content !== null &&
      typeof delta.reasoning_content !== "string"
    )
      return false;
  }
  if ("thinking_blocks" in delta) {
    if (delta.thinking_blocks === null) return false;
    return Array.isArray(delta.thinking_blocks);
  }
  return false;
}

type Delta =
  | ThinkingBlocksDelta
  | AssistantDelta
  | ChatContextFileDelta
  | ToolCallDelta
  | BaseDelta;

export type ChatChoice = {
  delta: Delta;
  finish_reason?: "stop" | "length" | "abort" | "tool_calls" | "error" | null;
  index: number;
};

export type ChatUserMessageResponse =
  | {
      id: string;
      role: "user" | "context_file" | "context_memory";
      content: string;
      checkpoints?: Checkpoint[];
      compression_strength?: CompressionStrength;
    }
  | {
      id: string;
      role: "user";
      content:
        | string
        | (
            | UserMessageContentWithImage
            | ProcessedUserMessageContentWithImages
          )[];
      checkpoints?: Checkpoint[];
      compression_strength?: CompressionStrength;
    };

export type ToolResponse = {
  id: string;
  role: "tool";
  tool_failed?: boolean;
} & ToolResult;

export function isChatUserMessageResponse(
  json: unknown,
): json is ChatUserMessageResponse {
  if (!json) return false;
  if (typeof json !== "object") return false;
  if (!("id" in json)) return false;
  if (!("content" in json)) return false;
  if (!("role" in json)) return false;
  return (
    json.role === "user" ||
    json.role === "context_file" ||
    json.role === "context_memory"
  );
}

export type UserMessageResponse = ChatUserMessageResponse & {
  role: "user";
};

export function isUserResponse(json: unknown): json is UserMessageResponse {
  if (!isChatUserMessageResponse(json)) return false;
  return json.role === "user";
}

export type ContextFileResponse = ChatUserMessageResponse & {
  role: "context_file";
};

export function isContextFileResponse(
  json: unknown,
): json is ContextFileResponse {
  if (!isChatUserMessageResponse(json)) return false;
  return json.role === "context_file";
}

export type SubchatContextFileResponse = {
  content: string;
  role: "context_file";
};

export function isSubchatContextFileResponse(
  json: unknown,
): json is SubchatContextFileResponse {
  if (!json) return false;
  if (typeof json !== "object") return false;
  if (!("content" in json)) return false;
  if (!("role" in json)) return false;
  return json.role === "context_file";
}

export type ContextMemoryResponse = ChatUserMessageResponse & {
  role: "context_memory";
};

export function isContextMemoryResponse(
  json: unknown,
): json is ContextMemoryResponse {
  if (!isChatUserMessageResponse(json)) return false;
  return json.role === "context_memory";
}

export function isToolResponse(json: unknown): json is ToolResponse {
  if (!json) return false;
  if (typeof json !== "object") return false;
  if (!("content" in json)) return false;
  if (!("role" in json)) return false;
  if (!("tool_call_id" in json)) return false;
  if (!("tool_failed" in json)) return false;
  return json.role === "tool";
}

export type DiffResponse = {
  role: "diff";
  content: string;
  tool_call_id: string;
};

export function isDiffResponse(json: unknown): json is DiffResponse {
  if (!json) return false;
  if (typeof json !== "object") return false;
  if (!("content" in json)) return false;
  if (!("role" in json)) return false;
  return json.role === "diff";
}
export interface PlainTextResponse {
  role: "plain_text";
  content: string;
  tool_call_id: string;
  tool_calls?: ToolCall[];
}

export function isPlainTextResponse(json: unknown): json is PlainTextResponse {
  if (!json) return false;
  if (typeof json !== "object") return false;
  if (!("role" in json)) return false;
  return json.role === "plain_text";
}

export type SubchatResponse = {
  add_message: ChatResponse;
  subchat_id: string;
  tool_call_id: string;
};

export function isSubchatResponse(json: unknown): json is SubchatResponse {
  if (!json) return false;
  if (typeof json !== "object") return false;
  if (!("add_message" in json)) return false;
  if (!("subchat_id" in json)) return false;
  if (!("tool_call_id" in json)) return false;
  return true;
}

export function isSystemResponse(json: unknown): json is SystemMessage {
  if (!json) return false;
  if (typeof json !== "object") return false;
  if (!("role" in json)) return false;
  return json.role === "system";
}

export function isCDInstructionResponse(
  json: unknown,
): json is CDInstructionMessage {
  if (!json) return false;
  if (typeof json !== "object") return false;
  if (!("role" in json)) return false;
  return json.role === "cd_instruction";
}

import type { MeteringUsd } from "./chat";
export type { MeteringUsd };

type CostInfo = {
  metering_prompt_tokens_n?: number;
  metering_generated_tokens_n?: number;
  metering_cache_creation_tokens_n?: number;
  metering_cache_read_tokens_n?: number;

  metering_usd?: MeteringUsd;
};

type ChatResponseChoice = {
  choices: ChatChoice[];
  created: number;
  model: string;
  id?: string;
  usage?: Usage | null;
  refact_agent_request_available?: null | number;
  refact_agent_max_request_num?: number;
} & CostInfo;

export function isChatResponseChoice(
  res: ChatResponse,
): res is ChatResponseChoice {
  if (!("choices" in res)) return false;
  return true;
}

export type CompressionStrength = "absent" | "low" | "medium" | "high";
export type ChatResponse =
  | ChatResponseChoice
  | ChatUserMessageResponse
  | ToolResponse
  | PlainTextResponse;

export function areAllFieldsBoolean(
  json: unknown,
): json is Record<string, boolean> {
  return (
    typeof json === "object" &&
    json !== null &&
    Object.values(json).every((value) => typeof value === "boolean")
  );
}

export type VecDbMemoRecord = {
  memid: string;
  thevec?: number[];
  distance?: number;
  m_type: string;
  m_goal: string;
  m_project: string;
  m_payload: string;
  m_origin: string;
  mstat_correct: number;
  mstat_relevant: number;
  mstat_times_used: number;
};

export type KnowledgeMemoRecord = {
  memid: string;
  tags: string[];
  content: string;
  file_path?: string;
  line_range?: [number, number];
  title?: string;
  created?: string;
  kind?: string;
  score?: number;
};

export type MemoRecord = KnowledgeMemoRecord;

export function isMemoRecord(obj: unknown): obj is MemoRecord {
  if (!obj) return false;
  if (typeof obj !== "object") return false;
  if (!("memid" in obj) || typeof obj.memid !== "string") return false;
  return true;
}

export type KnowledgeGraphNode = {
  id: string;
  node_type: string;
  label: string;
  title?: string;
  content?: string;
  tags?: string[];
  created?: string;
  file_path?: string;
  kind?: string;
};

export type KnowledgeGraphEdge = {
  source: string;
  target: string;
  edge_type: string;
};

export type KnowledgeGraphStats = {
  doc_count: number;
  tag_count: number;
  file_count: number;
  entity_count: number;
  edge_count: number;
  active_docs: number;
  deprecated_docs: number;
  trajectory_count: number;
};

export type KnowledgeGraphResponse = {
  nodes: KnowledgeGraphNode[];
  edges: KnowledgeGraphEdge[];
  stats: KnowledgeGraphStats;
};

export type VecDbStatus = {
  files_unprocessed: number;
  files_total: number;
  requests_made_since_start: number;
  vectors_made_since_start: number;
  db_size: number;
  db_cache_size: number;
  state: "starting" | "parsing" | "done" | "cooldown";
  queue_additions: boolean;
  vecdb_max_files_hit: boolean;
  vecdb_errors: Record<string, number>;
};

export function isVecDbStatus(obj: unknown): obj is VecDbStatus {
  if (!obj) return false;
  if (typeof obj !== "object") return false;
  if (
    !("files_unprocessed" in obj) ||
    typeof obj.files_unprocessed !== "number"
  ) {
    return false;
  }
  if (!("files_total" in obj) || typeof obj.files_total !== "number") {
    return false;
  }
  if (
    !("requests_made_since_start" in obj) ||
    typeof obj.requests_made_since_start !== "number"
  ) {
    return false;
  }
  if (
    !("vectors_made_since_start" in obj) ||
    typeof obj.vectors_made_since_start !== "number"
  ) {
    return false;
  }
  if (!("db_size" in obj) || typeof obj.db_size !== "number") {
    return false;
  }
  if (!("db_cache_size" in obj) || typeof obj.db_cache_size !== "number") {
    return false;
  }

  if (!("state" in obj) || typeof obj.state !== "string") {
    return false;
  }
  if (!("queue_additions" in obj) || typeof obj.queue_additions !== "boolean") {
    return false;
  }
  if (
    !("vecdb_max_files_hit" in obj) ||
    typeof obj.vecdb_max_files_hit !== "boolean"
  ) {
    return false;
  }
  if (!("vecdb_errors" in obj) || typeof obj.vecdb_errors !== "object") {
    return false;
  }

  return true;
}
export function isMCPArgumentsArray(json: unknown): json is MCPArgs {
  if (!json) return false;
  if (typeof json !== "object") return false;
  if (!Array.isArray(json)) return false;
  if (!json.every((arg) => typeof arg === "string")) return false;
  return true;
}

export function isMCPEnvironmentsDict(json: unknown): json is MCPEnvs {
  if (!json) return false;
  if (typeof json !== "object") return false;
  if (Array.isArray(json)) return false;

  return Object.values(json).every((value) => typeof value === "string");
}

export function isDictionary(json: unknown): json is Record<string, string> {
  if (!json) return false;
  if (typeof json !== "object") return false;
  if (Array.isArray(json)) return false;

  return Object.values(json).every((value) => typeof value === "string");
}

export type SuccessResponse = { success: true };

export function isSuccess(data: unknown): data is SuccessResponse {
  return (
    typeof data === "object" &&
    data !== null &&
    "success" in data &&
    typeof data.success === "boolean" &&
    data.success
  );
}

export type BuddyPulsePreference = {
  statement: string;
  confidence: number;
  last_updated: string;
};

export type BuddyPulseLesson = {
  title: string;
  preview: string;
  tags: string[];
  updated: string;
};

export type BuddyPulseFriction = {
  top_error_types: { type: string; count: number }[];
  stuck_tasks: number;
};

export type BuddyPulseReport = {
  workflow_id: string;
  title: string;
  preview: string;
  chat_id: string;
};

export type BuddyPulseActivity = {
  grouped: { type: string; count: number; details?: string[] }[];
  time_of_day_pattern: string;
};

export type BuddyPulsePayload = {
  preferences: BuddyPulsePreference[];
  lessons: BuddyPulseLesson[];
  friction: BuddyPulseFriction;
  recent_reports: BuddyPulseReport[];
  user_activity: BuddyPulseActivity;
  generated_at: string;
};

export function isBuddyPulsePayload(
  value: unknown,
): value is BuddyPulsePayload {
  if (typeof value !== "object" || value === null) return false;
  const v = value as Record<string, unknown>;
  return (
    Array.isArray(v.preferences) &&
    Array.isArray(v.lessons) &&
    typeof v.friction === "object" &&
    Array.isArray(v.recent_reports) &&
    typeof v.user_activity === "object" &&
    typeof v.generated_at === "string"
  );
}
