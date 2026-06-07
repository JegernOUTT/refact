import { LspChatMode } from "../../features/Chat";
import { Checkpoint } from "../../features/Checkpoints/types";
import { Usage } from "./chat";
import { MCPArgs, MCPEnvs } from "./integrations";
export type ChatRole = "user" | "assistant" | "error" | "context_file" | "system" | "tool" | "diff" | "plain_text" | "cd_instruction" | "compression_report" | "summarization" | "event" | "plan";
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
export type BackgroundAgentStatus = "queued" | "running" | "waiting_for_approval" | "completed" | "failed" | "cancelled" | "interrupted";
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
export declare const validateToolCall: (toolCall: ToolCall) => boolean;
type ToolContent = string | MultiModalToolContent[];
export declare function isToolContent(json: unknown): json is ToolContent;
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
export type ExecProcessStatus = "starting" | "running" | "running_in_background" | "exited" | "failed" | "killed" | "timed_out";
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
export type ExecToolMetadata = ExecSingleProcessMetadata | ExecProcessListMetadata;
export declare function isExecProcessStatus(value: unknown): value is ExecProcessStatus;
export declare function isExecToolMetadata(value: unknown): value is ExecToolMetadata;
export declare function extractExecMetadata(extra: Record<string, unknown> | undefined): ExecToolMetadata | undefined;
export type MultiModalToolContent = {
    m_type: string;
    m_content: string;
};
export declare function isMultiModalToolContent(content: unknown): content is MultiModalToolContent;
export declare function isMultiModalToolContentArray(content: ToolContent): boolean;
export declare function isMultiModalToolResult(toolResult: ToolResult): toolResult is MultiModalToolResult;
export declare function isSingleModelToolResult(toolResult: ToolResult): boolean;
interface BaseMessage {
    role: ChatRole;
    message_id?: string;
    content: string | ChatContextFile[] | MultiModalToolContent[] | DiffChunk[] | null | (UserMessageContentWithImage | ProcessedUserMessageContentWithImages)[];
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
    image_url: {
        url: string;
    };
};
export type UserMessageContentWithImage = {
    type: "text";
    text: string;
} | UserImage;
export interface UserMessage extends BaseMessage {
    role: "user";
    content: string | (UserMessageContentWithImage | ProcessedUserMessageContentWithImages)[];
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
export type UserErrorCategory = "ProviderTransient" | "ProviderRateLimit" | "ContextTooLarge" | "AuthenticationFailed" | "ModelUnavailable" | "BillingQuota" | "InvalidRequest" | "NetworkFailure" | "StreamCorrupted" | "ToolSchemaInvalid" | "ContentPolicy" | "Unknown";
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
export declare function isDiffChunk(json: unknown): boolean;
export interface DiffMessage extends BaseMessage {
    role: "diff";
    content: DiffChunk[];
    tool_call_id: string;
}
export declare function isUserMessage(message: ChatMessage): message is UserMessage;
export interface PlainTextMessage extends BaseMessage {
    role: "plain_text";
    content: string;
}
export interface CDInstructionMessage extends BaseMessage {
    role: "cd_instruction";
    content: string;
}
export type SummarizationTier = "tier0_deterministic" | "tier1_llm" | "tier1_merged" | "tier2_reactive";
export type CompressionInsertMode = string;
export type LlmSegmentSummaryCompressionMetadata = Record<string, unknown> & {
    kind: "llm_segment_summary";
    schema_version?: number;
    insert_mode?: CompressionInsertMode;
    source_hash?: string;
    source_message_ids?: string[];
    summarized_source_message_ids?: string[];
    preserved_source_message_ids?: string[];
    created_at?: string;
    summary_model?: string;
};
export declare function getAssistantCompressionMetadata(message: {
    extra?: Record<string, unknown>;
    compression?: unknown;
}): LlmSegmentSummaryCompressionMetadata | null;
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
    insert_mode?: CompressionInsertMode;
    source_message_count?: number;
    source_message_ids?: string[];
    summarized_source_message_ids?: string[];
    preserved_source_message_ids?: string[];
    source_hash?: string;
    summary_model?: string;
    context_files_removed?: number;
    context_messages_dropped?: number;
    tool_results_truncated?: number;
    preserved_context_file_count?: number;
    compressed_tool_output_count?: number;
    tokens_before?: number;
    tokens_after?: number;
    estimated_tokens_saved?: number;
    reduction_percent?: number;
};
export declare function getCompressionReportMetadata(message: {
    extra?: Record<string, unknown>;
    compression_report?: unknown;
}): ChatCompressionReportMetadata | null;
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
export type EventSubkind = "mode_switch" | "tool_decision" | "ide_callback" | "process_completed" | "cron_fire" | "tick" | "summarization_marker" | "verifier_report" | "cancellation_note" | "plan_delta" | "system_notice";
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
export declare function isEventSubkind(value: unknown): value is EventSubkind;
export declare function getEventMetadata(message: EventMessage): EventMetadata | null;
export declare function normalizeEventMessageMetadata(message: EventMessage): EventMessage;
export type PlanMetadata = {
    mode?: string;
    version?: number;
    created_at_ms?: number;
    supersedes?: string | null;
};
export type PlanMessage = Omit<MessageEnvelope, "extra"> & {
    role: "plan";
    content: string;
    extra?: Record<string, unknown> & {
        plan?: unknown;
    };
};
export declare function getPlanMetadata(message: PlanMessage): PlanMetadata;
export declare function isSummarizationMessage(message: ChatMessage): message is SummarizationMessage;
export declare function isCompressionReportMessage(message: ChatMessage): message is CompressionReportMessage;
export declare function isCompressedAssistantMessage(message: ChatMessage): message is AssistantMessage;
export declare function syntheticSummarizationMessage(msg: AssistantMessage): SummarizationMessage;
export declare function syntheticCompressionReportMessage(msg: CompressionReportMessage): SummarizationMessage;
export declare function isEventMessage(message: ChatMessage): message is EventMessage;
export declare function isVisibleCompressionFailureEvent(message: ChatMessage): message is EventMessage;
export declare function isPlanMessage(message: ChatMessage): message is PlanMessage;
export type ChatMessage = UserMessage | AssistantMessage | ErrorMessage | ChatContextFileMessage | SystemMessage | ToolMessage | DiffMessage | PlainTextMessage | CDInstructionMessage | CompressionReportMessage | SummarizationMessage | EventMessage | PlanMessage;
export type ChatMessages = ChatMessage[];
export type ChatMeta = {
    current_config_file?: string | undefined;
    chat_id?: string | undefined;
    request_attempt_id?: string | undefined;
    chat_mode: LspChatMode;
};
export declare function isChatContextFileMessage(message: ChatMessage): message is ChatContextFileMessage;
export declare function isAssistantMessage(message: ChatMessage): message is AssistantMessage;
export declare function isErrorMessage(message: ChatMessage): message is ErrorMessage;
export declare function isToolMessage(message: ChatMessage): message is ToolMessage;
export declare function isDiffMessage(message: ChatMessage): message is DiffMessage;
export declare function isSystemMessage(message: ChatMessage): message is SystemMessage;
export declare function isToolCallMessage(message: ChatMessage): message is ToolCallMessage;
export declare function isPlainTextMessage(message: ChatMessage): message is PlainTextMessage;
export declare function isCDInstructionMessage(message: ChatMessage): message is CDInstructionMessage;
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
export declare function isAssistantDelta(delta: unknown): delta is AssistantDelta;
interface ChatContextFileDelta extends BaseDelta {
    role: "context_file";
    content: ChatContextFile[];
}
export declare function isChatContextFileDelta(delta: unknown): delta is ChatContextFileDelta;
interface ToolCallDelta extends BaseDelta {
    tool_calls: ToolCall[];
}
export declare function isToolCallDelta(delta: unknown): delta is ToolCallDelta;
export type ThinkingBlock = {
    type?: "thinking";
    thinking: null | string;
    signature: null | string;
};
interface ThinkingBlocksDelta extends BaseDelta {
    thinking_blocks?: ThinkingBlock[];
    reasoning_content?: string | null;
}
export declare function isThinkingBlocksDelta(delta: unknown): delta is ThinkingBlocksDelta;
type Delta = ThinkingBlocksDelta | AssistantDelta | ChatContextFileDelta | ToolCallDelta | BaseDelta;
export type ChatChoice = {
    delta: Delta;
    finish_reason?: "stop" | "length" | "abort" | "tool_calls" | "error" | null;
    index: number;
};
export type ChatUserMessageResponse = {
    id: string;
    role: "user" | "context_file" | "context_memory";
    content: string;
    checkpoints?: Checkpoint[];
    compression_strength?: CompressionStrength;
} | {
    id: string;
    role: "user";
    content: string | (UserMessageContentWithImage | ProcessedUserMessageContentWithImages)[];
    checkpoints?: Checkpoint[];
    compression_strength?: CompressionStrength;
};
export type ToolResponse = {
    id: string;
    role: "tool";
    tool_failed?: boolean;
} & ToolResult;
export declare function isChatUserMessageResponse(json: unknown): json is ChatUserMessageResponse;
export type UserMessageResponse = ChatUserMessageResponse & {
    role: "user";
};
export declare function isUserResponse(json: unknown): json is UserMessageResponse;
export type ContextFileResponse = ChatUserMessageResponse & {
    role: "context_file";
};
export declare function isContextFileResponse(json: unknown): json is ContextFileResponse;
export type SubchatContextFileResponse = {
    content: string;
    role: "context_file";
};
export declare function isSubchatContextFileResponse(json: unknown): json is SubchatContextFileResponse;
export type ContextMemoryResponse = ChatUserMessageResponse & {
    role: "context_memory";
};
export declare function isContextMemoryResponse(json: unknown): json is ContextMemoryResponse;
export declare function isToolResponse(json: unknown): json is ToolResponse;
export type DiffResponse = {
    role: "diff";
    content: string;
    tool_call_id: string;
};
export declare function isDiffResponse(json: unknown): json is DiffResponse;
export interface PlainTextResponse {
    role: "plain_text";
    content: string;
    tool_call_id: string;
    tool_calls?: ToolCall[];
}
export declare function isPlainTextResponse(json: unknown): json is PlainTextResponse;
export type SubchatResponse = {
    add_message: ChatResponse;
    subchat_id: string;
    tool_call_id: string;
};
export declare function isSubchatResponse(json: unknown): json is SubchatResponse;
export declare function isSystemResponse(json: unknown): json is SystemMessage;
export declare function isCDInstructionResponse(json: unknown): json is CDInstructionMessage;
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
export declare function isChatResponseChoice(res: ChatResponse): res is ChatResponseChoice;
export type CompressionStrength = "absent" | "low" | "medium" | "high";
export type ChatResponse = ChatResponseChoice | ChatUserMessageResponse | ToolResponse | PlainTextResponse;
export declare function areAllFieldsBoolean(json: unknown): json is Record<string, boolean>;
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
export declare function isMemoRecord(obj: unknown): obj is MemoRecord;
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
export declare function isVecDbStatus(obj: unknown): obj is VecDbStatus;
export declare function isMCPArgumentsArray(json: unknown): json is MCPArgs;
export declare function isMCPEnvironmentsDict(json: unknown): json is MCPEnvs;
export declare function isDictionary(json: unknown): json is Record<string, string>;
export type SuccessResponse = {
    success: true;
};
export declare function isSuccess(data: unknown): data is SuccessResponse;
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
    top_error_types: {
        type: string;
        count: number;
    }[];
    stuck_tasks: number;
};
export type BuddyPulseReport = {
    workflow_id: string;
    title: string;
    preview: string;
    chat_id: string;
};
export type BuddyPulseActivity = {
    grouped: {
        type: string;
        count: number;
        details?: string[];
    }[];
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
export declare function isBuddyPulsePayload(value: unknown): value is BuddyPulsePayload;
