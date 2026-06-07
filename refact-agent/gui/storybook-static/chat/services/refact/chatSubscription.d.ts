import { type EngineApiConfig } from "./apiUrl";
import type { BackgroundAgentSummary, ChatMessage } from "./types";
import type { WorktreeMeta } from "./worktrees";
export type SessionState = "idle" | "generating" | "executing_tools" | "paused" | "waiting_ide" | "waiting_user_input" | "completed" | "error";
export type CompressionPhase = "checking" | "running" | "applied" | "skipped" | "failed";
export type CompressionReason = "auto_compact_disabled" | "session_compaction_disabled" | "max_attempts_reached" | "pending_tool_calls" | "no_eligible_segment" | "effective_context_unknown" | "provider_length_stop" | "context_length_stop" | "pressure_low" | "insufficient_savings" | "no_summary_model" | "input_too_large" | "transient_failure" | "source_changed";
export type ThreadParams = {
    id: string;
    title: string;
    model: string;
    mode: string;
    tool_use: string;
    boost_reasoning: boolean;
    context_tokens_cap: number | null;
    include_project_info: boolean;
    checkpoints_enabled: boolean;
    is_title_generated: boolean;
    use_compression?: boolean;
    auto_approve_editing_tools?: boolean;
    auto_approve_dangerous_commands?: boolean;
    reasoning_effort?: string | null;
    thinking_budget?: number | null;
    temperature?: number | null;
    frequency_penalty?: number | null;
    max_tokens?: number | null;
    parallel_tool_calls?: boolean | null;
    task_meta?: {
        task_id: string;
        role: string;
        agent_id?: string;
        card_id?: string;
    };
    previous_response_id?: string;
    auto_enrichment_enabled?: boolean | null;
    auto_compact_enabled?: boolean | null;
    reactive_compact_attempts?: number | null;
    worktree?: WorktreeMeta | null;
    parent_id?: string | null;
    link_type?: string | null;
    root_chat_id?: string | null;
};
export type PauseReason = {
    type: string;
    tool_name: string;
    command: string;
    rule: string;
    tool_call_id: string;
    integr_config_path: string | null;
};
export type QueuedItem = {
    client_request_id: string;
    priority: boolean;
    command_type: string;
    preview: string;
};
export type RuntimeState = {
    state: SessionState;
    paused: boolean;
    error: string | null;
    queue_size: number;
    pause_reasons: PauseReason[];
    queued_items: QueuedItem[];
    is_compressing?: boolean;
    compression_phase?: CompressionPhase | null;
    compression_reason?: CompressionReason | null;
};
export type DeltaOp = {
    op: "append_content";
    text: string;
} | {
    op: "append_reasoning";
    text: string;
} | {
    op: "set_tool_calls";
    tool_calls: unknown[];
} | {
    op: "set_thinking_blocks";
    blocks: unknown[];
} | {
    op: "add_citation";
    citation: unknown;
} | {
    op: "add_server_content_block";
    block: unknown;
} | {
    op: "set_usage";
    usage: unknown;
} | {
    op: "merge_extra";
    extra: Record<string, unknown>;
};
export type EventEnvelope = {
    chat_id: string;
    seq: string;
    type: "snapshot";
    thread: ThreadParams;
    runtime: RuntimeState;
    messages: ChatMessage[];
    background_agents: BackgroundAgentSummary[];
} | {
    chat_id: string;
    seq: string;
    type: "background_agent_updated";
    agent: BackgroundAgentSummary;
} | {
    chat_id: string;
    seq: string;
    type: "thread_updated";
    worktree?: WorktreeMeta | null;
    [key: string]: unknown;
} | {
    chat_id: string;
    seq: string;
    type: "message_added";
    message: ChatMessage;
    index: number;
} | {
    chat_id: string;
    seq: string;
    type: "process_completed";
    process_id: string;
    status: string;
    exit_code: number | null;
    short_description: string;
    mode: string;
} | {
    chat_id: string;
    seq: string;
    type: "message_updated";
    message_id: string;
    message: ChatMessage;
} | {
    chat_id: string;
    seq: string;
    type: "message_removed";
    message_id: string;
} | {
    chat_id: string;
    seq: string;
    type: "messages_truncated";
    from_index: number;
} | {
    chat_id: string;
    seq: string;
    type: "stream_started";
    message_id: string;
} | {
    chat_id: string;
    seq: string;
    type: "stream_delta";
    message_id: string;
    ops: DeltaOp[];
} | {
    chat_id: string;
    seq: string;
    type: "stream_finished";
    message_id: string;
    finish_reason: string | null;
} | {
    chat_id: string;
    seq: string;
    type: "pause_required";
    reasons: PauseReason[];
} | {
    chat_id: string;
    seq: string;
    type: "pause_cleared";
} | {
    chat_id: string;
    seq: string;
    type: "ide_tool_required";
    tool_call_id: string;
    tool_name: string;
    args: unknown;
} | {
    chat_id: string;
    seq: string;
    type: "subchat_update";
    tool_call_id: string;
    subchat_id: string;
    attached_files?: string[];
} | {
    chat_id: string;
    seq: string;
    type: "ack";
    client_request_id: string;
    accepted: boolean;
    result: unknown;
} | {
    chat_id: string;
    seq: string;
    type: "queue_updated";
    queue_size: number;
    queued_items: QueuedItem[];
} | {
    chat_id: string;
    seq: string;
    type: "runtime_updated";
    state: string;
    error?: string;
    is_compressing?: boolean;
    compression_phase?: CompressionPhase | null;
    compression_reason?: CompressionReason | null;
} | {
    chat_id: string;
    seq: string;
    type: "browser_context_oversize";
    total_bytes: number;
    action_count: number;
    action_bytes: number;
    console_count: number;
    console_bytes: number;
    network_count: number;
    network_bytes: number;
    mutation_bytes: number;
    pending_message_id: string;
} | {
    chat_id: string;
    seq: string;
    type: "browser_frame";
    tab_id: string;
    mime: string;
    data: string;
    diff_boxes?: {
        x: number;
        y: number;
        width: number;
        height: number;
    }[];
    changed_text?: string;
} | {
    chat_id: string;
    seq: string;
    type: "browser_status";
    runtime_id: string;
    connected: boolean;
    active_tab?: string | null;
    url?: string | null;
    title?: string | null;
    tabs?: {
        tab_id: string;
        url: string;
        title: string;
    }[];
} | {
    chat_id: string;
    seq: string;
    type: "browser_closed";
    runtime_id: string;
    reason: string;
} | {
    chat_id: string;
    seq: string;
    type: "browser_timeline";
    events: {
        timestamp: string;
        source: string;
        type: string;
        summary: string;
        details?: Record<string, unknown>;
    }[];
} | {
    chat_id: string;
    seq: string;
    type: "browser_toolbar_action";
    action: string;
};
export type ChatEventEnvelope = EventEnvelope;
export type ChatEventType = EventEnvelope["type"];
export type ChatSubscriptionCallbacks = {
    onEvent: (event: EventEnvelope) => void;
    onError: (error: Error) => void;
    onConnected?: () => void;
    onDisconnected?: () => void;
    onActivity?: () => void;
};
export type SubscriptionOptions = {
    connectTimeoutMs?: number;
    idleTimeoutMs?: number;
};
export declare function subscribeToChatEvents(chatId: string, config: EngineApiConfig, callbacks: ChatSubscriptionCallbacks, apiKey?: string, options?: SubscriptionOptions): () => void;
export declare function applyDeltaOps(message: ChatMessage, ops: DeltaOp[]): ChatMessage;
