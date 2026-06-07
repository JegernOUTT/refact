import { ToolConfirmationPauseReason, Usage } from "../../../services/refact";
import { SystemPrompts } from "../../../services/refact/prompts";
import type { CompressionPhase, CompressionReason } from "../../../services/refact/chatSubscription";
import { BackgroundAgentSummary, ChatMessages } from "../../../services/refact/types";
import type { WorktreeMeta } from "../../../services/refact/worktrees";
import { BuddyThreadMeta } from "../../Buddy/types";
export type ImageFile = {
    name: string;
    content: string | ArrayBuffer | null;
    type: string;
};
export type TextFile = {
    name: string;
    content: string;
};
export type ToolConfirmationStatus = {
    wasInteracted: boolean;
    confirmationStatus: boolean;
};
export type TodoStatus = "pending" | "in_progress" | "completed" | "failed";
export type TodoItem = {
    id: string;
    content: string;
    status: TodoStatus;
};
export type QueuedItem = {
    client_request_id: string;
    priority: boolean;
    command_type: string;
    preview: string;
    content?: string;
};
/** A single item returned by the wand-preview endpoint, shown as an editable chip. */
export type ManualPreviewItem = {
    /** "memory" | "trajectory" | "file" */
    kind: "memory" | "trajectory" | "file";
    /** Human-friendly display label for the chip */
    label: string;
    /** Full ContextFile to inject when the user sends */
    context_file: {
        file_name: string;
        file_content: string;
        line1: number;
        line2: number;
        usefulness: number;
        skip_pp?: boolean;
        gradient_type?: number;
    };
};
export type IntegrationMeta = {
    name?: string;
    path?: string;
    project?: string;
    shouldIntermediatePageShowUp?: boolean;
};
export type ReasoningEffort = "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max";
export declare function isReasoningEffort(v: unknown): v is ReasoningEffort;
export type ChatThread = {
    id: string;
    messages: ChatMessages;
    model: string;
    title?: string;
    createdAt?: string;
    updatedAt?: string;
    tool_use?: ToolUse;
    isTitleGenerated?: boolean;
    boost_reasoning?: boolean;
    /** Reasoning effort level: "low", "medium", "high", "xhigh", or "max". null = use backend default */
    reasoning_effort?: ReasoningEffort | null;
    /** Thinking budget in tokens (for Anthropic, Qwen, Gemini 2.5). null = use backend default */
    thinking_budget?: number | null;
    /** Temperature for sampling (0-2). null = use backend default */
    temperature?: number | null;
    /** Frequency penalty for sampling (-2 to 2). null = use backend default */
    frequency_penalty?: number | null;
    /** Maximum tokens for response. null = use backend default */
    max_tokens?: number | null;
    /** Whether to allow parallel tool calls. null = use backend default */
    parallel_tool_calls?: boolean | null;
    integration?: IntegrationMeta | null;
    mode?: ChatModeId;
    project_name?: string;
    last_user_message_id?: string;
    new_chat_suggested: SuggestedChat;
    auto_approve_editing_tools?: boolean;
    auto_approve_dangerous_commands?: boolean;
    modelMaximumContextTokens?: number;
    currentMaximumContextTokens?: number;
    currentMessageContextTokens?: number;
    increase_max_tokens?: boolean;
    include_project_info?: boolean;
    context_tokens_cap?: number;
    checkpoints_enabled?: boolean;
    /** If true, this chat belongs to a task workspace and should not appear in regular chat tabs */
    is_task_chat?: boolean;
    /** Task metadata for task-related chats */
    task_meta?: {
        task_id: string;
        role: string;
        agent_id?: string;
        card_id?: string;
        planner_chat_id?: string;
    };
    /** OpenAI Responses API multi-turn state: link next request to the previous response */
    previous_response_id?: string;
    /** Currently active skill name, set by activate_skill tool */
    active_skill?: string | null;
    auto_enrichment_enabled?: boolean;
    auto_compact_enabled?: boolean;
    worktree?: WorktreeMeta | null;
    parent_id?: string;
    link_type?: string;
    root_chat_id?: string;
    buddy_meta?: BuddyThreadMeta;
};
export type SuggestedChat = {
    wasSuggested: boolean;
    wasRejectedByUser?: boolean;
};
export type ToolUse = "quick" | "explore" | "agent";
export type ChatModeId = string;
export declare const DEFAULT_MODE: ChatModeId;
export declare function normalizeLegacyMode(mode: string | undefined): ChatModeId;
export type ThreadConfirmation = {
    pause: boolean;
    pause_reasons: ToolConfirmationPauseReason[];
    status: ToolConfirmationStatus;
};
export type ChatThreadRuntime = {
    thread: ChatThread;
    streaming: boolean;
    waiting_for_response: boolean;
    prevent_send: boolean;
    error: string | null;
    queued_items: QueuedItem[];
    send_immediately: boolean;
    attached_images: ImageFile[];
    attached_text_files: TextFile[];
    background_agents: Record<string, BackgroundAgentSummary>;
    confirmation: ThreadConfirmation;
    /** Whether the initial snapshot has been received from the backend */
    snapshot_received: boolean;
    /** Whether the engine is running segment summarization */
    is_compressing?: boolean;
    /** Latest compression attempt phase from the engine */
    compression_phase?: CompressionPhase;
    /** Latest structured compression skip/failure reason from the engine */
    compression_reason?: CompressionReason;
    compression_pulse_seq?: string;
    /** Task progress widget expanded/collapsed state */
    task_widget_expanded: boolean;
    /** Actual session state from backend (for waiting_user_input, completed, etc.) */
    session_state?: string;
    /** Last applied chat SSE event seq for duplicate/out-of-order protection */
    last_applied_seq?: string;
    /** Fast lookup index from message_id to message index (rebuilt on snapshots/mutations) */
    message_index_by_id?: Record<string, number>;
    memory_enrichment_user_touched: boolean;
    manual_preview_items: ManualPreviewItem[];
    manual_preview_ran: boolean;
};
export type Chat = {
    current_thread_id: string;
    open_thread_ids: string[];
    threads: Record<string, ChatThreadRuntime | undefined>;
    system_prompt: SystemPrompts;
    tool_use: ToolUse;
    checkpoints_enabled?: boolean;
    follow_ups_enabled?: boolean;
    max_new_tokens?: number;
    /** When set, useChatSubscription should reconnect to get fresh state */
    sse_refresh_requested: string | null;
    /** Increments on every stream_delta to force component re-renders */
    stream_version: number;
};
export type PayloadWithId = {
    id: string;
};
export type PayloadWithChatAndNumber = {
    chatId: string;
    value: number;
};
export type PayloadWithChatAndMessageId = {
    chatId: string;
    messageId: string;
};
export type PayloadWithChatAndBoolean = {
    chatId: string;
    value: boolean;
};
export type PayloadWithChatAndUsage = {
    chatId: string;
    usage: Usage;
};
export type PayloadWithChatAndCurrentUsage = {
    chatId: string;
    n_ctx: number;
    prompt_tokens: number;
};
export type PayloadWithIdAndTitle = {
    title: string;
    isTitleGenerated: boolean;
} & PayloadWithId;
export type DetailMessage = {
    detail: string;
};
export type StreamingErrorChunk = {
    error: {
        message: string;
        type: string;
        code?: string;
    };
};
export declare function checkForDetailMessage(str: string): DetailMessage | false;
export declare function isToolUse(str: string): str is ToolUse;
export type LspChatMode = string;
export declare function isServerExecutedTool(toolCallId: string | undefined): boolean;
